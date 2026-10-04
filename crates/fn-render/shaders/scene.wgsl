// Lit scene pass: baseplate ground (terrain colour variation material), skinned character parts and the pickaxe prop.
// Port of color.vert/color.frag/materials.glsl. Everything is linear HDR; exposure/tonemap live in post.wgsl.
// Common helpers (srgb, fog) are prepended from common.wgsl.
//
// RECOVERED DATA: sun colour/brightness (day TODM), SkyLightColor tint, character parameters from M_Med_Soldier_04 /
//   CharacterShader_Gloss props (RimLightColor, SpecRoughnessMin/Max, Skin Boost, spec multipliers), terrain HighLight/EdgeDark.
// NOT RECOVERED: the node wiring of CharacterShader and of the Athena grass material; channel meanings are inferred.

// Shared lighting (camera/light blocks, shadows, ambient, normal mapping, GGX, output_hdr) lives in lit_common.wgsl.

struct Material { kind: vec4<u32> }   // x: 0 generic untextured, 1 ground, 2 character, 3 prop (pickaxe)
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var t_diffuse: texture_2d<f32>;
@group(1) @binding(2) var t_normal: texture_2d<f32>;
@group(1) @binding(3) var t_spec: texture_2d<f32>;
@group(1) @binding(4) var t_mask: texture_2d<f32>;

struct Object { model: mat4x4<f32> }
@group(2) @binding(0) var<uniform> object: Object;
@group(2) @binding(1) var<storage, read> bones: array<mat4x4<f32>>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec3<f32>,   // vertex colour (props); white elsewhere
}

@vertex
fn vs_ground(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>) -> VsOut {
    var o: VsOut;
    let wp = object.model * vec4<f32>(pos, 1.0);
    o.clip = camera.view_proj * wp;
    o.world_pos = wp.xyz;
    o.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    o.uv = uv;
    o.color = vec3<f32>(1.0);
    return o;
}

// Procedural props (trees, rocks): world-space triangles with flat normals and vertex colours, drawn untextured.
@vertex
fn vs_prop(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>) -> VsOut {
    var o: VsOut;
    o.clip = camera.view_proj * vec4<f32>(pos, 1.0);
    o.world_pos = pos;
    o.world_normal = normal;
    o.uv = vec2<f32>(0.0);
    o.color = color;
    return o;
}

@vertex
fn vs_skinned(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
) -> VsOut {
    var o: VsOut;
    let m = bones[joints.x] * weights.x + bones[joints.y] * weights.y + bones[joints.z] * weights.z + bones[joints.w] * weights.w;
    let p_local = (m * vec4<f32>(pos, 1.0)).xyz;
    var n_local = (m * vec4<f32>(normal, 0.0)).xyz;
    n_local = n_local / max(length(n_local), 1e-6);
    let wp = object.model * vec4<f32>(p_local, 1.0);
    o.clip = camera.view_proj * wp;
    o.world_pos = wp.xyz;
    o.world_normal = (object.model * vec4<f32>(n_local, 0.0)).xyz;
    o.uv = uv;
    o.color = vec3<f32>(1.0);
    return o;
}

// Ambient (sky-light) part of the final lit colour, written by every shading path so the ambient share can be stored in the
// HDR target's alpha; post.wgsl then applies ambient occlusion to that share only (UE applies AO to ambient light, not the sun).
var<private> amb_out: vec3<f32>;
// ---- value noise for macro variation of the grass ----
fn hash21(p_in: vec2<f32>) -> f32 {
    var p = fract(p_in * vec2<f32>(123.34, 456.21));
    p = p + dot(p, p + 45.32);
    return fract(p.x * p.y);
}
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), f.x), mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), f.x), f.y);
}

// Terrain colour variation. RECOVERED: MF_TerrainColorVariation HighLight / EdgeDark, GrassColorMaskContrast -2.
// INFERRED blend lerp(EdgeDark, HighLight, CheapContrast(mask, -2)).  This temporary flat baseplate has no
// Athena landscape UVs, so it intentionally does NOT apply a made-up global mean of T_Athena_Terrain_Mask.
// The recovered variation constants remain here for the map-backed terrain path; `terrain_color_multiplier`
// returns neutral for the standalone baseplate.
const TERRAIN_HIGHLIGHT: vec3<f32> = vec3<f32>(3.0, 2.337033, 1.38);
const TERRAIN_EDGE_DARK: vec3<f32> = vec3<f32>(0.9, 0.9, 0.9);
const TERRAIN_MASK_CONTRAST: f32 = -2.0;
const TERRAIN_MASK_MEAN: f32 = 0.2086;
fn cheap_contrast(x: f32, c: f32) -> f32 { let s = x * x * (3.0 - 2.0 * x); return x + (s - x) * c; }
fn terrain_color_multiplier() -> vec3<f32> {
    return vec3<f32>(1.0, 1.0, 1.0);
}

// Baseplate grass: raw T_Grasslands_D (terrain variation waits for real terrain UVs), a second larger-scale tile to hide repetition (APPROXIMATION),
// luminance-only macro variation and a bump from albedo luminance (APPROXIMATION), rough dielectric response.
fn shade_ground(uv: vec2<f32>, world_pos: vec3<f32>, cam_pos: vec3<f32>) -> vec3<f32> {
    let mult = terrain_color_multiplier();
    let base = to_linear(textureSample(t_diffuse, tex_samp, uv).rgb) * mult;
    let macro_tile = to_linear(textureSample(t_diffuse, tex_samp, uv * 0.137 + vec2<f32>(0.31, 0.77)).rgb) * mult;
    let lum = luma709(base);
    let macro_lum = max(luma709(macro_tile), 1e-3);
    var albedo = base * mix(1.0, clamp(macro_lum / max(lum, 1e-3), 0.6, 1.6), select(0.35, 0.0, dbg(1u)));
    let variation = value_noise(world_pos.xz * 0.045) * 0.6 + value_noise(world_pos.xz * 0.31) * 0.4;
    if (!dbg(1u)) { albedo = albedo * mix(0.92, 1.08, variation); }
    var n = vec3<f32>(0.0, 1.0, 0.0);
    let dp1 = dpdx(world_pos);
    let dp2 = gl_dfdy3(world_pos);
    let dh1 = dpdx(lum);
    let dh2 = gl_dfdy1(lum);
    let r1 = cross(dp2, n);
    let r2 = cross(n, dp1);
    let det = dot(dp1, r1);
    let grad = (r1 * dh1 + r2 * dh2) / select(1e-12, det, abs(det) > 1e-12);
    if (!dbg(1u)) { n = normalize(n - clamp(grad * 0.04, vec3<f32>(-0.3), vec3<f32>(0.3))); }
    let sun_dir = light.sun_dir.xyz;
    let ndl = clamp(dot(n, sun_dir), 0.0, 1.0);
    let vis = sun_visibility(world_pos, n, ndl);
    let nl = ndl * vis;
    let v = normalize(cam_pos - world_pos);
    amb_out = albedo * ambient_light(n);
    let diffuse = albedo * light.sun_radiance.xyz * nl + amb_out;
    // sun_radiance already carries the Lambert 1/pi; the GGX BRDF is not divided by pi, so its light is Intensity = x pi.
    let spec = light.sun_radiance.xyz * 3.14159265 * ggx_spec(n, v, sun_dir, 0.9, 0.04) * vis;
    return diffuse + spec;
}

// Character body/head/props. Texture slots: Diffuse C, Normals N, SpecularMasks S, M. Channel use is INFERRED:
// S.r baked AO/cavity, S.g sparse specular mask, S.b roughness-like; M.b skin mask, M.g soft rim/occlusion mask.
fn shade_character(uv: vec2<f32>, world_normal: vec3<f32>, world_pos: vec3<f32>, cam_pos: vec3<f32>, is_prop: bool) -> vec3<f32> {
    let albedo = to_linear(textureSample(t_diffuse, tex_samp, uv).rgb);
    var tn = unpack_normal(textureSample(t_normal, tex_samp, uv));
    if (dbg(2u)) { tn = vec3<f32>(0.0, 0.0, 1.0); }
    let n = perturb_normal(normalize(world_normal), world_pos, uv, tn);
    let v = normalize(cam_pos - world_pos);
    let prop = is_prop || dbg(2u);
    var sm = textureSample(t_spec, tex_samp, uv).rgb;
    var mm = textureSample(t_mask, tex_samp, uv).rgb;
    if (prop) { sm = vec3<f32>(1.0, 0.25, 0.5); mm = vec3<f32>(1.0, 0.0, 0.0); }
    let ao = sm.r;
    let roughness = mix(0.2, 0.8, sm.b);               // SpecRoughnessMin/Max (instance values)
    let skin = select(mm.b, 0.0, prop);
    let spec_mult = mix(0.5, 0.5, skin);                // Cloth Spec Multiplier / Skin Spec Multiply (both 0.5)
    let sun_dir = light.sun_dir.xyz;
    let ndl = clamp(dot(n, sun_dir), 0.0, 1.0);
    let vis = sun_visibility(world_pos, n, ndl);
    let nl = ndl * vis;
    let ambient = ambient_light(n) * ao;
    amb_out = albedo * ambient;
    var lit = albedo * (light.sun_radiance.xyz * nl + ambient);
    lit = lit + light.sun_radiance.xyz * ggx_spec(n, v, sun_dir, roughness, 0.04) * (0.25 + 4.0 * sm.g) * spec_mult * vis;
    let nv = clamp(dot(n, v), 0.0, 1.0);
    // Skin Boost Color And Exponent (0.525,0.343875,0.367316, exp 2): fresnel-weighted subsurface-like lift.
    lit = lit + vec3<f32>(0.525, 0.343875, 0.367316) * albedo * ambient_light(vec3<f32>(0.0, 1.0, 0.0)) * pow(1.0 - nv, 2.0) * skin * 0.6;
    // RimLightColor (1.22958,1.48567,1.5): exponent and intensity are guesses; UpVectorPower=2 masks to up-facing normals.
    let up_mask = clamp(n.y * 0.5 + 0.5, 0.0, 1.0);
    lit = lit + vec3<f32>(1.22958, 1.48567, 1.5) * pow(1.0 - nv, 3.0) * (up_mask * up_mask) * select(mm.g, 0.0, prop) * 0.05;
    return lit;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let cam = camera.position.xyz;
    let kind = material.kind.x;
    let lit_enabled = light.misc.x > 0.5;
    var lit: vec3<f32>;
    amb_out = vec3<f32>(0.0);
    if (lit_enabled && kind == 1u) {
        lit = shade_ground(in.uv, in.world_pos, cam);
    } else if (lit_enabled && kind >= 2u) {
        lit = shade_character(in.uv, in.world_normal, in.world_pos, cam, kind == 3u);
    } else {
        // Untextured generic surface (white albedo) lit by sun + hemisphere sky light.
        let albedo = in.color;
        var irradiance = vec3<f32>(1.0);
        if (lit_enabled) {
            let n = normalize(in.world_normal);
            let ndl = max(dot(n, light.sun_dir.xyz), 0.0);
            let visibility = sun_visibility(in.world_pos, n, ndl);
            let hemisphere = clamp(0.5 + 0.5 * n.y, 0.0, 1.0);
            irradiance = light.sun_radiance.xyz * ndl * visibility + light.sky_radiance.xyz * hemisphere;
            amb_out = albedo * light.sky_radiance.xyz * hemisphere;
        }
        lit = albedo * irradiance;
    }
    return output_hdr(lit, amb_out, in.world_pos, cam);
}

// Static procedural props (trees, rocks; vs_prop): port of the old runtime's untextured generic path of color.frag with the
// vertex colour as linear albedo (the `shade` varying):
//   albedo * (sun_radiance * max(n.l, 0) * cascade_visibility + sky_radiance * clamp(0.5 + 0.5 n.y, 0, 1)), then fog.
// The sky light has a black lower hemisphere (UE default), so only exactly down-facing faces get no ambient; the flanks of the rock and
// of the canopy cones keep the blue-tinted sky ambient. No normal map, no specular, no skinning. The ambient share goes to alpha like the
// other paths so post.wgsl applies AO to it only.
@fragment
fn fs_prop(in: VsOut) -> @location(0) vec4<f32> {
    let cam = camera.position.xyz;
    let albedo = in.color;
    var lit = albedo;
    var amb = vec3<f32>(0.0);
    if (light.misc.x > 0.5) {
        let n = normalize(in.world_normal);
        let ndl = max(dot(n, light.sun_dir.xyz), 0.0);
        let visibility = sun_visibility(in.world_pos, n, ndl);
        let hemisphere = clamp(0.5 + 0.5 * n.y, 0.0, 1.0);
        amb = albedo * light.sky_radiance.xyz * hemisphere;
        lit = albedo * light.sun_radiance.xyz * ndl * visibility + amb;
    }
    return output_hdr(lit, amb, in.world_pos, cam);
}
