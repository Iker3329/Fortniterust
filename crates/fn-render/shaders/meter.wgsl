// Auto-exposure, UE 4.16 HISTOGRAM method (r.DefaultFeature.AutoExposure.Method 0 = AEM_Histogram, the exe ctor default of AutoExposureMethod).
// Port of PostProcessHistogram.usf + PostProcessHistogramCommon.usf (ComputeEyeAdaptationExposure) + PostProcessEyeAdaptation.usf (MainEyeAdaptationCommon):
//  * input = half-res scene colour (the engine feeds SceneColorHalfRes; here HDR mip 1), per pixel luminance = max(r, g, b) (NOT Rec.709 luma);
//  * 64 buckets over log2 luminance in [HistogramLogMin, HistogramLogMax] (-8 .. 4), each pixel split over two neighbouring buckets,
//    bucket 0 dropped ("good hack to prevent adapting to extreme darks");
//  * mean luminance (bucket luminance = exp2((i/64 - add) / mul)) of the buckets between AutoExposureLowPercent and HighPercent of the mass;
//  * clamped to [AutoExposureMinBrightness, AutoExposureMaxBrightness] IN LUMINANCE UNITS (no mid-grey divisor), blended over time in the
//    linear domain with 1 - exp2(-dt * speed) (speed up = scene got brighter), exposure scale = 2^AutoExposureBias / smoothed luminance.
// The fixed-point histogram uses 1/256 weights in u32 atomics (meter_histogram, many workgroups; the reduce below sums them). State = UE's EyeAdaptation.r (2^bias / smoothed luminance); the first frame starts from
// UE's default (previous scale 1, i.e. luminance = 2^bias) and adapts from there.
struct MeterParams {
    dt: f32, min_brightness: f32, max_brightness: f32, bias_ev: f32,
    first: f32,            // 1 on the first frame (no valid eye adaptation yet: previous scale = 1)
    fixed_exposure: f32,   // > 0 overrides the metered exposure
    adapt_up: f32, adapt_down: f32,
    low_fraction: f32, high_fraction: f32, log_min: f32, log_max: f32,
}
// exposure = the persistent state (EyeAdaptation.r, the scale post.wgsl applies); ev = this frame's smoothed luminance, target_ev = clamped target
// luminance, avg_luminance = unclamped histogram mean (the last three are reporting only).
struct ExposureState { ev: f32, target_ev: f32, exposure: f32, avg_luminance: f32 }
@group(0) @binding(0) var<uniform> mp: MeterParams;
@group(0) @binding(1) var half_res: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> state: ExposureState;
// PERF (2026-10-05): the histogram of the whole image, built by many workgroups (`meter_histogram`) and read by the single-workgroup reduce (`meter`),
// as UE4.16 splits it (PostProcessHistogram.usf: per-tile histograms of THREADGROUP_SIZEX x Y threads x LOOP_SIZEX x Y pixels, then
// PostProcessHistogramReduce.usf sums them). The weights are fixed-point u32, so the sum does not depend on the order: bit-identical to one workgroup.
@group(0) @binding(3) var<storage, read_write> global_hist: array<atomic<u32>, 64>;

var<workgroup> histogram: array<atomic<u32>, 64>;

const TILE: u32 = 64u;   // pixels per workgroup edge: 16 x 16 threads, 4 x 4 pixels each

@compute @workgroup_size(16, 16)
fn meter_histogram(@builtin(local_invocation_index) li: u32, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    if (li < 64u) { atomicStore(&histogram[li], 0u); }
    workgroupBarrier();
    let dims = textureDimensions(half_res);
    let mul = 1.0 / (mp.log_max - mp.log_min);
    let add = -mp.log_min * mul;
    for (var j = 0u; j < 4u; j = j + 1u) {
        for (var i = 0u; i < 4u; i = i + 1u) {
            let p = wid.xy * TILE + vec2<u32>(i * 16u, j * 16u) + lid.xy;
            if (p.x >= dims.x || p.y >= dims.y) { continue; }
            let c = textureLoad(half_res, vec2<i32>(p), 0).rgb;
            let l = max(c.r, max(c.g, c.b));
            var pos = 0.0;
            if (l > 0.0 && l < 1.0e30) { pos = log2(l) * mul + add; }
            let fb = clamp(pos, 0.0, 1.0) * 63.0 * 0.9999;
            let b0 = u32(fb);
            let w1 = fract(fb);
            atomicAdd(&histogram[b0], u32((1.0 - w1) * 256.0 + 0.5));
            atomicAdd(&histogram[b0 + 1u], u32(w1 * 256.0 + 0.5));
        }
    }
    workgroupBarrier();
    if (li < 64u) { let v = atomicLoad(&histogram[li]); if (v != 0u) { atomicAdd(&global_hist[li], v); } }
}

@compute @workgroup_size(64)
fn meter(@builtin(local_invocation_index) li: u32) {
    if (li != 0u) { return; }
    let mul = 1.0 / (mp.log_max - mp.log_min);
    let add = -mp.log_min * mul;
    var sum = 0.0;
    for (var i = 1u; i < 64u; i = i + 1u) { sum = sum + f32(atomicLoad(&global_hist[i])); }
    var min_frac = sum * mp.low_fraction;
    var max_frac = sum * mp.high_fraction;
    var acc_l = 0.0;
    var acc_w = 0.0;
    for (var i = 1u; i < 64u; i = i + 1u) {
        var v = f32(atomicLoad(&global_hist[i]));
        let sub = min(v, min_frac);
        v = v - sub;
        min_frac = min_frac - sub;
        max_frac = max_frac - sub;
        v = min(v, max_frac);
        max_frac = max_frac - v;
        let lum = exp2((f32(i) / 64.0 - add) / mul);
        acc_l = acc_l + lum * v;
        acc_w = acc_w + v;
    }
    let avg = acc_l / max(0.0001, acc_w);
    // ComputeEyeAdaptationValues (PostProcessEyeAdaptation.cpp): ExposureOffsetMultipler = 2^AutoExposureBias; a fixed exposure
    // (ExposureSettings.bFixed) sets the multiplier to 1 and Min = Max = the fixed luminance, so the result is exactly the fixed scale.
    var mult = exp2(mp.bias_ev);
    var hi = mp.max_brightness;
    var lo = min(mp.min_brightness, hi);
    if (mp.fixed_exposure > 0.0) { mult = 1.0; hi = 1.0 / mp.fixed_exposure; lo = hi; }
    let target_lum = clamp(avg, lo, hi);
    // EYE ADAPTATION STATE (2026-10-05): the stored value is UE's EyeAdaptation.r = ExposureOffsetMultipler / SmoothedExposure (the exposure SCALE).
    // MainEyeAdaptationCommon rebuilds the previous luminance with the CURRENT multiplier: OldExposure = Multiplier / (OldScale != 0 ? OldScale : 1),
    // so a bias change during a phase blend moves the old value too. Before the first eye-adaptation pass the histogram reduce reads GWhiteTexture
    // (PostProcessHistogramReduce.cpp, !HasValidEyeAdaptation) -> OldScale = 1 -> OldExposure = Multiplier: UE's default, adapted from there.
    var old_scale = state.exposure;
    if (mp.first > 0.5 || !(old_scale == old_scale)) { old_scale = 1.0; }
    let old_lum = mult / select(1.0, old_scale, old_scale != 0.0);
    // ComputeEyeAdaptation
    let diff = target_lum - old_lum;
    let speed = select(mp.adapt_down, mp.adapt_up, diff > 0.0);
    let smoothed = clamp(old_lum + diff * (1.0 - exp2(-mp.dt * speed)), lo, hi);
    state.ev = smoothed;
    state.target_ev = target_lum;
    state.exposure = mult * (1.0 / max(0.0001, smoothed));   // OutColor.r
    state.avg_luminance = avg;
}
