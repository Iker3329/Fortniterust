//! Constants and CPU mirrors of the UE 4.16 post-process chain implemented in shaders/post.wgsl and shaders/meter.wgsl.
//!
//! RECOVERED from the original package (identical in 1.8 and 1.10) (TODM_A_Child_Athena.DayPostProcessComponent):
//! WhiteTemp 7366.97, FilmSlope 0.92, FilmToe 0.45, FilmShoulder 0.06422, AutoExposureMinBrightness 0.75,
//! AutoExposureBias 0.47, AmbientOcclusionIntensity 0.56422. The tonemapper structure (LUT domain, white balance, ACES red
//! modifier, RRT/ODT saturation, film curve, cos^4 vignette) was read from the UE 4.16 global shaders (prototype README
//! "Tonemapper (UE 4.16)"). Also RECOVERED (TODM_A parent / exe FPostProcessSettings ctor, see each const): AutoExposureMax 1.0,
//! AO radius 160 cm / bias 2.4 cm, vignette 0.01, bloom threshold -1.0 + tints, ColorSaturationShadows/Midtones 1.25.
//! Not serialized (exe ctor defaults, exe-proven): FilmBlackClip 0, FilmWhiteClip 0.04, bloom intensity 0.675.
//! NOT implemented: depth of field, motion blur, volumetric fog, FXAA.
use bytemuck::{Pod, Zeroable};

pub const WHITE_TEMP: f64 = 7366.97216796875;
pub const FILM_SLOPE: f32 = 0.920_000_02;
pub const FILM_TOE: f32 = 0.449_999_99;
pub const FILM_SHOULDER: f32 = 0.064_219_996;
pub const FILM_BLACK_CLIP: f32 = 0.0;
pub const FILM_WHITE_CLIP: f32 = 0.04;
pub const AUTO_EXPOSURE_MIN: f32 = 0.75;
/// RECOVERED: TODM_A (parent) DayPostProcessComponent AutoExposureMaxBrightness = 1.0 (reports/postfx-evidence.json); the exe ctor
/// default 2.0 is overridden by the Day volume. (Was 2.0: with the sun in view the metered mean rose above 1.0 and darkened the frame.)
pub const AUTO_EXPOSURE_MAX: f32 = 1.0;
pub const AUTO_EXPOSURE_BIAS: f32 = 0.47;
/// Mid-grey reference of UE's auto-exposure (ASSUMED from memory of the 4.18 eye-adaptation shader).
pub const AUTO_EXPOSURE_GREY: f32 = 0.18;
pub const AO_INTENSITY: f32 = 0.564_220_01;
/// RECOVERED: TODM_A (parent) AmbientOcclusionRadius = 160 cm (override flag + tagged value; the exe FPostProcessSettings ctor default 200 cm is replaced).
pub const AO_RADIUS_M: f32 = 1.6;
/// RECOVERED: TODM_A (parent) AmbientOcclusionBias = 2.4 cm (exe ctor default 3.0).
pub const AO_BIAS_M: f32 = 0.024;
pub const BLOOM_INTENSITY: f32 = 0.675;
/// RECOVERED: exe FPostProcessSettings ctor BloomThreshold = -1.0 (reports exe-defaults.json); the Day volume sets the override flag
/// without a tagged value, i.e. it keeps the ctor default (the old 1.0 was recalled from UE source).
pub const BLOOM_THRESHOLD: f32 = -1.0;
/// RECOVERED: TODM_A (parent) VignetteIntensity = 0.01 (exe ctor default 0.4 is overridden).
pub const VIGNETTE_INTENSITY: f32 = 0.01;
/// RECOVERED bloom tints Bloom1..6Tint: 1,2,4,5 = exe ctor values (override flag, no tag), Bloom3Tint 0.5 and Bloom6Tint 0 from TODM_A.
/// UE 4.16 does NOT normalise the tints: bloom = sum(tint_i * blur_i) * BloomIntensity (sum 1.1165 -> total gain 0.754).
pub const BLOOM_TINTS: [f32; 6] = [0.3465, 0.138, 0.5, 0.066, 0.066, 0.0];
/// RECOVERED: TODM_A ColorSaturationShadows / ColorSaturationMidtones = 1.25 (rgb; w = 1); Highlights stay at the exe default 1.0.
pub const COLOR_SATURATION_SHADOWS: f32 = 1.25;
pub const COLOR_SATURATION_MIDTONES: f32 = 1.25;
/// Exe ctor defaults (override flag set, no tag in the Day volume): ColorCorrectionShadowsMax 0.09, ColorCorrectionHighlightsMin 0.5.
pub const COLOR_CORRECTION_SHADOWS_MAX: f32 = 0.09;
pub const COLOR_CORRECTION_HIGHLIGHTS_MIN: f32 = 0.5;
/// UE eye-adaptation speeds (defaults, unproven): stops per second up/down.
pub const ADAPT_UP: f32 = 3.0;
pub const ADAPT_DOWN: f32 = 1.0;
pub const RRT_SAT_FACTOR: f32 = 0.96;
pub const ODT_SAT_FACTOR: f32 = 0.93;
pub const AP1_LUMA: [f32; 3] = [0.272_228_72, 0.674_081_77, 0.053_689_52];

/// Daylight-locus chromaticity for a colour temperature (CIE D-series approximation, valid 4000..25000 K;
/// below 4000 K the Planckian approximation of Kim et al. is used).
pub fn temperature_xy(t: f64) -> (f64, f64) {
    let t = t.clamp(1667., 25000.);
    let x = if t < 4000. { -0.2661239e9 / (t * t * t) - 0.2343589e6 / (t * t) + 0.8776956e3 / t + 0.179910 }
        else if t <= 7000. { -4.6070e9 / (t * t * t) + 2.9678e6 / (t * t) + 0.09911e3 / t + 0.244063 }
        else { -2.0064e9 / (t * t * t) + 1.9018e6 / (t * t) + 0.24748e3 / t + 0.237040 };
    let y = if t < 2222. { -1.1063814 * x * x * x - 1.34811020 * x * x + 2.18555832 * x - 0.20219683 }
        else if t < 4000. { -0.9549476 * x * x * x - 1.37418593 * x * x + 2.09137015 * x - 0.16748867 }
        else { -3.000 * x * x + 2.870 * x - 0.275 };
    (x, y)
}
type M3 = [[f64; 3]; 3];
fn mul3(a: M3, b: M3) -> M3 {
    let mut r = [[0.; 3]; 3];
    for i in 0..3 { for j in 0..3 { for k in 0..3 { r[i][j] += a[i][k] * b[k][j]; } } }
    r
}
fn mulv(a: M3, v: [f64; 3]) -> [f64; 3] { std::array::from_fn(|i| a[i][0] * v[0] + a[i][1] * v[1] + a[i][2] * v[2]) }
const SRGB_TO_XYZ: M3 = [[0.4124564, 0.3575761, 0.1804375], [0.2126729, 0.7151522, 0.0721750], [0.0193339, 0.1191920, 0.9503041]];
fn inv3(m: M3) -> M3 {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let c = |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    [[c(1, 2, 1, 2) / det, -c(0, 2, 1, 2) / det, c(0, 1, 1, 2) / det],
     [-c(1, 2, 0, 2) / det, c(0, 2, 0, 2) / det, -c(0, 1, 0, 2) / det],
     [c(1, 2, 0, 1) / det, -c(0, 2, 0, 1) / det, c(0, 1, 0, 1) / det]]
}
const BRADFORD: M3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
/// Linear-sRGB matrix that maps a scene whose white is at `temp` Kelvin onto the 6500 K reference white
/// (UE's WhiteTemp default is 6500 = identity). Bradford chromatic adaptation. Row-major.
pub fn white_balance_matrix(temp: f64) -> [[f64; 3]; 3] {
    let xyz = |(x, y): (f64, f64)| [x / y, 1., (1. - x - y) / y];
    let src = mulv(BRADFORD, xyz(temperature_xy(temp)));
    let dst = mulv(BRADFORD, xyz(temperature_xy(6500.)));
    let scale = [[dst[0] / src[0], 0., 0.], [0., dst[1] / src[1], 0.], [0., 0., dst[2] / src[2]]];
    mul3(inv3(SRGB_TO_XYZ), mul3(inv3(BRADFORD), mul3(scale, mul3(BRADFORD, SRGB_TO_XYZ))))
}
/// CPU mirror of film_curve() in post.wgsl for one channel (AP1 working space, scene-referred input).
pub fn film_curve(x: f32, slope: f32, toe: f32, shoulder: f32, black: f32, white: f32) -> f32 {
    let toe_scale = 1. + black - toe;
    let shoulder_scale = 1. + white - shoulder;
    let (in_match, out_match) = (0.18f32, 0.18f32);
    let toe_match = if toe > 0.8 { (1. - toe - out_match) / slope + in_match.log10() } else {
        let bt = (out_match + black) / toe_scale - 1.;
        in_match.log10() - 0.5 * ((1. + bt) / (1. - bt)).ln() * (toe_scale / slope)
    };
    let straight_match = (1. - toe) / slope - toe_match;
    let shoulder_match = shoulder / slope - straight_match;
    let l = x.max(1e-10).log10();
    let straight = slope * (l + straight_match);
    let toe_c = if l < toe_match { -black + 2. * toe_scale / (1. + (-2. * (slope / toe_scale) * (l - toe_match)).exp()) } else { straight };
    let sh_c = if l > shoulder_match { (1. + white) - 2. * shoulder_scale / (1. + (2. * (slope / shoulder_scale) * (l - shoulder_match)).exp()) } else { straight };
    let mut t = ((l - toe_match) / (shoulder_match - toe_match)).clamp(0., 1.);
    if shoulder_match < toe_match { t = 1. - t; }
    t = (3. - 2. * t) * t * t;
    toe_c + (sh_c - toe_c) * t
}
/// Exposure scale: 2^bias / clamp(avg / 0.18, min, max) -- Unreal auto-exposure semantics (mirror of meter.wgsl).
pub fn exposure_scale(avg_luminance: f32, min: f32, max: f32, bias_ev: f32) -> f32 {
    let relative = if avg_luminance.is_finite() { avg_luminance / AUTO_EXPOSURE_GREY } else { min };
    2f32.powf(bias_ev) / relative.clamp(min, max)
}
/// Smooth the metered luminance in the log2 domain (stops/second up and down), mirror of meter.wgsl.
pub fn adapt_ev(current: f32, target: f32, dt: f32) -> f32 {
    let rate = if target > current { ADAPT_UP } else { ADAPT_DOWN };
    current + (target - current) * (1. - (-rate * dt).exp())
}
/// UE 4.16 natural (cos^4) vignette mask, RECOVERED from the tonemap VS/PS: p = sqrt(2)/sqrt(1+a^2) * (ndc.x, a*ndc.y) * intensity
/// with a = height/width, mask = 1/(1+|p|^2)^2. `uv` in 0..1.
pub fn vignette_mask(uv: (f32, f32), aspect_h_over_w: f32, intensity: f32) -> f32 {
    let a = aspect_h_over_w;
    let s = 2f32.sqrt() / (1. + a * a).sqrt() * intensity;
    let p = ((uv.0 * 2. - 1.) * s, (uv.1 * 2. - 1.) * a * s);
    let t = 1. / (1. + p.0 * p.0 + p.1 * p.1);
    t * t
}
// ---- CPU mirror of the whole colour path of post.wgsl (everything after exposure; bloom and AO excluded), used by the reference checks ----
pub const LUT_MIN: f32 = 0.002_667_708_6;
pub const LUT_MAX: f32 = 43.707_738;
fn srgb_encode(x: f32) -> f32 { if x <= 0.003_130_8 { x * 12.92 } else { 1.055 * x.powf(1. / 2.4) - 0.055 } }
fn aces_red_modifier(a: glam::Vec3) -> glam::Vec3 {
    let (minrgb, maxrgb) = (a.min_element(), a.max_element());
    let saturation = (maxrgb.max(1e-10) - minrgb.max(1e-10)) / maxrgb.max(1e-2);
    let mut hue = 0.;
    if !(a.x == a.y && a.y == a.z) {
        hue = (1.732_050_8 * (a.y - a.z)).atan2(2. * a.x - a.y - a.z).to_degrees();
        if hue < 0. { hue += 360.; }
    }
    let centered = if hue > 180. { hue - 360. } else { hue };
    let t = (1. - (2. * centered / 135.).abs()).clamp(0., 1.);
    let w = t * t * (3. - 2. * t);
    let mut o = a;
    o.x += w * w * saturation * (0.03 - a.x) * (1. - 0.82);
    o
}
/// CombineLUTs colour correction (RECOVERED from the 4.16 CombineLUTs DXBC, shader 1182809): linear sRGB -> AP1, luma = AP1_LUMA, three
/// regions (shadows weight 1 - smoothstep(luma / ShadowsMax), highlights weight smoothstep((luma - HighlightsMin) / (1 - HighlightsMin)),
/// midtones the remainder), each `max(luma + sat * (c - luma), 0)` (the Day volume leaves contrast / gamma / gain / offset at their identity
/// defaults, so only saturation differs: shadows 1.25, midtones 1.25, highlights 1.0), blended and converted back to linear sRGB.
/// Mirrored by `color_grade` in post.wgsl.
pub fn color_grade(c: glam::Vec3) -> glam::Vec3 {
    use glam::{Mat3, Vec3};
    let to_ap1 = Mat3::from_cols_array(&[0.613191, 0.070207, 0.020619, 0.339512, 0.916336, 0.109567, 0.047366, 0.013450, 0.869607]);
    let from_ap1 = Mat3::from_cols_array(&[1.705052, -0.130257, -0.024003, -0.621791, 1.140803, -0.128969, -0.083258, -0.010549, 1.152972]);
    let ap1 = to_ap1 * c;
    let luma = ap1.dot(Vec3::from(AP1_LUMA));
    let smooth = |x: f32| { let x = x.clamp(0., 1.); x * x * (3. - 2. * x) };
    let sw = 1. - smooth(luma / COLOR_CORRECTION_SHADOWS_MAX);
    let hw = smooth((luma - COLOR_CORRECTION_HIGHLIGHTS_MIN) / (1. - COLOR_CORRECTION_HIGHLIGHTS_MIN));
    let mw = 1. - sw - hw;
    let region = |sat: f32| (Vec3::splat(luma) + (ap1 - Vec3::splat(luma)) * sat).max(Vec3::ZERO);
    let graded = region(COLOR_SATURATION_SHADOWS) * sw + region(COLOR_SATURATION_MIDTONES) * mw + region(1.0) * hw;
    from_ap1 * graded
}
/// CPU reference of the display transform: scene-referred linear RGB (already multiplied by exposure and the vignette mask) -> sRGB-encoded
/// 0..1 colour, exactly the maths of the old post.frag after the vignette (LUT-domain clamp, white balance, ACES red modifier, AP1 saturation,
/// Film* curve, ODT saturation, sRGB encode, display exponent). The dither is not applied (neither runtime applies it).
pub fn display_transform(c_in: [f32; 3], display_exponent: f32) -> [f32; 3] {
    use glam::{Mat3, Vec3};
    let wb = white_balance_matrix(WHITE_TEMP);
    let wbm = Mat3::from_cols_array(&[wb[0][0] as f32, wb[1][0] as f32, wb[2][0] as f32, wb[0][1] as f32, wb[1][1] as f32, wb[2][1] as f32, wb[0][2] as f32, wb[1][2] as f32, wb[2][2] as f32]);
    let srgb_to_ap0 = Mat3::from_cols_array(&[0.439701, 0.089792, 0.017544, 0.382978, 0.813423, 0.111544, 0.177335, 0.096762, 0.870704]);
    let ap0_to_ap1 = Mat3::from_cols_array(&[1.451439, -0.076554, 0.008316, -0.236511, 1.176230, -0.006032, -0.214929, -0.099676, 0.997716]);
    let ap1_to_srgb = Mat3::from_cols_array(&[1.705052, -0.130257, -0.024003, -0.621791, 1.140803, -0.128969, -0.083258, -0.010549, 1.152972]);
    let luma = Vec3::from(AP1_LUMA);
    let mut c = Vec3::from(c_in).clamp(Vec3::splat(LUT_MIN), Vec3::splat(LUT_MAX));
    c = color_grade(wbm * c);
    let ap0 = aces_red_modifier(srgb_to_ap0 * c);
    let mut ap1 = (ap0_to_ap1 * ap0).max(Vec3::ZERO);
    ap1 = Vec3::splat(ap1.dot(luma)).lerp(ap1, RRT_SAT_FACTOR);
    let f = |x: f32| film_curve(x, FILM_SLOPE, FILM_TOE, FILM_SHOULDER, FILM_BLACK_CLIP, FILM_WHITE_CLIP);
    let mut toned = Vec3::new(f(ap1.x), f(ap1.y), f(ap1.z));
    toned = Vec3::splat(toned.dot(luma)).lerp(toned, ODT_SAT_FACTOR).max(Vec3::ZERO);
    c = (ap1_to_srgb * toned).max(Vec3::ZERO).clamp(Vec3::ZERO, Vec3::ONE);
    [srgb_encode(c.x).powf(display_exponent), srgb_encode(c.y).powf(display_exponent), srgb_encode(c.z).powf(display_exponent)]
}
/// Number of mips of the HDR scene target down to 1x1.
pub fn mip_count(w: u32, h: u32) -> u32 { w.max(h).max(1).ilog2() + 1 }

/// GPU parameter block of the post pass (struct PostParams in post.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PostParams {
    pub wb: [[f32; 4]; 3],
    pub film_stw: [f32; 4],
    pub film_clip: [f32; 4],
    pub bloom: [f32; 4],
    pub vignette: [f32; 4],
    pub ao: [f32; 4],
    pub proj: [f32; 4],
    /// width, height, display exponent, debug mode.
    pub view: [f32; 4],
    /// pause-menu backdrop: dim, blur, 0, 0.
    pub backdrop: [f32; 4],
}
/// GPU parameter block of the exposure compute pass (struct MeterParams in meter.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MeterParams {
    pub dt: f32,
    pub min_brightness: f32,
    pub max_brightness: f32,
    pub bias_ev: f32,
    pub first: f32,
    pub fixed_exposure: f32,
    pub adapt_up: f32,
    pub adapt_down: f32,
}
/// White-balance matrix as three padded column vectors for the WGSL mat3x3 constructor (column-major).
pub fn white_balance_columns() -> [[f32; 4]; 3] {
    let m = white_balance_matrix(WHITE_TEMP);
    std::array::from_fn(|c| [m[0][c] as f32, m[1][c] as f32, m[2][c] as f32, 0.])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn white_balance_is_identity_at_6500k_and_cools_warm_light_otherwise() {
        let id = white_balance_matrix(6500.);
        for i in 0..3 { for j in 0..3 { assert!((id[i][j] - if i == j { 1. } else { 0. }).abs() < 1e-6); } }
        // WhiteTemp 7366 K (bluish scene white) is mapped onto 6500 K: red gain > blue gain.
        let m = white_balance_matrix(WHITE_TEMP);
        let white = [m[0].iter().sum::<f64>(), m[1].iter().sum::<f64>(), m[2].iter().sum::<f64>()];
        assert!(white[0] > 1.0 && white[2] < 1.0, "{white:?}");
        // luminance-ish preserved: row sums average close to 1
        assert!(((white[0] + white[1] + white[2]) / 3. - 1.).abs() < 0.1);
    }
    #[test]
    fn film_curve_is_monotonic_and_maps_mid_grey_near_mid_grey() {
        let f = |x: f32| film_curve(x, FILM_SLOPE, FILM_TOE, FILM_SHOULDER, FILM_BLACK_CLIP, FILM_WHITE_CLIP);
        let mut prev = f(1e-4);
        for k in 1..400 {
            let x = 1e-4 * 1.05f32.powi(k);
            let y = f(x);
            assert!(y >= prev - 1e-6, "not monotonic at {x}: {y} < {prev}");
            prev = y;
        }
        let mid = f(0.18);
        assert!((mid - 0.18).abs() < 0.02, "mid grey maps to {mid}");
        assert!(f(100.) <= 1.0 + FILM_WHITE_CLIP + 1e-3);
    }
    #[test]
    fn exposure_matches_the_prototype_semantics() {
        // avg 0.2067 (idle capture of the prototype) -> 2^0.47 / (0.2067/0.18) = 1.2059
        let e = exposure_scale(0.2067, 0.75, 2., 0.47);
        assert!((e - 1.2059).abs() < 2e-3, "{e}");
        // clamped: a dark scene never exceeds 2^bias / 0.75, a bright one never goes below 2^bias / 2
        assert!((exposure_scale(0.001, 0.75, 2., 0.47) - 2f32.powf(0.47) / 0.75).abs() < 1e-5);
        assert!((exposure_scale(100., 0.75, 2., 0.47) - 2f32.powf(0.47) / 2.).abs() < 1e-5);
        assert!((exposure_scale(f32::NAN, 0.75, 2., 0.47) - 2f32.powf(0.47) / 0.75).abs() < 1e-5);
    }
    #[test]
    fn adaptation_is_faster_up_than_down_and_converges() {
        let up = adapt_ev(0., 1., 0.1) - 0.;
        let down = 0. - adapt_ev(0., -1., 0.1);
        assert!(up > down * 2.5);
        let mut ev = 0.;
        for _ in 0..600 { ev = adapt_ev(ev, 2., 1. / 60.); }
        assert!((ev - 2.).abs() < 1e-3);
    }
    #[test]
    fn vignette_corner_is_aspect_independent() {
        // README: the corner mask is 0.574 at intensity 0.4 for every aspect
        for a in [9. / 16., 3. / 4., 1., 9. / 21.] {
            let c = vignette_mask((1., 1.), a, 0.4); // formula check at 0.4; the Day value is VIGNETTE_INTENSITY = 0.01
            assert!((c - 0.574).abs() < 0.002, "a={a}: {c}");
        }
        assert!((vignette_mask((0.5, 0.5), 0.5625, 0.4) - 1.).abs() < 1e-6);
    }
    #[test]
    fn display_transform_maps_mid_grey_and_is_monotonic() {
        let mid = display_transform([0.18; 3], 1.0);
        assert!(mid.iter().all(|v| *v > 0.4 && *v < 0.65), "{mid:?}");
        let (a, b) = (display_transform([0.1; 3], 1.0), display_transform([2.0; 3], 1.0));
        assert!(b[1] > a[1] && b[1] <= 1.0);
        // display exponent < 1 brightens
        assert!(display_transform([0.18; 3], 0.8)[1] > mid[1]);
    }
    #[test]
    fn mip_chain_reaches_1x1() {
        assert_eq!(mip_count(1, 1), 1);
        assert_eq!(mip_count(1280, 720), 11);
        assert_eq!(mip_count(1920, 1080), 11);
        assert_eq!(mip_count(2560, 1440), 12);
    }
    #[test]
    fn gpu_blocks_have_the_declared_sizes() {
        assert_eq!(std::mem::size_of::<PostParams>(), 3 * 16 + 6 * 16 + 16 + 16);
        assert_eq!(std::mem::size_of::<MeterParams>(), 32);
    }
    #[test]
    fn wgsl_carries_the_recovered_constants() {
        let p = include_str!("../shaders/post.wgsl");
        for needle in ["RRT_SAT_FACTOR: f32 = 0.96", "ODT_SAT_FACTOR: f32 = 0.93", "LUT_MIN: f32 = 0.0026677086", "LUT_MAX: f32 = 43.707738", "0.439701, 0.089792, 0.017544", "1.451439, -0.076554, 0.008316", "1.705052, -0.130257, -0.024003"] {
            assert!(p.contains(needle), "post.wgsl lacks {needle}");
        }
        let m = include_str!("../shaders/meter.wgsl");
        assert!(m.contains("0.18") && m.contains("exp2(mp.bias_ev)"));
    }
}
