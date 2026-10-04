// Lighting shared by the scene shaders (scene.wgsl, static_scene.wgsl): camera/light blocks + group 0 bindings, cascade shadows,
// sky ambient, tangent-frame normal mapping, GGX and the fog/HDR tail. Prepended after common.wgsl by the Rust side.

struct Camera {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    view_inverse: mat4x4<f32>,
    position: vec4<f32>,   // xyz eye, w time
    viewport: vec4<f32>,   // w, h, 1/w, 1/h
    proj: vec4<f32>,       // tan half fov x, tan half fov y, near, far
}
struct Light {
    sun_dir: vec4<f32>,        // unit vector towards the sun (world)
    sun_radiance: vec4<f32>,
    sky_radiance: vec4<f32>,
    shadow_matrix: array<mat4x4<f32>, 3>,   // world -> shadow uv (xy 0..1, y down) + depth 0..1
    cascade_far: vec4<f32>,    // xyz: view depth (m) where each cascade ends, w: shadow distance
    cascade_texel: vec4<f32>,  // xyz: world metres per shadow texel
    shadow_params: vec4<f32>,  // x enabled, y penumbra (m), z normal offset (texels), w constant bias (texels)
    misc: vec4<f32>,           // x lit enabled, y material_debug bits, z shadow map size, w fog enabled
}
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> light: Light;
@group(0) @binding(2) var shadow_map: texture_depth_2d_array;
@group(0) @binding(3) var shadow_samp: sampler_comparison;
@group(0) @binding(4) var tex_samp: sampler;

// ---- screen-space derivatives with OpenGL orientation (window y up) so the ported tangent-frame maths keeps its sign ----
fn gl_dfdy3(v: vec3<f32>) -> vec3<f32> { return -dpdy(v); }
fn gl_dfdy2(v: vec2<f32>) -> vec2<f32> { return -dpdy(v); }
fn gl_dfdy1(v: f32) -> f32 { return -dpdy(v); }

fn dbg(bit: u32) -> bool { return ((u32(light.misc.y) / bit) % 2u) >= 1u; }

// ---- cascaded shadow maps: 3 cascades, hardware-compare bilinear PCF over a 5x5 tap kernel ----
fn pcf(layer: i32, c: vec3<f32>, radius_texels: f32) -> f32 {
    let map_size = light.misc.z;
    let step_uv = 0.5 * radius_texels / map_size;
    var sum = 0.0;
    for (var j = -2; j <= 2; j = j + 1) {
        for (var i = -2; i <= 2; i = i + 1) {
            let uv = c.xy + vec2<f32>(f32(i), f32(j)) * step_uv;
            var s = 1.0;   // outside the map = lit (GL used a white border)
            if (uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0 && c.z <= 1.0) {
                s = textureSampleCompareLevel(shadow_map, shadow_samp, uv, layer, c.z);
            }
            sum = sum + s;
        }
    }
    return sum / 25.0;
}
fn cascade_shadow(layer: i32, texel: f32, world_pos: vec3<f32>, n: vec3<f32>, ndl: f32) -> f32 {
    let radius = clamp(light.shadow_params.y / texel, 1.5, 6.0);
    let tan_theta = min(sqrt(max(1.0 - ndl * ndl, 0.0)) / max(ndl, 0.05), 3.0);
    let p = world_pos + n * texel * light.shadow_params.z * (1.0 - ndl * 0.5)
        + light.sun_dir.xyz * texel * (light.shadow_params.w + radius * min(tan_theta, 2.5));
    let c = light.shadow_matrix[layer] * vec4<f32>(p, 1.0);
    return pcf(layer, c.xyz, radius);
}
fn sun_shadow(world_pos: vec3<f32>, n: vec3<f32>, ndl: f32) -> f32 {
    let depth = -(camera.view * vec4<f32>(world_pos, 1.0)).z;
    if (light.shadow_params.x < 0.5 || depth >= light.cascade_far.w) { return 1.0; }
    var s: f32;
    if (depth < light.cascade_far.x) { s = cascade_shadow(0, light.cascade_texel.x, world_pos, n, ndl); }
    else if (depth < light.cascade_far.y) { s = cascade_shadow(1, light.cascade_texel.y, world_pos, n, ndl); }
    else { s = cascade_shadow(2, light.cascade_texel.z, world_pos, n, ndl); }
    return mix(1.0, s, 1.0 - smoothstep(0.9 * light.cascade_far.w, light.cascade_far.w, depth));
}
fn sun_visibility(world_pos: vec3<f32>, n: vec3<f32>, ndl: f32) -> f32 {
    if (ndl > 0.0) { return sun_shadow(world_pos, n, ndl); }
    return 1.0;
}

fn ambient_light(n: vec3<f32>) -> vec3<f32> { return light.sky_radiance.xyz * clamp(0.5 + 0.5 * n.y, 0.0, 1.0); }

// Tangent frame from screen-space derivatives (the exported meshes carry no usable tangents), in world space.
fn perturb_normal(n: vec3<f32>, p: vec3<f32>, tex: vec2<f32>, tn: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(p);
    let dp2 = gl_dfdy3(p);
    let duv1 = dpdx(tex);
    let duv2 = gl_dfdy2(tex);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let invmax = inverseSqrt(max(max(dot(t, t), dot(b, b)), 1e-20));
    return normalize(mat3x3<f32>(t * invmax, b * invmax, n) * tn);
}
fn unpack_normal(texel: vec4<f32>) -> vec3<f32> {
    let xy = texel.rg * 2.0 - 1.0;   // UE tangent space (DirectX, +G toward +V); z reconstructed
    return vec3<f32>(xy, sqrt(max(1.0 - dot(xy, xy), 0.0)));
}

// GGX specular for the sun only (UE4 default lit: D_GGX * Vis_SmithJointApprox * F_Schlick).
fn ggx_spec(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, roughness: f32, f0: f32) -> f32 {
    let h = normalize(v + l);
    let nl = clamp(dot(n, l), 0.0, 1.0);
    let nv = max(dot(n, v), 1e-3);
    let nh = clamp(dot(n, h), 0.0, 1.0);
    let vh = clamp(dot(v, h), 0.0, 1.0);
    let a = roughness * roughness;
    let a2 = a * a;
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (3.14159265 * dd * dd);
    let vis = 0.5 / (nl * (nv * (1.0 - a) + a) + nv * (nl * (1.0 - a) + a) + 1e-4);
    let f = f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
    return d * vis * f * nl;
}

// Shared tail: height fog, then linear HDR out. Exposure, bloom, AO, vignette, white balance, film curve and the sRGB encode happen in post.wgsl.
// HDR target alpha = share of this pixel's final luminance that is ambient light, after fog attenuation.
fn output_hdr(lit_in: vec3<f32>, amb: vec3<f32>, world_pos: vec3<f32>, cam: vec3<f32>) -> vec4<f32> {
    var lit = lit_in;
    var fog = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (light.misc.w > 0.5) { fog = height_fog(world_pos, cam, light.sun_dir.xyz, -camera.view_inverse[2].xyz); }
    if (dbg(8u)) { lit = vec3<f32>((1.0 - fog.a) / (1.0 - FOG_MAX_OPACITY + 1e-4) * 0.8); }
    else if (!dbg(4u)) { lit = lit * fog.a + fog.rgb; }
    let amb_luma = luma709(amb) * select(fog.a, 1.0, dbg(8u) || dbg(4u));
    let lit_pos = max(lit, vec3<f32>(0.0));
    return vec4<f32>(lit_pos, clamp(amb_luma / max(luma709(lit_pos), 1e-4), 0.0, 1.0));
}
