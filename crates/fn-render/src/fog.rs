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

/// `FVolumetricFogUniformParameters` / `View.VolumetricFogGridZParams` (B, O, S): the disassembled shaders use
/// `depth = (2^(slice / S) - O) / B` and `slice = log2(depth * B + O) * S`.
/// The (B, O) construction is the engine's C++ (`GetVolumetricFogGridZParams`, RECALLED from the 4.16 source; the shaders only consume it and a static scan of the exe did not
/// locate the writer; consistent with the disassembled inverse above):`O = (F - N * 2^((GridSizeZ - 1) / S)) / (F - N)`, `B = (1 - O) / N`.
pub fn grid_z_params(near_cm: f32, far_cm: f32, grid_size_z: u32, depth_distribution_scale: f32) -> [f32; 3] {
    let far = far_cm.max(near_cm + 1.0);
    let o = (far - near_cm * ((grid_size_z as f32 - 1.0) / depth_distribution_scale).exp2()) / (far - near_cm);
    let b = (1.0 - o) / near_cm;
    [b, o, depth_distribution_scale]
}
/// View-space depth (cm) of a (possibly fractional) Z slice: `FVolumetric*CS` `div / exp / add / div` sequence.
pub fn depth_from_slice(z: [f32; 3], slice: f32) -> f32 { ((slice / z[2]).exp2() - z[1]) / z[0] }
/// Z texture coordinate (0..1 over the grid) of a view-space depth, as the fog pixel shader computes it (`log`, `mul cb1[176].z`, `mul cb1[175].z`).
pub fn volume_w_from_depth(z: [f32; 3], depth_cm: f32, grid_size_z: u32) -> f32 { (depth_cm * z[0] + z[1]).max(1e-30).log2() * z[2] / grid_size_z as f32 }
/// Grid size in voxels for a viewport (`GridPixelSize` 16, `GridSizeZ` 64).
pub fn grid_size(width: u32, height: u32) -> [u32; 3] {
    [width.div_ceil(engine::GRID_PIXEL_SIZE), height.div_ceil(engine::GRID_PIXEL_SIZE), engine::GRID_SIZE_Z]
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
    /// The Athena Day values (remapped as the engine does).
    pub fn athena_day(volumetric: bool) -> Self {
        HeightFogPs {
            density: day::FOG_DENSITY * engine::DENSITY_REMAP, falloff: day::FOG_HEIGHT_FALLOFF * engine::DENSITY_REMAP, fog_height_cm: day::HEIGHT_FOG_Z_OFFSET,
            max_opacity: day::FOG_MAX_OPACITY, start_distance_cm: day::START_DISTANCE, cutoff_distance_cm: 0.0, fog_color: day::FOG_INSCATTERING_COLOR,
            dir_color: day::DIRECTIONAL_INSCATTERING_COLOR, dir_exponent: day::DIRECTIONAL_INSCATTERING_EXPONENT, dir_start_cm: day::DIRECTIONAL_INSCATTERING_START_DISTANCE,
            volumetric_max_distance_cm: if volumetric { day::VOLUMETRIC_FOG_DISTANCE } else { 0.0 },
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
trait NegExp2 { fn neg_exp2(self) -> f32; }
impl NegExp2 for f32 { fn neg_exp2(self) -> f32 { (-self).exp2() } }

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
        assert!((depth_from_slice(z, 0.0) - 10.0).abs() < 1e-2);
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
}
