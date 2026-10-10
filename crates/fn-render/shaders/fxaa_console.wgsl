// FXAA of the Medium Anti-Aliasing level: UE 4.16 `FRCPassPostProcessAA(Quality 2)` = FXAA_PRESET 1 = FXAA 3.11 with FXAA_PC_CONSOLE 1
// (FXAAShader.usf:21-23, Fxaa3_11.usf:1256-1332), parameters from PostProcessAA.cpp:94-135: fxaaConsoleRcpFrameOpt = 0.5 texel,
// fxaaConsoleRcpFrameOpt2 = 2 texels, EdgeSharpness 8, EdgeThreshold 0.125, EdgeThresholdMin 0.05. Corners = centre -+ 0.5 texel (FxaaVS).
// Luma: FXAA_GREEN_AS_LUMA 0 reads the alpha the tonemapper wrote (`saturate(dot(OutDeviceColor, (0.299, 0.587, 0.114)))`, PostProcessTonemap.usf);
// it is recomputed here from the (bilinearly filtered) colour, which is the same linear combination.
struct FxaaConsoleU { inv_size: vec4<f32> }   // 1/w, 1/h
@group(0) @binding(0) var<uniform> fu: FxaaConsoleU;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    o.clip = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return o;
}
fn tex_top(p: vec2<f32>) -> vec4<f32> { let c = textureSampleLevel(tex, samp, p, 0.0); return vec4<f32>(c.rgb, clamp(dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114)), 0.0, 1.0)); }

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let pos = in.uv;
    let rcp = fu.inv_size.xy;
    let pp = vec4<f32>(pos - 0.5 * rcp, pos + 0.5 * rcp);
    let opt = 0.5 * rcp;
    let opt2 = 2.0 * rcp;
    let sharpness = 8.0;
    let threshold = 0.125;
    let threshold_min = 0.05;
    let luma_nw = tex_top(pp.xy).w;
    let luma_sw = tex_top(pp.xw).w;
    var luma_ne = tex_top(pp.zy).w;
    let luma_se = tex_top(pp.zw).w;
    let rgby_m = tex_top(pos);
    let luma_m = rgby_m.w;
    let luma_max_nw_sw = max(luma_nw, luma_sw);
    luma_ne = luma_ne + 1.0 / 384.0;
    let luma_min_nw_sw = min(luma_nw, luma_sw);
    let luma_max_ne_se = max(luma_ne, luma_se);
    let luma_min_ne_se = min(luma_ne, luma_se);
    let luma_max = max(luma_max_ne_se, luma_max_nw_sw);
    let luma_min = min(luma_min_ne_se, luma_min_nw_sw);
    let luma_max_scaled = luma_max * threshold;
    let luma_min_m = min(luma_min, luma_m);
    let luma_max_scaled_clamped = max(threshold_min, luma_max_scaled);
    let luma_max_m = max(luma_max, luma_m);
    let dir_sw_minus_ne = luma_sw - luma_ne;
    let luma_max_sub_min_m = luma_max_m - luma_min_m;
    let dir_se_minus_nw = luma_se - luma_nw;
    if (luma_max_sub_min_m < luma_max_scaled_clamped) { return vec4<f32>(rgby_m.rgb, 1.0); }
    let dir = vec2<f32>(dir_sw_minus_ne + dir_se_minus_nw, dir_sw_minus_ne - dir_se_minus_nw);
    let dir1 = normalize(dir);
    let rgby_n1 = tex_top(pos - dir1 * opt);
    let rgby_p1 = tex_top(pos + dir1 * opt);
    let dir_abs_min_times_c = min(abs(dir1.x), abs(dir1.y)) * sharpness;
    let dir2 = clamp(dir1 / dir_abs_min_times_c, vec2<f32>(-2.0), vec2<f32>(2.0));
    let rgby_n2 = tex_top(pos - dir2 * opt2);
    let rgby_p2 = tex_top(pos + dir2 * opt2);
    let rgby_a = rgby_n1 + rgby_p1;
    var rgby_b = ((rgby_n2 + rgby_p2) * 0.25) + (rgby_a * 0.25);
    let two_tap = (rgby_b.w < luma_min) || (rgby_b.w > luma_max);
    if (two_tap) { rgby_b = vec4<f32>(rgby_a.xyz * 0.5, rgby_b.w); }
    return vec4<f32>(rgby_b.rgb, 1.0);
}
