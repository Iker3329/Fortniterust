// REFLECTION APPLY (SSR): port of UE 4.16 screen space reflections (ScreenSpaceReflections.usf `ScreenSpaceReflectionsPS`, ScreenSpaceRayCast.usf `RayCast`,
// `ReprojectHit`, `SampleScreenColor`) and of `SSRTemporalAAPS` (PostProcessTemporalAA.usf:260 + PostProcessTemporalCommon.usf) for views without temporal AA.
// See reflection.rs for the parameters (r.SSR.Quality, the post-process SSR settings, ComputeSSRQuality / ComputeSSRParams).
//
// Port conventions: the GBuffer is the lit passes' reflection target 2 (xyz world normal, w roughness, xyz = 0 for ShadingModelID 0); the scene depth is
// this renderer's (0 = near, 1 = far); the ray maths run in UE's view space (x right, y up, z = view depth) with UE's reversed infinite projection
// (screen z = near / view depth), so the HZB (hzb.wgsl), the compare tolerance and the hit refinement are UE's numbers. ClipToPrevClip is this renderer's
// (port clip space, no-AA matrices, as taa.wgsl): a hit's UE device Z is turned back into this renderer's depth before the reprojection.
struct SsrU {
    view: mat4x4<f32>,          // world -> view (renderer: -z forward)
    clip_to_prev: mat4x4<f32>,  // View.ClipToPrevClip (renderer clip space, no-AA matrices)
    proj: vec4<f32>,            // ViewToClip: x P[0][0], y P[1][1], zw TAA jitter (NDC) added to the screen position
    params: vec4<f32>,          // SSRParams.x intensity, .y RoughnessMaskScale, z View.StateFrameIndexMod8, w NumSteps
    hzb: vec4<f32>,             // HZBUvFactorAndInvFactor
    viewport: vec4<f32>,        // w, h, 1 / w, 1 / h
    depth: vec4<f32>,           // x port near (m), y port far (m), z UE near (m) of the device Z, w flags (1 = velocity texture, 2 = debug: trace as roughness 0)
}
@group(0) @binding(0) var<uniform> su: SsrU;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var gbuf: texture_2d<f32>;
@group(0) @binding(3) var hzb_tex: texture_2d<f32>;
@group(0) @binding(4) var point_samp: sampler;
@group(0) @binding(5) var color_tex: texture_2d<f32>;
@group(0) @binding(6) var velocity_tex: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn is_nan_or_inf(x: f32) -> bool { return x != x || abs(x) > 3.0e38; }
// HLSL saturate(a / b) including its NaN -> 0 / inf -> 1 behaviour
fn saturate_div(a: f32, b: f32) -> f32 {
    if (b == 0.0) { return select(0.0, 1.0, a > 0.0); }
    return clamp(a / b, 0.0, 1.0);
}
fn viewport_px(uv: vec2<f32>) -> vec2<i32> { return clamp(vec2<i32>(floor(uv * su.viewport.xy)), vec2<i32>(0), vec2<i32>(su.viewport.xy) - vec2<i32>(1)); }

// InterleavedGradientNoise (Random.usf:27)
fn interleaved_gradient_noise(uv_in: vec2<f32>, frame_id: f32) -> f32 {
    let uv = uv_in + frame_id * (vec2<f32>(47.0, 17.0) * 0.695);
    return fract(52.9829189 * fract(dot(uv, vec2<f32>(0.06711056, 0.00583715))));
}
// View.ViewToClip of a UE view-space point (w = 1) or direction (w = 0): clip.xy = P xy + jitter * z, clip.z = near * w, clip.w = z
fn view_to_clip(p: vec3<f32>, w: f32) -> vec4<f32> {
    return vec4<f32>(p.x * su.proj.x + su.proj.z * p.z, p.y * su.proj.y + su.proj.w * p.z, su.depth.z * w, p.z);
}

// RayCast (ScreenSpaceRayCast.usf:40) with USE_HZB 1, SCALAR_BRANCHLESS 0, VECTORIZED_BRANCHLESS 0: the first `#elif 1` loop (:197), 4 HZB taps per iteration.
// Returns (hit screen position xy, hit UE device Z, time: 0.5 = hit, 1 = miss).
fn ray_cast(start_v: vec3<f32>, dir: vec3<f32>, roughness: f32, scene_depth: f32, num_steps: i32, step_offset: f32) -> vec4<f32> {
    let ray_dir_v = dir * scene_depth;
    let end_v = start_v + ray_dir_v;
    let start_clip = view_to_clip(start_v, 1.0);
    let end_clip = view_to_clip(end_v, 1.0);
    let start_s = start_clip.xyz / start_clip.w;
    let end_s = end_clip.xyz / end_clip.w;
    let depth_clip = start_clip + view_to_clip(vec3<f32>(0.0, 0.0, scene_depth), 0.0);
    let depth_s = depth_clip.xyz / depth_clip.w;
    var step_s = end_s - start_s;
    {
        // clip the step to the viewport
        let inv_factor = 0.5 * length(step_s.xy);
        let abs_step = abs(step_s.xy);
        let s = (abs_step - max(abs(step_s.xy + start_s.xy * inv_factor) - inv_factor, vec2<f32>(0.0))) / abs_step;
        step_s = step_s * (min(s.x, s.y) / inv_factor);
    }
    let start_uvz = vec3<f32>((start_s.xy * vec2<f32>(0.5, -0.5) + 0.5) * su.hzb.xy, start_s.z);
    var step_uvz = vec3<f32>(step_s.xy * vec2<f32>(0.5, -0.5) * su.hzb.xy, step_s.z);
    let step = 1.0 / f32(num_steps);
    let tolerance = max(abs(step_uvz.z), (start_s.z - depth_s.z) * 4.0) * step;
    var result = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    var last_diff = 0.0;
    var level = 1.0;   // HZB_LEVEL_OFFSET
    step_uvz = step_uvz * step;
    var ray = start_uvz + step_uvz * step_offset;
    if (any(step_uvz != step_uvz) || any(ray != ray)) { return result; }   // degenerate step (NaN): no taps can hit
    for (var i = 0; i < num_steps; i = i + 4) {
        let uv0 = ray.xyxy + step_uvz.xyxy * vec4<f32>(1.0, 1.0, 2.0, 2.0);
        let uv1 = ray.xyxy + step_uvz.xyxy * vec4<f32>(3.0, 3.0, 4.0, 4.0);
        let sample_z = ray.zzzz + step_uvz.zzzz * vec4<f32>(1.0, 2.0, 3.0, 4.0);
        let sample_depth = vec4<f32>(
            textureSampleLevel(hzb_tex, point_samp, uv0.xy, level).r, textureSampleLevel(hzb_tex, point_samp, uv0.zw, level).r,
            textureSampleLevel(hzb_tex, point_samp, uv1.xy, level).r, textureSampleLevel(hzb_tex, point_samp, uv1.zw, level).r);
        let diff = sample_z - sample_depth;
        let hit = abs(-diff - vec4<f32>(tolerance)) < vec4<f32>(tolerance);
        if (any(hit)) {
            var d0 = diff.z;
            var d1 = diff.w;
            var min_time = 3.0;
            if (hit.z) { d0 = diff.y; d1 = diff.z; min_time = 2.0; }
            if (hit.y) { d0 = diff.x; d1 = diff.y; min_time = 1.0; }
            if (hit.x) { d0 = last_diff; d1 = diff.x; min_time = 0.0; }
            let time_lerp = saturate_div(d0, d0 - d1);
            result = vec4<f32>(ray + step_uvz * (min_time + time_lerp), 0.5);
            break;
        }
        last_diff = diff.w;
        level = level + roughness * (16.0 / f32(num_steps));
        ray = ray + 4.0 * step_uvz;
    }
    // HZB uv -> screen position
    let xy = result.xy * su.hzb.zw * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    return vec4<f32>(xy, result.z, result.w);
}

// this renderer's depth (0 near .. 1 far, perspective_rh) of a UE device Z (near / view depth)
fn port_depth_of_ue_z(z: f32) -> f32 {
    let n = su.depth.x;
    let f = su.depth.y;
    if (z <= 0.0) { return f / (f - n); }
    let w = su.depth.z / z;
    return f * (w - n) / (w * (f - n));
}
fn decode_velocity(v: vec2<f32>) -> vec2<f32> { let inv = 1.0 / (0.499 * 0.5); return v * inv - 32767.0 / 65535.0 * inv; }   // Common.usf:1743

// ReprojectHit (ScreenSpaceRayCast.usf:462): the hit's previous screen position (velocity texture when the hit pixel moved, else ClipToPrevClip)
fn reproject_hit(hit: vec3<f32>) -> vec2<f32> {
    let this_clip = vec4<f32>(hit.xy, port_depth_of_ue_z(hit.z), 1.0);
    let prev_clip = su.clip_to_prev * this_clip;
    var prev_screen = prev_clip.xy / prev_clip.w;
    if ((u32(su.depth.w) & 1u) != 0u) {
        let uv = vec2<f32>(hit.x * 0.5 + 0.5, 0.5 - hit.y * 0.5);
        let vel = textureLoad(velocity_tex, viewport_px(uv), 0).xy;
        if (vel.x > 0.0) { prev_screen = this_clip.xy - decode_velocity(vel); }
    }
    return prev_screen;
}
// SampleScreenColor (ScreenSpaceRayCast.usf:479): point sample (PostprocessInput0 point clamp), alpha 1, off-screen vignette, NaN / negative -> 0
fn sample_screen_color(prev_screen: vec2<f32>) -> vec4<f32> {
    if (is_nan_or_inf(prev_screen.x) || is_nan_or_inf(prev_screen.y)) { return vec4<f32>(0.0); }
    let uv = vec2<f32>(prev_screen.x * 0.5 + 0.5, 0.5 - prev_screen.y * 0.5);
    var c = vec4<f32>(textureLoad(color_tex, viewport_px(uv), 0).rgb, 1.0);
    let vignette = clamp(abs(prev_screen) * 5.0 - 4.0, vec2<f32>(0.0), vec2<f32>(1.0));
    c = c * clamp(1.0 - dot(vignette, vignette), 0.0, 1.0);
    let rgb = select(c.rgb, vec3<f32>(0.0), c.rgb != c.rgb);
    return vec4<f32>(max(rgb, vec3<f32>(0.0)), c.a);
}

// ScreenSpaceReflectionsPS, SSR_QUALITY 1 / 2 (one ray: NumRays 1; quality 3 / 4 take the same branch below roughness 0.1 with 16 steps, see reflection.rs)
@fragment
fn fs_trace(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(floor(frag.xy));
    let g = textureLoad(gbuf, px, 0);
    // GetRoughnessFade: min(Roughness * SSRParams.y + 2, 1); early out at <= 0 or ShadingModelID 0 (no normal written)
    // debug (FN_SSR_DEBUG=1, reflection.rs): every lit pixel traced as a mirror, to check the trace / HZB / reprojection on any surface
    let roughness = select(g.w, 0.0, (u32(su.depth.w) & 2u) != 0u);
    let fade = min(roughness * su.params.y + 2.0, 1.0);
    if (fade <= 0.0 || dot(g.xyz, g.xyz) < 0.25) { return vec4<f32>(0.0); }
    let d = textureLoad(depth_tex, px, 0);
    // no scene depth here: the sky, or the backdrop pass (landscape / ocean / far skybox), whose depth the scene pass clears (port limitation, reflection.rs)
    if (d >= 1.0) { return vec4<f32>(0.0); }
    let n_port = su.depth.x;
    let f_port = su.depth.y;
    let scene_depth = n_port * f_port / (f_port - d * (f_port - n_port));
    let uv = frag.xy * su.viewport.zw;
    let screen = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    // PositionTranslatedWorld = (ScreenPos * SceneDepth, SceneDepth, 1) x ScreenToTranslatedWorld, here directly in UE view space
    let p = vec3<f32>((screen.x - su.proj.z) / su.proj.x * scene_depth, (screen.y - su.proj.w) / su.proj.y * scene_depth, scene_depth);
    let nr = (su.view * vec4<f32>(normalize(g.xyz), 0.0)).xyz;
    let n = vec3<f32>(nr.x, nr.y, -nr.z);
    let v = normalize(-p);
    let r = 2.0 * dot(v, n) * n - v;   // reflect(-V, N)
    let step_offset = interleaved_gradient_noise(frag.xy, su.params.z) - 0.5;
    let hit = ray_cast(p, r, roughness, scene_depth, i32(su.params.w), step_offset);
    var out = vec4<f32>(0.0);
    if (hit.w < 1.0) { out = sample_screen_color(reproject_hit(hit.xyz)); }    return out * fade * su.params.x;
}
