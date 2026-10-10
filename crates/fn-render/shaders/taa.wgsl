// TEMPORAL AA: port of UE 4.16 `MainTemporalAAPS` / `MainFastTemporalAAPS` (PostProcessTemporalAA.usf:345-480) = PostProcessTemporalCommon.usf
// with the Main defines: AA_FILTERED 1, AA_BORDER 1, AA_CROSS 2, AA_YCOCG 1, AA_BICUBIC 1, AA_TONE 1, AA_CLAMP 1, AA_NAN 1, AA_DYNAMIC 1,
// AA_FILTERED_CONTRAST_LIMIT 1, AA_ALPHA = POST_PROCESS_ALPHA = 0, AA_LERP 0; `Main` adds AA_DYNAMIC_ANTIGHOST 1 (Epic, r.PostProcessAAQuality 4),
// `MainFast` does not (High, quality 3). AA_AABB / AA_ROUND only affect the non-YCoCg path and are unused.
// Inputs (PostProcessTemporalAA.cpp:120-125 FilterTable): Input0 scene colour (point), Input1 history (bilinear, clamp), Input3 velocity (point),
// SceneDepthTexture (point). Everything is the same pixel grid (no screen-percentage history rescale: history and view have the same size).
// Port differences: this renderer's depth is 0 = near / 1 = far, so the "nearest depth" of the AA_CROSS search is the MINIMUM (UE: inverted Z, max);
// output 1 additionally writes the resolved colour back to the scene colour with the CURRENT frame's alpha (the ambient share post.wgsl uses
// for its AO), UE's TAA output alpha stays in the history (output 0).
struct TaaU {
    plus0: vec4<f32>,          // PlusWeights[0..3]
    plus1: vec4<f32>,          // PlusWeights[4], CurrentFrameWeight, VelocityScaling, flags (1 = AA_DYNAMIC_ANTIGHOST, 2 = responsive everywhere: no history / camera cut)
    clip_to_prev: mat4x4<f32>, // View.ClipToPrevClip (no-AA matrices), column vectors: PrevClip = M * ThisClip
    viewport: vec4<f32>,       // w, h, 1/w, 1/h (ViewportSize = PostprocessInput0Size = PostprocessInput1Size)
}
struct ExposureState { ev: f32, target_ev: f32, exposure: f32, avg_luminance: f32 }
@group(0) @binding(0) var<uniform> tu: TaaU;
@group(0) @binding(1) var scene_in: texture_2d<f32>;
@group(0) @binding(2) var history: texture_2d<f32>;
@group(0) @binding(3) var lin_samp: sampler;
@group(0) @binding(4) var depth_tex: texture_depth_2d;
@group(0) @binding(5) var velocity_tex: texture_2d<f32>;
@group(0) @binding(6) var<storage, read> exposure_state: ExposureState;
// STORM TAA: responsive-AA mask (UE stencil bit STENCIL_TEMPORAL_RESPONSIVE_AA_MASK): 0 = a bEnableResponsiveAA translucent draw covered the pixel (taa.rs)
@group(0) @binding(7) var responsive_mask: texture_2d<f32>;

struct VsOut { @builtin(position) clip: vec4<f32> }
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    o.clip = vec4<f32>(x, y, 0.0, 1.0);
    return o;
}

fn clampi(p: vec2<i32>) -> vec2<i32> { return clamp(p, vec2<i32>(0), vec2<i32>(tu.viewport.xy) - vec2<i32>(1)); }
fn color_at(p: vec2<i32>) -> vec4<f32> { return textureLoad(scene_in, clampi(p), 0); }
fn depth_at(p: vec2<i32>) -> f32 { return textureLoad(depth_tex, clampi(p), 0); }
fn velocity_at(p: vec2<i32>) -> vec2<f32> { return textureLoad(velocity_tex, clampi(p), 0).xy; }

fn rgb_to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(c, vec3<f32>(1.0, 2.0, 1.0)), dot(c, vec3<f32>(2.0, 0.0, -2.0)), dot(c, vec3<f32>(-1.0, 2.0, -1.0)));
}
fn ycocg_to_rgb(c: vec3<f32>) -> vec3<f32> {
    let y = c.x * 0.25; let co = c.y * 0.25; let cg = c.z * 0.25;
    return vec3<f32>(y + co - cg, y + cg, y - co - cg);
}
fn hdr_weight_y(y: f32, e: f32) -> f32 { return 1.0 / (y * e + 1.0); }
fn hdr_weight_inv_y(y: f32, e: f32) -> f32 { return 1.0 / (y * -e + 1.0); }
// DecodeVelocityFromTexture (Common.usf:1743-1750)
fn decode_velocity(v: vec2<f32>) -> vec2<f32> { let inv = 1.0 / (0.499 * 0.5); return v * inv - 32767.0 / 65535.0 * inv; }

// Texture2DSampleBicubic (Common.usf:405-471): Catmull-Rom in five bilinear taps, corners dropped and the weights renormalised
fn sample_bicubic(uv_in: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let inv = 1.0 / size;
    let uv = uv_in * size;
    let tc = floor(uv - 0.5) + 0.5;
    let f = uv - tc;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = f2 - 0.5 * (f3 + f);
    let w1 = 1.5 * f3 - 2.5 * f2 + 1.0;
    let w3 = 0.5 * (f3 - f2);
    let w2 = 1.0 - w0 - w1 - w3;
    let wt0 = w0; let wt1 = w1 + w2; let wt2 = w3;
    let s0 = (tc - 1.0) * inv;
    let s1 = (tc + w2 / wt1) * inv;
    let s2 = (tc + 2.0) * inv;
    var c = textureSampleLevel(history, lin_samp, vec2<f32>(s1.x, s0.y), 0.0) * wt1.x * wt0.y;
    c = c + textureSampleLevel(history, lin_samp, vec2<f32>(s0.x, s1.y), 0.0) * wt0.x * wt1.y;
    c = c + textureSampleLevel(history, lin_samp, vec2<f32>(s1.x, s1.y), 0.0) * wt1.x * wt1.y;
    c = c + textureSampleLevel(history, lin_samp, vec2<f32>(s2.x, s1.y), 0.0) * wt2.x * wt1.y;
    c = c + textureSampleLevel(history, lin_samp, vec2<f32>(s1.x, s2.y), 0.0) * wt1.x * wt2.y;
    let corner = wt1.x * wt0.y + wt0.x * wt1.y + wt1.x * wt1.y + wt2.x * wt1.y + wt1.x * wt2.y;
    return c / corner;
}

struct FsOut { @location(0) history: vec4<f32>, @location(1) scene: vec4<f32> }

@fragment
fn fs_main(in: VsOut) -> FsOut {
    let px = vec2<i32>(floor(in.clip.xy));
    let uv = in.clip.xy * tu.viewport.zw;
    // ScreenPos = (UV * Size - 0.5 - ScreenPosToPixel.zw) / ScreenPosToPixel.xy (RenderingCompositionGraph.cpp:902-907): x right, y up
    let screen_pos = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let flags = u32(tu.plus1.w);
    let antighost = (flags & 1u) != 0u;
    let responsive = (flags & 2u) != 0u;
    let exposure = exposure_state.exposure;   // InExposureScale = the eye adaptation of the previous frame (FPostProcessTonemapVS)
    let current_frame_weight = tu.plus1.y;

    // FIND MOTION OF PIXEL AND NEAREST IN NEIGHBORHOOD (AA_CROSS 2)
    var pos_z = depth_at(px);
    var velocity_offset = vec2<i32>(0);
    let d = vec4<f32>(depth_at(px + vec2<i32>(-2, -2)), depth_at(px + vec2<i32>(2, -2)), depth_at(px + vec2<i32>(-2, 2)), depth_at(px + vec2<i32>(2, 2)));
    var depth_offset = vec2<i32>(2, 2);
    var depth_offset_xx = 2;
    if (d.x < d.y) { depth_offset_xx = -2; }
    if (d.z < d.w) { depth_offset.x = -2; }
    let d_xy = min(d.x, d.y);
    let d_zw = min(d.z, d.w);
    if (d_xy < d_zw) { depth_offset.y = -2; depth_offset.x = depth_offset_xx; }
    let d_xyzw = min(d_xy, d_zw);
    if (d_xyzw < pos_z) { velocity_offset = depth_offset; pos_z = d_xyzw; }

    // camera motion for the pixel (or the nearest pixel)
    let this_clip = vec4<f32>(screen_pos, pos_z, 1.0);
    let prev_clip = tu.clip_to_prev * this_clip;
    let prev_screen = prev_clip.xy / prev_clip.w;
    var back_n = screen_pos - prev_screen;
    // AA_DYNAMIC
    let velocity_n = velocity_at(px + velocity_offset) * tu.plus1.z;
    if (velocity_n.x > 0.0) { back_n = decode_velocity(velocity_n); }
    back_n = screen_pos - back_n;
    let off_screen = max(abs(back_n.x), abs(back_n.y)) >= 1.0;
    back_n.x = clamp(back_n.x, -1.0 + tu.viewport.z, 1.0 - tu.viewport.z);
    back_n.y = clamp(back_n.y, -1.0 + tu.viewport.w, 1.0 - tu.viewport.w);
    let history_uv = vec2<f32>(back_n.x * 0.5 + 0.5, 0.5 - back_n.y * 0.5);

    // FILTER PIXEL AND GET NEIGHBORHOOD (YCoCg, 5 taps)
    let center = color_at(px);
    var n1 = rgb_to_ycocg(color_at(px + vec2<i32>(0, -1)).rgb);
    var n3 = rgb_to_ycocg(color_at(px + vec2<i32>(-1, 0)).rgb);
    var n4 = rgb_to_ycocg(center.rgb);
    var n5 = rgb_to_ycocg(color_at(px + vec2<i32>(1, 0)).rgb);
    var n7 = rgb_to_ycocg(color_at(px + vec2<i32>(0, 1)).rgb);
    // AA_FILTERED_CONTRAST_LIMIT && AA_TONE
    let luma_m1 = n1.x + n3.x + n4.x + n5.x + n7.x;
    let luma_m2 = n1.x * n1.x + n3.x * n3.x + n4.x * n4.x + n5.x * n5.x + n7.x * n7.x;
    let luma_avg = luma_m1 * (1.0 / 5.0);
    let luma_var = abs(luma_m2 * (1.0 / 5.0) - luma_avg * luma_avg);
    // AA_TONE
    n1 = n1 * hdr_weight_y(n1.x, exposure);
    n3 = n3 * hdr_weight_y(n3.x, exposure);
    n4 = n4 * hdr_weight_y(n4.x, exposure);
    n5 = n5 * hdr_weight_y(n5.x, exposure);
    n7 = n7 * hdr_weight_y(n7.x, exposure);
    var filtered = n1 * tu.plus0.x + n3 * tu.plus0.y + n4 * tu.plus0.z + n5 * tu.plus0.w + n7 * tu.plus1.x;
    if (luma_var > 1.0) { filtered = n4; }
    // AA_BORDER: unfiltered for the 1 pixel border
    let test_pos = abs(screen_pos) + tu.viewport.zw * 2.0;
    if (max(test_pos.x, test_pos.y) >= 1.0) { filtered = n4; }
    let neighbor_min = min(min(min(n1, n3), min(n4, n5)), n7);
    let neighbor_max = max(max(max(n1, n3), max(n4, n5)), n7);

    // FETCH HISTORY (AA_BICUBIC); no history (first frame / camera cut) = BlackDummy
    var history_color = vec4<f32>(0.0);
    if (!responsive) { history_color = sample_bicubic(history_uv, tu.viewport.xy); }
    var hist = rgb_to_ycocg(history_color.rgb);
    hist = hist * hdr_weight_y(hist.x, exposure);
    var hist_a = history_color.a;

    // AA_DYNAMIC_ANTIGHOST (Main only)
    let dyn4 = velocity_at(px).x > 0.0;
    if (antighost) {
        let dynamic = velocity_at(px + vec2<i32>(0, -1)).x > 0.0 || velocity_at(px + vec2<i32>(-1, 0)).x > 0.0 || dyn4
            || velocity_at(px + vec2<i32>(1, 0)).x > 0.0 || velocity_at(px + vec2<i32>(0, 1)).x > 0.0;
        if (!dynamic && hist_a > 0.0) { hist = filtered; hist_a = 0.0; }
    }

    // AA_CLAMP (YCoCg: plain clamp)
    hist = clamp(hist, neighbor_min, neighbor_max);

    // COMPUTE BLEND AMOUNT: AA_TONE -> CurrentFrameWeight; responsive -> 1/4
    var blend_final = current_frame_weight;
    if (responsive) { blend_final = 1.0 / 4.0; }
    // STORM TAA: bIsResponsiveAAPixel of a tagged pixel (PostProcessTemporalCommon.usf:114-123 RESPONSIVE pass / :565): 1/4 of the new frame, the history
    // still fetched, clamped and anti-ghosted above (only the no-history flag 2 drops it)
    if (textureLoad(responsive_mask, clampi(px), 0).r < 0.5) { blend_final = 1.0 / 4.0; }
    if (off_screen) { hist = filtered; hist_a = 0.0; }

    var out_rgb = mix(hist, filtered, blend_final);
    var out_a = 0.0;
    if (responsive) { out_a = max(hist_a, 1.0 / 2.0); }
    out_rgb = out_rgb * hdr_weight_inv_y(out_rgb.x, exposure);
    out_rgb = ycocg_to_rgb(out_rgb);
    // AA_NAN: NaN and negative colours to black
    out_rgb = select(vec3<f32>(0.0), out_rgb, out_rgb > vec3<f32>(0.0));
    // AA_DYNAMIC_ANTIGHOST: alpha = Dynamic4, otherwise !AA_ALPHA: 0
    if (antighost) { out_a = select(0.0, 1.0, dyn4); } else { out_a = 0.0; }

    var o: FsOut;
    o.history = vec4<f32>(out_rgb, out_a);
    o.scene = vec4<f32>(out_rgb, center.a);
    // VELOCITY DEBUG (verification aid, taa.rs FN_TAA_DEBUG): the velocity buffer of this pixel instead of the resolve
    if ((flags & 4u) != 0u) {
        let raw = velocity_at(px);
        var own = 0.0;
        if (raw.x > 0.0) { own = length(decode_velocity(raw) * 0.5 * tu.viewport.xy); }
        let cam_px = length((screen_pos - prev_screen) * 0.5 * tu.viewport.xy);
        o.scene = vec4<f32>(select(0.0, 1.0, raw.x > 0.0), own / 10.0, cam_px / 10.0, center.a);
        if ((flags & 8u) != 0u) { o.scene = vec4<f32>(select(0.0, 1.0, own > 0.5), select(0.0, 1.0, own > 2.0), select(0.0, 1.0, own > 8.0), center.a); }
    }
    return o;
}
