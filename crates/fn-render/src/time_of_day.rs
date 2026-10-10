//! Time of day: the single source of truth for sun direction, sun colour/intensity, sky-light, height-fog phase values and the
//! phase blend. `lighting.rs` (sun/shadows), `fog.rs`/`volfog.rs` and the post-process crate should READ this instead of keeping
//! their own constants; `sky.rs` reads it for the dome / sun disc.
//!
//! Source of every number: `/Game/TimeOfDay/TODM/TODM_A` (parent Blueprint) merged with `/Game/TimeOfDay/TODM/TODM_A_Child_Athena`
//! (the `WorldTimeOfDayManager` of `Athena_Terrain`; a child serialises only the fields that differ from the parent CDO, so each
//! phase below is parent-then-child merged). Layout of `AFortTimeOfDayManager` / `FDayPhaseInfo` is from the 1.8 SDK dump; the
//! phase-blend maths is ported from the 1.8 exe (`FUN_140631e50` picks (from,to) phases, `FUN_140631d20` computes the blend).
//!
//! # Clock start (exe-verified)
//! The clock starts at [`START_TIME_OF_DAY`] 7.0 (`AFortTimeOfDayManager::PreInitializeComponents` @1406430a0 copies `StartTimeOfDayInGame` into
//! `TimeOfDay`). At match start `AFortGameModeAthena::HandleMatchHasStarted` @1403f78a0 then calls `SetTimeOfDayFloat(rand() * (1 / 32767) * 23.99)`
//! because `bUseRandomTimeOfDay` = true (CookedIniVersion.txt L3294; rdata 0x143fe36ec = 3.0518509e-5, 0x144062898 = 23.99): see
//! [`random_start_hour`] / [`start_match`]. `FN_TOD_HOUR=<h>` overrides the start hour (test / screenshot aid); scripted runs skip the random
//! hour so captures stay deterministic. Call [`set_hour`] to move the clock; every reader sees the change on its next [`current`] call.
//! Everything is plain data; no GPU.
//!
//! # Conventions
//! * Rotators are Unreal (Pitch, Yaw, Roll) degrees; `to_sun` is the unit vector FROM the scene TOWARDS the sun in the renderer
//!   world (X right, Y up, -Z forward), the same convention as `lighting::sun_direction()`.
//! * Colours are LINEAR. `FColor` light colours are converted with the sRGB curve (as `ULightComponent` does).
//! * Radiance follows `lighting::sun_radiance()`: Brightness / pi * linear colour (UE Lambert divides albedo by pi).
use glam::Vec3;
use std::sync::RwLock;

/// Phase indices, in `LightAndFogPhaseSettings` order.
pub const MORNING: usize = 0;
pub const DAY: usize = 1;
pub const EVENING: usize = 2;
pub const NIGHT: usize = 3;

/// `SunriseDirectionalLightRotation` of TODM_A (Pitch, Yaw, Roll deg): serialised bytes 0000 a843 / 0000 f0c1 / 0.
pub const SUNRISE_ROTATION_PYR_DEG: [f32; 3] = [337.0, -30.0, 0.0];
/// `SunsetDirectionalLightRotation` of TODM_A: serialised bytes 0000 4843 / 0000 e041 / 0 (pitch 200 = light travelling 20 deg downwards).
pub const SUNSET_ROTATION_PYR_DEG: [f32; 3] = [200.0, 28.0, 0.0];
/// `StartTimeOfDayInGame` (parent, child inherits) and `DefaultTimeOfDaySpeed`.
pub const START_TIME_OF_DAY: f32 = 7.0;
pub const DEFAULT_TIME_OF_DAY_SPEED: f32 = 80.0;
/// A pure-Evening test hour (see [`blend_at`]: Evening is 100 % between 19.6 h and 21.4 h) with the sun ~19 deg above the horizon. NOT the
/// default clock any more (that was an old user test lock, lifted 2026-10-05): tests and captures use it as a fixed sunset reference.
pub const SUNSET_HOUR: f32 = 20.5;

/// One merged `FDayPhaseInfo` (parent TODM_A overlaid with TODM_A_Child_Athena).
#[derive(Clone, Copy, Debug)]
pub struct PhaseData {
    pub name: &'static str,
    /// `TimePhaseBegins` (hours). Night's is not serialised anywhere (struct default); derived as Morning begin - Night length.
    pub begins: f32,
    pub length_hours: f32,
    pub pct_transition_in: f32,
    pub pct_transition_out: f32,
    /// `DirectionalLightValues.LightColor` (FColor, sRGB bytes) and `Brightness`.
    pub sun_color_srgb: [u8; 3],
    pub sun_brightness: f32,
    /// `SkyLightValues.SkyLightColor` rgb (linear) and its alpha (meaning of alpha unverified: Day 1.0, Evening 0.8, Night 0.75).
    pub sky_light_color: [f32; 3],
    pub sky_light_alpha: f32,
    pub fog: FogPhase,
}

/// `FExponentialHeightFogValues` of a phase (merged).
#[derive(Clone, Copy, Debug)]
pub struct FogPhase {
    pub density: f32,
    pub height_falloff: f32,
    pub max_opacity: f32,
    pub start_distance: f32,
    pub directional_inscattering_exponent: f32,
    pub directional_inscattering_start_distance: f32,
    pub directional_inscattering_color: [f32; 3],
    pub inscattering_color: [f32; 3],
    pub volumetric_scattering_distribution: f32,
    /// Not serialised for Day in the child (parent: 0.9): the child value 0.1 is tagged, see fog.rs.
    pub volumetric_scattering_scale: f32,
}

/// Merged phase table. Evening/Night `begins` come from the parent (19) / are derived (22).
pub const PHASES: [PhaseData; 4] = [
    PhaseData {
        name: "Morning", begins: 3.0, length_hours: 4.0, pct_transition_in: 0.356_428, pct_transition_out: 0.5,
        sun_color_srgb: [255, 187, 153], sun_brightness: 1.5,
        sky_light_color: [0.83, 0.728_359, 0.587_762], sky_light_alpha: 0.7,
        fog: FogPhase {
            density: 0.0002, height_falloff: 0.09, max_opacity: 0.2, start_distance: 0.0,
            directional_inscattering_exponent: 2.0, directional_inscattering_start_distance: 0.0,
            directional_inscattering_color: [0.45, 0.154_971, 0.069_128], inscattering_color: [0.73, 0.282_014, 0.179_708],
            volumetric_scattering_distribution: 0.2, volumetric_scattering_scale: 0.1,
        },
    },
    PhaseData {
        name: "Day", begins: 7.0, length_hours: 12.0, pct_transition_in: 0.116, pct_transition_out: 0.2,
        sun_color_srgb: [255, 225, 161], sun_brightness: 3.0,
        sky_light_color: [3.575, 4.175_99, 5.0], sky_light_alpha: 1.0,
        fog: FogPhase {
            density: 0.00022, height_falloff: 0.09, max_opacity: 0.8, start_distance: 0.0,
            directional_inscattering_exponent: 3.0, directional_inscattering_start_distance: 10000.0,
            directional_inscattering_color: [0.027_802, 0.023_625, 0.035], inscattering_color: [0.110_45, 0.277_685, 0.47],
            volumetric_scattering_distribution: 0.05, volumetric_scattering_scale: 0.1,
        },
    },
    PhaseData {
        // Child overrides: SkyLight, light colour+Brightness 1, fog density/falloff/opacity/exponent/colours. Parent: begin 19, length 3,
        // transition 20 % in / 20 % out (36 min each), VolumetricFogScatteringDistribution 0.34 and Scale 0.6.
        name: "Evening", begins: 19.0, length_hours: 3.0, pct_transition_in: 0.2, pct_transition_out: 0.2,
        sun_color_srgb: [255, 174, 114], sun_brightness: 1.0,
        sky_light_color: [0.735, 0.574_911, 0.319_725], sky_light_alpha: 0.8,
        fog: FogPhase {
            density: 0.0003, height_falloff: 0.08, max_opacity: 1.0, start_distance: 0.0,
            directional_inscattering_exponent: 5.0, directional_inscattering_start_distance: 0.0,
            directional_inscattering_color: [0.325, 0.033_422, 0.0], inscattering_color: [0.62, 0.135_486, 0.073_669],
            volumetric_scattering_distribution: 0.34, volumetric_scattering_scale: 0.6,
        },
    },
    PhaseData {
        // Child overrides: length 5, transition 30 min in / 7.5 min out, SkyLight, light colour (Brightness NOT overridden: parent 8),
        // fog density/opacity/colours. Parent: transition 10 % in / 2.5 % out, exponent 5, start distance 2500.
        name: "Night", begins: 22.0, length_hours: 5.0, pct_transition_in: 0.1, pct_transition_out: 0.025,
        sun_color_srgb: [76, 97, 177], sun_brightness: 8.0,
        sky_light_color: [0.3828, 0.461_439, 0.66], sky_light_alpha: 0.75,
        fog: FogPhase {
            density: 0.0013, height_falloff: 0.09, max_opacity: 0.9, start_distance: 0.0,
            directional_inscattering_exponent: 5.0, directional_inscattering_start_distance: 2500.0,
            directional_inscattering_color: [0.010_201, 0.005_591, 0.03], inscattering_color: [0.016_433, 0.015, 0.1],
            // VolumetricFogScatteringScale is untagged for Night in both packages: the exe struct default 1.0 (fog.rs PhaseFog::NIGHT agrees; was 0.0)
            volumetric_scattering_distribution: 0.3, volumetric_scattering_scale: 1.0,
        },
    },
];

/// `M_AridSky_Inst_<phase>` (the child's `StarMapMaterialInstance`) parameters of the sky dome. Read from the 1.8 paks with `fn-inspect props
/// /Game/Packages/Fortress_Sky/MaterialInstance/Arid/M_AridSky_Inst_<Phase>`. Parameters an instance does not override take the parent
/// `M_AridSky` default: Horiz-X 2, Horiz-Add -1 (both phases) and, for Day only, Horiz-Pow 5 / Star Brightness 0 (tagged).
#[derive(Clone, Copy, Debug)]
pub struct SkyDome {
    pub horizon: [f32; 3],
    pub zenith: [f32; 3],
    pub star_brightness: f32,
    pub horiz_x: f32,
    pub horiz_add: f32,
    pub horiz_pow: f32,
}
pub const SKY_DOME: [SkyDome; 4] = [
    SkyDome { horizon: [2.0, 0.890_85, 0.589_936], zenith: [0.004_219, 0.000_85, 0.01], star_brightness: 0.05, horiz_x: 2.0, horiz_add: -1.0, horiz_pow: 6.0 },
    SkyDome { horizon: [0.274_565, 0.611_981, 1.0], zenith: [0.0, 0.013_41, 0.03], star_brightness: 0.0, horiz_x: 2.0, horiz_add: -1.0, horiz_pow: 5.0 },
    SkyDome { horizon: [2.0, 0.990_22, 0.28], zenith: [0.075, 0.004_958, 0.0], star_brightness: 0.05, horiz_x: 2.0, horiz_add: -1.0, horiz_pow: 7.0 },
    SkyDome { horizon: [0.622_665, 0.619_793, 10.0], zenith: [0.000_044, 0.000_091, 0.001], star_brightness: 0.1, horiz_x: 2.0, horiz_add: -1.0, horiz_pow: 12.0 },
];

/// `M_AridClouds_Inst_<phase>` (the child's `SkyMaterialInstance`) parameters that differ per phase (everything else is the parent default,
/// the same in all four: see `sky_params::ARIDCLOUDS_DAY`).
#[derive(Clone, Copy, Debug)]
pub struct CloudPhase {
    pub sun_tint: [f32; 3],
    pub normal_modulator: [f32; 3],
    pub ground_cloud: [f32; 3],
    pub direct_sun_light: [f32; 3],
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    pub fog_color: [f32; 3],
    pub opacity_master: f32,
}
pub const CLOUDS: [CloudPhase; 4] = [
    CloudPhase { sun_tint: [1.0, 0.841_067, 0.738_751], normal_modulator: [1.0; 3], ground_cloud: [0.26, 0.075_907, 0.036_895], direct_sun_light: [1.0, 0.675_995, 0.555],
        zenith: [1.0, 0.837_366, 0.756_152], horizon: [0.38, 0.063_928, 0.029_85], fog_color: [1.0, 0.312_468, 0.162_381], opacity_master: 2.0 },
    CloudPhase { sun_tint: [1.0; 3], normal_modulator: [1.0; 3], ground_cloud: [0.665; 3], direct_sun_light: [1.0; 3], zenith: [1.0; 3], horizon: [30.0; 3], fog_color: [1.0; 3], opacity_master: 0.4 },
    CloudPhase { sun_tint: [1.0, 0.484_868, 0.374_826], normal_modulator: [1.0; 3], ground_cloud: [0.65, 0.203_665, 0.0], direct_sun_light: [5.0, 2.023_475, 0.025],
        zenith: [0.54, 0.123_294, 0.036_885], horizon: [0.91, 0.359_639, 0.3003], fog_color: [1.0, 0.312_468, 0.162_381], opacity_master: 1.0 },
    CloudPhase { sun_tint: [0.106_811, 0.0675, 0.3], normal_modulator: [1.0; 3], ground_cloud: [0.034_65, 0.088_041, 0.63], direct_sun_light: [0.332_601, 0.379_165, 1.0],
        zenith: [0.015_223, 0.011_018, 0.1], horizon: [7.056_202, 1.200_001, 20.0], fog_color: [0.337_21, 0.133, 0.475], opacity_master: 2.0 },
];

fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
impl PhaseData {
    pub fn sun_color_linear(&self) -> [f32; 3] { std::array::from_fn(|i| srgb_to_linear(self.sun_color_srgb[i])) }
}

/// Phase blend at `hour`: (from, to, t) with `t` the smoothstep weight of `to`. Port of the exe's `FUN_140631e50`/`FUN_140631d20`:
/// the transition from phase A to the next phase B spans `[B.begins - A.pctOut*A.length, B.begins + B.pctIn*B.length]`, blend = smoothstep.
pub fn blend_at(hour: f32) -> (usize, usize, f32) {
    let h = hour.rem_euclid(24.);
    // Evaluate the 4 transitions, the window that contains `h` (mod 24) wins.
    for a in 0..4 {
        let b = (a + 1) & 3;
        let (pa, pb) = (&PHASES[a], &PHASES[b]);
        let out_h = pa.pct_transition_out * pa.length_hours;
        let in_h = pb.pct_transition_in * pb.length_hours;
        let total = out_h + in_h;
        if total <= 0. { continue; }
        let start = (pb.begins - out_h).rem_euclid(24.);
        let mut d = h - start;
        if d < 0. { d += 24.; }
        if d < total {
            let x = (d / total).min(1.);
            return (a, b, x * x * (3. - 2. * x));
        }
    }
    // Not inside any transition: the phase whose [begins, next.begins) contains h.
    for a in 0..4 {
        let b = (a + 1) & 3;
        let mut d = h - PHASES[a].begins;
        if d < 0. { d += 24.; }
        let mut span = PHASES[b].begins - PHASES[a].begins;
        if span <= 0. { span += 24.; }
        if d < span { return (a, a, 0.); }
    }
    (NIGHT, NIGHT, 0.)
}

/// Object rotations of the sun/moon model at sunrise / sunset: NOT serialised in TODM_A / the child, so these are the `AFortTimeOfDayManager`
/// constructor defaults (exe `AFortTimeOfDayManager::AFortTimeOfDayManager` @ 0x14062c240, stores at +0x40c / +0x418 / +0x424).
pub const SUNRISE_OBJECT_ROTATION_PYR_DEG: [f32; 3] = [370.0, -30.0, 0.0];
pub const SUNSET_OBJECT_ROTATION_PYR_DEG: [f32; 3] = [170.0, 28.0, 0.0];
/// `SunObjectDivergencePower` (ctor default, not serialised).
pub const SUN_OBJECT_DIVERGENCE_POWER: f32 = 5.0;

/// Day-progress `p` of the sun arc (port of `FUN_140631920`, called by the sun/moon positioning code `FUN_140641e30`): 0 at the earliest phase
/// start (Morning, 3 h) rising linearly to 1 at the latest phase start (Night, 22 h), then falling back 1 -> 0 across the Night phase
/// (so the moon retraces the arc). Hours wrap at 24.
pub fn sun_arc_progress(hour: f32) -> f32 {
    let t = hour.rem_euclid(24.);
    let first = PHASES.iter().map(|p| p.begins).fold(f32::MAX, f32::min); // 3
    let last = PHASES.iter().map(|p| p.begins).fold(f32::MIN, f32::max); // 22
    let last_len = PHASES.iter().find(|p| p.begins == last).unwrap().length_hours; // 5
    if t >= first && t < last {
        (t - first) / (24. - last_len)
    } else if t >= last {
        1. - (t - last) / last_len
    } else {
        1. - ((24. - last) + t) / last_len
    }
}

/// `FRotator::Quaternion()` (Unreal formula).
pub fn ue_quat(pyr_deg: [f32; 3]) -> glam::Quat {
    let (sp, cp) = (pyr_deg[0].to_radians() * 0.5).sin_cos();
    let (sy, cy) = (pyr_deg[1].to_radians() * 0.5).sin_cos();
    let (sr, cr) = (pyr_deg[2].to_radians() * 0.5).sin_cos();
    glam::Quat::from_xyzw(cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy)
}
/// SUN SLERP (2026-10-07): `FQuat::SlerpFullPath_NotNormalized` (exe 0x1412f1c60 = UE4.16 UnrealMath.cpp:1219): NO hemisphere flip (glam's `slerp` negates
/// `b` when the dot is negative = UE's shortest-path `Slerp_NotNormalized`, a different rotation), cos clamped to [-1, 1], `|angle| < KINDA_SMALL_NUMBER` -> `a`.
pub fn slerp_full_path_not_normalized(a: glam::Quat, b: glam::Quat, t: f32) -> glam::Quat {
    let cos = a.dot(b).clamp(-1., 1.);
    let angle = cos.acos();
    if angle.abs() < 1.0e-4 { return a; }
    let inv = 1. / angle.sin();
    let (s0, s1) = (((1. - t) * angle).sin() * inv, (t * angle).sin() * inv);
    glam::Quat::from_xyzw(a.x * s0 + b.x * s1, a.y * s0 + b.y * s1, a.z * s0 + b.z * s1, a.w * s0 + b.w * s1)
}
/// `FQuat::GetNormalized(SMALL_NUMBER)` as the exe inlines it after each slerp (FUN_140631bd0 / FUN_140641e30: rsqrt of the square sum when it is >= 1e-8,
/// else the identity).
pub fn quat_get_normalized(q: glam::Quat) -> glam::Quat {
    let s = q.length_squared();
    if s >= 1.0e-8 { q * (1. / s.sqrt()) } else { glam::Quat::IDENTITY }
}
/// `FQuat::RotateVector((1, 0, 0))`: the forward axis (Unreal convention) a rotation points along.
pub fn quat_forward(q: glam::Quat) -> Vec3 {
    let (qv, w) = (Vec3::new(q.x, q.y, q.z), q.w);
    let t = 2. * qv.cross(Vec3::X);
    (Vec3::X + w * t + qv.cross(t)).normalize()
}

/// Unreal-space (X forward, Y right, Z up) direction the sun MODEL's rotation points along at `hour`, ported from `FUN_140641e30`
/// (`PositionTheSunAndMoonModels`): `Q = SlerpFullPath(N(SlerpFullPath(SunriseLight, SunriseObject, d)), N(SlerpFullPath(SunsetLight, SunsetObject, d)), p)`
/// (each result `GetNormalized`), `p = sun_arc_progress(hour)`, `d = (|p - 0.5| * 2) ^ SunObjectDivergencePower` (constants 0.5 / 2.0 read from the exe's .rdata).
/// SUN SLERP (2026-10-07): all three slerps are `FQuat::SlerpFullPath_NotNormalized` (exe calls at +86 / +138 / +180 lines of the function); was glam's
/// shortest-path slerp.
pub fn light_travel_direction_ue(hour: f32) -> Vec3 {
    let p = sun_arc_progress(hour);
    let d = ((p - 0.5).abs() * 2.).powf(SUN_OBJECT_DIVERGENCE_POWER);
    let a = quat_get_normalized(slerp_full_path_not_normalized(ue_quat(SUNRISE_ROTATION_PYR_DEG), ue_quat(SUNRISE_OBJECT_ROTATION_PYR_DEG), d));
    let b = quat_get_normalized(slerp_full_path_not_normalized(ue_quat(SUNSET_ROTATION_PYR_DEG), ue_quat(SUNSET_OBJECT_ROTATION_PYR_DEG), d));
    quat_forward(quat_get_normalized(slerp_full_path_not_normalized(a, b, p)))
}

/// MATERIAL PARAMETERS (2026-10-06): `AFortTimeOfDayManager::DistanceToSunOrMoon` (+0x428, cm): the constructor default (exe @0x14062cd04
/// `mov dword ptr [rdi+428h], 4AF1B300h` = 7.92e6), not serialised by TODM_A / TODM_A_Child_Athena. `PositionTheSunAndMoonModels` (@0x140641e30) puts the
/// sun / moon model at `dir * DistanceToSunOrMoon` (world) and writes that to `FortniteMaterialParameters.SunAndMoonModelWorldPosition`.
pub const DISTANCE_TO_SUN_OR_MOON_CM: f32 = 7_920_000.0;
/// MATERIAL PARAMETERS: Unreal-space unit vector from the world origin to the sun / moon MODEL at `hour`: `PositionTheSunAndMoonModels` negates
/// `FRotator(Q).Vector()` of the slerped rotation ([`light_travel_direction_ue`]). It is also the value of the collection's `SunlightDirectionalVector` and
/// `SunAndMoonModelDirectionalVector`.
pub fn sun_model_direction_ue(hour: f32) -> Vec3 { -light_travel_direction_ue(hour) }

/// Sun-light rotation (Pitch, Yaw, Roll = 0 deg) at `hour`, derived from [`light_travel_direction_ue`].
pub fn sun_rotation_pyr_deg_at(hour: f32) -> [f32; 3] {
    let l = light_travel_direction_ue(hour);
    [l.z.asin().to_degrees(), l.y.atan2(l.x).to_degrees(), 0.]
}

/// Direction the light travels, from an Unreal rotator (FRotator::Vector()), in the renderer world.
pub fn light_travel_direction(pitch_deg: f32, yaw_deg: f32) -> Vec3 {
    let (p, y) = (pitch_deg.to_radians(), yaw_deg.to_radians());
    let ue = Vec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin());
    crate::lighting::ue_to_world(ue).normalize() // SUN-AXIS FIX: was a private (y, z, -x) copy; one canonical map (lighting::ue_to_world)
}

/// Everything a consumer needs at one instant.
#[derive(Clone, Copy, Debug)]
pub struct TodState {
    pub hour: f32,
    /// Blend (from phase, to phase, weight of `to`).
    pub from: usize,
    pub to: usize,
    pub blend: f32,
    pub sun_rotation_pyr_deg: [f32; 3],
    /// Unit vector from the scene towards the sun (renderer world).
    pub to_sun: Vec3,
    /// Linear light colour (0..1) and Brightness (the light component's Intensity) of the blended phases.
    pub sun_color: [f32; 3],
    pub sun_brightness: f32,
    /// `sun_color * brightness / pi`, what the lit shaders multiply with N.L.
    pub sun_radiance: [f32; 3],
    /// Linear SkyLightColor (blended) and alpha.
    pub sky_light_color: [f32; 3],
    pub sky_light_alpha: f32,
    pub fog: FogPhase,
    /// Sky dome and cloud-dome material parameters, blended with the same (from,to,t) as everything else (UE interpolates the MID params linearly).
    pub sky_dome: SkyDome,
    pub clouds: CloudPhase,
    /// Per-phase weights [morning, day, evening, night] (sum 1) for consumers that mix per-phase assets (sky materials, post-process).
    pub weights: [f32; 4],
}

fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] { std::array::from_fn(|i| lerp(a[i], b[i], t)) }
fn lerp_fog(a: &FogPhase, b: &FogPhase, t: f32) -> FogPhase {
    FogPhase {
        density: lerp(a.density, b.density, t),
        height_falloff: lerp(a.height_falloff, b.height_falloff, t),
        max_opacity: lerp(a.max_opacity, b.max_opacity, t),
        start_distance: lerp(a.start_distance, b.start_distance, t),
        directional_inscattering_exponent: lerp(a.directional_inscattering_exponent, b.directional_inscattering_exponent, t),
        directional_inscattering_start_distance: lerp(a.directional_inscattering_start_distance, b.directional_inscattering_start_distance, t),
        directional_inscattering_color: lerp3(a.directional_inscattering_color, b.directional_inscattering_color, t),
        inscattering_color: lerp3(a.inscattering_color, b.inscattering_color, t),
        volumetric_scattering_distribution: lerp(a.volumetric_scattering_distribution, b.volumetric_scattering_distribution, t),
        volumetric_scattering_scale: lerp(a.volumetric_scattering_scale, b.volumetric_scattering_scale, t),
    }
}

fn lerp_dome(a: &SkyDome, b: &SkyDome, t: f32) -> SkyDome {
    SkyDome { horizon: lerp3(a.horizon, b.horizon, t), zenith: lerp3(a.zenith, b.zenith, t), star_brightness: lerp(a.star_brightness, b.star_brightness, t),
        horiz_x: lerp(a.horiz_x, b.horiz_x, t), horiz_add: lerp(a.horiz_add, b.horiz_add, t), horiz_pow: lerp(a.horiz_pow, b.horiz_pow, t) }
}
fn lerp_clouds(a: &CloudPhase, b: &CloudPhase, t: f32) -> CloudPhase {
    CloudPhase { sun_tint: lerp3(a.sun_tint, b.sun_tint, t), normal_modulator: lerp3(a.normal_modulator, b.normal_modulator, t), ground_cloud: lerp3(a.ground_cloud, b.ground_cloud, t),
        direct_sun_light: lerp3(a.direct_sun_light, b.direct_sun_light, t), zenith: lerp3(a.zenith, b.zenith, t), horizon: lerp3(a.horizon, b.horizon, t),
        fog_color: lerp3(a.fog_color, b.fog_color, t), opacity_master: lerp(a.opacity_master, b.opacity_master, t) }
}

/// State at an arbitrary hour (pure).
pub fn state_at(hour: f32) -> TodState {
    let (from, to, t) = blend_at(hour);
    let (a, b) = (&PHASES[from], &PHASES[to]);
    let l = light_travel_direction_ue(hour);
    let rot = [l.z.asin().to_degrees(), l.y.atan2(l.x).to_degrees(), 0.];
    let to_sun = -crate::lighting::ue_to_world(l); // SUN-AXIS FIX: was a private (y, z, -x) copy (90 deg yaw off the props); see lighting::ue_to_world
    let sun_color = lerp3(a.sun_color_linear(), b.sun_color_linear(), t);
    let sun_brightness = lerp(a.sun_brightness, b.sun_brightness, t);
    let k = sun_brightness / std::f32::consts::PI;
    let mut weights = [0.; 4];
    weights[from] += 1. - t;
    weights[to] += t;
    TodState {
        hour, from, to, blend: t, sun_rotation_pyr_deg: rot, to_sun, sun_color, sun_brightness,
        sun_radiance: std::array::from_fn(|i| sun_color[i] * k),
        sky_light_color: lerp3(a.sky_light_color, b.sky_light_color, t),
        sky_light_alpha: lerp(a.sky_light_alpha, b.sky_light_alpha, t),
        fog: lerp_fog(&a.fog, &b.fog, t),
        sky_dome: lerp_dome(&SKY_DOME[from], &SKY_DOME[to], t),
        clouds: lerp_clouds(&CLOUDS[from], &CLOUDS[to], t),
        weights,
    }
}

/// `TimeOfDay` of the manager: [`START_TIME_OF_DAY`] after `PreInitializeComponents` (@1406430a0), until [`start_match`] randomises it.
static CLOCK: RwLock<f32> = RwLock::new(CLOCK_START);
#[cfg(not(test))]
const CLOCK_START: f32 = START_TIME_OF_DAY;
/// This crate's unit tests (sky / fog) read the global clock at the fixed [`SUNSET_HOUR`] reference, so they never depend on the start value.
#[cfg(test)]
const CLOCK_START: f32 = SUNSET_HOUR;
/// True when `FN_TOD_HOUR` fixed the start hour ([`start_match`] then keeps it).
static HOUR_FROM_ENV: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// One-shot `FN_TOD_HOUR=<hours>` override of the start hour (test / screenshot aid, never set in play); applied on the first clock access.
fn init_clock() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        if let Some(h) = fn_live::var("FN_TOD_HOUR").ok().and_then(|v| v.trim().parse::<f32>().ok()).filter(|h| h.is_finite()) {
            *CLOCK.write().unwrap() = h.rem_euclid(24.);
            HOUR_FROM_ENV.store(true, std::sync::atomic::Ordering::Relaxed);
            *SPEED.write().unwrap() = 0.0;
        }
        // TOD LIVE: `FN_TOD_SPEED=<in-game seconds per real second>` runs the clock (see SPEED)
        if let Some(s) = fn_live::var("FN_TOD_SPEED").ok().and_then(|v| v.trim().parse::<f32>().ok()).filter(|s| s.is_finite() && *s >= 0.) { *SPEED.write().unwrap() = s; }
    });
}
/// TOD LIVE (2026-10-05): `TimeOfDaySpeed` of the clock (in-game seconds per real second; the exe's `AFortTimeOfDayManager::PostInitializeComponents` sets it
/// to `DefaultTimeOfDaySpeed`, [`DEFAULT_TIME_OF_DAY_SPEED`] = 80 for TODM_A, when that is >= 0). The port runs at the exe rate by default (the user
/// lifted the old fixed-sunset lock 2026-10-05); `FN_TOD_SPEED=<speed>` overrides it (0 freezes the clock), and scripted runs freeze it unless set.
static SPEED: RwLock<f32> = RwLock::new(DEFAULT_TIME_OF_DAY_SPEED);
/// The clock speed ([`SPEED`]).
pub fn speed() -> f32 { init_clock(); *SPEED.read().unwrap() }
/// Set the clock speed (in-game seconds per real second; negative values are refused like `SetTimeOfDaySpeed`).
pub fn set_speed(s: f32) { init_clock(); if s >= 0.0 && s.is_finite() { *SPEED.write().unwrap() = s; } }
/// The clock advance of the TOD manager tick (`AFortTimeOfDayManager::Tick` -> `FUN_14064fdb0`): `TimeOfDay += TimeOfDaySpeed * DeltaSeconds / 3600`
/// (rdata 1/3600 = 0.00027777778), wrapped with `FMath::Fmod(TimeOfDay, 24)` (`(float)(int)(t * (1/24)) * 24` subtracted unless larger than `t`) and
/// +24 when negative. The tick then re-blends every phase value from the new hour (`FUN_14064e920`), which here is every reader's next [`current`] call.
pub fn advance(hour: f32, speed: f32, dt: f32) -> f32 {
    let t = speed * dt * 0.000_277_777_78 + hour;
    let q = (t * 0.041_666_668) as i32 as f32 * 24.0;
    let mut t = t - if t.abs() < q.abs() { t } else { q };
    if t < 0.0 { t += 24.0; }
    t
}
/// One tick of the live clock with the frame's world delta time (no-op at speed 0, the default).
pub fn tick(dt: f32) {
    let s = speed();
    if s == 0.0 || !(dt > 0.0) { return; }
    let mut c = CLOCK.write().unwrap();
    let before = *c;
    *c = advance(*c, s, dt);
    if before as i32 != *c as i32 { let (a, b, t) = blend_at(*c); eprintln!("[tod] {:.2} h (speed {s}): {} -> {} {:.0} %", *c, PHASES[a].name, PHASES[b].name, t * 100.); }
}
/// Current clock hour (0..24).
pub fn hour() -> f32 { init_clock(); *CLOCK.read().unwrap() }
/// Move the clock (hours, wraps at 24). Every reader picks it up on its next [`current`] call.
pub fn set_hour(h: f32) { init_clock(); *CLOCK.write().unwrap() = h.rem_euclid(24.); }
/// `AFortTimeOfDayManager::SetTimeOfDayFloat` (@140648960): accepts only `0 <= h < 24` (rdata 0x144031d5c = 24.0), otherwise the call is ignored.
pub fn set_time_of_day_float(h: f32) -> bool { if (0.0..24.0).contains(&h) { set_hour(h); true } else { false } }
/// The start hour `HandleMatchHasStarted` (@1403f78a0) computes from one CRT `rand()` value (0..=32767): `(float)r * (1/32767) * 23.99`, in f32 like the exe.
pub fn random_start_hour(rand: i32) -> f32 { rand as f32 * 3.051_850_9e-5_f32 * 23.99_f32 }
/// MSVC CRT `rand()` (`seed = seed * 214013 + 2531011; return (seed >> 16) & 0x7fff`) seeded like UE 4.16 `FMath::RandInit(FPlatformTime::Cycles())`
/// (LaunchEngineLoop.cpp:1250-1256, not fixed-seed). The exe's sequence position at match start is not reproducible (other `rand()` callers run first),
/// so this is one draw from a time-seeded generator.
fn crt_rand_time_seeded() -> i32 {
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u32);
    ((seed.wrapping_mul(214_013).wrapping_add(2_531_011) >> 16) & 0x7fff) as i32
}
/// Match start of `AFortGameModeAthena::HandleMatchHasStarted`: with `bUseRandomTimeOfDay` (true in the cooked ini) the clock jumps to a random hour
/// 0..23.99. Skipped (returns `None`) for `deterministic` runs (scripted captures) and when `FN_TOD_HOUR` set the start hour.
pub fn start_match(deterministic: bool) -> Option<f32> {
    init_clock();
    if deterministic || HOUR_FROM_ENV.load(std::sync::atomic::Ordering::Relaxed) { return None; }
    let h = random_start_hour(crt_rand_time_seeded());
    set_time_of_day_float(h).then_some(h)
}

/// `DAT_143ff9024` = 4.0 (read from the exe .rdata): the priority step of the TOD post-process components.
pub const PP_PRIORITY_STEP: f32 = 4.0;
/// One post-process component of the TOD manager as `UWorld::PostProcessVolumes` sees it (all `bEnabled`, `bUnbound`: no extent, no blend radius).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PpComponent {
    /// Phase whose `FPostProcessSettings` the component carries (`MORNING..NIGHT`).
    pub phase: usize,
    /// `true` for the `<Phase>LowPriClonedPPC` copies (Day, Evening, Night only).
    pub clone: bool,
    pub priority: f32,
    /// `BlendWeight` as the TOD manager set it (DoPostProcessVolume clamps it to 0..1; <= 0 skips the component).
    pub weight: f32,
}
/// The seven TOD post-process components with their priorities and blend weights for the blend (`from`, `to`, `t`) of [`blend_at`], in REGISTRATION
/// order (the ctor's `CreateDefaultSubobject` order: Morning, Day, Evening, Night, then the Day / Evening / Night clones).
/// * ctor `AFortTimeOfDayManager` @14062c240: `DayPhasePostProcessComponent[i].Priority = i - 4` (Morning -4 .. Night -1), `bUnbound` (|= 2 at +0x76c);
///   `<Phase>LowPriClonedPPC` for i = 1..3 with `Priority = Priority[i] - 4` (Day -7, Evening -6, Night -5). The cooked TODM_A / TODM_A_Child_Athena
///   components serialise no Priority / bUnbound (only Settings, and Evening a BlendWeight 0 that the manager overwrites), so the ctor values stand.
/// * `FUN_14064ee70` (every phase update): `BlendWeight[i] = (i <= to) ? 1 : 0`; on the first call (or before BeginPlay) each clone receives a copy of
///   `Settings[i]` and `BlendWeight` 1.
/// * `FUN_14064e920`: while a transition runs (`from != to`) `BlendWeight[to] = t` afterwards.
pub fn pp_components(from: usize, to: usize, t: f32) -> [PpComponent; 7] {
    let to = to.min(3);
    let main = |i: usize| PpComponent {
        phase: i, clone: false, priority: i as f32 - PP_PRIORITY_STEP,
        weight: if i > to { 0.0 } else if i == to && from != to { t } else { 1.0 },
    };
    let clone = |i: usize| PpComponent { phase: i, clone: true, priority: i as f32 - PP_PRIORITY_STEP - PP_PRIORITY_STEP, weight: 1.0 };
    [main(MORNING), main(DAY), main(EVENING), main(NIGHT), clone(DAY), clone(EVENING), clone(NIGHT)]
}
/// UE 4.16 `InsertVolume` (UnrealClient.cpp:1022): each volume, in registration order, goes before the first one with a HIGHER priority (equal
/// priorities keep registration order). Returns the indices of `priorities` in `UWorld::PostProcessVolumes` order, which
/// `FSceneView::StartFinalPostprocessSettings` (SceneView.cpp:1612) walks front to back with `DoPostProcessVolume` -> `OverridePostProcessSettings`.
pub fn pp_volume_order(priorities: &[f32]) -> Vec<usize> {
    let mut list: Vec<usize> = Vec::with_capacity(priorities.len());
    for (k, &p) in priorities.iter().enumerate() {
        let at = list.iter().position(|&j| p < priorities[j]).unwrap_or(list.len());
        list.insert(at, k);
    }
    list
}
/// `DoPostProcessVolume` (SceneView.cpp:1501) for an enabled unbound component: the weight it passes to `OverridePostProcessSettings`, or `None`
/// when the component is skipped (`LocalWeight <= 0`). `OverridePostProcessSettings` clamps `> 1` to 1 again.
pub fn pp_local_weight(blend_weight: f32) -> Option<f32> {
    let w = blend_weight.clamp(0.0, 1.0);
    (w > 0.0).then_some(w)
}
/// The TOD components of the blend at `hour`, already in `PostProcessVolumes` order.
pub fn pp_stack_at(hour: f32) -> Vec<PpComponent> {
    let (from, to, t) = blend_at(hour);
    let c = pp_components(from, to, t);
    pp_volume_order(&c.map(|c| c.priority)).into_iter().map(|i| c[i]).collect()
}
/// State at the current clock.
pub fn current() -> TodState { state_at(hour()) }
/// Unit vector towards the sun at the current clock.
pub fn sun_direction() -> Vec3 { current().to_sun }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_pure_evening_sunset() {
        let s = state_at(SUNSET_HOUR);
        assert_eq!((s.from, s.to), (EVENING, EVENING));
        let elev = s.to_sun.y.asin().to_degrees();
        assert!(elev > 3. && elev < 30., "sunset sun should be low, got {elev}");
        assert!((s.sun_brightness - 1.0).abs() < 1e-6);
    }
    #[test]
    fn sun_path_matches_the_recovered_arc() {
        // p = 0 at 3 h (sunrise rotation, below horizon: object rotation pitch 370 deg => light travels up), 1 at 22 h; noon sun high.
        assert!((sun_arc_progress(3.) - 0.).abs() < 1e-6 && (sun_arc_progress(22.) - 1.).abs() < 1e-6 && (sun_arc_progress(12.5) - 0.5).abs() < 1e-6);
        let elev = |h: f32| state_at(h).to_sun.y.asin().to_degrees();
        assert!(elev(12.5) > 60. && elev(20.) > elev(21.) && elev(21.) > 0. && elev(22.) < 0.);
    }
    /// TOD LIVE: the exe's clock advance (FUN_14064fdb0): speed 80 = 80 in-game seconds per second (one in-game hour in 45 s), wrapping at 24.
    #[test]
    fn clock_advance_matches_the_tod_tick() {
        assert!((advance(16.0, DEFAULT_TIME_OF_DAY_SPEED, 45.0) - 17.0).abs() < 1e-5);
        assert!((advance(23.9, 3600.0, 0.2) - 0.1).abs() < 1e-5, "{}", advance(23.9, 3600.0, 0.2));
        assert_eq!(advance(20.5, 0.0, 1.0), 20.5);
        // 16 h -> 20 h in 240 frames of 1/30 s at speed 1800
        let mut h = 16.0;
        for _ in 0..240 { h = advance(h, 1800.0, 1.0 / 30.0); }
        assert!((h - 20.0).abs() < 1e-3, "{h}");
    }
    /// RANDOM START: `rand() * (1/32767) * 23.99` (HandleMatchHasStarted @1403f78a0) covers 0 .. 23.99, always accepted by SetTimeOfDayFloat (< 24).
    #[test]
    fn random_start_hour_spans_the_exe_range() {
        assert_eq!(random_start_hour(0), 0.0);
        assert!((random_start_hour(32767) - 23.99).abs() < 1e-5, "{}", random_start_hour(32767));
        assert!((random_start_hour(16384) - 11.995366).abs() < 1e-4);
        assert!((0..=32767).all(|r| (0.0..24.0).contains(&random_start_hour(r))));
        assert!(START_TIME_OF_DAY == 7.0, "PreInitializeComponents start value (StartTimeOfDayInGame)");
    }
    #[test]
    fn night_volumetric_scattering_scale_is_the_struct_default() {
        assert_eq!(PHASES[NIGHT].fog.volumetric_scattering_scale, 1.0);
        assert_eq!(PHASES[NIGHT].fog.volumetric_scattering_scale, crate::fog::PhaseFog::NIGHT.vol_scattering_scale);
    }
    #[test]
    fn noon_is_day_phase() {
        let s = state_at(12.);
        assert_eq!((s.from, s.to, s.blend), (DAY, DAY, 0.));
        assert!((s.weights.iter().sum::<f32>() - 1.).abs() < 1e-6);
    }
    #[test]
    fn day_to_evening_transition_window() {
        // Day out = 0.2*12 = 2.4 h before 19, Evening in = 0.2*3 = 0.6 h after
        assert_eq!(blend_at(16.5).2, 0.);
        let (a, b, t) = blend_at(19.0);
        assert_eq!((a, b), (DAY, EVENING));
        assert!((t - 0.896).abs() < 1e-3, "{t}"); // d = 2.4/3.0 = 0.8 -> smoothstep 0.896
        assert_eq!(blend_at(19.7), (EVENING, EVENING, 0.));
    }
}
