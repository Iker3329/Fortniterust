//! ADS HOOK: the simulation side of aim down sights (the state machine itself is `fn_weapons::recovered_ads::AdsMachine`, driven by the front-end).
//!
//! * Camera: the held weapon's hold mode (`CameraBase3PClass`, default = the pickaxe's Melee mode) and ADS mode (`CameraTargeting3PClass`) are blended per frame:
//!   both views are composed from the one smoothed camera state and mixed by the machine's weight (eye and FOV). The smoothed state (pivot lag, shoulder offset,
//!   boom penetration) advances with the ADS mode while it has any weight. How the original camera manager blends two modes is native (see
//!   `recovered_ads::ViewBlend`), so the mix is an approximation of it that is exact at weight 0 and 1.
//! * Movement: while targeting the pawn cannot sprint and its speed stats are limited to `SpeedWalk` (ASSUMED rule, recovered value).
//! * `hide_pawn`: the scope hides the pawn once its timeline finished forward (`SetPawnVisibility(true)` in the Finished event).
use fn_camera::{self as camera, CamState, Mode, View};
use fn_movement::params as mp;
use glam::Vec3;

#[derive(Clone, Copy, Debug)]
pub struct AdsView {
    /// Hold camera / ADS camera of the held weapon (None = the pickaxe's Melee mode / no ADS mode).
    pub base: Option<&'static Mode>, pub target: Option<&'static Mode>,
    /// Blend weight 0..1 (hold -> ADS) and whether the ADS mode is running (targeting, or still blending out).
    pub weight: f32, pub active: bool,
    pub hide_pawn: bool,
    /// Walking-speed cap (cm/s) while targeting.
    pub speed_cap_cm: Option<f32>,
    /// Horizontal FOV of the composed view (degrees); 0 until the first compose.
    pub fov: f32,
}
impl Default for AdsView { fn default() -> Self { AdsView { base: None, target: None, weight: 0., active: false, hide_pawn: false, speed_cap_cm: None, fov: 0. } } }
impl AdsView {
    pub fn base_mode(&self) -> &'static Mode { self.base.unwrap_or(&camera::MELEE) }
    /// The mode the smoothed camera state advances with this tick.
    pub fn tick_mode(&self) -> &'static Mode { if self.active { self.target.unwrap_or_else(|| self.base_mode()) } else { self.base_mode() } }
    /// The view for this frame from the interpolated camera state: hold view mixed with the ADS view by `weight`. Also stores `fov`.
    pub fn compose(&mut self, st: &CamState, feet: Vec3, yaw: f32, pitch: f32, trace: impl Fn(Vec3, Vec3, f32) -> Option<f32>) -> View {
        let base = self.base_mode();
        let mut v = Camera_compose(base, st, feet, yaw, pitch, &trace);
        self.fov = base.fov;
        if let (true, Some(t)) = (self.active, self.target) {
            let w = self.weight.clamp(0., 1.);
            if w > 0. {
                let vt = Camera_compose(t, st, feet, yaw, pitch, &trace);
                v.eye = v.eye.lerp(vt.eye, w); v.safe = v.safe.lerp(vt.safe, w); v.desired = v.desired.lerp(vt.desired, w);
                v.blocked_pct += (vt.blocked_pct - v.blocked_pct) * w;
                if w >= 0.5 { v.hide_pawn = vt.hide_pawn; }
                self.fov = base.fov + (t.fov - base.fov) * w;
            }
        }
        v.hide_pawn |= self.hide_pawn;
        v
    }
    /// Applies the targeting speed limit to the controller's stats (restores the recovered values when not targeting). Returns whether sprinting must be ignored.
    pub fn apply_speed(&self, controller: &mut fn_movement::Controller) -> bool {
        let p = &mut controller.component_mut().p;
        let cap = self.speed_cap_cm.unwrap_or(f32::INFINITY);
        p.speed_run = fn_live::f32("FN_MOVE_SPEED_RUN", mp::SPEED_RUN).min(cap); p.speed_sprint = fn_live::f32("FN_MOVE_SPEED_SPRINT", mp::SPEED_SPRINT).min(cap);
        p.speed_crouched_run = fn_live::f32("FN_MOVE_SPEED_CROUCHED_RUN", mp::SPEED_CROUCHED_RUN).min(cap); p.speed_crouched_sprint = fn_live::f32("FN_MOVE_SPEED_CROUCHED_SPRINT", mp::SPEED_CROUCHED_SPRINT).min(cap);
        p.max_acceleration = fn_live::f32("FN_MOVE_MAX_ACCELERATION", mp::MAX_ACCELERATION); p.ground_friction = fn_live::f32("FN_MOVE_GROUND_FRICTION", mp::GROUND_FRICTION);
        p.braking_deceleration_walking = fn_live::f32("FN_MOVE_BRAKING_DECELERATION_WALKING", mp::BRAKING_DECELERATION_WALKING);
        self.speed_cap_cm.is_some()
    }
}
#[allow(non_snake_case)]
fn Camera_compose(mode: &Mode, st: &CamState, feet: Vec3, yaw: f32, pitch: f32, trace: &impl Fn(Vec3, Vec3, f32) -> Option<f32>) -> View { camera::Camera::compose(mode, st, feet, yaw, pitch, trace) }

#[cfg(test)]
mod tests {
    use super::*;
    fn leak(m: Mode) -> &'static Mode { Box::leak(Box::new(m)) }
    fn st() -> CamState { let mut c = camera::Camera::default(); for _ in 0..240 { c.advance(&camera::MELEE, camera::Input { feet: Vec3::ZERO, crouched: false, yaw_deg: 0., pitch_deg: 0., dt: 1. / 120. }, camera::trace_ground); } c.state() }
    #[test] fn weight_zero_is_the_hold_view_weight_one_is_the_ads_view_and_fov_follows() {
        let mut ads_mode = camera::MELEE; ads_mode.fov = 15.; ads_mode.forward_distance = -90.;
        let t = leak(ads_mode);
        let mut a = AdsView { target: Some(t), active: true, ..Default::default() };
        let s = st();
        let hold = camera::Camera::compose(&camera::MELEE, &s, Vec3::ZERO, 0., 0., camera::trace_ground);
        let ads = camera::Camera::compose(t, &s, Vec3::ZERO, 0., 0., camera::trace_ground);
        a.weight = 0.; let v0 = a.compose(&s, Vec3::ZERO, 0., 0., camera::trace_ground); assert!((v0.eye - hold.eye).length() < 1e-6 && a.fov == 80.);
        a.weight = 1.; let v1 = a.compose(&s, Vec3::ZERO, 0., 0., camera::trace_ground); assert!((v1.eye - ads.eye).length() < 1e-6 && a.fov == 15.);
        assert!(v1.eye.distance(hold.eye) > 1.5, "the ADS boom is 190 cm shorter");
        a.weight = 0.5; let vh = a.compose(&s, Vec3::ZERO, 0., 0., camera::trace_ground); assert!((vh.eye - (hold.eye + ads.eye) * 0.5).length() < 1e-5 && (a.fov - 47.5).abs() < 1e-4);
    }
    #[test] fn inactive_ads_never_reads_the_target_mode() {
        let t = leak(camera::BASE); let mut a = AdsView { target: Some(t), active: false, weight: 1., ..Default::default() };
        assert_eq!(a.tick_mode().name, camera::MELEE.name); let _ = a.compose(&st(), Vec3::ZERO, 0., 0., camera::trace_ground); assert_eq!(a.fov, 80.);
    }
    #[test] fn targeting_limits_every_speed_stat_to_walk_and_releasing_restores_the_recovered_values() {
        let mut c = fn_movement::Controller::default();
        let a = AdsView { speed_cap_cm: Some(mp::SPEED_WALK), ..Default::default() };
        assert!(a.apply_speed(&mut c)); let p = &c.component().p; assert_eq!((p.speed_run, p.speed_sprint, p.speed_crouched_run, p.speed_crouched_sprint), (200., 200., 200., 200.));
        assert!(!AdsView::default().apply_speed(&mut c)); let p = &c.component().p; assert_eq!((p.speed_run, p.speed_sprint, p.speed_crouched_run, p.speed_crouched_sprint), (410., 550., 290., 420.));
    }
}
