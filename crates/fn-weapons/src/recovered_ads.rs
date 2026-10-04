//! Aim down sights ("targeting" in the original), recovered from the 1.8 data and bytecode. No file access here: `fn_pose::loadout_ads` reads the
//! numbers from the install into these types, the front-end drives [`AdsMachine`].
//!
//! RECOVERED (serialized data / Kismet of the shipped Blueprints)
//! * Per weapon actor Blueprint: `CameraBase3PClass` (hold camera), `CameraTargeting3PClass` (ADS camera), `bUseScopeTargeting`
//!   (`B_Rifle_Sniper_Athena`: true; the AR / pump shotgun do not set it).
//! * Camera mode Blueprint class defaults along the `super` chain down to `Athena_PlayerCameraModeBase`: `FOV`, `CameraSpaceForwardDistance`,
//!   `ViewTargetSpaceViewOffset(+_Crouch)`, `CameraSpaceViewOffset`, origin / offset interpolation speeds, `TransitionTime` / `TransitionOutTime`,
//!   `TransitionParams` (`BlendFunction` EaseInOut, `BlendTime`, `BlendExp`).
//! * `B_Ranged_Generic` bytecode (`OnSetTargeting`, the `AnimateScopePostProcess` timeline events): scoped weapons wait 0.2 s after the press
//!   (`Delay(0.2)` in the bNewIsTargeting branch) and then play the 0.3 s timeline forward (`CurveFloat_0` = `DownSightPostProcessAmount`: 0 until
//!   0.15 s, linear to 1 at 0.2 s, 1 until 0.3 s) with the ZoomIn sound; on release they wait 0.1 s (`Delay(0.1)`), restore the pawn / reticle and play
//!   the timeline in reverse with the ZoomOut sound. The Finished event hides `ScopeMesh1P` and zeroes both `PostProcessComponent::BlendWeight`s after a
//!   reverse run; after a forward run (still targeting) it hides the pawn and the 3P weapon mesh, shows `ScopeMesh1P` and hides the reticle HUD elements
//!   (`ReticleHUDElementTags`).
//! * The stat row's `*Downsights*` columns (kept as read: the runtime has no spread / recoil yet).
//!
//! NOT RECOVERED (native C++, never in a pak): the native `AFortWeapon::SetTargeting` / `bIsTargeting` replication path, the camera manager's mode
//! blend itself (see `ViewBlend`), the walking-speed rule while targeting, the mouse-sensitivity formula behind `MouseTargetingMultiplier`, and the
//! compiled post-process material (`DownScopePostProcess`: DXBC in the package) that paints the scope.
//!
//! ASSUMED (labelled where used): see the `ASSUMED` notes on [`ViewBlend::in_duration`], [`ViewBlend::ENGINE_DEFAULT_EXP`], [`AdsMachine::look_scale`].

/// One `InterpOffsetData` sample: pitch angle, view offset (cm, X forward / Y right / Z up).
pub type OffsetSample = (f32, [f32; 3]);

/// `FViewTargetTransitionParams` as serialized on a camera mode (`BlendFunction` / `BlendTime` / `BlendExp`; fields at their archetype value are not
/// serialized, so `None` = inherited from the parent class, resolved along the chain by the loader and left `None` only when nothing sets it).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewBlend { pub function: String, pub blend_time: Option<f32>, pub blend_exp: Option<f32> }
impl ViewBlend {
    /// ASSUMED: `FViewTargetTransitionParams::BlendExp` default (2) of UE 4.16 when no class in the chain serializes it.
    pub const ENGINE_DEFAULT_EXP: f32 = 2.;
    /// Engine `EViewTargetBlendFunction` evaluation (`FMath::InterpEaseInOut` etc., UE 4.16): weight for a linear progress `alpha` in 0..1.
    pub fn weight(&self, alpha: f32) -> f32 {
        let a = alpha.clamp(0., 1.);
        let e = self.blend_exp.unwrap_or(Self::ENGINE_DEFAULT_EXP);
        match self.function.as_str() {
            "VTBlend_Linear" => a,
            "VTBlend_Cubic" => a * a * (3. - 2. * a),
            "VTBlend_EaseIn" => a.powf(e),
            "VTBlend_EaseOut" => 1. - (1. - a).powf(e),
            // VTBlend_EaseInOut (the only function the camera modes of the starter weapons use)
            _ => if a < 0.5 { 0.5 * (2. * a).powf(e) } else { 1. - 0.5 * (2. * (1. - a)).powf(e) },
        }
    }
    /// Seconds to blend INTO the mode. `TransitionTime` belongs to the camera
    /// mode and is the transition duration; `TransitionParams.BlendTime` only
    /// controls its blend curve.  The scope mode carries both values (0.25 s
    /// and 0.1 s respectively), so using the latter made its camera snap too
    /// early.
    pub fn in_duration(&self, transition_time: f32) -> f32 { transition_time }
}

/// A `FortCameraMode_ThirdPerson` Blueprint with the class defaults of its whole `super` chain applied (root = `Athena_PlayerCameraModeBase`).
#[derive(Clone, Debug, PartialEq)]
pub struct CameraModeData {
    /// Leaf class first, `Athena_PlayerCameraModeBase` last.
    pub chain: Vec<String>,
    pub fov: f32,
    pub forward_distance: f32,
    pub origin_interp: [f32; 3],
    pub vt_offset: Vec<OffsetSample>, pub vt_offset_crouch: Vec<OffsetSample>,
    pub vt_interp: [f32; 3],
    pub cs_offset: Vec<OffsetSample>,
    pub push_out: f32,
    /// `HidePawnPenetrationPercent`: not serialized on the Base class (native default 0).
    pub hide_pawn_pct: f32,
    pub transition_in: f32, pub transition_out: f32,
    pub blend: ViewBlend,
}

/// The scope part of a weapon (`bUseScopeTargeting`): everything `B_Ranged_Generic` / the sniper Blueprint / the scope material instance serialize.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeData {
    /// `AnimateScopePostProcess_Template.TimelineLength`.
    pub timeline_length: f32,
    /// `CurveFloat_0` keys (time, value), all `RCIM_Linear`.
    pub curve: Vec<(f32, f32)>,
    /// `Delay` immediates of `OnSetTargeting`: scope-in (bNewIsTargeting) and scope-out.
    pub start_delay: f32, pub stop_delay: f32,
    /// `DownScopePostProcess` blendable: the material instance, its parent, and the original reticle texture bound to the `Reticle` parameter.
    pub material: String, pub parent_material: String, pub reticle_texture: String,
    /// Scalar / vector parameters of the material (parent defaults overridden by the instance), `(name, value)`.
    pub scalars: Vec<(String, f32)>, pub vectors: Vec<(String, [f32; 4])>,
    /// `ReticleHUDElementTags` hidden while the timeline finished forward.
    pub reticle_hud_tags: Vec<String>,
    /// `Sound_ScopeZoomIn` / `Sound_ScopeZoomOut` object paths.
    pub sound_zoom_in: String, pub sound_zoom_out: String,
    /// The stage-1 `PostProcessComponent` settings of the sniper Blueprint (`DownScopePostProcess_GEN_VARIABLE`): vignette / scene fringe intensity.
    pub vignette_intensity: Option<f32>, pub scene_fringe_intensity: Option<f32>,
    pub scope_camera_offset_amount: f32,
    /// The scope mesh `S_SniperScope_Proto` (ScopeMesh1P) object path.
    pub scope_mesh: String,
}
impl ScopeData {
    /// `DownSightPostProcessAmount` at timeline position `t` (linear keys, clamped at both ends).
    pub fn amount(&self, t: f32) -> f32 {
        let k = &self.curve;
        if k.is_empty() { return 0.; }
        if t <= k[0].0 { return k[0].1; }
        for w in k.windows(2) { if t <= w[1].0 { let f = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-9); return w[0].1 + (w[1].1 - w[0].1) * f; } }
        k[k.len() - 1].1
    }
}

/// Everything ADS needs to know about one weapon.
#[derive(Clone, Debug, PartialEq)]
pub struct AdsProfile {
    /// Weapon item definition path and its actor Blueprint path.
    pub item: String, pub actor: String,
    pub base: CameraModeData, pub targeting: CameraModeData,
    pub use_scope: bool,
    pub scope: Option<ScopeData>,
    /// Stat row `SpreadDownsights`, `SpreadDownsightsAdditionalCooldownTime`, `RecoilDownsightsMultiplier`, `AthenaRecoilDownsightsMultiplier` (no spread / recoil system yet).
    pub spread_downsights: f32, pub spread_downsights_cooldown: f32, pub recoil_downsights_multiplier: f32, pub athena_recoil_downsights_multiplier: f32,
    /// Actor `bMaintainAimLocationDuringTargeting` / `bShouldAimFromMuzzleAtCloseRange` (no projectile origin logic yet).
    pub maintain_aim_location: bool, pub aim_from_muzzle_at_close_range: bool,
}
impl AdsProfile {
    pub fn fov_base(&self) -> f32 { self.base.fov }
    pub fn fov_targeting(&self) -> f32 { self.targeting.fov }
}

/// One frame of ADS intent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AdsInput {
    /// The Target binding is down (hold) / was pressed this frame (toggle: `pressed_edge`).
    pub held: bool,
    /// A weapon with an ADS profile is in hand and nothing blocks aiming.
    pub can_aim: bool,
    /// Input.ToggleTargeting: a press flips the request instead of following the key.
    pub toggle: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Dir { #[default] Idle, Forward, Backward }

/// Per-frame ADS state of the local pawn. All timing comes from the profile.
#[derive(Clone, Debug, Default)]
pub struct AdsMachine {
    held_prev: bool,
    /// `bIsTargeting` (the weapon's targeting request after hold / toggle).
    pub targeting: bool,
    /// Camera blend progress 0..1 (linear time; weight = `ViewBlend::weight(progress)`).
    pub cam_progress: f32,
    // scope sequence
    since_press: f32, pending_play: bool, stop_wait: Option<f32>,
    dir: Dir, pos: f32,
    scope_mesh: bool, pawn_hidden: bool, reticle_hidden: bool,
    scope_active: bool,
    /// Counters for tests / logs: zoom-in / zoom-out sounds requested (the front-end plays them).
    pub zoom_in_events: u32, pub zoom_out_events: u32,
}
impl AdsMachine {
    pub fn new() -> Self { Self::default() }
    /// Drops all state (weapon change / unequip). The scope sequence is cut like `K2_OnUnEquip` (timeline stopped, mesh hidden).
    pub fn reset(&mut self) { *self = AdsMachine { zoom_in_events: self.zoom_in_events, zoom_out_events: self.zoom_out_events, ..Default::default() }; }
    pub fn is_targeting(&self) -> bool { self.targeting }
    /// Camera weight of the ADS mode (0 = hold camera, 1 = targeting camera) for `p`.
    pub fn camera_weight(&self, p: &AdsProfile) -> f32 { p.targeting.blend.weight(self.cam_progress) }
    /// True while the ADS camera has any weight (the camera must keep running the targeting mode).
    pub fn camera_active(&self) -> bool { self.targeting || self.cam_progress > 0. }
    /// `DownSightPostProcessAmount` (the blend weight of the scope post-process).
    pub fn scope_amount(&self, p: &AdsProfile) -> f32 { p.scope.as_ref().map_or(0., |s| s.amount(self.pos)) }
    pub fn scope_mesh_visible(&self) -> bool { self.scope_mesh }
    pub fn pawn_hidden(&self) -> bool { self.pawn_hidden }
    pub fn reticle_hidden(&self) -> bool { self.reticle_hidden }
    pub fn timeline_pos(&self) -> f32 { self.pos }

    /// ASSUMED: the Mouse ADS / Mouse Scope sensitivity sliders (`MouseTargetingMultiplier` / `MouseScopedMultiplier`, 0..1, tooltip "Slow down ...") multiply the
    /// look rate while targeting; the native formula is not recoverable. The scope slider applies to a scoped weapon, the ADS slider to every other one.
    pub fn look_scale(&self, p: &AdsProfile, ads_mult: f32, scope_mult: f32) -> f32 {
        if !self.targeting { 1. } else if p.use_scope { scope_mult } else { ads_mult }
    }
    /// ASSUMED (native `GetMaxSpeed` is not recoverable): while targeting the pawn is limited to the `SpeedWalk` stat and cannot sprint.
    pub fn limits_movement(&self) -> bool { self.targeting }

    /// Advances by `dt`. `p` is the held weapon's profile (None = nothing aimable: any state is dropped).
    pub fn update(&mut self, dt: f32, inp: AdsInput, p: Option<&AdsProfile>) {
        let Some(p) = p.filter(|_| inp.can_aim) else {
            if self.targeting || self.cam_progress > 0. || self.dir != Dir::Idle || self.scope_mesh || self.pawn_hidden { self.targeting = false; self.held_prev = inp.held; self.cut(dt, p); } else { self.held_prev = inp.held; }
            return;
        };
        let edge = inp.held && !self.held_prev; self.held_prev = inp.held;
        let want = if inp.toggle { if edge { !self.targeting } else { self.targeting } } else { inp.held };
        let changed = want != self.targeting;
        self.targeting = want;
        // ---- camera blend: progress runs at 1 / duration towards 1 (targeting) or 0 (released), continuing from wherever it is
        let dur = if self.targeting { p.targeting.blend.in_duration(p.targeting.transition_in) } else { p.targeting.transition_out }.max(1e-4);
        // Advance both directions in normalized transition time.  The old
        // targeting branch added raw seconds while the release branch divided
        // by `dur`, causing an asymmetric scope-in camera transition.
        self.cam_progress = (self.cam_progress + (if self.targeting { dt } else { -dt }) / dur).clamp(0., 1.);
        // ---- scope sequence (OnSetTargeting / timeline events of B_Ranged_Generic)
        let Some(sc) = p.scope.as_ref().filter(|_| p.use_scope) else { return };
        if changed {
            if self.targeting { self.since_press = 0.; self.pending_play = true; self.stop_wait = None; self.scope_active = true; }
            else { self.pending_play = false; self.stop_wait = Some(sc.stop_delay); }
        }
        if self.targeting && self.pending_play { self.since_press += dt; if self.since_press >= sc.start_delay { self.pending_play = false; self.dir = Dir::Forward; self.zoom_in_events += 1; } }
        if let Some(w) = self.stop_wait.as_mut() { *w -= dt; if *w <= 0. {
            self.stop_wait = None;
            // unscope continuation: reticle restored, pawn shown, ZoomOut sound, timeline reversed
            self.reticle_hidden = false; self.pawn_hidden = false; self.dir = Dir::Backward; self.zoom_out_events += 1;
        } }
        match self.dir {
            Dir::Forward => { self.pos += dt; if self.pos >= sc.timeline_length { self.pos = sc.timeline_length; self.dir = Dir::Idle;
                if self.targeting { self.pawn_hidden = true; self.scope_mesh = true; self.reticle_hidden = true; } } }
            Dir::Backward => { self.pos -= dt; if self.pos <= 0. { self.pos = 0.; self.dir = Dir::Idle; self.scope_mesh = false; self.scope_active = false; } }
            Dir::Idle => {}
        }
    }
    /// Weapon switched / aiming impossible: everything snaps back (the timeline is stopped, the camera blend runs out at the mode's TransitionOutTime).
    fn cut(&mut self, dt: f32, p: Option<&AdsProfile>) {
        let dur = p.map_or(0.2, |p| p.targeting.transition_out).max(1e-4);
        self.cam_progress = (self.cam_progress - dt / dur).max(0.);
        self.pending_play = false; self.stop_wait = None; self.dir = Dir::Idle; self.pos = 0.; self.scope_mesh = false; self.pawn_hidden = false; self.reticle_hidden = false; self.scope_active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mode(fov: f32, tin: f32, tout: f32, bt: Option<f32>, exp: Option<f32>) -> CameraModeData {
        CameraModeData { chain: vec![], fov, forward_distance: -160., origin_interp: [60., 1e7, 60.], vt_offset: vec![(0., [83., 31., 70.])], vt_offset_crouch: vec![(0., [83., 31., 30.])], vt_interp: [8.; 3], cs_offset: vec![(0., [0.; 3])],
            push_out: 0.2, hide_pawn_pct: 0.35, transition_in: tin, transition_out: tout, blend: ViewBlend { function: "VTBlend_EaseInOut".into(), blend_time: bt, blend_exp: exp } }
    }
    fn scope() -> ScopeData {
        ScopeData { timeline_length: 0.3, curve: vec![(0., 0.), (0.15, 0.), (0.2, 1.), (0.3, 1.)], start_delay: 0.2, stop_delay: 0.1, material: String::new(), parent_material: String::new(), reticle_texture: String::new(), scalars: vec![], vectors: vec![],
            reticle_hud_tags: vec!["HUD.Center.Reticle".into()], sound_zoom_in: String::new(), sound_zoom_out: String::new(), vignette_intensity: None, scene_fringe_intensity: None, scope_camera_offset_amount: 30., scope_mesh: String::new() }
    }
    fn profile(scoped: bool) -> AdsProfile {
        AdsProfile { item: "w".into(), actor: "a".into(), base: mode(80., 0.25, 0.25, None, None), targeting: if scoped { mode(15., 0.25, 0.25, Some(0.1), Some(5.)) } else { mode(80., 0.2, 0.2, None, None) },
            use_scope: scoped, scope: scoped.then(scope), spread_downsights: 0., spread_downsights_cooldown: 0., recoil_downsights_multiplier: 0.4, athena_recoil_downsights_multiplier: 0.4, maintain_aim_location: true, aim_from_muzzle_at_close_range: true }
    }
    fn run(m: &mut AdsMachine, p: &AdsProfile, secs: f32, held: bool, toggle: bool) { for _ in 0..(secs * 120.).round() as usize { m.update(1. / 120., AdsInput { held, can_aim: true, toggle }, Some(p)); } }

    #[test] fn hold_aims_while_down_and_the_camera_blends_in_transition_time() {
        let p = profile(false); let mut m = AdsMachine::new();
        run(&mut m, &p, 0.1, true, false); assert!(m.is_targeting()); assert!((m.cam_progress - 0.5).abs() < 0.02);
        run(&mut m, &p, 0.15, true, false); assert_eq!(m.cam_progress, 1.); assert!((m.camera_weight(&p) - 1.).abs() < 1e-6);
        run(&mut m, &p, 0.1, false, false); assert!(!m.is_targeting()); assert!((m.cam_progress - 0.5).abs() < 0.02);
        run(&mut m, &p, 0.15, false, false); assert!(!m.camera_active());
    }
    #[test] fn ease_in_out_matches_the_engine_formula_and_is_symmetric() {
        let b = ViewBlend { function: "VTBlend_EaseInOut".into(), blend_time: None, blend_exp: Some(5.) };
        assert_eq!(b.weight(0.), 0.); assert_eq!(b.weight(1.), 1.); assert!((b.weight(0.5) - 0.5).abs() < 1e-6);
        assert!((b.weight(0.25) - 0.5 * 0.5f32.powf(5.)).abs() < 1e-6); assert!((b.weight(0.75) - (1. - 0.5 * 0.5f32.powf(5.))).abs() < 1e-6);
        let lin = ViewBlend { function: "VTBlend_Linear".into(), blend_time: None, blend_exp: None }; assert_eq!(lin.weight(0.3), 0.3);
    }
    #[test] fn blend_in_uses_the_camera_transition_time_not_the_curve_parameter() {
        let p = profile(true); assert_eq!(p.targeting.blend.in_duration(p.targeting.transition_in), 0.25);
        let q = profile(false); assert_eq!(q.targeting.blend.in_duration(q.targeting.transition_in), 0.2);
    }
    #[test] fn toggle_flips_on_press_edges_only() {
        let p = profile(false); let mut m = AdsMachine::new();
        run(&mut m, &p, 0.05, true, true); assert!(m.is_targeting(), "press 1 on");
        run(&mut m, &p, 0.05, true, true); assert!(m.is_targeting(), "holding the key does not flip");
        run(&mut m, &p, 0.05, false, true); assert!(m.is_targeting(), "release keeps it on");
        run(&mut m, &p, 0.05, true, true); assert!(!m.is_targeting(), "press 2 off");
    }
    #[test] fn scope_timeline_waits_0_2_s_then_plays_0_3_s_and_the_amount_follows_the_curve() {
        let p = profile(true); let mut m = AdsMachine::new();
        run(&mut m, &p, 0.19, true, false); assert_eq!(m.timeline_pos(), 0.); assert_eq!(m.zoom_in_events, 0);
        run(&mut m, &p, 0.02, true, false); assert_eq!(m.zoom_in_events, 1); // after the 0.2 s Delay
        run(&mut m, &p, 0.1, true, false); assert!(m.scope_amount(&p) < 0.01, "curve is still 0 before 0.15 s: {}", m.scope_amount(&p));
        run(&mut m, &p, 0.065, true, false); let a = m.scope_amount(&p); assert!(a > 0.3 && a < 0.7, "{a}");
        assert!(!m.pawn_hidden() && !m.scope_mesh_visible());
        run(&mut m, &p, 0.2, true, false); assert_eq!(m.scope_amount(&p), 1.);
        assert!(m.pawn_hidden() && m.scope_mesh_visible() && m.reticle_hidden(), "Finished (forward, still targeting)");
    }
    #[test] fn releasing_waits_0_1_s_then_restores_and_reverses_the_timeline() {
        let p = profile(true); let mut m = AdsMachine::new();
        run(&mut m, &p, 0.8, true, false);
        run(&mut m, &p, 0.09, false, false); assert!(m.pawn_hidden() && m.scope_mesh_visible(), "still waiting out Delay(0.1)"); assert_eq!(m.zoom_out_events, 0);
        run(&mut m, &p, 0.02, false, false); assert!(!m.pawn_hidden() && !m.reticle_hidden() && m.zoom_out_events == 1);
        assert!(m.scope_mesh_visible(), "the mesh stays until the reverse run finishes");
        run(&mut m, &p, 0.35, false, false); assert!(!m.scope_mesh_visible() && m.scope_amount(&p) == 0. && m.timeline_pos() == 0.);
    }
    #[test] fn a_tap_shorter_than_the_start_delay_never_plays_the_timeline() {
        let p = profile(true); let mut m = AdsMachine::new();
        run(&mut m, &p, 0.15, true, false); run(&mut m, &p, 0.5, false, false);
        assert_eq!(m.zoom_in_events, 0); assert_eq!(m.timeline_pos(), 0.); assert!(!m.scope_mesh_visible());
    }
    #[test] fn unscoped_weapons_never_run_the_scope_sequence() {
        let p = profile(false); let mut m = AdsMachine::new(); run(&mut m, &p, 1., true, false);
        assert!(m.is_targeting() && !m.pawn_hidden() && !m.scope_mesh_visible() && m.scope_amount(&p) == 0. && m.zoom_in_events == 0);
    }
    #[test] fn losing_the_weapon_drops_everything() {
        let p = profile(true); let mut m = AdsMachine::new(); run(&mut m, &p, 0.8, true, false); assert!(m.pawn_hidden());
        m.update(1. / 60., AdsInput { held: true, can_aim: false, toggle: false }, Some(&p));
        assert!(!m.is_targeting() && !m.pawn_hidden() && !m.scope_mesh_visible() && m.timeline_pos() == 0.);
    }
    #[test] fn look_scale_picks_the_scope_or_ads_slider_only_while_targeting() {
        let (s, a) = (profile(true), profile(false)); let mut m = AdsMachine::new();
        assert_eq!(m.look_scale(&s, 0.3, 0.6), 1.);
        run(&mut m, &s, 0.05, true, false); assert_eq!(m.look_scale(&s, 0.3, 0.6), 0.6); assert_eq!(m.look_scale(&a, 0.3, 0.6), 0.3);
    }
}
