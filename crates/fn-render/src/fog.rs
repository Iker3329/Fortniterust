//! FOG / ATMOSPHERE HOOK: the recovered Athena fog actor data and exact CPU ports of the 1.8 (UE 4.16) fog shaders.
//!
//! Provenance (all read-only from the 1.8 install):
//! * No Athena map package (all 109 `Athena/Maps/**.umap`) contains an ExponentialHeightFog / AtmosphericFog / SkyLight / DirectionalLight actor.
//!   The only fog in the baseplate is the time-of-day manager `TODM_A_Child_Athena_C` (`WorldTimeOfDayManager` of `Athena_Terrain`), whose
//!   `ActiveHeightFogComponent` (`UExponentialHeightFogComponent`) is driven by the day-phase structs below. There is no AtmosphericFog.
//! * Phase values: `/Game/TimeOfDay/TODM/TODM_A_Child_Athena` CDO `LightAndFogPhaseSettings[1]` (Day), merged over the parent `TODM_A`
//!   phase (delta serialisation: an untagged child field equals the parent field, e.g. `StartDistance` 0 tagged in the parent).
//! * Struct / component defaults: `AFortTimeOfDayManager` ctor and `UExponentialHeightFogComponent` ctor in the exe (`fn-exe-defaults`, `classdump`).
//! * VOLUMETRIC FOG IS ENABLED: the `AFortTimeOfDayManager` ctor (exe RVA 0x62c68d) stores byte 1 at `ActiveHeightFogComponent + 0x2f4`
//!   (the `bEnableVolumetricFog` bit-field; the component's own ctor never touches it, default false) and byte 1 at `+0x308`
//!   (`bOverrideLightColorsWithFogInscatteringColors`). `TODM_Disabled` serialises exactly these two bits as false. `r.VolumetricFog` = 1.
//! * Shader maths: `FVolumetricFogMaterialSetupCS`, `FVolumetricFogFinalIntegrationCS`, `TVolumetricFogLightScatteringCS`,
//!   `TExponentialHeightFogPS<DirectionalLightInscatteringAndVolumetricFog>` disassembled from `GlobalShaderCache-PCD3D_SM5.bin`.
//!
//! Units: this module works in Unreal units (cm, Z up) exactly like the shaders; the renderer is metres / Y up.

/// Day phase `ExpHeightFogValues` after the parent merge (child values RECOVERED; untagged = parent / struct default).
pub mod day {
    /// `FogDensity` (the component divides by 1000 when it builds `FExponentialHeightFogSceneInfo`; recalled engine remap, see `DENSITY_REMAP`).
    pub const FOG_DENSITY: f32 = 0.00022;
    pub const FOG_HEIGHT_FALLOFF: f32 = 0.09;
    pub const FOG_MAX_OPACITY: f32 = 0.8;
    /// Tagged `0.0` in TODM_A phase 1, untagged in the child, so 0 (the struct default is 2000).
    pub const START_DISTANCE: f32 = 0.0;
    pub const DIRECTIONAL_INSCATTERING_EXPONENT: f32 = 3.0;
    pub const DIRECTIONAL_INSCATTERING_START_DISTANCE: f32 = 10000.0;
    pub const DIRECTIONAL_INSCATTERING_COLOR: [f32; 3] = [0.027_802, 0.023_625, 0.035];
    pub const FOG_INSCATTERING_COLOR: [f32; 3] = [0.110_45, 0.277_685, 0.47];
    /// Child tags 0.05 (parent 0.3).
    pub const VOLUMETRIC_FOG_SCATTERING_DISTRIBUTION: f32 = 0.05;
    /// Child tags 0.1.
    pub const VOLUMETRIC_FOG_SCATTERING_SCALE: f32 = 0.1;
    /// Untagged in both: `FExponentialHeightFogValues` ctor default (manager ctor stores at 0xa88).
    pub const VOLUMETRIC_FOG_ABSORPTION_SCALE: f32 = 1.0;
    /// Untagged in both: struct default 4500 cm (== component ctor default 0x304).
    pub const VOLUMETRIC_FOG_DISTANCE: f32 = 4500.0;
    /// Child `HeightFogZOffset` (cm). The child's `AltitudeAdjustments` array is empty (no camera-altitude adjustment).
    pub const HEIGHT_FOG_Z_OFFSET: f32 = 65000.0;
    /// Day directional light values of the child phase: Brightness 3.0, LightColor FColor(255,225,161), `VolumetricScatteringIntensity` 1.0.
    pub const SUN_VOLUMETRIC_SCATTERING_INTENSITY: f32 = 1.0;
    /// Day sky light: `SkyLightColor` (3.575, 4.17599, 5.0); `VolumetricScatteringIntensity` untagged in the child -> parent phase 1 value 0.65.
    pub const SKY_VOLUMETRIC_SCATTERING_INTENSITY: f32 = 0.65;
}

/// Stock / engine facts recovered from the exe (not from assets).
pub mod engine {
    /// `r.VolumetricFog` and friends (`fn-exe-defaults` cvar table).
    pub const GRID_PIXEL_SIZE: u32 = 16;
    pub const GRID_SIZE_Z: u32 = 64;
    pub const DEPTH_DISTRIBUTION_SCALE: f32 = 64.0;
    pub const HISTORY_WEIGHT: f32 = 0.9;
    pub const JITTER: bool = true;
    pub const TEMPORAL_REPROJECTION: bool = true;
    pub const INVERSE_SQUARED_LIGHT_DISTANCE_BIAS_SCALE: f32 = 1.0;
    /// SETTLED from the exe: the fog scene-proxy constructor (RVA 0x1930f20) does `mulss [component+0x290 FogDensity], 0.001f` and
    /// `mulss [component+0x2e4 FogHeightFalloff], 0.001f`, and stores `component+0x178` (the component's world Z, `GetComponentLocation().Z`) as the fog height.
    /// So `Density = FogDensity / 1000`, `HeightFalloff = FogHeightFalloff / 1000`.
    pub const DENSITY_REMAP: f32 = 1.0 / 1000.0;
}

/// One time-of-day phase of `LightAndFogPhaseSettings` (ExpHeightFogValues + the light values the fog needs), child `TODM_A_Child_Athena` merged over parent
/// `TODM_A` (an untagged child field takes the parent value: proven by the phase times, e.g. the child Evening is untagged yet begins at the parent's 19:00, right
/// after the child Day's 07:00 + 12 h). `fog_density` / `fog_height_falloff` are the RAW values (the component remaps them with `engine::DENSITY_REMAP`).
/// Dumped with `fn-inspect props FortniteGame/Content/TimeOfDay/TODM/TODM_A_Child_Athena Default__TODM_A_Child_Athena_C` (and `TODM_A` / `Default__TODM_A_C`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhaseFog {
    pub fog_density: f32,
    pub fog_height_falloff: f32,
    pub fog_max_opacity: f32,
    pub start_distance: f32,
    pub dir_exponent: f32,
    pub dir_start_distance: f32,
    pub dir_color: [f32; 3],
    pub fog_color: [f32; 3],
    pub vol_scattering_distribution: f32,
    pub vol_scattering_scale: f32,
    pub vol_absorption_scale: f32,
    /// `DirectionalLightValues.VolumetricScatteringIntensity`.
    pub sun_vol_intensity: f32,
    /// `SkyLightValues.VolumetricScatteringIntensity`.
    pub sky_vol_intensity: f32,
    /// `DirectionalLightValues.LightColor` (FColor, sRGB bytes) and `Brightness`.
    pub sun_color_srgb: [u8; 3],
    pub sun_brightness: f32,
    /// `SkyLightValues.SkyLightColor` (linear).
    pub sky_light_color: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Morning, Day, Evening, Night }

impl PhaseFog {
    /// Child Morning: density/falloff/opacity/colours/exponent/scale/absorption child-tagged; start distance, distribution, sun+sky volumetric intensity from the parent.
    pub const MORNING: PhaseFog = PhaseFog {
        fog_density: 0.0002, fog_height_falloff: 0.09, fog_max_opacity: 0.2, start_distance: 0.0, dir_exponent: 2.0, dir_start_distance: 0.0,
        dir_color: [0.45, 0.154_971, 0.069_128], fog_color: [0.73, 0.282_014, 0.179_708],
        vol_scattering_distribution: 0.2, vol_scattering_scale: 0.1, vol_absorption_scale: 0.5, sun_vol_intensity: 2.0, sky_vol_intensity: 0.8,
        sun_color_srgb: [255, 187, 153], sun_brightness: 1.5, sky_light_color: [0.83, 0.728_359, 0.587_762],
    };
    /// Child Day (identical to the [`day`] module).
    pub const DAY: PhaseFog = PhaseFog {
        fog_density: 0.00022, fog_height_falloff: 0.09, fog_max_opacity: 0.8, start_distance: 0.0, dir_exponent: 3.0, dir_start_distance: 10000.0,
        dir_color: [0.027_802, 0.023_625, 0.035], fog_color: [0.110_45, 0.277_685, 0.47],
        vol_scattering_distribution: 0.05, vol_scattering_scale: 0.1, vol_absorption_scale: 1.0, sun_vol_intensity: 1.0, sky_vol_intensity: 0.65,
        sun_color_srgb: [255, 225, 161], sun_brightness: 3.0, sky_light_color: [3.575, 4.175_99, 5.0],
    };
    /// SUNSET. Child Evening tags: sun (255,174,114) Brightness 1, VolumetricScatteringIntensity 5; fog density 0.0003, falloff 0.08, max opacity 1.0, dir exponent 5,
    /// dir start 0, dir colour (0.325, 0.033422, 0), fog colour (0.62, 0.135486, 0.073669); sky light (0.735, 0.574911, 0.319725) vol 0.8.
    /// From the parent (child untagged): scattering distribution 0.34, scattering scale 0.6. Absorption untagged in both -> 1.0 (struct default, see `day`).
    pub const EVENING: PhaseFog = PhaseFog {
        fog_density: 0.0003, fog_height_falloff: 0.08, fog_max_opacity: 1.0, start_distance: 0.0, dir_exponent: 5.0, dir_start_distance: 0.0,
        dir_color: [0.325, 0.033_422, 0.0], fog_color: [0.62, 0.135_486, 0.073_669],
        vol_scattering_distribution: 0.34, vol_scattering_scale: 0.6, vol_absorption_scale: 1.0, sun_vol_intensity: 5.0, sky_vol_intensity: 0.8,
        sun_color_srgb: [255, 174, 114], sun_brightness: 1.0, sky_light_color: [0.735, 0.574_911, 0.319_725],
    };
    /// Child Night over parent. UNVERIFIED: scattering scale (untagged in both, 1.0 = ctor default assumed), sun Brightness (untagged in the child -> parent 8).
    pub const NIGHT: PhaseFog = PhaseFog {
        fog_density: 0.0013, fog_height_falloff: 0.09, fog_max_opacity: 0.9, start_distance: 0.0, dir_exponent: 5.0, dir_start_distance: 2500.0,
        dir_color: [0.010_201, 0.005_591, 0.03], fog_color: [0.016_433, 0.015, 0.1],
        vol_scattering_distribution: 0.3, vol_scattering_scale: 1.0, vol_absorption_scale: 1.0, sun_vol_intensity: 5.0, sky_vol_intensity: 0.6,
        sun_color_srgb: [76, 97, 177], sun_brightness: 8.0, sky_light_color: [0.3828, 0.461_439, 0.66],
    };
    pub fn of(p: Phase) -> PhaseFog { match p { Phase::Morning => Self::MORNING, Phase::Day => Self::DAY, Phase::Evening => Self::EVENING, Phase::Night => Self::NIGHT } }

    /// UE light colour = `FLinearColor(FColor)` (sRGB decode) * Brightness (no 1/pi in UE units).
    pub fn sun_light_rgb(&self) -> [f32; 3] {
        let lin = |v: u8| { let c = v as f32 / 255.0; if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) } };
        self.sun_color_srgb.map(|v| lin(v) * self.sun_brightness)
    }
    /// The phase blend of the TOD manager tick (exe `FUN_14064e920`, every tick while `TransitionFromPhase != TransitionToPhase`):
    /// * `ExpHeightFogValues`: `FUN_14064e6f0` lerps all 18 floats of `FExponentialHeightFogValues` as `(B - A) * t + A` and hands the result to `FUN_14064e200`,
    ///   which writes them to `ActiveHeightFogComponent` (no altitude adjustment: the child's `AltitudeAdjustments` is empty; storm strength 0; the
    ///   DirectionalInscatteringColor multiplier 1) and calls `MarkRenderStateDirty`, i.e. the fog scene info, and so the view's fog uniforms, are rebuilt every frame;
    /// * `DirectionalLightValues` (`UpdateDirectionalLightForTimeOfDay` @14064d5d0): Brightness and VolumetricScatteringIntensity as floats, the FColor
    ///   `LightColor` per byte as `(char)(long)((float)(int)(B - A) * t + A)` (TRUNCATED, the same as `lighting::sun_radiance_of`);
    /// * `SkyLightValues` (`FUN_14064f190`): float lerp of every member.
    /// FOGFIX TOD-LIVE (2026-10-05): the sun bytes were rounded; now truncated like the exe.
    pub fn lerp(a: &PhaseFog, b: &PhaseFog, t: f32) -> PhaseFog {
        let t = t.clamp(0.0, 1.0);
        let f = |x: f32, y: f32| (y - x) * t + x;
        let v = |x: [f32; 3], y: [f32; 3]| [f(x[0], y[0]), f(x[1], y[1]), f(x[2], y[2])];
        let sun = |i: usize| ((b.sun_color_srgb[i] as i32 - a.sun_color_srgb[i] as i32) as f32 * t + a.sun_color_srgb[i] as f32) as i64 as u8;
        PhaseFog {
            fog_density: f(a.fog_density, b.fog_density), fog_height_falloff: f(a.fog_height_falloff, b.fog_height_falloff), fog_max_opacity: f(a.fog_max_opacity, b.fog_max_opacity),
            start_distance: f(a.start_distance, b.start_distance), dir_exponent: f(a.dir_exponent, b.dir_exponent), dir_start_distance: f(a.dir_start_distance, b.dir_start_distance),
            dir_color: v(a.dir_color, b.dir_color), fog_color: v(a.fog_color, b.fog_color),
            vol_scattering_distribution: f(a.vol_scattering_distribution, b.vol_scattering_distribution), vol_scattering_scale: f(a.vol_scattering_scale, b.vol_scattering_scale),
            vol_absorption_scale: f(a.vol_absorption_scale, b.vol_absorption_scale), sun_vol_intensity: f(a.sun_vol_intensity, b.sun_vol_intensity), sky_vol_intensity: f(a.sky_vol_intensity, b.sky_vol_intensity),
            sun_color_srgb: [sun(0), sun(1), sun(2)], sun_brightness: f(a.sun_brightness, b.sun_brightness), sky_light_color: v(a.sky_light_color, b.sky_light_color),
        }
    }
    /// Fog values at an in-game hour: the phase table above blended with the exe-ported phase blend of `time_of_day::blend_at` (smoothstep between the two
    /// phases picked by `FUN_140631e50`; sibling module) and [`PhaseFog::lerp`] (the exe's per-member blend).
    pub fn at_hour(hour: f32) -> PhaseFog {
        let (from, to, t) = crate::time_of_day::blend_at(hour);
        let idx = |i: usize| match i { 0 => Phase::Morning, 1 => Phase::Day, 2 => Phase::Evening, _ => Phase::Night };
        if from == to || t <= 0.0 { Self::of(idx(from)) } else if t >= 1.0 { Self::of(idx(to)) } else { Self::lerp(&Self::of(idx(from)), &Self::of(idx(to)), t) }
    }
}

/// The fog state of the live time-of-day clock (`time_of_day::hour()`).
pub fn active() -> PhaseFog { PhaseFog::at_hour(crate::time_of_day::hour()) }

/// FOG HEIGHT RE-BASE (2026-10-07): `HeightFogZOffset` is an ABSOLUTE world Z. The TOD tick `FUN_14064e200` (the `SetActiveHeightFogValues` path) computes
/// `Z = HeightFogZOffset (+0x3d0)`, adds the camera Z only when `bBaseHeightFogOnAltitude` (bit 3 of +0x3cc) is set and the `AltitudeAdjustments` (+0x3d8)
/// curve only when the array is non-empty, then calls `USceneComponent::SetWorldLocation` on `ActiveHeightFogComponent` (+0x438) with that Z: the fog
/// component's WORLD Z, so the TOD actor's own location does not enter (it is spawned anyway, not placed: `FUN_14065e2a0` spawns
/// `FortWorldSettings.WorldTimeOfDayManager` with a null location / rotation, and no Athena umap holds a TODM actor, fn-inspect exports of all 109).
/// The native ctor (0x14062c240) sets `bBaseHeightFogOnAltitude`, but `TODM_A` serialises it false (the child inherits) and the child's EMPTY
/// `AltitudeAdjustments` overrides the parent's four entries (fn-inspect props): fog Z = 65000 cm absolute in every phase.
/// The renderer's world Y is NOT absolute: `backdrop::to_game` / `fn_terrain::Mapping` re-base heights by the terrain ground reference (`BackdropData::
/// ground_ref_cm`, 768 cm at the default layout), and every fog consumer (WGSL `HeightFogU`, the FX `View` origin, the CPU ports) takes camera / pixel Z
/// as `y * 100` in that frame. So the fog plane in the renderer's frame is `HeightFogZOffset - ground_ref` (was 65000 un-rebased: 768 cm too high, the
/// density at the ground ~5 % too HIGH: 2^(768 * 9e-5)). Set by `Renderer::set_backdrop`; 0 without an Athena backdrop (lobby / test scenes).
static WORLD_Z_REF_CM: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The absolute UE Z (cm) of the renderer's world y = 0 (the installed Athena backdrop's `ground_ref_cm`).
pub fn set_world_z_ref_cm(z: f32) { WORLD_Z_REF_CM.store(z.to_bits(), std::sync::atomic::Ordering::Relaxed); }
pub fn world_z_ref_cm() -> f32 { f32::from_bits(WORLD_Z_REF_CM.load(std::sync::atomic::Ordering::Relaxed)) }
/// The fog plane Z (cm) in a frame whose z = 0 lies at absolute UE Z `ref_z_cm`.
pub fn fog_height_cm(ref_z_cm: f32) -> f32 { day::HEIGHT_FOG_Z_OFFSET - ref_z_cm }

/// `FVolumetricFogUniformParameters` / `View.VolumetricFogGridZParams` (B, O, S): the disassembled shaders use
/// `depth = (2^(slice / S) - O) / B` and `slice = log2(depth * B + O) * S`.
/// The (B, O) construction is the engine's C++ (`GetVolumetricFogGridZParams`, RECALLED from the 4.16 source; the shaders only consume it and a static scan of the exe did not
/// locate the writer; consistent with the disassembled inverse above):`O = (F - N * 2^((GridSizeZ - 1) / S)) / (F - N)`, `B = (1 - O) / N`.
/// FOGFIX C1 (2026-10-05): N = NearPlane + `NearOffset` 0.095 * 100 cm ("don't spend lots of resolution right in front of the near plane", UE4.16
/// VolumetricFog.cpp:794-799); the 1.8 exe `GetVolumetricFogGridZParams` @1419ed640 adds `DAT_14460dcc8` = 9.5 in double, takes the exp2 as a float `powf`,
/// and does the rest in double before storing floats. Was: N = NearPlane (the offset missing). `far` is kept above N only as a division guard (the exe has none).
pub fn grid_z_params(near_cm: f32, far_cm: f32, grid_size_z: u32, depth_distribution_scale: f32) -> [f32; 3] {
    const NEAR_OFFSET_CM: f64 = 0.095 * 100.0;
    let n = near_cm as f64 + NEAR_OFFSET_CM;
    let f = (far_cm as f64).max(n + 1.0);
    let e = 2f32.powf((grid_size_z as f32 - 1.0) / depth_distribution_scale) as f64;
    let o = (f - e * n) / (f - n);
    let b = (1.0 - o) / n;
    [b as f32, o as f32, depth_distribution_scale]
}
/// View-space depth (cm) of a (possibly fractional) Z slice: `FVolumetric*CS` `div / exp / add / div` sequence.
pub fn depth_from_slice(z: [f32; 3], slice: f32) -> f32 { ((slice / z[2]).exp2() - z[1]) / z[0] }
/// Z texture coordinate (0..1 over the grid) of a view-space depth, as the fog pixel shader computes it (`log`, `mul cb1[176].z`, `mul cb1[175].z`).
pub fn volume_w_from_depth(z: [f32; 3], depth_cm: f32, grid_size_z: u32) -> f32 { (depth_cm * z[0] + z[1]).max(1e-30).log2() * z[2] / grid_size_z as f32 }
/// Grid size in voxels for a viewport (`GridPixelSize` 16, `GridSizeZ` 64).
pub fn grid_size(width: u32, height: u32) -> [u32; 3] {
    [width.div_ceil(engine::GRID_PIXEL_SIZE), height.div_ceil(engine::GRID_PIXEL_SIZE), engine::GRID_SIZE_Z]
}
/// Grid size for the scalability values of `r.VolumetricFog.GridPixelSize` / `GridSizeZ` (`GetVolumetricFogGridSize`: viewport / pixel size rounded up).
pub fn grid_size_for(width: u32, height: u32, pixel_size: u32, size_z: u32) -> [u32; 3] {
    let px = pixel_size.max(1);
    [width.div_ceil(px), height.div_ceil(px), size_z.max(1)]
}

/// `FVolumetricFogMaterialSetupCS` (exact): global fog density at world Z `z_cm`, scattering (grey) and absorption written to VBufferA.
/// `cb0[2].x * 0.5 * 2^(-(z - FogHeight) * Falloff)`; scattering = d * ScatteringScale (rgb), absorption = d * AbsorptionScale (a).
/// `density` / `falloff` are the component values after the engine remap (see `engine::DENSITY_REMAP`).
pub fn material_setup(z_cm: f32, density: f32, falloff: f32, fog_height_cm: f32, scattering_scale: f32, absorption_scale: f32) -> ([f32; 3], f32) {
    let d = ((z_cm - fog_height_cm) * -falloff).exp2() * (density * 0.5);
    ([d * scattering_scale; 3], d * absorption_scale)
}
/// UE `Luminance` weights used by the scattering shader for extinction = luma(scattering) + absorption.
pub fn ue_luma(c: [f32; 3]) -> f32 { c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11 }

/// `FVolumetricFogFinalIntegrationCS` (exact): integrate the per-voxel (scattering rgb light, extinction) slices front to back.
/// `slices[i] = (in-scattered light rgb, extinction per cm)`, `step_cm[i]` = world distance between the previous and this slice centre.
/// Returns (accumulated light rgb, transmittance) AFTER each slice (what the CS stores).
pub fn final_integration(slices: &[([f32; 3], f32)], step_cm: &[f32]) -> Vec<([f32; 3], f32)> {
    let (mut light, mut trans) = ([0f32; 3], 1f32);
    slices.iter().zip(step_cm).map(|(&(s, ext), &len)| {
        let t = (-ext * len * 1.442_695).exp2();
        let k = ext.max(0.00001);
        for i in 0..3 { light[i] += (s[i] - s[i] * t) / k * trans; }
        trans *= t;
        (light, trans)
    }).collect()
}

/// Henyey-Greenstein phase exactly as the light scattering CS evaluates it (`cos_theta = dot(L_to_light, -view_ray)`).
pub fn hg_phase(g: f32, cos_theta: f32) -> f32 {
    (1.0 - g * g) / (12.566_371 * (1.0 + g * g + 2.0 * g * cos_theta).powf(1.5))
}

/// `TVolumetricFogLightScatteringCS`, directional-light term only (what the Athena baseplate has: no local lights), no shadowing/light function:
/// `light_color` already includes the light's `VolumetricScatteringIntensity`; with `bOverrideLightColorsWithFogInscatteringColors` the colour is
/// `luma(light_color) * DirectionalInscatteringColor` (SETTLED from the disassembly of the light scattering CS: `dp3(cb2[5].xyz, luma) * cb3[6].xyz` when `cb0[84].x` is set,
/// cb3[6] = `HeightFogDirectionalLightInscatteringColor`; `cb2[5].w` is the light's `VolumetricScatteringIntensity`), otherwise `light_color`.
pub fn directional_inscatter(light_color: [f32; 3], override_color: Option<[f32; 3]>, visibility: f32, g: f32, cos_theta: f32) -> [f32; 3] {
    let c = match override_color { Some(oc) => { let l = ue_luma(light_color); [l * oc[0], l * oc[1], l * oc[2]] } None => light_color };
    let k = visibility * hg_phase(g, cos_theta);
    [c[0] * k, c[1] * k, c[2] * k]
}

/// Sky-light term of `TVolumetricFogLightScatteringCS`, SETTLED from the disassembly (`cb0[80].x * max(cb3[5] * 0.282095, 0)` when `cb0[84].x` is set, i.e. the
/// `bOverrideLightColorsWithFogInscatteringColors` bit): a flat `FogInscatteringColor * Y00` scaled by `SkyLightVolumetricScatteringIntensity`
/// (no SH evaluation, no sun, no occlusion).
pub fn sky_scatter_override(fog_inscattering_color: [f32; 3], sky_volumetric_intensity: f32) -> [f32; 3] {
    fog_inscattering_color.map(|c| sky_volumetric_intensity * (c * 0.282_095).max(0.0))
}

/// Parameters of the global (non-volumetric) fog pixel shader (`TExponentialHeightFogPS`), cm units, Z up, remapped density / falloff.
#[derive(Clone, Copy, Debug)]
pub struct HeightFogPs {
    pub density: f32,
    pub falloff: f32,
    pub fog_height_cm: f32,
    pub max_opacity: f32,
    pub start_distance_cm: f32,
    pub cutoff_distance_cm: f32,
    pub fog_color: [f32; 3],
    pub dir_color: [f32; 3],
    pub dir_exponent: f32,
    pub dir_start_cm: f32,
    /// `View.VolumetricFogMaxDistance` (cm); 0 when the volumetric fog is off.
    pub volumetric_max_distance_cm: f32,
}
impl HeightFogPs {
    /// A phase's values (remapped as the engine does); `HeightFogZOffset` 65000 cm is the child's, valid for every phase.
    pub fn from_phase(p: &PhaseFog, volumetric: bool) -> Self {
        HeightFogPs {
            density: p.fog_density * engine::DENSITY_REMAP, falloff: p.fog_height_falloff * engine::DENSITY_REMAP, fog_height_cm: day::HEIGHT_FOG_Z_OFFSET,
            max_opacity: p.fog_max_opacity, start_distance_cm: p.start_distance, cutoff_distance_cm: 0.0, fog_color: p.fog_color,
            // FOGFIX C2 (2026-10-05): `View.DirectionalInscatteringColor = FogInfo.DirectionalInscatteringColor * Light->GetColor().ComputeLuminance()` when both
            // luminances exceed KINDA_SMALL_NUMBER, else the light is not used (FogRendering.cpp:316-324; exe FUN_141833920), the same rule as
            // `shader_constants`. Was: the raw DirectionalInscatteringColor (no light luminance).
            dir_color: { let l = ue_luma(p.sun_light_rgb()); if l > 1.0e-4 && ue_luma(p.dir_color) > 1.0e-4 { p.dir_color.map(|c| c * l) } else { [0.0; 3] } },
            dir_exponent: p.dir_exponent, dir_start_cm: p.dir_start_distance,
            volumetric_max_distance_cm: if volumetric { day::VOLUMETRIC_FOG_DISTANCE } else { 0.0 },
        }
    }
    /// The Athena Day values (kept for the Day-specific tests; the rendered scene uses [`HeightFogPs::active`]).
    pub fn athena_day(volumetric: bool) -> Self { Self::from_phase(&PhaseFog::DAY, volumetric) }
    /// The live clock's fog ([`active`]; sunset by default), what [`live_uniform`] uploads every frame, in the renderer's frame ([`world_z_ref_cm`]).
    pub fn active(volumetric: bool) -> Self { Self::from_phase(&active(), volumetric).rebased(world_z_ref_cm()) }
    /// FOG HEIGHT RE-BASE: these values in a frame whose z = 0 is at absolute UE Z `ref_z_cm` ([`fog_height_cm`]).
    pub fn rebased(mut self, ref_z_cm: f32) -> Self { self.fog_height_cm = fog_height_cm(ref_z_cm); self }
    /// This state as the per-frame `HeightFogU` block of `shaders/common.wgsl`.
    pub fn uniform(&self) -> HeightFogUniform {
        HeightFogUniform {
            p0: [self.density, self.falloff, self.fog_height_cm, self.max_opacity],
            p1: [self.start_distance_cm, self.dir_exponent, self.dir_start_cm, self.volumetric_max_distance_cm],
            dir_color: [self.dir_color[0], self.dir_color[1], self.dir_color[2], self.cutoff_distance_cm],
            color: [self.fog_color[0], self.fog_color[1], self.fog_color[2], 0.0],
        }
    }
    /// Exact port of the disassembled pixel shader. `cam_to_receiver_cm` = receiver - camera (Z up), `view_forward` unit camera forward,
    /// `to_sun` unit vector towards the light, `camera_z_cm` world Z of the camera. Returns (inscatter rgb, transmittance) BEFORE the volumetric combine.
    pub fn eval(&self, cam_to_receiver_cm: [f32; 3], view_forward: [f32; 3], to_sun: [f32; 3], camera_z_cm: f32) -> ([f32; 3], f32) {
        let c = cam_to_receiver_cm;
        let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt().max(1e-6);
        let along = c[0] * view_forward[0] + c[1] * view_forward[1] + c[2] * view_forward[2];
        let vol_exclude = (len / along * self.volumetric_max_distance_cm).max(0.0);
        let exclude = vol_exclude.max(self.start_distance_cm);
        let inv_len = 1.0 / len;
        let (origin_density, ray_len, dz);
        if exclude > 0.0 {
            let t = exclude * inv_len; // ExcludeIntersectionTime
            let to_excl_z = c[2] * t;
            let excl_z = camera_z_cm + to_excl_z;
            origin_density = (self.falloff * (excl_z - self.fog_height_cm)).max(-127.0).neg_exp2() * self.density;
            ray_len = len * (1.0 - t);
            dz = c[2] - to_excl_z;
        } else {
            // cb0[0].x = RayOriginTerms (density at the camera)
            origin_density = self.density * (-(self.falloff * (camera_z_cm - self.fog_height_cm)).max(-127.0)).exp2();
            ray_len = len;
            dz = c[2];
        }
        let dz = if dz.abs() > 0.01 { dz } else { 0.01 };
        let x = (dz * self.falloff).max(-127.0);
        let line = (1.0 - (-x).exp2()) * origin_density / x; // LineIntegral
        let optical = ray_len * line;
        let dir = [c[0] * inv_len, c[1] * inv_len, c[2] * inv_len];
        let cosl = (dir[0] * to_sun[0] + dir[1] * to_sun[1] + dir[2] * to_sun[2]).clamp(0.0, 1.0);
        let dir_gain = cosl.powf(self.dir_exponent);
        let dir_factor = 1.0 - (-((ray_len - self.dir_start_cm).max(0.0) * line)).exp2().min(1.0);
        let dir_in = [self.dir_color[0] * dir_gain * dir_factor, self.dir_color[1] * dir_gain * dir_factor, self.dir_color[2] * dir_gain * dir_factor];
        let trans = (-optical).exp2().min(1.0).max(1.0 - self.max_opacity);
        let (dir_in, trans) = if self.cutoff_distance_cm > 0.0 && self.cutoff_distance_cm < len { ([0.0; 3], 1.0) } else { (dir_in, trans) };
        let a = 1.0 - trans;
        ([self.fog_color[0] * a + dir_in[0], self.fog_color[1] * a + dir_in[1], self.fog_color[2] * a + dir_in[2]], trans)
    }
}
/// FOG INPUTS (2026-10-05): the values `FExponentialHeightFogShaderParameters::Set` (UE4.16 FogRendering.h:26-52 + `FVolumetricFogParameters::Set`,
/// VolumetricFog.h:45) uploads to every base-pass program that binds `FHeightFogShaderParameters` (the translucent base-pass VS: `HeightFogParameters.Set`,
/// BasePassRendering.h:377), from the view constants `FSceneRenderer::InitFogConstants` derives (FogRendering.cpp:258-330). Array order = the serializer's
/// (fn-fx `shader_params::HeightFogParams`): ExponentialFogParameters, ExponentialFogColorParameter, ExponentialFogParameters3,
/// SinCosInscatteringColorCubemapRotation, FogInscatteringTextureParameters, InscatteringLightDirection, DirectionalInscatteringColor,
/// DirectionalInscatteringStartDistance, ApplyVolumetricFog (each padded to a float4; the program reads only its member's size).
/// * `p` = the phase's `ExpHeightFogValues` (`active()`), density / falloff remapped like `FExponentialHeightFogSceneInfo` (SceneCore.cpp:427-428, /1000);
///   `FogHeight` = the component's Z = [`day::HEIGHT_FOG_Z_OFFSET`] re-based into the frame of `camera_z_cm`, whose z = 0 is at absolute UE Z `ref_z_cm`
///   ([`fog_height_cm`]; the renderer passes [`world_z_ref_cm`]);
/// * `camera_z_cm` = `View.ViewMatrices.GetViewOrigin().Z`; `to_light_ue` = `-Proxy->GetDirection()` of the directional light (UE axes), the light the
///   renderer's own fog shades its directional inscattering with (common.wgsl `height_fog(.., light.sun_dir, ..)`), i.e. the sun taken as the atmosphere sun light;
/// * `LightTerminatorAngle` is always 0 (`FExponentialHeightFogSceneInfo` ctor, SceneCore.cpp:433), no inscattering cubemap (the TODM phases set none), so
///   Parameters3.z = 0, the rotation is angle 0 and `FogInscatteringTextureParameters` uses the component defaults Fully / NonDirectionalInscatteringColorDistance
///   100000 / 1000 (ExponentialHeightFogComponent.cpp:29-30) with NumMips 1; `FogCutoffDistance` 0 (ctor :38);
/// * `volumetric` = the volumetric fog ran this frame (`View.VolumetricFogResources.IntegratedLightScattering != NULL`, VolumetricFog.h:47).
pub fn shader_constants(p: &PhaseFog, camera_z_cm: f32, ref_z_cm: f32, to_light_ue: [f32; 3], volumetric: bool) -> [[f32; 4]; 9] {
    const DELTA: f32 = 0.00001; // UnrealMathUtility.h:54
    const KINDA_SMALL_NUMBER: f32 = 1.0e-4; // UnrealMathUtility.h:42
    let (density, falloff, height) = (p.fog_density * engine::DENSITY_REMAP, p.fog_height_falloff * engine::DENSITY_REMAP, fog_height_cm(ref_z_cm));
    let cos_terminator = 0f32.cos().clamp(-1.0 + DELTA, 1.0 - DELTA);
    let power = (-falloff * (camera_z_cm - height)).clamp(-126.0 + 1.0, 127.0 - 1.0);
    let (fully, non) = (100000.0f32, 1000.0f32);
    let inv_range = 1.0 / (fully - non).max(0.00001);
    // FLinearColor::ComputeLuminance (Color.cpp:161) of the light's colour (LightColor * Intensity) and of DirectionalInscatteringColor
    let light_lum = ue_luma(p.sun_light_rgb());
    let use_dir = light_lum > KINDA_SMALL_NUMBER && ue_luma(p.dir_color) > KINDA_SMALL_NUMBER;
    let dir_color = if use_dir { p.dir_color.map(|c| c * light_lum) } else { [0.0; 3] };
    let light_dir = if use_dir { [to_light_ue[0], to_light_ue[1], to_light_ue[2], 1.0] } else { [0.0; 4] };
    [
        [density * power.exp2(), falloff, cos_terminator, p.start_distance],
        [p.fog_color[0], p.fog_color[1], p.fog_color[2], 1.0 - p.fog_max_opacity],
        [density, height, 0.0, 0.0],
        [0f32.sin(), 0f32.cos(), 0.0, 0.0],
        [inv_range, -non * inv_range, 1.0, 0.0],
        light_dir,
        [dir_color[0], dir_color[1], dir_color[2], p.dir_exponent.clamp(0.000001, 1000.0)],
        [p.dir_start_distance, 0.0, 0.0, 0.0],
        [if volumetric { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
    ]
}

/// FOG INPUTS: [`shader_constants`] of a view without height fog (fog shown off): the `FViewInfo` defaults `InitFogConstants` leaves in place
/// (SceneRendering.cpp:461-473: ExponentialFogParameters (0,1,1,0), colour 0 with FogMaxOpacity 1, the rest 0, the exponent clamped by `Set` to 1e-6).
pub fn no_fog_constants(volumetric: bool) -> [[f32; 4]; 9] {
    let mut c = [[0.0f32; 4]; 9];
    c[0] = [0.0, 1.0, 1.0, 0.0];
    c[6][3] = 0.000001;
    c[8][0] = if volumetric { 1.0 } else { 0.0 };
    c
}

trait NegExp2 { fn neg_exp2(self) -> f32; }
impl NegExp2 for f32 { fn neg_exp2(self) -> f32 { (-self).exp2() } }

/// TOD LIVE (2026-10-05): the height-fog view uniforms of one frame, `struct HeightFogU` of `shaders/common.wgsl` (member order is the contract). Was: WGSL
/// `const`s baked into `common.wgsl` when the pipelines were created (`common_wgsl` / `with_fog_consts`), so the fog never followed a moving clock.
/// UE rebuilds the fog every frame: the TOD manager tick writes the blended `ExpHeightFogValues` to `ActiveHeightFogComponent` and marks its render state
/// dirty (exe `FUN_14064e200`), `FSceneRenderer::InitFogConstants` (FogRendering.cpp:258) re-derives the view's fog constants from the scene info each frame
/// and `FExponentialHeightFogShaderParameters::Set` uploads them per draw. Here the renderer writes this block into the existing light (scene, statics, FX
/// lit, particles, backdrop) and sky uniform buffers every frame (no new buffers).
/// * `p0`: FogDensity, FogHeightFalloff (both /1000 remapped, `engine::DENSITY_REMAP`), FogHeight (cm, the component Z = `HeightFogZOffset`), FogMaxOpacity;
/// * `p1`: StartDistance (cm), DirectionalInscatteringExponent, DirectionalInscatteringStartDistance (cm), `View.VolumetricFogMaxDistance` (cm; 0 when the
///   volumetric fog does not run this frame, VolumetricFog.cpp:855-862);
/// * `dir_color`: rgb `View.DirectionalInscatteringColor` (= the fog's colour x the sun light's luminance, FogRendering.cpp:316-324), w FogCutoffDistance (cm);
/// * `color`: rgb FogInscatteringColor.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HeightFogUniform {
    pub p0: [f32; 4],
    pub p1: [f32; 4],
    pub dir_color: [f32; 4],
    pub color: [f32; 4],
}
/// The fog block of the current clock (per frame): [`HeightFogPs::active`] packed by [`HeightFogPs::uniform`]. `volumetric` = the volumetric fog runs this frame.
pub fn live_uniform(volumetric: bool) -> HeightFogUniform { HeightFogPs::active(volumetric).uniform() }
/// [`live_uniform`] at an arbitrary hour (pure; absolute UE Z, no re-base).
pub fn uniform_at(hour: f32, volumetric: bool) -> HeightFogUniform { HeightFogPs::from_phase(&PhaseFog::at_hour(hour), volumetric).uniform() }

/// `CombineVolumetricFog` at the end of the pixel shader: `rgb' = rgb * V.a + V.rgb`, `a' = a * V.a`.
pub fn combine_volumetric(fog: ([f32; 3], f32), vol: ([f32; 3], f32)) -> ([f32; 3], f32) {
    ([fog.0[0] * vol.1 + vol.0[0], fog.0[1] * vol.1 + vol.0[1], fog.0[2] * vol.1 + vol.0[2]], fog.1 * vol.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn z_params_round_trip_and_end_at_the_volume_distance() {
        let z = grid_z_params(10.0, day::VOLUMETRIC_FOG_DISTANCE, 64, 64.0);
        assert!((depth_from_slice(z, 63.0) - day::VOLUMETRIC_FOG_DISTANCE).abs() < 1.0, "{}", depth_from_slice(z, 63.0));
        assert!((depth_from_slice(z, 0.0) - 19.5).abs() < 1e-2, "slice 0 = NearPlane + 9.5 cm NearOffset (VolumetricFog.cpp:796)");
        for d in [20.0f32, 300.0, 4000.0] {
            let slice = volume_w_from_depth(z, d, 64) * 64.0;
            assert!((depth_from_slice(z, slice) - d).abs() / d < 1e-3);
        }
    }
    #[test]
    fn grid_matches_the_cvars() {
        assert_eq!(grid_size(1920, 1080), [120, 68, 64]);
    }
    #[test]
    fn material_setup_is_half_density_with_scale_split() {
        let (s, a) = material_setup(65000.0, 2.2e-7, 9e-5, 65000.0, 0.1, 1.0);
        assert!((s[0] - 1.1e-8).abs() < 1e-12 && (a - 1.1e-7).abs() < 1e-12);
        let (s2, _) = material_setup(0.0, 2.2e-7, 9e-5, 65000.0, 0.1, 1.0);
        assert!(s2[0] > s[0] * 50.0, "denser below the fog height");
    }
    #[test]
    fn integration_conserves_energy_for_a_uniform_medium() {
        let n = 8;
        let slices = vec![([1.0f32, 1.0, 1.0], 0.001f32); n];
        let out = final_integration(&slices, &vec![100.0; n]);
        let (l, t) = out[n - 1];
        // sum over the slabs of S*(1-T_i)/ext * T_before; the transmittance is exp(-total optical depth)
        assert!((t - (-0.001f32 * 800.0).exp()).abs() < 1e-4);
        assert!(l[0] > 0.0 && l[0] < 800.0);
    }
    #[test]
    fn exclusion_removes_the_height_fog_inside_the_volume() {
        let p = HeightFogPs::athena_day(true);
        let q = HeightFogPs::athena_day(false);
        let fwd = [1.0, 0.0, 0.0];
        // 40 m along the view axis, inside the 45 m volume: only the volumetric fog is left
        let near = p.eval([4000.0, 0.0, -100.0], fwd, [0.0, 0.0, 1.0], 100.0);
        assert!((near.1 - 1.0).abs() < 1e-6, "{near:?}");
        let off = q.eval([4000.0, 0.0, -100.0], fwd, [0.0, 0.0, 1.0], 100.0);
        assert!(off.1 < 1.0);
        // far away the fog is the same family: opacity capped by FogMaxOpacity
        let far = p.eval([2.0e6, 0.0, -100.0], fwd, [0.0, 0.0, 1.0], 100.0);
        assert!((far.1 - (1.0 - day::FOG_MAX_OPACITY)).abs() < 1e-5);
    }
    #[test]
    fn combine_scales_the_global_fog_by_the_volume_transmittance() {
        let c = combine_volumetric(([0.2, 0.3, 0.4], 0.5), ([0.01, 0.02, 0.03], 0.9));
        assert!((c.0[0] - 0.19).abs() < 1e-6 && (c.1 - 0.45).abs() < 1e-6);
    }
    /// TOD LIVE: the per-frame fog block of the pure Evening phase (the default clock) carries exactly the values the old baked `common.wgsl` consts had.
    #[test]
    fn wgsl_fog_uniform_matches_the_evening_phase() {
        let u = uniform_at(crate::time_of_day::SUNSET_HOUR, true);
        assert_eq!(u, HeightFogPs::from_phase(&PhaseFog::EVENING, true).uniform());
        assert_eq!(u.p0, [3.0000004e-7, 8.0000005e-5, 65000.0, 1.0]);
        assert_eq!(u.p1, [0.0, 5.0, 0.0, 4500.0]);
        assert_eq!(u.dir_color, [0.18467721, 0.018991638, 0.0, 0.0]);
        assert_eq!(&u.color[..3], &[0.62, 0.135486, 0.073669]);
        // View.VolumetricFogMaxDistance is 0 when the volumetric fog does not run (VolumetricFog.cpp:862)
        assert_eq!(uniform_at(crate::time_of_day::SUNSET_HOUR, false).p1[3], 0.0);
        assert_eq!(live_uniform(true), uniform_at(crate::time_of_day::hour(), true));
        // the WGSL block declares the members in the Rust order (the upload contract)
        let src = fn_live::wgsl!("../shaders/common.wgsl");
        let s = src.find("struct HeightFogU").expect("struct HeightFogU in common.wgsl");
        let body = &src[s..s + src[s..].find('}').unwrap()];
        let pos: Vec<usize> = ["p0:", "p1:", "dir_color:", "color:"].iter().map(|m| body.find(m).unwrap_or_else(|| panic!("{m} missing"))).collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "{pos:?}");
        assert_eq!(std::mem::size_of::<HeightFogUniform>(), 64);
        assert!(!src.contains("const FOG_DENSITY") && !src.contains("FOG-CONSTS"), "no baked fog constants left");
    }
    /// TOD LIVE: the per-frame values follow the clock through the Day -> Evening transition (16.6 h .. 19.6 h) without a pop, and the fixed hours land on
    /// their phases (noon = Day: blue inscattering, FogMaxOpacity 0.8).
    #[test]
    fn live_fog_uniform_blends_continuously_with_the_clock() {
        let day = uniform_at(12.0, true);
        assert_eq!(day, HeightFogPs::from_phase(&PhaseFog::DAY, true).uniform());
        assert!(day.color[2] > day.color[0] && day.p0[3] == 0.8);
        assert_eq!(uniform_at(23.5, true), HeightFogPs::from_phase(&PhaseFog::NIGHT, true).uniform());
        let (n, (h0, h1)) = (4000usize, (16.0f32, 20.0f32));
        let flat = |u: &HeightFogUniform| [u.p0, u.p1, u.dir_color, u.color].concat();
        let (lo, hi) = (flat(&HeightFogPs::from_phase(&PhaseFog::DAY, true).uniform()), flat(&HeightFogPs::from_phase(&PhaseFog::EVENING, true).uniform()));
        let sweep: Vec<[f32; 16]> = (0..=n).map(|i| flat(&uniform_at(h0 + (h1 - h0) * i as f32 / n as f32, true)).try_into().unwrap()).collect();
        let mut changed = 0;
        for k in 0..16 {
            let mag = sweep.iter().map(|v| v[k].abs()).fold(0f32, f32::max);
            for (i, w) in sweep.windows(2).enumerate() {
                // no pop: one 3.6 s step of the clock moves no member by more than 2 % of its largest magnitude over the sweep (the smoothstep weight moves
                // <= 1.5 * dh / 3 h = 0.05 % per step; the sun's FColor bytes add +-1-byte truncation steps to the directional-inscattering luminance)
                assert!((w[1][k] - w[0][k]).abs() <= 0.02 * mag + 1e-12, "pop at {} h, member {k}: {} -> {}", h0 + (h1 - h0) * (i + 1) as f32 / n as f32, w[0][k], w[1][k]);
                if k < 8 || k >= 12 {
                    // the plain lerped members stay between the Day and Evening values (DirectionalInscatteringColor is a product of two lerps, it may not)
                    assert!(w[1][k] >= lo[k].min(hi[k]) - 1e-6 * mag && w[1][k] <= lo[k].max(hi[k]) + 1e-6 * mag, "member {k}: {}", w[1][k]);
                }
            }
        }
        for w in sweep.windows(2) { if w[0] != w[1] { changed += 1; } }
        assert!(changed > n / 2, "the fog changes along the transition ({changed} of {n} steps)");
        assert_eq!(uniform_at(16.5, true), day, "16.5 h is still pure Day");
        assert_eq!(uniform_at(20.0, true), HeightFogPs::from_phase(&PhaseFog::EVENING, true).uniform(), "20 h is pure Evening");
    }
    /// The exe's FColor blend of the sun (UpdateDirectionalLightForTimeOfDay) truncates the lerped bytes; the fog's directional-inscattering luminance uses it.
    #[test]
    fn sun_bytes_blend_truncated_like_the_exe() {
        let m = PhaseFog::lerp(&PhaseFog::DAY, &PhaseFog::EVENING, 0.5);
        // (225 + 174) / 2 = 199.5 -> 199, (161 + 114) / 2 = 137.5 -> 137
        assert_eq!(m.sun_color_srgb, [255, 199, 137]);
        assert_eq!(m.sun_brightness, 2.0);
        let st = crate::time_of_day::state_at(19.0);
        let p = PhaseFog::at_hour(19.0);
        let k = std::f32::consts::PI;
        let r = crate::lighting::sun_radiance_of(&st);
        let l = p.sun_light_rgb();
        for i in 0..3 { assert!((l[i] / k - r[i]).abs() <= 1e-6 * r[i].max(1e-6), "fog sun {l:?} vs lighting {r:?}"); }
    }
    /// FOG INPUTS: `InitFogConstants` (FogRendering.cpp:282-323) for the Day phase, camera 2 m above the plate, sun overhead; and the per-phase values follow the clock.
    #[test]
    fn shader_constants_follow_init_fog_constants() {
        let d = PhaseFog::DAY;
        let c = shader_constants(&d, 200.0, 0.0, [0.0, 0.0, 1.0], true);
        let (dens, fall) = (0.00022 * engine::DENSITY_REMAP, 0.09 * engine::DENSITY_REMAP); // FogDensity / FogHeightFalloff / 1000 (SceneCore.cpp:427-428)
        let want_x = dens * (fall * (65000.0f32 - 200.0)).exp2();
        assert!((c[0][0] - want_x).abs() / want_x < 1e-5, "{} vs {want_x}", c[0][0]);
        assert_eq!((c[0][1], c[0][2], c[0][3]), (fall, 1.0 - 0.00001, 0.0));
        assert_eq!(c[1], [d.fog_color[0], d.fog_color[1], d.fog_color[2], 1.0 - 0.8]);
        assert_eq!(c[2], [dens, 65000.0, 0.0, 0.0]);
        assert_eq!(c[3], [0.0, 1.0, 0.0, 0.0]);
        assert!((c[4][0] - 1.0 / 99000.0).abs() < 1e-12 && (c[4][1] + 1000.0 / 99000.0).abs() < 1e-7 && c[4][2] == 1.0);
        assert_eq!(c[5], [0.0, 0.0, 1.0, 1.0]);
        let lum = ue_luma(d.sun_light_rgb());
        for k in 0..3 { assert!((c[6][k] - d.dir_color[k] * lum).abs() < 1e-7); }
        assert_eq!((c[6][3], c[7][0], c[8][0]), (3.0, 10000.0, 1.0));
        // the exponent is clamped to [-125, 126]: a camera far above the fog height
        let hi = shader_constants(&d, 65000.0 + 2.0e7, 0.0, [0.0, 0.0, 1.0], false);
        assert!((hi[0][0] - dens * (-125f32).exp2()).abs() < 1e-40 && hi[8][0] == 0.0);
        // night: denser fog, the night colour
        let n = shader_constants(&PhaseFog::NIGHT, 200.0, 0.0, [0.0, 0.0, 1.0], true);
        assert!(n[0][0] > c[0][0] && n[1][0] == PhaseFog::NIGHT.fog_color[0]);
        assert_eq!(no_fog_constants(false)[0], [0.0, 1.0, 1.0, 0.0]);
    }
    /// FOG HEIGHT RE-BASE (2026-10-07): the renderer's frame has z = 0 at the terrain ground reference (768 cm at the default layout); fog evaluated there with
    /// the re-based plane equals the fog at the absolute UE Z (FUN_14064e200 puts the component at world Z = HeightFogZOffset = 65000), for every consumer.
    #[test]
    fn fog_in_the_rebased_frame_equals_fog_at_the_absolute_z() {
        let r = 768.0;
        assert_eq!(fog_height_cm(r), 64232.0);
        for h in [6.0, 12.5, 20.0, 23.0] {
            let p = PhaseFog::at_hour(h);
            let (port, abs) = (shader_constants(&p, 200.0, r, [0.0, 0.0, 1.0], true), shader_constants(&p, 200.0 + r, 0.0, [0.0, 0.0, 1.0], true));
            assert!((port[0][0] - abs[0][0]).abs() <= 1e-5 * abs[0][0], "h {h}: {} vs {}", port[0][0], abs[0][0]);
            assert_eq!((port[2][1], abs[2][1]), (64232.0, 65000.0));
            let (fp, fa) = (HeightFogPs::from_phase(&p, true).rebased(r), HeightFogPs::from_phase(&p, true));
            for c in [[30000.0f32, 0.0, -150.0], [-80000.0, 40000.0, 900.0], [0.0, 500000.0, 20000.0]] {
                let f = { let l = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt(); [c[0] / l, c[1] / l, c[2] / l] };
                let (a, b) = (fp.eval(c, f, [0.0, 0.6, 0.8], 200.0), fa.eval(c, f, [0.0, 0.6, 0.8], 200.0 + r));
                assert!((a.1 - b.1).abs() < 1e-5 && (0..3).all(|i| (a.0[i] - b.0[i]).abs() < 1e-5), "h {h} {c:?}: {a:?} vs {b:?}");
            }
            // the un-rebased plane (the old behaviour) sat 768 cm too high above the camera: 2^(768 * 9e-5) = ~5 % MORE density at the ground
            let old = shader_constants(&p, 200.0, 0.0, [0.0, 0.0, 1.0], true);
            assert!(old[0][0] > abs[0][0] * 1.03, "h {h}");
        }
    }
    /// LIVE DUMP (2026-10-07, MATCH track): every TOD-driven fog / light value of the real 1.8 client in Athena_Terrain at 12.0 h and 17.5 h
    /// (C:\FortniteRef\lighting_ref\match\lightdump_001_user_tod12.json / lightdump_000_user_tod17p5.json: `ActiveHeightFogComponent`,
    /// `DayNightDirectionalLightComponent`, `SkyLightComponent`, the manager's `CurrentTimeOfDayFogValues` / `TransitionBlendPercent` 0.216 at 17.5 h).
    #[test]
    fn phase_blend_matches_the_live_dump() {
        // (hour, blend, density, falloff, max opacity, dir exponent, dir start, dir colour, fog colour, vol distribution, vol scale, sun bytes, intensity, sun VSI, sky VSI, sky intensity)
        let cases: [(f32, f32, f32, f32, f32, f32, f32, [f32; 3], [f32; 3], f32, f32, [u8; 3], f32, f32, f32, f32); 2] = [
            (12.0, 0.0, 0.00022, 0.09, 0.8, 3.0, 10000.0, [0.027802, 0.023625, 0.035], [0.11045, 0.277685, 0.47], 0.05, 0.1, [255, 225, 161], 3.0, 1.0, 0.65, 1.0),
            (17.5, 0.21599981, 0.00023728, 0.087840006, 0.84319997, 3.4319997, 7840.002, [0.09199671, 0.02574115, 0.027440008], [0.22051269, 0.24697003, 0.38439256],
             0.11263995, 0.20799991, [255, 213, 150], 2.5680003, 1.8639992, 0.6823999, 0.95680004),
        ];
        let close = |a: f32, b: f32| (a - b).abs() <= 1e-5 * b.abs().max(1e-3);
        for (h, t, dens, fall, opa, exp, dst, dc, fc, vd, vs, sb, br, svsi, kvsi, kint) in cases {
            let (_, _, bt) = crate::time_of_day::blend_at(h);
            assert!(h == 12.0 || close(bt, t), "h {h}: blend {bt} vs {t}");
            let p = PhaseFog::at_hour(h);
            assert!(close(p.fog_density, dens) && close(p.fog_height_falloff, fall) && close(p.fog_max_opacity, opa) && p.start_distance == 0.0, "h {h}: {p:?}");
            assert!(close(p.dir_exponent, exp) && close(p.dir_start_distance, dst) && (0..3).all(|i| close(p.dir_color[i], dc[i]) && close(p.fog_color[i], fc[i])), "h {h}: {p:?}");
            assert!(close(p.vol_scattering_distribution, vd) && close(p.vol_scattering_scale, vs) && p.vol_absorption_scale == 1.0, "h {h}: {p:?}");
            assert!(p.sun_color_srgb == sb && close(p.sun_brightness, br) && close(p.sun_vol_intensity, svsi) && close(p.sky_vol_intensity, kvsi), "h {h}: {p:?}");
            let st = crate::time_of_day::state_at(h);
            assert!(close(st.sky_light_alpha, kint) && close(crate::lighting::sky_light_tint_of(&st)[0], kint), "h {h}: sky intensity {}", st.sky_light_alpha);
        }
    }
    #[test]
    fn phase_table_day_matches_the_day_module_and_phase_windows_chain() {
        let d = PhaseFog::DAY;
        assert_eq!((d.fog_density, d.fog_height_falloff, d.fog_max_opacity, d.dir_exponent, d.dir_start_distance), (day::FOG_DENSITY, day::FOG_HEIGHT_FALLOFF, day::FOG_MAX_OPACITY, day::DIRECTIONAL_INSCATTERING_EXPONENT, day::DIRECTIONAL_INSCATTERING_START_DISTANCE));
        assert_eq!((d.dir_color, d.fog_color), (day::DIRECTIONAL_INSCATTERING_COLOR, day::FOG_INSCATTERING_COLOR));
        assert_eq!(PhaseFog::at_hour(12.0), PhaseFog::DAY);
        assert_eq!(PhaseFog::at_hour(20.5), PhaseFog::EVENING);
        assert_eq!(PhaseFog::at_hour(23.5), PhaseFog::NIGHT);
        assert_eq!(PhaseFog::at_hour(1.0), PhaseFog::NIGHT);
        let mid = PhaseFog::at_hour(19.0 + 18.0 / 60.0);
        assert!(mid.fog_density > PhaseFog::DAY.fog_density && mid.fog_density < PhaseFog::EVENING.fog_density);
    }
    #[test]
    fn sunset_fog_is_warm_and_the_sun_side_glows() {
        let p = HeightFogPs::from_phase(&PhaseFog::EVENING, true);
        let fwd = [1.0, 0.0, 0.0];
        // 3 km along a ground ray (camera 2 m up): looking at the sun vs away from it
        let toward = p.eval([300000.0, 0.0, -150.0], fwd, [1.0, 0.0, 0.0], 200.0);
        let away = p.eval([300000.0, 0.0, -150.0], fwd, [-1.0, 0.0, 0.0], 200.0);
        assert!(toward.0[0] > toward.0[2] && away.0[0] > away.0[2], "orange/red haze {toward:?} {away:?}");
        assert!(toward.0[0] > away.0[0] * 1.2, "directional inscattering brightens the sun side {toward:?} {away:?}");
        assert!(toward.1 < 1.0 && toward.1 > 0.0);
        // far fog is capped by FogMaxOpacity 1.0 -> fully fogged
        assert!(p.eval([5.0e6, 0.0, -150.0], fwd, [1.0, 0.0, 0.0], 200.0).1 < 0.2);
    }
}
