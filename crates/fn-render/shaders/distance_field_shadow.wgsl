// DISTANCE FIELD HOOK (DF SHADOW SCREEN PASS, 2026-10-06): the ray-traced distance-field shadow of the sun at GetDFShadowDownsampleFactor() resolution, as UE4.16 1.8
// computes it before the lights: DistanceFieldShadowing.usf DistanceFieldShadowingCS (the DFS_DirectionalLightScatterTileCulling permutation: no groupshared
// data, every thread is one texel, so a full-screen pass writes the same texels) into the PF_G16R16F `RayTracedShadows` target (GetBufferSizeForDFShadows =
// floor(BufferSize / factor)); the lit shaders then run DistanceFieldShadowingUpsamplePS per pixel (lit_common.wgsl df_shadow_upsample).
// Appended after common.wgsl + lit_common.wgsl (DfU at binding 5, df_space / df_line_box). Own bind group (distance_field_shadow.rs): bindings 5..9 + 17 / 18.

struct DfsU {
    view_inverse: mat4x4<f32>,   // renderer view -> world (the jittered view the pre-pass rasterized)
    eye: vec4<f32>,              // xyz camera position (m)
    proj: vec4<f32>,             // tan half fov x, tan half fov y, near (m), far (m) of the main projection
    size: vec4<f32>,             // View.BufferSizeAndInvSize: W, H, 1 / W, 1 / H (full resolution)
    misc: vec4<f32>,             // xy TAA projection jitter (NDC, CameraParams::jitter), z DownsampleFactor, w unused
}
@group(0) @binding(6) var<storage, read> df_objects: array<vec4<f32>>;
@group(0) @binding(7) var<storage, read> df_tiles: array<u32>;
@group(0) @binding(8) var df_atlas: texture_3d<f32>;
@group(0) @binding(9) var df_samp: sampler;
@group(0) @binding(17) var dfs_depth: texture_depth_2d;
@group(0) @binding(18) var<uniform> dfs: DfsU;

// ShadowRayTraceThroughCulledObjects (DistanceFieldShadowingShared.usf:46) for the light tile of `p` (DF space), scene depth in cm.
// Moved verbatim from lit_common.wgsl (the per-fragment trace of the forward lit shaders, 2026-10-05).
fn df_shadow(p: vec3<f32>, scene_depth_cm: f32) -> f32 {
    let l = dfu.sh3.xyz;
    let ray_start_offset = 2.0 + dfu.sh1.w * scene_depth_cm;
    let trace = dfu.sh1.z;
    let start = p + l * ray_start_offset;
    let end = p + l * trace;
    let max_ray_time = trace;
    let tan_angle = dfu.sh1.y;
    let min_sphere = 0.4;
    let max_sphere = 100.0;
    // GetShadowTileCulledData
    let hs = dfu.world_to_shadow * vec4<f32>(p, 1.0);
    var nsp = hs.xy * 0.5 + 0.5;
    nsp.y = 1.0 - nsp.y;
    let tiles = dfu.sh2.yz;
    let tp = vec2<i32>(nsp * tiles);
    if (any(tp < vec2<i32>(0)) || any(vec2<f32>(tp) >= tiles)) { return 1.0; }
    let ti = u32(tp.y) * u32(tiles.x) + u32(tp.x);
    let head_start = df_tiles[ti * 2u];
    let count = min(df_tiles[ti * 2u + 1u], u32(dfu.sh2.w));
    var min_ray_time = max_ray_time;
    var min_vis = 1.0;
    let dir_w = normalize(end - start);
    for (var k = 0u; k < count && min_ray_time >= max_ray_time; k = k + 1u) {
        let oi = df_tiles[head_start + k];
        let b = oi * 9u;
        let sph = df_objects[b];
        let along = dot(sph.xyz - start, dir_w);
        if (along > -sph.w) {
            let lpe = df_objects[b + 5u].xyz;
            let w2v = mat4x4<f32>(df_objects[b + 1u], df_objects[b + 2u], df_objects[b + 3u], df_objects[b + 4u]);
            let uvs = df_objects[b + 6u];
            let two_sided = uvs.w < 0.0;
            let vol_scale = abs(uvs.w);
            let uva = df_objects[b + 7u];
            let mad = df_objects[b + 8u].xy;
            let vs = (w2v * vec4<f32>(start, 1.0)).xyz;
            let ve = (w2v * vec4<f32>(end, 1.0)).xyz;
            var vdir = ve - vs;
            let vlen = length(vdir);
            vdir = vdir / vlen;
            let w2v_scale = 1.0 / vol_scale;
            let vmin_sphere = min_sphere * w2v_scale;
            let vmax_sphere = max_sphere * w2v_scale;
            let vbias = dfu.sh2.x * w2v_scale;
            let self_scale = 1.0 / max(uva.w * w2v_scale, 0.0001);
            let cone_at = min(tan_angle * max(along, 0.0) * w2v_scale, vmax_sphere);
            let it = df_line_box(vs, ve, -lpe - cone_at, lpe + cone_at);
            if (it.x < it.y && it.x < 1.0) {
                var t = it.x * vlen;
                let max_steps = 64u;
                let min_step = 1.0 / f32(4u * max_steps);
                var step = 0u;
                for (; step < max_steps; step = step + 1u) {
                    let sp = vs + vdir * t;
                    let cl = clamp(sp, -lpe, lpe);
                    let dc = length(cl - sp);
                    var d = textureSampleLevel(df_atlas, df_samp, cl * uvs.xyz + uva.xyz, 0.0).x * mad.x + mad.y + dc;
                    if (two_sided) { d = d - vbias; }
                    let self_vis = 1.0 - clamp(t * self_scale, 0.0, 1.0);
                    let r = clamp(tan_angle * t, vmin_sphere, vmax_sphere);
                    let sv = max(clamp(d / r, 0.0, 1.0), self_vis);
                    min_vis = min(min_vis, sv);
                    t = t + max(abs(d), min_step);
                    if (min_vis < 0.01 || t > it.y * vlen) { break; }
                }
                if (min_vis < 0.01 || step == max_steps) {
                    min_vis = 0.0;
                    min_ray_time = min(min_ray_time, t * vol_scale);
                }
                min_vis = min(min_vis, 1.0 - f32(step) / f32(max_steps));
            }
        }
    }
    return min_vis;
}

// Full-screen triangle over the RayTracedShadows target (one fragment per texel).
@vertex
fn vs_dfs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

// DistanceFieldShadowingCS (DistanceFieldShadowing.usf:487-608) for the texel DispatchThreadId = floor(frag.xy):
//   ScreenUV = (DispatchThreadId * DownsampleFactor + ScissorRectMin (0) + .5) * View.BufferSizeAndInvSize.zw   -> the full-resolution texel (2x, 2y), the
//   top-left of its block; SceneDepth = CalcSceneDepth(ScreenUV) of the ONE scene depth (pre-pass + the merged backdrop, renderer.rs); OpaqueWorldPosition =
//   ScreenToWorld(ScreenPosition * SceneDepth, SceneDepth) with the jittered view; Result = 0 unless MinDepth < SceneDepth < MaxDepth, else the ray trace;
//   RWShadowFactors = float2(Result, SceneDepth) (cm, stored as fp16 by the PF_G16R16F target).
// SubsurfaceDensity / bUseSubsurfaceTransmission (the GBuffer shading model's transmission, :571-580) are NOT applied: the forward port has no GBuffer before
// its base pass; the per-fragment trace this replaces had none either (OPT_LOG.md item 1, a pre-existing gap).
@fragment
fn fs_dfs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let factor = i32(dfs.misc.z);
    let t = vec2<i32>(floor(frag.xy)) * factor;
    let dev = textureLoad(dfs_depth, t, 0);
    let n = dfs.proj.z;
    let f = dfs.proj.w;
    let d = n * f / (f - dev * (f - n));   // linear view depth (m) of the wgpu 0..1 perspective (camera.rs, distance_field_ao.wgsl linear_depth_cm)
    let scene_depth_cm = d * 100.0;
    let uv = (vec2<f32>(t) + 0.5) * dfs.size.zw;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0) - dfs.misc.xy;   // ScreenPosition of the jittered projection -> the unjittered ray
    let world = dfs.eye.xyz + (dfs.view_inverse * vec4<f32>(ndc.x * dfs.proj.x * d, ndc.y * dfs.proj.y * d, -d, 0.0)).xyz;
    var result = 0.0;
    if (scene_depth_cm > dfu.sh0.y && scene_depth_cm < dfu.sh0.z) { result = df_shadow(df_space(world), scene_depth_cm); }
    return vec4<f32>(result, scene_depth_cm, 0.0, 0.0);
}
