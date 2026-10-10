//! POSTFX SETTINGS: the `FFinalPostProcessSettings` fields of the post effects that fn-postfx renders (motion blur, depth of field, lens flares,
//! scene colour fringe, vignette, bloom intensity) for one view, built exactly like UE 4.16 `FSceneView` does it:
//!
//! 1. `FPostProcessSettings` constructor defaults of the 1.8 exe (`crates/fn-exe-defaults/reports/exe-defaults.json`, `FPostProcessSettings` layout;
//!    the LensFlareTints[1..7] are the UE 4.16 `Scene.h:1340-1347` literals, only [0] is in the report and matches);
//! 2. `StartFinalPostprocessSettings` (SceneView.cpp:1560-1590): `r.DefaultFeature.MotionBlur` = True keeps MotionBlurAmount, `r.DefaultFeature.LensFlare`
//!    = False zeroes LensFlareIntensity (CookedIniVersion.txt L68 / L69), `r.DefaultFeature.Bloom` = True keeps BloomIntensity (L64);
//! 3. the post-process volumes / components in priority order with `OverridePostProcessSettings(Src, Weight)` (SceneView.cpp:1207-1425): every field
//!    with `bOverride_X` becomes `lerp(Dest, Src, Weight)`, the DOF method is copied, weight <= 0 skips the component. For Athena, in
//!    `UWorld::PostProcessVolumes` order (`time_of_day::pp_volume_order`): the seven TOD components of `TODM_A` + `TODM_A_Child_Athena` (Day / Evening /
//!    Night `LowPriClonedPPC` at priority -7 / -6 / -5, weight 1; Morning .. Night at -4 .. -1 with the manager's weights, `time_of_day::pp_components`),
//!    then the storm component `OutsideSafeZonePP0` of `PlayerPawn_Athena` (priority 0) while the pawn is outside the safe zone (see [`STORM_PP`]),
//!    then the scope component (priority 10);
//! 4. `EndFinalPostprocessSettings` (SceneView.cpp:1640-1890) with the cooked scalability of the level (`quality::PostProcessScalability`):
//!    `r.SceneColorFringeQuality <= 0` -> fringe 0, `r.BloomQuality <= 0` -> bloom 0, `r.Tonemapper.Quality < 2` -> vignette 0,
//!    `r.DepthOfFieldQuality <= 0` -> DepthOfFieldScale 0, `r.MotionBlur.Scale` 1 / `.Amount` -1 / `.Max` -1 (exe defaults: no change).
//!
//! The TOD phases override these fields with the SAME value in all four phases (Morning..Night), so the phase blend cannot change them; the per-phase
//! override SETS still differ (Day / Morning also override DepthOfFieldFocalRegion, Morning / Evening / Night DepthOfFieldOcclusion and VignetteSize).
//! Because the clones always sit underneath, Day inherits DepthOfFieldOcclusion 0.25 / DepthOfFieldVignetteSize 0 from the Night clone (not the
//! ctor 0.4 / 200).
use crate::quality::PostProcessScalability;

/// `EDepthOfFieldMethod` (Scene.h): 0 = DOFM_BokehDOF (the ctor default: memset 0), 1 = DOFM_Gaussian, 2 = DOFM_CircleDOF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DofMethod { Bokeh = 0, Gaussian = 1, Circle = 2 }

/// The final settings of the effects of one view (UE units: distances in cm, sizes in percent of the view width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectSettings {
    pub bloom_intensity: f32,
    pub scene_fringe_intensity: f32,
    pub vignette_intensity: f32,
    pub lens_flare_intensity: f32,
    pub lens_flare_tint: [f32; 4],
    pub lens_flare_bokeh_size: f32,
    pub lens_flare_threshold: f32,
    pub lens_flare_tints: [[f32; 4]; 8],
    pub dof_method: DofMethod,
    pub dof_focal_distance: f32,
    pub dof_focal_region: f32,
    pub dof_near_transition: f32,
    pub dof_far_transition: f32,
    pub dof_scale: f32,
    pub dof_max_bokeh_size: f32,
    pub dof_near_blur_size: f32,
    pub dof_far_blur_size: f32,
    pub dof_occlusion: f32,
    pub dof_sky_focus_distance: f32,
    pub dof_vignette_size: f32,
    pub motion_blur_amount: f32,
    pub motion_blur_max: f32,
    pub motion_blur_per_object_size: f32,
}

/// One field override (`bOverride_X` true with value).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ov {
    BloomIntensity(f32),
    SceneFringeIntensity(f32),
    VignetteIntensity(f32),
    LensFlareIntensity(f32),
    LensFlareTint([f32; 4]),
    LensFlareBokehSize(f32),
    LensFlareThreshold(f32),
    Method(DofMethod),
    DofFocalDistance(f32),
    DofFocalRegion(f32),
    DofNearTransition(f32),
    DofFarTransition(f32),
    DofScale(f32),
    DofMaxBokehSize(f32),
    DofNearBlurSize(f32),
    DofFarBlurSize(f32),
    DofOcclusion(f32),
    DofSkyFocusDistance(f32),
    DofVignetteSize(f32),
    MotionBlurAmount(f32),
    MotionBlurMax(f32),
    MotionBlurPerObjectSize(f32),
}

impl EffectSettings {
    /// `FPostProcessSettings()` of the 1.8 exe (exe-defaults.json offsets 0x204..0x454): fringe 0, bloom 0.675, lens flare 1 / tint 1 / bokeh 3 /
    /// threshold 8, vignette 0.4, DOF method 0 (Bokeh), focal 1000, region 0, near / far transition 300 / 500, scale 0, max bokeh 15, near / far size 15,
    /// occlusion 0.4, sky focus 0, vignette size 200, motion blur 0.5 / 5 / 0.5.
    pub const EXE_DEFAULT: EffectSettings = EffectSettings {
        bloom_intensity: 0.675, scene_fringe_intensity: 0., vignette_intensity: 0.4,
        lens_flare_intensity: 1., lens_flare_tint: [1., 1., 1., 1.], lens_flare_bokeh_size: 3., lens_flare_threshold: 8.,
        lens_flare_tints: [[1., 0.8, 0.4, 0.6], [1., 1., 0.6, 0.53], [0.8, 0.8, 1., 0.46], [0.5, 1., 0.4, 0.39], [0.5, 0.8, 1., 0.31], [0.9, 1., 0.8, 0.27], [1., 0.8, 0.4, 0.22], [0.9, 0.7, 0.7, 0.15]],
        dof_method: DofMethod::Bokeh, dof_focal_distance: 1000., dof_focal_region: 0., dof_near_transition: 300., dof_far_transition: 500., dof_scale: 0.,
        dof_max_bokeh_size: 15., dof_near_blur_size: 15., dof_far_blur_size: 15., dof_occlusion: 0.4, dof_sky_focus_distance: 0., dof_vignette_size: 200.,
        motion_blur_amount: 0.5, motion_blur_max: 5., motion_blur_per_object_size: 0.5,
    };

    /// `StartFinalPostprocessSettings` with the cooked `r.DefaultFeature.*` (MotionBlur True, LensFlare False, Bloom True).
    pub fn start() -> EffectSettings {
        let mut s = Self::EXE_DEFAULT;
        s.lens_flare_intensity = 0.; // r.DefaultFeature.LensFlare=False (CookedIniVersion.txt L69)
        s
    }

    /// `OverridePostProcessSettings(Src, Weight)` for the effect fields (SceneView.cpp:1207-1425).
    pub fn apply(&mut self, ovs: &[Ov], weight: f32) {
        if weight <= 0. { return; }
        let w = weight.min(1.);
        let l = |d: &mut f32, s: f32| *d += (s - *d) * w;
        for o in ovs {
            match *o {
                Ov::BloomIntensity(v) => l(&mut self.bloom_intensity, v),
                Ov::SceneFringeIntensity(v) => l(&mut self.scene_fringe_intensity, v),
                Ov::VignetteIntensity(v) => l(&mut self.vignette_intensity, v),
                Ov::LensFlareIntensity(v) => l(&mut self.lens_flare_intensity, v),
                Ov::LensFlareTint(v) => for i in 0..4 { l(&mut self.lens_flare_tint[i], v[i]) },
                Ov::LensFlareBokehSize(v) => l(&mut self.lens_flare_bokeh_size, v),
                Ov::LensFlareThreshold(v) => l(&mut self.lens_flare_threshold, v),
                Ov::Method(m) => self.dof_method = m,
                Ov::DofFocalDistance(v) => l(&mut self.dof_focal_distance, v),
                Ov::DofFocalRegion(v) => l(&mut self.dof_focal_region, v),
                Ov::DofNearTransition(v) => l(&mut self.dof_near_transition, v),
                Ov::DofFarTransition(v) => l(&mut self.dof_far_transition, v),
                Ov::DofScale(v) => l(&mut self.dof_scale, v),
                Ov::DofMaxBokehSize(v) => l(&mut self.dof_max_bokeh_size, v),
                Ov::DofNearBlurSize(v) => l(&mut self.dof_near_blur_size, v),
                Ov::DofFarBlurSize(v) => l(&mut self.dof_far_blur_size, v),
                Ov::DofOcclusion(v) => l(&mut self.dof_occlusion, v),
                Ov::DofSkyFocusDistance(v) => l(&mut self.dof_sky_focus_distance, v),
                Ov::DofVignetteSize(v) => l(&mut self.dof_vignette_size, v),
                Ov::MotionBlurAmount(v) => l(&mut self.motion_blur_amount, v),
                Ov::MotionBlurMax(v) => l(&mut self.motion_blur_max, v),
                Ov::MotionBlurPerObjectSize(v) => l(&mut self.motion_blur_per_object_size, v),
            }
        }
    }

    /// `EndFinalPostprocessSettings` (SceneView.cpp:1655-1745) for the cooked scalability `p` (show flags all on).
    pub fn end(&mut self, p: &PostProcessScalability) {
        if p.scene_color_fringe_quality <= 0 { self.scene_fringe_intensity = 0.; }
        if p.bloom_quality <= 0 { self.bloom_intensity = 0.; }
        if p.tonemapper_quality < 2 { self.vignette_intensity = 0.; }
        if p.depth_of_field_quality <= 0 { self.dof_scale = 0.; }
        // r.MotionBlur.Scale 1 (clamp 0..50), r.MotionBlur.Amount / .Max -1 = no override (exe defaults, no ini line): unchanged
    }

    /// The final settings of an Athena view: the TOD stack for the blend (`from`, `to`, `t`) of `time_of_day::blend_at` (the same stack as
    /// tonemap::PostSettings::blend), the storm component at `storm_weight` (0 = inside the safe zone), the scope component at `scope_weight`, then the
    /// scalability of the post-process level. Components are registered TOD manager (level actor) -> pawn (storm) -> weapon (scope) and walked in
    /// `InsertVolume` order (priority ascending, equal priority in registration order): clones -7..-5, phases -4..-1, storm 0, scope 10.
    pub fn athena(from: usize, to: usize, t: f32, storm_weight: f32, scope_weight: f32, p: &PostProcessScalability) -> EffectSettings {
        use crate::time_of_day::{pp_components, pp_local_weight, pp_volume_order};
        let mut s = Self::start();
        let scope = scope_pp(scope_weight);
        let mut comps: Vec<(f32, &[Ov], f32)> = pp_components(from, to, t).iter().map(|c| (c.priority, TOD_PHASES[c.phase], c.weight)).collect();
        comps.push((STORM_PRIORITY, STORM_PP, storm_weight));
        comps.push((SCOPE_PRIORITY, &scope, scope_weight));
        let pri: Vec<f32> = comps.iter().map(|c| c.0).collect();
        for i in pp_volume_order(&pri) {
            if let Some(w) = pp_local_weight(comps[i].2) { s.apply(comps[i].1, w); }
        }
        s.end(p);
        s
    }
    /// [`Self::athena`] at the current `time_of_day` clock.
    pub fn current(storm_weight: f32, scope_weight: f32, p: &PostProcessScalability) -> EffectSettings {
        let st = crate::time_of_day::current();
        Self::athena(st.from, st.to, st.blend, storm_weight, scope_weight, p)
    }

    /// `IsMotionBlurEnabled` (VelocityRendering.cpp:596-613; 1.8 exe @1419ed9b0 tests `(EngineShowFlags & 0x1001) == 0x1001`): show flags
    /// PostProcessing + MotionBlur (the user's `bMotionBlur`, `UFortGameUserSettings::ApplyNonResolutionSettings` @1405bcb10 sets bit 12), amount and max
    /// above 0.001, `r.MotionBlurQuality` > 0.
    pub fn motion_blur_enabled(&self, show_flag: bool, p: &PostProcessScalability) -> bool {
        show_flag && self.motion_blur_amount > 0.001 && self.motion_blur_max > 0.001 && p.motion_blur_quality.clamp(0, 4) > 0
    }
    /// Gaussian / circle DOF run when `r.DepthOfFieldQuality > 0` and the method is not Bokeh (PostProcessing.cpp:1369-1374); Bokeh additionally
    /// needs `DepthOfFieldScale > 0` (:1411-1414).
    pub fn dof_gaussian(&self, p: &PostProcessScalability) -> bool { p.depth_of_field_quality > 0 && self.dof_method == DofMethod::Gaussian }
    /// `USE_COLOR_FRINGE` of the tonemapper: `SceneFringeIntensity > 0.01` (PostProcessTonemap.cpp:620).
    pub fn fringe_on(&self) -> bool { self.scene_fringe_intensity > 0.01 }
    /// `FringeUVParams` (PostProcessTonemap.cpp:1368-1385 / the PS twin): (1 / (1 + Offset * ScaleG), 1 / (1 + Offset * ScaleR)), Offset = intensity / 100,
    /// Scale = 0.007 * (primary wavelength - 464.3 nm), primaries R 611.3, G 549.1 nm. `.x` scales the G sample position, `.y` the R one.
    pub fn fringe_uv_params(&self) -> [f32; 2] {
        let off = self.scene_fringe_intensity * 0.01;
        let (r, g, b) = (611.3f32, 549.1f32, 464.3f32);
        let (scale_r, scale_g) = (0.007 * (r - b), 0.007 * (g - b));
        [1. / (1. + off * scale_g), 1. / (1. + off * scale_r)]
    }
    /// `TonemapperVignette` permutation: `VignetteIntensity > 0` (PostProcessTonemap.cpp:571).
    pub fn vignette_on(&self) -> bool { self.vignette_intensity > 0. }
}

use DofMethod::Gaussian;
use Ov::*;
/// The phase components (`DayPhasePostProcessComponent[0..3]`: Morning, Day, Evening, Night) of the merged TODM_A + TODM_A_Child_Athena, the effect fields
/// only. RECOVERED from `reports/postfx-evidence.json` `todm.phases.<phase>.applied_tagged` (tagged + flag) and `overridden_untagged_values_assumed` (flag,
/// no tag = the ctor default: DepthOfFieldFocalRegion 0, SceneFringeIntensity 0); the bloom fields are owned by tonemap.rs / fn_postfx::params.
const TOD_PHASES: [&[Ov]; 4] = [
    // Morning
    &[Method(Gaussian), DofFocalDistance(15000.), DofFocalRegion(0.), DofNearTransition(0.), DofFarTransition(50000.), DofNearBlurSize(0.), DofFarBlurSize(0.019_999_999_552_965_164),
      DofOcclusion(0.25), DofSkyFocusDistance(200000.), DofVignetteSize(0.), LensFlareIntensity(0.), MotionBlurAmount(0.125), MotionBlurMax(2.), VignetteIntensity(0.009_999_999_776_482_582), SceneFringeIntensity(0.)],
    // Day
    &[Method(Gaussian), DofFocalDistance(15000.), DofFocalRegion(0.), DofNearTransition(0.), DofFarTransition(50000.), DofNearBlurSize(0.), DofFarBlurSize(0.019_999_999_552_965_164),
      DofSkyFocusDistance(200000.), LensFlareIntensity(0.), MotionBlurAmount(0.125), MotionBlurMax(2.), VignetteIntensity(0.009_999_999_776_482_582), SceneFringeIntensity(0.)],
    // Evening
    &[Method(Gaussian), DofFocalDistance(15000.), DofNearTransition(0.), DofFarTransition(50000.), DofNearBlurSize(0.), DofFarBlurSize(0.019_999_999_552_965_164),
      DofOcclusion(0.25), DofSkyFocusDistance(200000.), DofVignetteSize(0.), LensFlareIntensity(0.), MotionBlurAmount(0.125), MotionBlurMax(2.), VignetteIntensity(0.009_999_999_776_482_582), SceneFringeIntensity(0.)],
    // Night
    &[Method(Gaussian), DofFocalDistance(15000.), DofNearTransition(0.), DofFarTransition(50000.), DofNearBlurSize(0.), DofFarBlurSize(0.019_999_999_552_965_164),
      DofOcclusion(0.25), DofSkyFocusDistance(200000.), DofVignetteSize(0.), LensFlareIntensity(0.), MotionBlurAmount(0.125), MotionBlurMax(2.), VignetteIntensity(0.009_999_999_776_482_582), SceneFringeIntensity(0.)],
];

/// STORM: `OutsideSafeZonePP0` (PostProcessComponent of `PlayerPawn_Athena`, export [350], created by the native AFortPlayerPawnAthena ctor @140413f00:
/// BlendWeight 0, Priority 0, unbound), the effect fields it overrides ([OV] = bOverride true): BloomIntensity 2.5, SceneFringeIntensity 0.7,
/// VignetteIntensity 1.1. Its DOF values (Gaussian, focal 1560, far 2.8, far transition 4862) carry NO override flag and are ignored by the blend; no lens
/// flare / motion blur overrides. Its bloom threshold / size (0.25 / 2.0) are applied by the renderer's bloom params ([`STORM_BLOOM`]); its colour grading,
/// LUT, exposure and the M_Screen_Storm_Overlay blendable are NOT ported here. Weight: see fn-app storm_hook.rs (FInterpTo speed 2 towards 1 outside the zone).
const STORM_PP: &[Ov] = &[BloomIntensity(2.5), SceneFringeIntensity(0.7), VignetteIntensity(1.1)];
/// STORM: `OutsideSafeZonePP0` Priority (AFortPlayerPawnAthena ctor @140413f00, see [`STORM_PP`]).
const STORM_PRIORITY: f32 = 0.0;
/// SCOPE: `DownScopePostProcess` Priority (B_Ranged_Generic export [317], see [`scope_pp`]).
const SCOPE_PRIORITY: f32 = 10.0;
/// STORM: the bloom chain fields of `OutsideSafeZonePP0` ([OV]): (BloomThreshold, BloomSizeScale) = (0.25, 2.0); Bloom1-5 sizes / tints, Bloom3Tint 0.5,
/// Bloom6 0 equal the TOD values.
pub const STORM_BLOOM: (f32, f32) = (0.25, 2.0);

/// SCOPE: `DownScopePostProcess` of `B_Ranged_Generic` (export [317], Priority 10, unbound, template BlendWeight 0), [OV] fields: DOF Gaussian, FocalDistance 0,
/// NearBlurSize 20, NearTransitionRegion 0, SceneFringeIntensity 1.0, VignetteIntensity 1.0, DepthOfFieldVignetteSize (the timeline update writes
/// `Lerp(200, 62, w)` and forces its override on). FarTransitionRegion 0 is unflagged. Weight w = the `AnimateScopePostProcess` timeline (0.3 s: 0 until 0.15,
/// 1 at 0.20), the same value as the material's DownScopePostProcessAmount (fn-app scope_hook.rs). Only weapons with UseScopeTargeting (the Athena sniper,
/// B_Assault_Surgical_Athena, B_Rifle_TripleShot_Athena) run it.
fn scope_pp(w: f32) -> [Ov; 7] {
    [Method(Gaussian), DofFocalDistance(0.), DofNearBlurSize(20.), DofNearTransition(0.), SceneFringeIntensity(1.), VignetteIntensity(1.), DofVignetteSize(200. + (62. - 200.) * w.clamp(0., 1.))]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn athena_high_matches_the_cooked_tod_and_scalability() {
        let s = EffectSettings::athena(1, 1, 0., 0., 0., &PostProcessScalability::level(2));
        assert_eq!(s.dof_method, DofMethod::Gaussian);
        assert!((s.motion_blur_amount - 0.125).abs() < 1e-7 && (s.motion_blur_max - 2.).abs() < 1e-7 && s.motion_blur_per_object_size == 0.5);
        assert_eq!(s.lens_flare_intensity, 0.);
        assert_eq!(s.scene_fringe_intensity, 0.);
        assert!((s.vignette_intensity - 0.01).abs() < 1e-6);
        // Day does not override DepthOfFieldOcclusion / VignetteSize: the Night clone underneath (priority -5) supplies 0.25 / 0, not the ctor 0.4 / 200
        assert_eq!((s.dof_occlusion, s.dof_vignette_size), (0.25, 0.));
        assert!(s.dof_gaussian(&PostProcessScalability::level(2)));
        assert!(s.motion_blur_enabled(true, &PostProcessScalability::level(2)));
        assert!(!s.motion_blur_enabled(false, &PostProcessScalability::level(2)), "bMotionBlur = false clears ShowFlag.MotionBlur");
        // Low: r.MotionBlurQuality 0, r.DepthOfFieldQuality 0, r.Tonemapper.Quality 0 -> no motion blur, no DOF, no vignette
        let low = PostProcessScalability::level(0);
        let s0 = EffectSettings::athena(1, 1, 0., 0., 0., &low);
        assert!(!s0.motion_blur_enabled(true, &low) && !s0.dof_gaussian(&low) && s0.vignette_intensity == 0.);
    }
    #[test]
    fn storm_and_scope_components_blend_on_top_of_the_tod() {
        let p = PostProcessScalability::level(2);
        let s = EffectSettings::athena(1, 1, 0., 1., 0., &p);
        assert_eq!((s.bloom_intensity, s.scene_fringe_intensity, s.vignette_intensity), (2.5, 0.7, 1.1));
        assert!((s.dof_far_blur_size - 0.02).abs() < 1e-6, "the storm DOF fields are not overridden");
        let h = EffectSettings::athena(1, 1, 0., 0.5, 0., &p);
        assert!((h.scene_fringe_intensity - 0.35).abs() < 1e-6);
        // scoped: Gaussian near layer 20 %, focal 0, DOF vignette 62 at full weight; fringe / vignette 1.0; Low has no fringe (r.SceneColorFringeQuality 0)
        let sc = EffectSettings::athena(1, 1, 0., 0., 1., &p);
        assert_eq!((sc.dof_near_blur_size, sc.dof_focal_distance, sc.dof_vignette_size, sc.scene_fringe_intensity, sc.vignette_intensity), (20., 0., 62., 1., 1.));
        assert_eq!(EffectSettings::athena(1, 1, 0., 0., 1., &PostProcessScalability::level(1)).scene_fringe_intensity, 0.);
    }
    /// PP STACK: the effect fields follow the same component stack as the tonemapper at every hour; storm (0) and scope (10) stay above every TOD
    /// component, so at full weight they win at any hour, and at half weight they lerp from the stacked TOD value.
    #[test]
    fn tod_stack_storm_and_scope_order_holds_at_every_hour() {
        let p = PostProcessScalability::level(2);
        for k in 0..96 {
            let h = k as f32 * 0.25;
            let (f, to, t) = crate::time_of_day::blend_at(h);
            let s = EffectSettings::athena(f, to, t, 0., 0., &p);
            assert!((s.dof_occlusion - 0.25).abs() < 1e-6 && s.dof_vignette_size.abs() < 1e-4, "h {h}: {s:?}");
            assert!((s.motion_blur_amount - 0.125).abs() < 1e-6 && (s.vignette_intensity - 0.01).abs() < 1e-6 && s.dof_method == Gaussian);
            let st = EffectSettings::athena(f, to, t, 1., 0., &p);
            assert_eq!((st.scene_fringe_intensity, st.vignette_intensity), (0.7, 1.1), "h {h}");
            let half = EffectSettings::athena(f, to, t, 0.5, 0., &p);
            assert!((half.vignette_intensity - (0.01 + (1.1 - 0.01) * 0.5)).abs() < 1e-5, "h {h}");
            let sc = EffectSettings::athena(f, to, t, 1., 1., &p);
            assert_eq!((sc.scene_fringe_intensity, sc.vignette_intensity, sc.dof_focal_distance), (1., 1., 0.), "scope above storm at h {h}");
        }
    }
    #[test]
    fn override_lerps_by_weight_and_copies_the_method() {
        let mut s = EffectSettings::start();
        s.apply(&[MotionBlurAmount(1.5), Method(Gaussian)], 0.5);
        assert!((s.motion_blur_amount - 1.0).abs() < 1e-6);
        assert_eq!(s.dof_method, DofMethod::Gaussian);
        let mut z = EffectSettings::start();
        z.apply(&[Method(Gaussian)], 0.);
        assert_eq!(z.dof_method, DofMethod::Bokeh, "weight <= 0 skips the component");
    }
    #[test]
    fn fringe_uv_params_follow_the_tonemapper() {
        let mut s = EffectSettings::start();
        s.scene_fringe_intensity = 0.7;
        let [g, r] = s.fringe_uv_params();
        assert!((g - 1. / (1. + 0.007 * 0.007 * (549.1 - 464.3))).abs() < 1e-6);
        assert!((r - 1. / (1. + 0.007 * 0.007 * (611.3 - 464.3))).abs() < 1e-6);
        assert!(r < g && g < 1.);
    }
}
