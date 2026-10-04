// FXAA on the tone-mapped, sRGB-encoded LDR image (Video > Anti-Aliasing). Port of the old runtime's fxaa.frag (public FXAA 3.11 "quality"
// structure: luma contrast test, edge-direction classification, end-of-edge search, sub-pixel blend). This is NOT the UE4 post-process AA.
// Texture coordinates are y-down here; "N" is +y (the old shader's labels are symmetric, only tie-breaks can differ).
struct FxaaParams { inv_size: vec2<f32>, pad: vec2<f32>, params: vec4<f32> }   // params: sub-pixel strength, edge threshold, edge threshold min, search steps
@group(0) @binding(0) var<uniform> fp: FxaaParams;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    o.clip = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);   // texture coordinates (y down)
    return o;
}

fn luma_at(p: vec2<f32>) -> f32 { return dot(textureSampleLevel(tex, samp, p, 0.0).rgb, vec3<f32>(0.299, 0.587, 0.114)); }
fn step_len(i: i32) -> f32 {
    // 1,1,1,1,1,1.5,2,2,2,2,4,8 texels (FXAA_QUALITY__PRESET 12 progression)
    if (i < 5) { return 1.0; }
    if (i == 5) { return 1.5; }
    if (i < 10) { return 2.0; }
    if (i == 10) { return 4.0; }
    return 8.0;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let px = fp.inv_size;
    let rgb_m = textureSampleLevel(tex, samp, uv, 0.0).rgb;
    let luma_m = dot(rgb_m, vec3<f32>(0.299, 0.587, 0.114));
    var luma_n = luma_at(uv + vec2<f32>(0.0, px.y));
    var luma_s = luma_at(uv - vec2<f32>(0.0, px.y));
    let luma_e = luma_at(uv + vec2<f32>(px.x, 0.0));
    let luma_w = luma_at(uv - vec2<f32>(px.x, 0.0));
    let range_max = max(max(luma_n, luma_s), max(max(luma_e, luma_w), luma_m));
    let range_min = min(min(luma_n, luma_s), min(min(luma_e, luma_w), luma_m));
    let range = range_max - range_min;
    if (range < max(fp.params.z, range_max * fp.params.y)) { return vec4<f32>(rgb_m, 1.0); }

    let luma_nw = luma_at(uv + vec2<f32>(-px.x, px.y));
    let luma_ne = luma_at(uv + vec2<f32>(px.x, px.y));
    let luma_sw = luma_at(uv + vec2<f32>(-px.x, -px.y));
    let luma_se = luma_at(uv + vec2<f32>(px.x, -px.y));
    let luma_ns = luma_n + luma_s;
    let luma_we = luma_w + luma_e;
    let subpix_nswe = luma_ns + luma_we;
    let edge_horz1 = -2.0 * luma_m + luma_ns;
    let edge_vert1 = -2.0 * luma_m + luma_we;
    let luma_nese = luma_ne + luma_se;
    let luma_nwne = luma_nw + luma_ne;
    let luma_nwsw = luma_nw + luma_sw;
    let luma_swse = luma_sw + luma_se;
    let edge_horz2 = -2.0 * luma_e + luma_nese;
    let edge_vert2 = -2.0 * luma_n + luma_nwne;
    let edge_horz3 = -2.0 * luma_w + luma_nwsw;
    let edge_vert3 = -2.0 * luma_s + luma_swse;
    let edge_horz = abs(edge_horz3) + abs(edge_horz1) * 2.0 + abs(edge_horz2);
    let edge_vert = abs(edge_vert3) + abs(edge_vert1) * 2.0 + abs(edge_vert2);
    let subpix_nwswnese = luma_nwsw + luma_nese;
    let horz_span = edge_horz >= edge_vert;
    let subpix_a = subpix_nswe * 2.0 + subpix_nwswnese;

    var length_sign = select(px.x, px.y, horz_span);
    if (!horz_span) { luma_n = luma_w; luma_s = luma_e; }
    let subpix_b = subpix_a * (1.0 / 12.0) - luma_m;
    let gradient_n = luma_n - luma_m;
    let gradient_s = luma_s - luma_m;
    var luma_nn = luma_n + luma_m;
    let luma_ss = luma_s + luma_m;
    let pair_n = abs(gradient_n) >= abs(gradient_s);
    let gradient = max(abs(gradient_n), abs(gradient_s));
    if (pair_n) { length_sign = -length_sign; }
    let subpix_c = clamp(abs(subpix_b) / range, 0.0, 1.0);

    var pos_b = uv;
    let off_np = select(vec2<f32>(0.0, px.y), vec2<f32>(px.x, 0.0), horz_span);
    if (!horz_span) { pos_b.x = pos_b.x + length_sign * 0.5; } else { pos_b.y = pos_b.y + length_sign * 0.5; }
    var pos_n = pos_b - off_np;
    var pos_p = pos_b + off_np;
    let subpix_d = -2.0 * subpix_c + 3.0;
    var luma_end_n = luma_at(pos_n);
    let subpix_e = subpix_c * subpix_c;
    var luma_end_p = luma_at(pos_p);
    if (!pair_n) { luma_nn = luma_ss; }
    let gradient_scaled = gradient * 0.25;
    let luma_mm = luma_m - luma_nn * 0.5;
    let subpix_f = subpix_d * subpix_e;
    let luma_mm_lt_zero = luma_mm < 0.0;
    luma_end_n = luma_end_n - luma_nn * 0.5;
    luma_end_p = luma_end_p - luma_nn * 0.5;
    var done_n = abs(luma_end_n) >= gradient_scaled;
    var done_p = abs(luma_end_p) >= gradient_scaled;
    if (!done_n) { pos_n = pos_n - off_np * step_len(0); }
    if (!done_p) { pos_p = pos_p + off_np * step_len(0); }
    let steps = i32(clamp(fp.params.w, 1.0, 12.0));
    for (var i = 1; i < 12; i = i + 1) {
        if (i >= steps || (done_n && done_p)) { break; }
        if (!done_n) { luma_end_n = luma_at(pos_n) - luma_nn * 0.5; done_n = abs(luma_end_n) >= gradient_scaled; }
        if (!done_p) { luma_end_p = luma_at(pos_p) - luma_nn * 0.5; done_p = abs(luma_end_p) >= gradient_scaled; }
        if (!done_n) { pos_n = pos_n - off_np * step_len(i); }
        if (!done_p) { pos_p = pos_p + off_np * step_len(i); }
    }
    let dst_n = select(uv.y - pos_n.y, uv.x - pos_n.x, horz_span);
    let dst_p = select(pos_p.y - uv.y, pos_p.x - uv.x, horz_span);
    let good_span_n = (luma_end_n < 0.0) != luma_mm_lt_zero;
    let good_span_p = (luma_end_p < 0.0) != luma_mm_lt_zero;
    let span_length = dst_p + dst_n;
    let direction_n = dst_n < dst_p;
    let dst = min(dst_n, dst_p);
    let good_span = select(good_span_p, good_span_n, direction_n);
    let subpix_g = subpix_f * subpix_f;
    let pixel_offset = dst * (-1.0 / span_length) + 0.5;
    let subpix_h = subpix_g * fp.params.x;
    let pixel_offset_good = select(0.0, pixel_offset, good_span);
    let pixel_offset_subpix = max(pixel_offset_good, subpix_h);
    var pos_m = uv;
    if (!horz_span) { pos_m.x = pos_m.x + pixel_offset_subpix * length_sign; } else { pos_m.y = pos_m.y + pixel_offset_subpix * length_sign; }
    return vec4<f32>(textureSampleLevel(tex, samp, pos_m, 0.0).rgb, 1.0);
}
