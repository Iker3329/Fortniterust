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
//! RECOVERED FROM THE EXE (static decompile of `FortniteClient-Win64-Shipping.exe`, image base 0x140000000)
//! * Camera mode blend: `AFortPlayerCameraBase::UpdateCamera` (vfunc 215, RVA 0x140502ff0) keeps a blend stack; weights come from `FUN_1404e6f00` (see
//!   [`ViewBlend::weight`]); the seconds of a new entry from `UFortCameraMode::vfunc_72` (RVA 0x1404efc30, see [`AdsMachine::camera_duration`]); native defaults from
//!   `UFortCameraMode::UFortCameraMode` (RVA 0x1404d4370). See [`AdsMachine::step_camera`].
//! * Movement while targeting: `AFortPlayerPawn::SetTargeting` (vfunc 384, RVA 0x1407ea830) stores `bIsTargeting` (+0x104b) and calls `FUN_14080d0a0`, which sets the
//!   movement style to 1 (Walking) while targeting (3 when an ability-system tag is present), so the speed is the Walking stat ([`AdsMachine::limits_movement`]).
//! * Look sensitivity: `UFortPlayerInputAthena::vfunc_71` (RVA 0x1404392c0), see [`AdsMachine::look_scale`].
//!
//! NOT RECOVERED (never in a pak, config-driven or not read): the scope post-process material that paints the scope (DXBC in the package), the config values
//! `bEnableFOVScaling` / `FOVScale` (DefaultInput, encrypted), and the pawn-side predicate of `scoped` (see [`AdsMachine::look_scale`]).

/// One `InterpOffsetData` sample: pitch angle, view offset (cm, X forward / Y right / Z up).
pub type OffsetSample = (f32, [f32; 3]);

/// `FViewTargetTransitionParams` as serialized on a camera mode (`BlendFunction` / `BlendTime` / `BlendExp`; fields at their archetype value are not
/// serialized, so `None` = inherited from the parent class, resolved along the chain by the loader and left `None` only when nothing sets it).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewBlend { pub function: String, pub blend_time: Option<f32>, pub blend_exp: Option<f32> }
impl ViewBlend {
    /// `UFortCameraMode::UFortCameraMode` (RVA 0x1404d4370) writes `BlendExp = 2.0` (`+0x40 = 0x40000000`) and, after first setting `BlendFunction = 1`, ends with
    /// `BlendFunction = 0` (`VTBlend_Linear`): the native defaults when no class in the chain serializes the field.
    pub const ENGINE_DEFAULT_EXP: f32 = 2.;
    pub const DEFAULT_FUNCTION: &'static str = "VTBlend_Linear";
    /// The blend-weight function the camera stack applies to an entry's `t` (`FUN_1404e6f00`, RVA 0x1404e6f00; switch on `BlendFunction` at params+4, exponent at +8,
    /// `BlendTime` at +0 is NOT read): 0 Linear `t`; 1 Cubic `3t^2 - 2t^3`; 2 EaseIn `t^exp`; 3 EaseOut `t^(1/exp)` (exponent inverted only while `|exp| > 1e-8`);
    /// 4 EaseInOut `0.5 (2t)^exp` below 0.5, else `1 - 0.5 (2 (1 - t))^exp`; any other value 1.
    pub fn weight(&self, alpha: f32) -> f32 {
        let a = alpha.clamp(0., 1.);
        let e = self.blend_exp.unwrap_or(Self::ENGINE_DEFAULT_EXP);
        match self.function.as_str() {
            "VTBlend_Linear" => a,
            "VTBlend_Cubic" => a * a * 3. - (a * a * a + a * a * a),
            "VTBlend_EaseIn" => a.powf(e),
            "VTBlend_EaseOut" => a.powf(if e.abs() > 1.0e-8 { 1. / e } else { 1. }),
            "VTBlend_EaseInOut" => if a < 0.5 { 0.5 * (2. * a).powf(e) } else { 1. - 0.5 * (2. * (1. - a)).powf(e) },
            _ => 1.,
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
    /// What the native `SFortReticle` paints for this weapon (see [`ReticleData`]).
    pub reticle: ReticleData,
}

/// `ReticleCornerAngles` default of `UFortWeaponItemDefinition` (native ctor RVA 0x9164b0 appends 0, 90, 180, 270 at 0x91662a..0x9166c8). A WID that does not serialize the
/// property (the assault rifle) therefore has four corner images.
pub const DEFAULT_CORNER_ANGLES: [f32; 4] = [0., 90., 180., 270.];

/// The reticle properties `SFortReticle::Construct` (RVA 0x1430227a0) reads from the weapon: every texture is `WID.<X>` if that asset pointer is set, else the weapon
/// actor Blueprint's own property (`GetReticleImage` RVA 0x929d00, `GetReticleCenterImage` 0x929900: WID asset ptr at +0x570 / +0x5f0, fallback actor +0x3f0 / +0x418).
/// Texture names are file names (`<package stem>.png`) as the UI asset source resolves them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReticleData {
    /// `ReticleImage`: the brush of every corner image (`SImage` array at `SFortReticle+0x510`).
    pub image: Option<String>,
    /// `ReticleCenterImage`: the single centre image (`SFortReticle+0x520`).
    pub center_image: Option<String>,
    /// `HitNotifyImage` / `HitNotifyReticleImage`.
    pub hit_image: Option<String>,
    /// `MuzzleBlockedImage` (WID +0x5c0, fallback actor `MuzzleBlockedReticleImage` +0x420).
    pub muzzle_blocked_image: Option<String>,
    /// `WID.HitNotifyDuration` (+0x61c; the native ctor default is 1.0 s).
    pub hit_duration: f32,
    /// `WID.ReticleCornerAngles` (degrees, clockwise from straight up); one corner image per entry, image `i` is also rotated by `i * 360 / n` degrees.
    pub corner_angles: Vec<f32>,
    /// `WID.bReticleCornerOutsideSpreadRadius`: the corner image is pushed out by half its own size on top of the spread radius.
    pub corner_outside_spread_radius: bool,
}
impl AdsProfile {
    pub fn fov_base(&self) -> f32 { self.base.fov }
    pub fn fov_targeting(&self) -> f32 { self.targeting.fov }
}

/// The mouse look factor of the exe's `FOV * FOVScale` term (`UFortPlayerInputAthena::vfunc_71`, RVA 0x1404392c0, `GetFOVAngle()` vtable +0x650 of the camera manager),
/// relative to the hold camera's FOV: the unscaled base look rate of the runtime is the one at the hold FOV, and `FOVScale` itself is an encrypted config value, so only
/// the ratio is applied (1 whenever the FOV equals the hold FOV; the scope's 15 deg gives 15 / 80).
pub fn fov_look_ratio(fov: f32, hold_fov: f32) -> f32 { if fov > 0. && hold_fov > 0. { fov / hold_fov } else { 1. } }

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

/// One entry of the camera blend stack of `AFortPlayerCameraBase::UpdateCamera` (vfunc 215, RVA 0x140502ff0; entry layout read from its decompile and from the debug
/// printout of vfunc 187 `[Weight] [Time] [Rate]`): +0x00 mode instance, +0x10 `t` (0..1 progress), +0x14 `rate` (1 / transition seconds), +0x18 weight.
/// `target` = the entry is the weapon's targeting mode (otherwise its hold mode).
#[derive(Clone, Copy, Debug, PartialEq)]
struct CamEntry { target: bool, t: f32, rate: f32 }
/// `SMALL_NUMBER` (exe constant 0x322bcc77 at RVA 0x3ff9008): the stack's "is gone" threshold.
const CAM_EPS: f32 = 1.0e-8;

/// Per-frame ADS state of the local pawn. All timing comes from the profile.
#[derive(Clone, Debug, Default)]
pub struct AdsMachine {
    held_prev: bool,
    /// `bIsTargeting` (the weapon's targeting request after hold / toggle).
    pub targeting: bool,
    /// `AFortPlayerCameraBase`'s camera blend stack (+0x19c8, 0x20-byte entries), reduced to the two modes of one weapon; entry 0 is the top. See [`CamEntry`].
    cam: Vec<CamEntry>,
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
    /// Camera weight of the ADS mode (0 = hold camera, 1 = targeting camera) for `p`: every stack entry weighs `its mode's BlendFunction(t)` (`FUN_1404e6f00`, RVA 0x1404e6f00,
    /// from `TransitionParams` at mode+0x38) and the weights are normalized by their sum (vfunc 215 lines 380-383: `(1 / sum) * weight`), this being the target entries' share.
    pub fn camera_weight(&self, p: &AdsProfile) -> f32 {
        if self.cam.is_empty() { return 0.; }
        let (mut sum, mut target) = (0f32, 0f32);
        for e in &self.cam { let w = if e.target { p.targeting.blend.weight(e.t) } else { p.base.blend.weight(e.t) }; sum += w; if e.target { target += w; } }
        if sum > 0. { target / sum } else { 0. }
    }
    /// True while the ADS camera has any weight (the camera must keep running the targeting mode).
    pub fn camera_active(&self) -> bool { self.targeting || self.cam.iter().any(|e| e.target && e.t > 0.) }
    /// `UFortCameraMode::vfunc_72` (RVA 0x1404efc30), the seconds a NEW stack entry blends in: when the previous top mode is a subclass of the class the pawn now asks for
    /// (`IsA(old, pawnModeClass)`, here `old.chain` contains the new mode's class) it is the OLD mode's `TransitionOutTime` (+0x34), else the new mode's `TransitionTime` (+0x30).
    /// (The pawn's class query at `[controller + 0x6d0] -> vfunc +0x660` is read as "the camera class the weapon wants now": INFERENCE.)
    pub fn camera_duration(p: &AdsProfile, entering_target: bool) -> f32 {
        let (new, old) = if entering_target { (&p.targeting, &p.base) } else { (&p.base, &p.targeting) };
        match new.chain.first() { Some(c) if old.chain.iter().any(|o| o == c) => old.transition_out, _ => new.transition_in }
    }
    /// One tick of the blend stack (vfunc 215 lines 216-383). The hold entry exists from the equip with `t = 1` and the rate it blended in with (`1 / TransitionTime` of the
    /// hold mode, `vfunc_72(hold, pickaxe mode)`); asking for the other mode either moves its still-present entry to the top (keeping its stored rate) or pushes a new one
    /// (`t = 0`, `rate = 1 / camera_duration`); then the top's `t` rises by `dt * rate` while every other entry's falls by the same amount and is dropped at `t <= SMALL_NUMBER`.
    fn step_camera(&mut self, dt: f32, want_target: bool, p: &AdsProfile) {
        if self.cam.is_empty() { let d = p.base.transition_in; self.cam.push(CamEntry { target: false, t: 1., rate: if d > 0. { 1. / d } else { 0. } }); }
        if self.cam[0].target != want_target {
            if let Some(i) = self.cam.iter().position(|e| e.target == want_target) {
                let e = self.cam.remove(i); self.cam.insert(0, e);
                if self.cam[0].rate <= CAM_EPS { self.cam[0].t = 1.; self.cam.truncate(1); }
            } else {
                let dur = Self::camera_duration(p, want_target);
                if dur <= 0. { self.cam.clear(); self.cam.push(CamEntry { target: want_target, t: 1., rate: 0. }); } else { self.cam.insert(0, CamEntry { target: want_target, t: 0., rate: 1. / dur }); }
            }
        }
        let d = dt * self.cam[0].rate;
        self.cam[0].t = (self.cam[0].t + d).clamp(0., 1.);
        for e in self.cam.iter_mut().skip(1) { e.t = (e.t - d).clamp(0., 1.); }
        let mut i = 1; while i < self.cam.len() { if self.cam[i].t <= CAM_EPS { self.cam.remove(i); } else { i += 1; } }
    }
    /// `DownSightPostProcessAmount` (the blend weight of the scope post-process).
    pub fn scope_amount(&self, p: &AdsProfile) -> f32 { p.scope.as_ref().map_or(0., |s| s.amount(self.pos)) }
    pub fn scope_mesh_visible(&self) -> bool { self.scope_mesh }
    pub fn pawn_hidden(&self) -> bool { self.pawn_hidden }
    pub fn reticle_hidden(&self) -> bool { self.reticle_hidden }
    pub fn timeline_pos(&self) -> f32 { self.pos }

    /// `UFortPlayerInputAthena::vfunc_71` (RVA 0x1404392c0, the per-axis value modifier): for the mouse axes the value is multiplied by `TargetingMultiplier` (+0x4f4) while the
    /// pawn `bIsTargeting` and by `ScopedMultiplier` (+0x4f8) while it is scoped (`FUN_14047ecf0` = targeting AND the held ranged weapon has flag bit 3 `[+0xabc]`, the
    /// scope-targeting weapon; the extra owner predicate at `FUN_14092e7e0` is not decoded, so `scoped` = targeting with a `bUseScopeTargeting` weapon: INFERENCE), and
    /// afterwards by `FOV * FOVScale` (see [`fov_look_ratio`]). The sliders are the Settings multipliers `ads_mult` / `scope_mult`.
    pub fn look_scale(&self, p: &AdsProfile, ads_mult: f32, scope_mult: f32) -> f32 {
        if !self.targeting { 1. } else if p.use_scope { scope_mult } else { ads_mult }
    }
    /// While targeting the pawn walks: `SetTargeting` -> movement style Walking (RVA 0x14080d0a0), so every speed is limited to the `SpeedWalk` stat and it cannot sprint.
    pub fn limits_movement(&self) -> bool { self.targeting }

    /// Advances by `dt`. `p` is the held weapon's profile (None = nothing aimable: any state is dropped).
    pub fn update(&mut self, dt: f32, inp: AdsInput, p: Option<&AdsProfile>) {
        let Some(p) = p.filter(|_| inp.can_aim) else {
            if self.targeting || self.camera_active() || self.dir != Dir::Idle || self.scope_mesh || self.pawn_hidden { self.targeting = false; self.held_prev = inp.held; self.cut(dt, p); } else { self.held_prev = inp.held; }
            return;
        };
        let edge = inp.held && !self.held_prev; self.held_prev = inp.held;
        let want = if inp.toggle { if edge { !self.targeting } else { self.targeting } } else { inp.held };
        let changed = want != self.targeting;
        self.targeting = want;
        // ---- camera blend: the exe's blend stack (see `step_camera`)
        self.step_camera(dt, self.targeting, p);
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
        match p { Some(p) => self.step_camera(dt, false, p), None => self.cam.clear() }
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
            use_scope: scoped, scope: scoped.then(scope), spread_downsights: 0., spread_downsights_cooldown: 0., recoil_downsights_multiplier: 0.4, athena_recoil_downsights_multiplier: 0.4, maintain_aim_location: true, aim_from_muzzle_at_close_range: true, reticle: ReticleData::default() }
    }
    fn run(m: &mut AdsMachine, p: &AdsProfile, secs: f32, held: bool, toggle: bool) { for _ in 0..(secs * 120.).round() as usize { m.update(1. / 120., AdsInput { held, can_aim: true, toggle }, Some(p)); } }

    /// A profile whose hold camera (Ranged: 0.45 s) and targeting camera (0.2 s) are the AR's, as read from the 1.8 paks.
    fn ar() -> AdsProfile { let mut p = profile(false); p.base.transition_in = 0.45; p.base.transition_out = 0.45; p }
    #[test] fn hold_aims_while_down_and_the_camera_blends_in_transition_time() {
        let p = ar(); let mut m = AdsMachine::new();
        run(&mut m, &p, 0.1, true, false); assert!(m.is_targeting()); assert!((m.camera_weight(&p) - 0.5).abs() < 0.02, "EaseInOut(0.5) = 0.5 at half the 0.2 s TransitionTime: {}", m.camera_weight(&p));
        run(&mut m, &p, 0.15, true, false); assert!((m.camera_weight(&p) - 1.).abs() < 1e-6);
        assert_eq!(m.cam.len(), 1, "the hold entry decayed to t = 0 and left the stack");
        // release: the hold mode is pushed again and blends in over ITS TransitionTime (0.45 s), not the 0.2 s of the targeting mode
        run(&mut m, &p, 0.1, false, false); assert!(!m.is_targeting()); assert!(m.camera_weight(&p) > 0.7, "slow release: {}", m.camera_weight(&p));
        run(&mut m, &p, 0.125, false, false); assert!((m.camera_weight(&p) - 0.5).abs() < 0.03, "half way after 0.225 s: {}", m.camera_weight(&p));
        run(&mut m, &p, 0.25, false, false); assert!(!m.camera_active() && m.camera_weight(&p) == 0.);
    }
    #[test] fn a_scope_camera_that_derives_from_the_hold_camera_releases_with_its_own_out_time() {
        let mut p = profile(true); p.base.chain = vec!["Sniper".into()]; p.targeting.chain = vec!["TargetingScope".into(), "Sniper".into()];
        p.base.transition_in = 0.45; p.targeting.transition_out = 0.25;
        assert_eq!(AdsMachine::camera_duration(&p, false), 0.25, "IsA(old, new): the old mode's TransitionOutTime (vfunc_72)");
        assert_eq!(AdsMachine::camera_duration(&p, true), 0.25, "the targeting mode's own TransitionTime");
        assert_eq!(AdsMachine::camera_duration(&ar(), false), 0.45); assert_eq!(AdsMachine::camera_duration(&ar(), true), 0.2);
    }
    #[test] fn releasing_mid_blend_keeps_the_hold_entry_and_its_stored_rate() {
        let p = ar(); let mut m = AdsMachine::new();
        run(&mut m, &p, 0.1, true, false); let w = m.camera_weight(&p); assert!((w - 0.5).abs() < 0.02);
        run(&mut m, &p, 0.1, false, false); // hold moves back to the top at t = 0.5 and rises at 1/0.45: after 0.1 s t = 0.72, the targeting entry fell to 0.28
        assert!(m.camera_weight(&p) < 0.3, "{}", m.camera_weight(&p)); assert!(m.camera_weight(&p) > 0.1);
    }
    #[test] fn view_blend_functions_follow_the_exe_switch() {
        let b = |f: &str, e: Option<f32>| ViewBlend { function: f.into(), blend_time: Some(9.), blend_exp: e };
        assert_eq!(b("VTBlend_Linear", None).weight(0.3), 0.3);
        assert!((b("VTBlend_Cubic", None).weight(0.25) - (3. * 0.0625 - 2. * 0.015625)).abs() < 1e-6);
        assert!((b("VTBlend_EaseIn", Some(3.)).weight(0.5) - 0.125).abs() < 1e-6);
        assert!((b("VTBlend_EaseOut", Some(2.)).weight(0.25) - 0.5).abs() < 1e-6, "t^(1/exp)");
        assert!((b("VTBlend_EaseOut", Some(0.)).weight(0.25) - 0.25).abs() < 1e-6, "|exp| <= 1e-8 keeps exponent 1");
        assert!((b("VTBlend_EaseInOut", None).weight(0.25) - 0.125).abs() < 1e-6, "default BlendExp 2");
        assert_eq!(ViewBlend::DEFAULT_FUNCTION, "VTBlend_Linear");
    }
    #[test] fn the_fov_look_ratio_is_one_at_the_hold_fov_and_scales_the_scope() { assert_eq!(fov_look_ratio(80., 80.), 1.); assert!((fov_look_ratio(15., 80.) - 0.1875).abs() < 1e-6); assert_eq!(fov_look_ratio(0., 80.), 1.); }
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
