// VOLUMETRIC FOG (UE 4.16 `FVolumetricFog*`), ported from the disassembled 1.8 global shaders (see src/fog.rs / src/volfog.rs):
//   material_setup  = FVolumetricFogMaterialSetupCS            (4x4x4)   VBufferA = (scattering rgb, absorption)
//   scatter         = TVolumetricFogLightScatteringCS<history>  (4x4x4)   directional light + sky light, temporal history
//   integrate       = FVolumetricFogFinalIntegrationCS          (8x8x1)   front-to-back integration along every view ray
//   vs_composite/fs_composite = the volume lookup of the exponential height fog PS (CombineVolumetricFog): dst * V.a + V.rgb (fixed-function blend)
// Units: depths / step lengths of the fog maths are Unreal cm; positions are renderer metres (GL, Y up = Unreal Z).
// Every pass declares only the bindings it uses (separate bind group layouts, one shared binding-number table).

struct Camera {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    view_inverse: mat4x4<f32>,
    position: vec4<f32>,   // xyz eye, w time
    viewport: vec4<f32>,   // w, h, 1/w, 1/h
    proj: vec4<f32>,       // tan half fov x, tan half fov y, near, far
}
// must mirror `struct Light` of lit_common.wgsl (the renderer's light block)
struct Light {
    sun_dir: vec4<f32>,
    sun_radiance: vec4<f32>,
    sky_radiance: vec4<f32>,
    shadow_matrix: array<mat4x4<f32>, 3>,
    cascade_far: vec4<f32>,
    cascade_texel: vec4<f32>,
    shadow_params: vec4<f32>,  // x enabled, y cascade count, z r.ShadowQuality
    misc: vec4<f32>,
    cascade_ext: vec4<f32>,
    cascade_bias: vec4<f32>,
    light_fn: vec4<f32>,
}
// mirrors `VFogUniform` in volfog.rs
struct VFog {
    grid: vec4<f32>,          // gx, gy, gz, history valid (1 = reproject the previous frame)
    z_params: vec4<f32>,      // B, O, S (View.VolumetricFogGridZParams), w unused
    jitter: vec4<f32>,        // xyz cell offset added to the integer grid coordinate (0.5 = centre), w history weight
    fog: vec4<f32>,           // x density (after /1000), y falloff (after /1000), z fog height (cm), w scattering distribution g
    scale: vec4<f32>,         // x scattering scale, y absorption scale, z directional light enabled, w unused
    dir_color: vec4<f32>,     // rgb light colour with the bOverrideLightColorsWithFogInscatteringColors rule applied, w VolumetricScatteringIntensity
    sky_color: vec4<f32>,     // rgb sky light colour (override: FogInscatteringColor * 0.282095), w SkyLightVolumetricScatteringIntensity
    prev_view_proj: mat4x4<f32>,
    lf_plane: vec4<f32>,      // xyz: a world point (m) on the light-function plane (VolumetricFogLightFunction.cpp:156 sphere centre)
}

@group(0) @binding(0) var<uniform> vf: VFog;
@group(0) @binding(1) var<uniform> camera: Camera;
@group(0) @binding(2) var<uniform> light: Light;
@group(0) @binding(3) var shadow_map: texture_depth_2d_array;
@group(0) @binding(4) var shadow_samp: sampler_comparison;
@group(0) @binding(5) var lin_samp: sampler;
@group(0) @binding(6) var vbuf_a_in: texture_3d<f32>;
@group(0) @binding(7) var history_in: texture_3d<f32>;
@group(0) @binding(8) var light_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(9) var vbuf_a_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(10) var integrated_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(11) var light_in: texture_3d<f32>;
@group(0) @binding(12) var integrated_in: texture_3d<f32>;
@group(0) @binding(13) var scene_depth: texture_depth_2d;

const LN2_RCP: f32 = 1.442695;   // the literal of the disassembly: exp(x) is emitted as exp2(x * 1.442695)

// depth = (2^(slice / S) - O) / B  (cm)
fn slice_depth_cm(slice: f32) -> f32 { return (exp2(slice / vf.z_params.z) - vf.z_params.y) / vf.z_params.x; }
// volume W (0..1 over the grid) of a view depth (cm): log2(depth * B + O) * S / GridSizeZ
fn depth_to_w(depth_cm: f32) -> f32 { return log2(max(depth_cm * vf.z_params.x + vf.z_params.y, 1e-30)) * vf.z_params.z / vf.grid.z; }
// world position (metres) of the point at view depth `depth_cm` under the volume uv (0..1, y down)
fn cell_world(uv: vec2<f32>, depth_cm: f32) -> vec3<f32> {
    let ndc = (uv * 2.0 - 1.0) * vec2<f32>(1.0, -1.0);
    let d = depth_cm * 0.01;
    return (camera.view_inverse * vec4<f32>(ndc.x * camera.proj.x * d, ndc.y * camera.proj.y * d, -d, 1.0)).xyz;
}
fn ue_luma(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.3, 0.59, 0.11)); }

// ---- FVolumetricFogMaterialSetupCS ----------------------------------------------------------------------------------------------------
@compute @workgroup_size(4, 4, 4)
fn material_setup(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id >= vec3<u32>(u32(vf.grid.x), u32(vf.grid.y), u32(vf.grid.z)))) { return; }
    let cell = vec3<f32>(id) + 0.5;
    let depth_cm = slice_depth_cm(cell.z);
    let world = cell_world(cell.xy / vf.grid.xy, depth_cm);
    // density = Density * 0.5 * 2^(-(z - FogHeight) * Falloff); scattering = density * ScatteringScale (rgb), absorption = density * AbsorptionScale (a)
    let d = exp2((world.y * 100.0 - vf.fog.z) * -vf.fog.y) * (vf.fog.x * 0.5);
    textureStore(vbuf_a_out, vec3<i32>(id), vec4<f32>(vec3<f32>(d * vf.scale.x), d * vf.scale.y));
}

// ---- TVolumetricFogLightScatteringCS (directional light + sky light, temporal history) --------------------------------------------------
fn hg_phase(g: f32, cos_theta: f32) -> f32 {
    return (1.0 - g * g) / (12.566371 * pow(1.0 + g * g + 2.0 * g * cos_theta, 1.5));
}
// VolumetricFog.usf (LightScatteringCS): the cascade is chosen with CascadeEndDepths = SplitFar (the EXTENDED far of each cascade), the shadow map is POINT
// sampled and compared without the receiver bias against `ShadowDepth - DirectionalLightDepthBias`, where UE uploads the bias of cascade 0 for all cascades
// (LightGridInjection.cpp:495); the voxel is lit outside the cascade's viewport rectangle. View depth in metres.
fn dir_visibility(world: vec3<f32>, depth_m: f32) -> f32 {
    if (light.shadow_params.x < 0.5) { return 1.0; }
    // CascadeEndDepths[i] = SplitFar of each allocated CSM cascade (LightGridInjection.cpp:492): the nominal end + SplitFarFadeRegion, which the last
    // cascade also gets when the ray-traced DF cascade exists (cascade_ext of the last cascade is 0 otherwise); shadow_params.y = cascade count
    let n = i32(light.shadow_params.y);
    var layer = n;
    for (var i = n - 1; i >= 0; i = i - 1) { if (depth_m < light.cascade_far[i] + light.cascade_ext[i]) { layer = i; } }
    if (layer >= n) { return 1.0; }
    let c = light.shadow_matrix[layer] * vec4<f32>(world, 1.0);
    let size = light.misc.z;
    let lo = 4.0 / size;
    if (c.x < lo || c.x > 1.0 || c.y < lo || c.y > 1.0) { return 1.0; }
    let t = vec2<i32>(floor(c.xy * size));
    let d = textureLoad(shadow_map, t, layer, 0);
    return select(0.0, 1.0, c.z < d - light.cascade_bias.x);
}
// VolumetricFog.usf: ShadowFactor *= GetLightFunction(WorldPosition); FVolumetricFogLightFunctionPS = (grey - DisabledBrightness) * distance fade + DisabledBrightness (see lit_common.wgsl light_function)
// The light function is a light-space texture rendered at depth 0.5 of its projection = the plane through vf.lf_plane perpendicular to the light
// (VolumetricFogLightFunction.usf:17-21 / :45): the fade distance is measured from the camera to the voxel's projection onto that plane, not to the voxel.
fn dir_light_function(world: vec3<f32>) -> f32 {
    let l = light.sun_dir.xyz;
    let on_plane = world - l * dot(world - vf.lf_plane.xyz, l);
    let a = clamp((light.light_fn.z - length(on_plane - camera.position.xyz)) / (0.2 * light.light_fn.z), 0.0, 1.0);
    // LIGHT FUNCTION (2026-10-07): light_fn.x = the High program evaluated at the camera (CPU, light_function.rs eval_cpu), not a per-voxel light-function texture
    return light.light_fn.y + (light.light_fn.x - light.light_fn.y) * a;
}

@compute @workgroup_size(4, 4, 4)
fn scatter(@builtin(global_invocation_id) id: vec3<u32>) {
    let grid_u = vec3<u32>(u32(vf.grid.x), u32(vf.grid.y), u32(vf.grid.z));
    if (any(id >= grid_u)) { return; }
    let cell = vec3<f32>(id) + vf.jitter.xyz;
    let depth_cm = slice_depth_cm(cell.z);
    let world = cell_world(cell.xy / vf.grid.xy, depth_cm);
    let ray = normalize(world - camera.position.xyz);   // camera -> cell
    var lighting = vec3<f32>(0.0);
    if (vf.scale.z > 0.5) {
        let vis = dir_visibility(world, depth_cm * 0.01) * dir_light_function(world);
        // phase uses dot(direction to the light, -ray); the light function (constant 0.6 faded to 1 beyond 200 m, FVolumetricFogLightFunctionPS) is applied in dir_light_function
        lighting = lighting + vf.dir_color.rgb * (vis * vf.dir_color.w * hg_phase(vf.fog.w, dot(light.sun_dir.xyz, -ray)));
    }
    if (vf.sky_color.w > 0.0) { lighting = lighting + vf.sky_color.w * max(vf.sky_color.rgb, vec3<f32>(0.0)); }
    let vb = textureLoad(vbuf_a_in, vec3<i32>(id), 0);
    // in-scattered light * scattering (+ VBufferB emissive = 0), extinction = luma(scattering) + absorption
    var cur = vec4<f32>(lighting * vb.rgb, ue_luma(vb.rgb) + vb.a);
    // temporal history: reproject the UNJITTERED cell centre into the previous frame's volume
    let centre = vec3<f32>(id) + 0.5;
    let wc = cell_world(centre.xy / vf.grid.xy, slice_depth_cm(centre.z));
    let pc = vf.prev_view_proj * vec4<f32>(wc, 1.0);
    let prev_depth_cm = pc.w * 100.0;
    let uvw = vec3<f32>(pc.x / pc.w * 0.5 + 0.5, -pc.y / pc.w * 0.5 + 0.5, depth_to_w(prev_depth_cm));
    let inside = all(uvw >= vec3<f32>(0.0)) && all(uvw <= vec3<f32>(1.0)) && pc.w > 0.0 && (prev_depth_cm * vf.z_params.x + vf.z_params.y) > 0.0;
    let weight = select(0.0, vf.jitter.w, inside && vf.grid.w > 0.5);
    let hist = textureSampleLevel(history_in, lin_samp, uvw, 0.0);
    var res = mix(cur, hist, weight);
    if (any(res != res) || any(abs(res) > vec4<f32>(3.0e38))) { res = vec4<f32>(0.0); }   // the NaN / Inf guard of the shader
    textureStore(light_out, vec3<i32>(id), res);
}

// ---- FVolumetricFogFinalIntegrationCS --------------------------------------------------------------------------------------------------
@compute @workgroup_size(8, 8, 1)
fn integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u32(vf.grid.x) || id.y >= u32(vf.grid.y)) { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / vf.grid.xy;
    var prev = camera.position.xyz;
    var acc = vec4<f32>(0.0, 0.0, 0.0, 1.0);   // (light rgb, transmittance)
    for (var z = 0u; z < u32(vf.grid.z); z = z + 1u) {
        let s = textureLoad(light_in, vec3<i32>(vec2<i32>(id.xy), i32(z)), 0);
        let pos = cell_world(uv, slice_depth_cm(f32(z) + 0.5));
        let len_cm = length(pos - prev) * 100.0;
        let t = exp2(-s.a * len_cm * LN2_RCP);
        let k = max(s.a, 0.00001);
        acc = vec4<f32>(acc.rgb + (s.rgb - s.rgb * t) / k * acc.a, acc.a * t);
        textureStore(integrated_out, vec3<i32>(vec2<i32>(id.xy), i32(z)), acc);
        prev = pos;
    }
}

// ---- composite: CombineVolumetricFog (dst * V.a + V.rgb) ---------------------------------------------------------------------------------
@vertex
fn vs_composite(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment
fn fs_composite(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy * camera.viewport.zw;
    let z = textureLoad(scene_depth, vec2<i32>(pos.xy), 0);
    // device depth (wgpu 0..1, finite far) -> view depth (cm)
    let n = camera.proj.z; let f = camera.proj.w;
    let depth_cm = (n * f / (f - z * (f - n))) * 100.0;
    let v = textureSampleLevel(integrated_in, lin_samp, vec3<f32>(uv, depth_to_w(depth_cm)), 0.0);
    return v;   // blend: colour = V.rgb + dst * V.a, alpha = dst.a * V.a (the HDR alpha is the ambient share)
}
