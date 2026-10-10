// REFLECTION APPLY (SSR): `SSRTemporalAAPS` of UE 4.16 (PostProcessTemporalAA.usf:260), see ssr.wgsl / reflection.rs. Own module: its bindings reuse the trace's numbers.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}
fn decode_velocity(v: vec2<f32>) -> vec2<f32> { let inv = 1.0 / (0.499 * 0.5); return v * inv - 32767.0 / 65535.0 * inv; }   // Common.usf:1743
// ---- SSRTemporalAAPS (views without temporal AA: IsSSRTemporalPassRequired, ScreenSpaceReflections.cpp:91) ----
// PostProcessTemporalCommon.usf with AA_ALPHA 1, AA_CROSS 0, AA_DYNAMIC 1, AA_LERP 8, AA_NAN 1 and the defaults AA_AABB 1, AA_FILTERED 1, AA_ROUND 1, AA_TONE 1,
// AA_CLAMP 1, AA_YCOCG 0, AA_BICUBIC 0, AA_BORDER 0. Inputs: 0 the SSR trace (point), 1 SSRHistoryRT (bilinear clamp), 3 velocity (point), scene depth (point).
struct TempU {
    clip_to_prev: mat4x4<f32>,
    weights0: vec4<f32>,   // SampleWeights[0..3]
    weights1: vec4<f32>,   // SampleWeights[4..7]
    weights2: vec4<f32>,   // x SampleWeights[8], y has history (0 = BlackDummy), z velocity texture (1), w VelocityScaling
    viewport: vec4<f32>,
}
struct ExposureState { ev: f32, target_ev: f32, exposure: f32, avg_luminance: f32 }
@group(0) @binding(0) var<uniform> tu: TempU;
@group(0) @binding(1) var ssr_in: texture_2d<f32>;
@group(0) @binding(2) var history_tex: texture_2d<f32>;
@group(0) @binding(3) var lin_samp: sampler;
@group(0) @binding(4) var t_depth: texture_depth_2d;
@group(0) @binding(5) var t_velocity: texture_2d<f32>;
@group(0) @binding(6) var<storage, read> exposure_state: ExposureState;

fn t_clampi(p: vec2<i32>) -> vec2<i32> { return clamp(p, vec2<i32>(0), vec2<i32>(tu.viewport.xy) - vec2<i32>(1)); }
fn luma4(c: vec3<f32>) -> f32 { return c.g * 2.0 + (c.r + c.b); }
fn hdr_weight4(c: vec3<f32>, e: f32) -> f32 { return 1.0 / (luma4(c) * e + 4.0); }
fn hdr_weight_inv4(c: vec3<f32>, e: f32) -> f32 { return 4.0 / (luma4(c) * -e + 1.0); }
fn safe_rcp(x: f32) -> f32 { if (x == 0.0) { return 3.0e38; } return 1.0 / x; }
fn intersect_aabb(dir: vec3<f32>, org: vec3<f32>, box_: vec3<f32>) -> f32 {
    let rcp_dir = vec3<f32>(safe_rcp(dir.x), safe_rcp(dir.y), safe_rcp(dir.z));
    let t_neg = (box_ - org) * rcp_dir;
    let t_pos = (-box_ - org) * rcp_dir;
    return max(max(min(t_neg.x, t_pos.x), min(t_neg.y, t_pos.y)), min(t_neg.z, t_pos.z));
}
fn history_clamp(history: vec3<f32>, filtered: vec3<f32>, nmin: vec3<f32>, nmax: vec3<f32>) -> f32 {
    let mn = min(filtered, min(nmin, nmax));
    let mx = max(filtered, max(nmin, nmax));
    let avg2 = mx + mn;
    let dir = filtered - history;
    let org = history - avg2 * 0.5;
    let scale = mx - avg2 * 0.5;
    return clamp(intersect_aabb(dir, org, scale), 0.0, 1.0);
}

struct TempOut { @location(0) history: vec4<f32>, @location(1) ssr: vec4<f32> }
@fragment
fn fs_temporal(@builtin(position) frag: vec4<f32>) -> TempOut {
    let px = vec2<i32>(floor(frag.xy));
    let uv = frag.xy * tu.viewport.zw;
    let screen_pos = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let exposure = exposure_state.exposure;   // InExposureScale (FPostProcessTonemapVS: the eye adaptation)
    // FIND MOTION OF PIXEL (AA_CROSS 0)
    let pos_z = textureLoad(t_depth, t_clampi(px), 0);
    let prev_clip = tu.clip_to_prev * vec4<f32>(screen_pos, pos_z, 1.0);
    var back_n = screen_pos - prev_clip.xy / prev_clip.w;
    if (tu.weights2.z > 0.5) {
        let vel = textureLoad(t_velocity, t_clampi(px), 0).xy * tu.weights2.w;
        if (vel.x > 0.0) { back_n = decode_velocity(vel); }
    }
    let back_temp = back_n * tu.viewport.xy;
    let history_blur = clamp(abs(back_temp.x) * 2.0 + abs(back_temp.y) * 2.0, 0.0, 1.0);   // HistoryBlurAmp 2
    back_n = screen_pos - back_n;
    let off_screen = max(abs(back_n.x), abs(back_n.y)) >= 1.0;
    back_n.x = clamp(back_n.x, -1.0 + tu.viewport.z, 1.0 - tu.viewport.z);
    back_n.y = clamp(back_n.y, -1.0 + tu.viewport.w, 1.0 - tu.viewport.w);
    let history_uv = vec2<f32>(back_n.x * 0.5 + 0.5, 0.5 - back_n.y * 0.5);
    // FILTER PIXEL AND GET NEIGHBORHOOD (9 taps, HdrWeight4)
    var nb: array<vec4<f32>, 9>;
    for (var j = 0; j < 3; j = j + 1) {
        for (var i = 0; i < 3; i = i + 1) {
            var c = textureLoad(ssr_in, t_clampi(px + vec2<i32>(i - 1, j - 1)), 0);
            c = vec4<f32>(c.rgb * hdr_weight4(c.rgb, exposure), c.a);
            nb[j * 3 + i] = c;
        }
    }
    var w = array<f32, 9>(tu.weights0.x, tu.weights0.y, tu.weights0.z, tu.weights0.w, tu.weights1.x, tu.weights1.y, tu.weights1.z, tu.weights1.w, tu.weights2.x);
    var filtered = vec4<f32>(0.0);
    for (var k = 0; k < 9; k = k + 1) { filtered = filtered + nb[k] * w[k]; }
    // AA_ROUND: mean +- 1 sigma of Neighbor0..7
    var m1 = vec4<f32>(0.0);
    var m2 = vec4<f32>(0.0);
    for (var k = 0; k < 8; k = k + 1) { m1 = m1 + nb[k]; m2 = m2 + nb[k] * nb[k]; }
    let mu = m1 * (1.0 / 8.0);
    let sigma = sqrt(abs(m2 * (1.0 / 8.0) - mu * mu));
    let nmin = mu - sigma;
    let nmax = mu + sigma;
    // FETCH HISTORY (bilinear; BlackDummy without a history)
    var history = vec4<f32>(0.0);
    if (tu.weights2.y > 0.5) { history = textureSampleLevel(history_tex, lin_samp, history_uv, 0.0); }
    history = vec4<f32>(history.rgb * hdr_weight4(history.rgb, exposure), history.a);
    let luma_min = luma4(nmin.rgb);
    let luma_max = luma4(nmax.rgb);
    let luma_contrast = luma_max - luma_min;
    // AA_CLAMP + AA_AABB (AA_ALPHA: the alpha follows the colour's clamp blend)
    let clamp_blend = history_clamp(history.rgb, filtered.rgb, nmin.rgb, nmax.rgb);
    history = mix(history, filtered, clamp_blend);
    // ADD BACK IN ALIASING TO SHARPEN (AA_FILTERED && !AA_BICUBIC)
    var add_aliasing = history_blur * 0.5;
    add_aliasing = clamp(add_aliasing + 1.0 / (1.0 + luma_contrast * 32.0), 0.0, 1.0);
    filtered = vec4<f32>(mix(filtered.rgb, nb[4].rgb, add_aliasing), filtered.a);
    // AA_LERP 8: fixed 1/8 of the new frame, everything on an off-screen history
    var fixed_lerp = 1.0 / 8.0;
    if (off_screen) { history = filtered; fixed_lerp = 1.0; }
    var out = mix(history, filtered, fixed_lerp);
    out = vec4<f32>(out.rgb * hdr_weight_inv4(out.rgb, exposure), out.a);
    let rgb = select(out.rgb, vec3<f32>(0.0), out.rgb != out.rgb);   // AA_NAN
    out = vec4<f32>(max(rgb, vec3<f32>(0.0)), out.a);
    var o: TempOut;
    o.history = out;
    o.ssr = out;
    return o;
}
