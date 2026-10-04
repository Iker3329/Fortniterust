// Auto-exposure (UE eye adaptation, simplified): meters the 1x1 mean of the HDR mip chain, adapts in the log2 domain
// (3 stops/s up, 1 stop/s down) and writes exposure = 2^bias / clamp(avg / 0.18, min, max) into a storage buffer that post.wgsl reads.
// ASSUMED from memory of the 4.18 shader (constants are not serialized): see fn-render README / README "Tonemapper".
struct MeterParams {
    dt: f32, min_brightness: f32, max_brightness: f32, bias_ev: f32,
    first: f32,            // 1 on the first frame (snap to the metered value)
    fixed_exposure: f32,   // > 0 overrides the metered exposure
    adapt_up: f32, adapt_down: f32,
}
struct ExposureState { ev: f32, target_ev: f32, exposure: f32, avg_luminance: f32 }
@group(0) @binding(0) var<uniform> mp: MeterParams;
@group(0) @binding(1) var last_mip: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> state: ExposureState;

@compute @workgroup_size(1)
fn meter() {
    let px = textureLoad(last_mip, vec2<i32>(0, 0), 0);
    let lum = dot(px.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let fallback = log2(mp.min_brightness * 0.18);
    var target_ev = fallback;
    if (lum == lum && lum < 1.0e30) { target_ev = log2(max(lum, 1e-4)); }
    var ev = state.ev;
    if (mp.first > 0.5) { ev = target_ev; }
    else {
        let rate = select(mp.adapt_down, mp.adapt_up, target_ev > ev);
        ev = ev + (target_ev - ev) * (1.0 - exp(-rate * mp.dt));
    }
    let avg = exp2(ev);
    var exposure = exp2(mp.bias_ev) / clamp(avg / 0.18, mp.min_brightness, mp.max_brightness);
    if (mp.fixed_exposure > 0.0) { exposure = mp.fixed_exposure; }
    state.ev = ev;
    state.target_ev = target_ev;
    state.exposure = exposure;
    state.avg_luminance = avg;
}
