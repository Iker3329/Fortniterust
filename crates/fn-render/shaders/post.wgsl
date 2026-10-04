// Fullscreen post-process: AO, bloom, exposure, vignette, white balance, UE 4.16 filmic tonemapper, sRGB encode.
// Port of post.frag. Order follows UE4's tonemapper (bloom added to scene colour, exposure, vignette, then the colour-grading
// stage = white balance + ACES film path), every stage is an APPROXIMATION unless marked RECOVERED.
// Also contains the HDR mip downsample (bloom + exposure metering) and the exposure-adaptation compute shader.
// Common helpers (srgb) are prepended from common.wgsl.

struct PostParams {
    wb0: vec4<f32>, wb1: vec4<f32>, wb2: vec4<f32>,   // white balance matrix columns (linear sRGB -> linear sRGB)
    film_stw: vec4<f32>,       // slope, toe, shoulder
    film_clip: vec4<f32>,      // black clip, white clip
    bloom: vec4<f32>,          // intensity, threshold, enabled
    vignette: vec4<f32>,       // intensity, enabled
    ao: vec4<f32>,             // intensity (RECOVERED 0.56422), radius metres, enabled, bias
    proj: vec4<f32>,           // tan(half fov x), tan(half fov y), near, far
    view: vec4<f32>,           // width, height, display_exponent, debug_mode
    backdrop: vec4<f32>,       // pause-menu backdrop: dim (0..1), blur (0..1)
}
struct ExposureState { ev: f32, target_ev: f32, exposure: f32, avg_luminance: f32 }

@group(0) @binding(0) var<uniform> pp: PostParams;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_depth_2d;
@group(0) @binding(3) var<storage, read> exposure_state: ExposureState;
@group(0) @binding(4) var lin_samp: sampler;

struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> }   // uv: GL convention (y up)

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    // full-screen triangle: (-1,-1), (3,-1), (-1,3)
    o.clip = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(x, y) * 0.5 + 0.5;
    return o;
}

fn tc(p: vec2<f32>) -> vec2<f32> { return vec2<f32>(p.x, 1.0 - p.y); }   // GL uv -> texture coordinate

// ---- AO (screen space, depth only) ----
fn depth_at(p: vec2<f32>) -> f32 {
    let dims = vec2<f32>(textureDimensions(depth_tex));
    let q = clamp(vec2<i32>(floor(tc(p) * dims)), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    return textureLoad(depth_tex, q, 0);
}
fn lin_depth(p: vec2<f32>) -> f32 {
    let d = depth_at(p);
    let n = pp.proj.z;
    let f = pp.proj.w;
    return n * f / (f - d * (f - n));
}
fn view_pos(p: vec2<f32>, d: f32) -> vec3<f32> {
    let ndc = p * 2.0 - 1.0;
    return vec3<f32>(ndc.x * pp.proj.x * d, ndc.y * pp.proj.y * d, -d);
}
fn ign(p: vec2<f32>) -> f32 { return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715)))); }
fn ambient_occlusion(uv: vec2<f32>, frag: vec2<f32>) -> f32 {
    let d = lin_depth(uv);
    if (d > pp.proj.w * 0.9) { return 1.0; }
    let px = 1.0 / pp.view.xy;
    let p = view_pos(uv, d);
    let dr = lin_depth(uv + vec2<f32>(px.x, 0.0));
    let dl = lin_depth(uv - vec2<f32>(px.x, 0.0));
    let du = lin_depth(uv + vec2<f32>(0.0, px.y));
    let dd = lin_depth(uv - vec2<f32>(0.0, px.y));
    var ex: vec3<f32>;
    if (abs(dr - d) < abs(dl - d)) { ex = view_pos(uv + vec2<f32>(px.x, 0.0), dr) - p; } else { ex = p - view_pos(uv - vec2<f32>(px.x, 0.0), dl); }
    var ey: vec3<f32>;
    if (abs(du - d) < abs(dd - d)) { ey = view_pos(uv + vec2<f32>(0.0, px.y), du) - p; } else { ey = p - view_pos(uv - vec2<f32>(0.0, px.y), dd); }
    var n = normalize(cross(ex, ey));
    if (dot(n, p) > 0.0) { n = -n; }
    let t = normalize(cross(n, select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.y) < 0.9)));
    let b = cross(n, t);
    let radius = pp.ao.y;
    let noise = ign(vec2<f32>(frag.x, pp.view.y - frag.y));
    var occ = 0.0;
    let N = 16;
    for (var i = 0; i < N; i = i + 1) {
        let f = (f32(i) + 0.5) / f32(N);
        let ang = f32(i) * 2.39996323 + noise * 6.2831853;
        let h = sqrt(1.0 - f);                       // hemisphere elevation distribution
        let s = sqrt(1.0 - h * h);
        let dir = t * (cos(ang) * s) + b * (sin(ang) * s) + n * h;
        let sp = p + dir * radius * mix(0.2, 1.0, f * f);
        let suv = vec2<f32>(sp.x / (-sp.z * pp.proj.x), sp.y / (-sp.z * pp.proj.y)) * 0.5 + 0.5;
        if (suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0) { continue; }
        let sd = lin_depth(suv);
        let range = smoothstep(0.0, 1.0, radius / max(abs(d - sd), 1e-4));
        occ = occ + select(0.0, 1.0, sd < -sp.z - pp.ao.w) * range;
    }
    return 1.0 - occ / f32(N);
}

// ---- Bloom: UE4's six-Gaussian structure (sizes % of screen, tints). Sizes/tints are the engine defaults as recalled, NOT
// serialized by Fortnite's day post-process; kernels are mip-chain taps. ----
// Tints RECOVERED (exe ctor + TODM_A: Bloom3Tint 0.5, Bloom6Tint 0 -> sixth kernel off), NOT normalised (UE 4.16: sum(tint*blur) * intensity,
// total gain 1.1165 * 0.675 = 0.754). Mirrors tonemap::BLOOM_TINTS. Threshold luma weights (0.3, 0.59, 0.11) are the literals of
// FPostProcessBloomSetupPS (DXBC-verified).
var<private> BLOOM_SIZE = array<f32, 6>(0.3, 1.0, 2.0, 10.0, 30.0, 64.0);
var<private> BLOOM_TINT = array<f32, 6>(0.3465, 0.138, 0.5, 0.066, 0.066, 0.0);
fn bloom_color(uv: vec2<f32>, exposure: f32) -> vec3<f32> {
    let vp = pp.view.xy;
    let max_lod = floor(log2(max(vp.x, vp.y)));
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 6; i = i + 1) {
        let lod = floor(clamp(log2(max(BLOOM_SIZE[i] * 0.01 * vp.x * 0.5, 1.0)), 0.0, max_lod));
        let texel = 1.0 / vec2<f32>(textureDimensions(scene, i32(lod)));
        var acc = vec3<f32>(0.0);
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let w = select(1.0, 2.0, x == 0) * select(1.0, 2.0, y == 0);
                let c = textureSampleLevel(scene, lin_samp, tc(uv + vec2<f32>(f32(x), f32(y)) * texel * 1.5), lod).rgb * exposure;
                let l = dot(max(c, vec3<f32>(0.0)), vec3<f32>(0.3, 0.59, 0.11));
                acc = acc + c * clamp((l - pp.bloom.y) * 0.5, 0.0, 1.0) * w;
            }
        }
        sum = sum + acc / 16.0 * BLOOM_TINT[i];
    }
    return sum * pp.bloom.x;
}

// ---- Filmic tonemapper curve (UE4 FilmToneMap structure: log-domain toe/straight/shoulder blend) ----
fn film_curve(untonemapped: vec3<f32>) -> vec3<f32> {
    let slope = pp.film_stw.x;
    let toe = pp.film_stw.y;
    let shoulder = pp.film_stw.z;
    let black = pp.film_clip.x;
    let white = pp.film_clip.y;
    let toe_scale = 1.0 + black - toe;
    let shoulder_scale = 1.0 + white - shoulder;
    let in_match = 0.18;
    let out_match = 0.18;
    var toe_match: f32;
    if (toe > 0.8) { toe_match = (1.0 - toe - out_match) / slope + log(in_match) / 2.302585093; }
    else {
        let bt = (out_match + black) / toe_scale - 1.0;
        toe_match = log(in_match) / 2.302585093 - 0.5 * log((1.0 + bt) / (1.0 - bt)) * (toe_scale / slope);
    }
    let straight_match = (1.0 - toe) / slope - toe_match;
    let shoulder_match = shoulder / slope - straight_match;
    let log_color = log(max(untonemapped, vec3<f32>(1e-10))) / 2.302585093;
    let straight = slope * (log_color + straight_match);
    var toe_color = -black + 2.0 * toe_scale / (1.0 + exp(-2.0 * (slope / toe_scale) * (log_color - toe_match)));
    var shoulder_color = (1.0 + white) - 2.0 * shoulder_scale / (1.0 + exp(2.0 * (slope / shoulder_scale) * (log_color - shoulder_match)));
    toe_color = select(straight, toe_color, log_color < vec3<f32>(toe_match));
    shoulder_color = select(straight, shoulder_color, log_color > vec3<f32>(shoulder_match));
    var t = clamp((log_color - toe_match) / (shoulder_match - toe_match), vec3<f32>(0.0), vec3<f32>(1.0));
    if (shoulder_match < toe_match) { t = 1.0 - t; }
    t = (3.0 - 2.0 * t) * t * t;
    return mix(toe_color, shoulder_color, t);
}
// ACES matrices exactly as compiled into the UE 4.16 CombineLUTs shader (WGSL mat3x3 is column-major, so each constructor lists
// the transposed rows of the HLSL dp3 constants). They include the D65 <-> D60 adaptation.
const SRGB_TO_AP0 = mat3x3<f32>(0.439701, 0.089792, 0.017544, 0.382978, 0.813423, 0.111544, 0.177335, 0.096762, 0.870704);
const AP0_TO_AP1 = mat3x3<f32>(1.451439, -0.076554, 0.008316, -0.236511, 1.176230, -0.006032, -0.214929, -0.099676, 0.997716);
const AP1_TO_SRGB = mat3x3<f32>(1.705052, -0.130257, -0.024003, -0.621791, 1.140803, -0.128969, -0.083258, -0.010549, 1.152972);
const AP1_LUMA = vec3<f32>(0.2722287168, 0.6740817658, 0.0536895174);
const RRT_SAT_FACTOR: f32 = 0.96;   // RECOVERED: 0.96 / 0.93 are literals in the 4.16 film path
const ODT_SAT_FACTOR: f32 = 0.93;
// LUT domain: LogToLin(0) .. LogToLin(1), LogToLin(x) = 0.18 * 2^((x - 0.434018) * 14). RECOVERED.
const LUT_MIN: f32 = 0.0026677086;
const LUT_MAX: f32 = 43.707738;

// CombineLUTs colour correction, RECOVERED from the 4.16 CombineLUTs DXBC (shader 1182809): sRGB -> AP1, AP1 luma, shadows / midtones /
// highlights regions (weights 1 - smoothstep(luma / ShadowsMax), smoothstep((luma - HighlightsMin) / (1 - HighlightsMin)), remainder),
// each max(luma + sat * (c - luma), 0). ColorSaturationShadows = Midtones = 1.25 (TODM_A), Highlights 1.0, ShadowsMax 0.09 and
// HighlightsMin 0.5 (exe ctor defaults); contrast/gamma/gain/offset stay identity. Mirrors tonemap::color_grade.
const SRGB_TO_AP1 = mat3x3<f32>(0.613191, 0.070207, 0.020619, 0.339512, 0.916336, 0.109567, 0.047366, 0.013450, 0.869607);
fn color_grade(c: vec3<f32>) -> vec3<f32> {
    let ap1 = SRGB_TO_AP1 * c;
    let luma = dot(ap1, AP1_LUMA);
    let xs = clamp(luma / 0.09, 0.0, 1.0);
    let xh = clamp((luma - 0.5) / 0.5, 0.0, 1.0);
    let sw = 1.0 - xs * xs * (3.0 - 2.0 * xs);
    let hw = xh * xh * (3.0 - 2.0 * xh);
    let mw = 1.0 - sw - hw;
    let d = ap1 - vec3<f32>(luma);
    let shadows = max(vec3<f32>(luma) + d * 1.25, vec3<f32>(0.0));
    let mids = max(vec3<f32>(luma) + d * 1.25, vec3<f32>(0.0));
    let highs = max(ap1, vec3<f32>(0.0));
    return AP1_TO_SRGB * (shadows * sw + mids * mw + highs * hw);
}
// ACES RRT hue-dependent red modifier (RECOVERED, 4.16): pivot 0.03, scale 0.82, hue 0, width 135.
fn aces_red_modifier(aces_in: vec3<f32>) -> vec3<f32> {
    var aces = aces_in;
    let minrgb = min(aces.r, min(aces.g, aces.b));
    let maxrgb = max(aces.r, max(aces.g, aces.b));
    let saturation = (max(maxrgb, 1e-10) - max(minrgb, 1e-10)) / max(maxrgb, 1e-2);
    var hue = 0.0;
    if (!(aces.r == aces.g && aces.g == aces.b)) {
        hue = degrees(atan2(1.7320508 * (aces.g - aces.b), 2.0 * aces.r - aces.g - aces.b));
        if (hue < 0.0) { hue = hue + 360.0; }
    }
    let centered = select(hue, hue - 360.0, hue > 180.0);
    let w = smoothstep(0.0, 1.0, 1.0 - abs(2.0 * centered / 135.0));
    aces.r = aces.r + w * w * saturation * (0.03 - aces.r) * (1.0 - 0.82);
    return aces;
}
// UE ComputeVignetteMask, RECOVERED from the 4.16 tonemap VS/PS: natural cos^4 vignette, circle-space position normalised so the
// screen corner sits at radius sqrt(2) for every aspect: p = sqrt2/sqrt(1+a^2) * (ndc.x, a*ndc.y), a = height/width; mask = 1/(1+|p I|^2)^2.
fn vignette_mask(uv: vec2<f32>) -> f32 {
    let a = pp.view.y / pp.view.x;
    let p = (uv * 2.0 - 1.0) * vec2<f32>(1.0, a) * (1.4142135624 / sqrt(1.0 + a * a)) * pp.vignette.x;
    let t = 1.0 / (1.0 + dot(p, p));
    return t * t;
}

// Pause-menu backdrop blur: trilinear taps of the HDR mip chain (box-filtered), 3x3 kernel at lod = blur * 5.
fn blurred_scene(uv: vec2<f32>) -> vec4<f32> {
    let lod = clamp(pp.backdrop.y, 0.0, 1.0) * 5.0;
    let texel = 1.0 / vec2<f32>(textureDimensions(scene, i32(floor(lod))));
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let w = select(1.0, 2.0, x == 0) * select(1.0, 2.0, y == 0);
            acc = acc + textureSampleLevel(scene, lin_samp, tc(uv + vec2<f32>(f32(x), f32(y)) * texel * 1.5), lod) * w;
            wsum = wsum + w;
        }
    }
    return acc / wsum;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let debug_mode = i32(pp.view.w);
    let exposure = exposure_state.exposure;
    var scene_sample = textureSampleLevel(scene, lin_samp, tc(uv), 0.0);
    let blurred = pp.backdrop.y > 0.0;
    if (blurred) { scene_sample = blurred_scene(uv); }
    var c = scene_sample.rgb;
    var ao = 1.0;
    if (pp.ao.z > 0.5 && !blurred) { ao = ambient_occlusion(uv, in.clip.xy); }
    // UE applies AO to ambient (sky) light only, never to the direct sun: the scene alpha holds the ambient share of each pixel.
    let ambient_share = clamp(scene_sample.a, 0.0, 1.0);
    c = c * (1.0 - ambient_share * pp.ao.x * (1.0 - ao));
    var bloom = vec3<f32>(0.0);
    if (pp.bloom.z > 0.5 && !blurred) { bloom = bloom_color(uv, exposure); }
    c = c * exposure;
    c = c + bloom;
    if (pp.vignette.y > 0.5) { c = c * vignette_mask(uv); }
    if (debug_mode == 5) { return vec4<f32>(vec3<f32>(vignette_mask(uv)), 1.0); }
    if (debug_mode == 2) { return vec4<f32>(vec3<f32>(ao), 1.0); }
    if (debug_mode == 3) { return vec4<f32>(to_srgb(clamp(bloom * 4.0, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0); }
    if (debug_mode == 4) { return vec4<f32>(to_srgb(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0); }
    // The tonemap pass samples the 32^3 colour-grading LUT with saturate(log2(c)/14 + 0.610727): input clamped to the LUT domain
    // before white balance (RECOVERED). Direct evaluation replaces the trilinear 32^3 interpolation; the dither is not applied.
    c = clamp(c, vec3<f32>(LUT_MIN), vec3<f32>(LUT_MAX));
    let wb = mat3x3<f32>(pp.wb0.xyz, pp.wb1.xyz, pp.wb2.xyz);
    c = color_grade(wb * c);
    if (debug_mode == 1) { return vec4<f32>(to_srgb(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0); }
    // UE 4.16 CombineLUTs film path (RECOVERED): sRGB -> AP0, ACES red modifier, AP0 -> AP1 (negatives clamped), RRT saturation 0.96
    // about the AP1 luma, the Film* curve, ODT saturation 0.93, AP1 -> sRGB (negatives clamped).
    var ap0 = SRGB_TO_AP0 * c;
    ap0 = aces_red_modifier(ap0);
    var ap1 = max(AP0_TO_AP1 * ap0, vec3<f32>(0.0));
    ap1 = mix(vec3<f32>(dot(ap1, AP1_LUMA)), ap1, RRT_SAT_FACTOR);
    var toned = film_curve(ap1);
    toned = max(mix(vec3<f32>(dot(toned, AP1_LUMA)), toned, ODT_SAT_FACTOR), vec3<f32>(0.0));
    c = max(AP1_TO_SRGB * toned, vec3<f32>(0.0));
    let enc = to_srgb(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
    return vec4<f32>(pow(enc, vec3<f32>(pp.view.z)) * (1.0 - clamp(pp.backdrop.x, 0.0, 1.0)), 1.0);
}
