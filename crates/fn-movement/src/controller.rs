//! Player controller of the Fortnite 1.8 runtime: a metres / Y-up wrapper around the UE 4.16 `UCharacterMovementComponent` port
//! (`movement_cmc.rs`, UE centimetres / Z-up) plus the facing/turn helpers the camera and animation use.
//!
//! Axis mapping (see movement_cmc.rs): controller `position.x -> X`, `position.y` (= world z - 12) `-> Y`, `height -> Z`
//! (feet above the baseplate); metres = UE cm / 100. `velocity` is the horizontal velocity (m/s), `vertical_speed` is velocity Z (m/s).
//!
//! `step(dt)` advances whatever `dt` it is given; the movement component sub-steps long steps internally (`MaxSimulationTimeStep`
//! 0.05 s, `MaxSimulationIterations` 8). The game calls it from the fixed 1/`SIM_HZ` (30 Hz) tick (`sim_tick.rs`, `App::sim_tick`) and the renderer
//! interpolates between the last two ticks; with `FNRT_SIM_HZ=0` it is called once per rendered frame with the real frame time (the old behaviour,
//! which `jump_feel`, `jump_air_test` and `--movement-test` still exercise directly). `render_position()` is the position of the last step.
#![allow(dead_code)]
use glam::{Vec2, Vec3};
use crate::cmc::{Cmc, Params};
use crate::scene::{Blocker, Scene};

/// Longest frame time simulated in one `step` (a hitch guard, not an engine value; the engine sub-steps anything longer than 0.05 s).
pub const MAX_FRAME_DT: f32 = 0.25;
/// Mouse look: UE's default per-count yaw/pitch scale (0.07 deg) times the assumed default
/// sensitivity multiplier 1.10 (a number, not a version). NOTE: DefaultInput.ini/DefaultGameUserSettings.ini are encrypted in the
/// paks (encrypted=1 in reports/pak-index.json), so these come from the task description, UNVERIFIED.
pub const MOUSE_DEGREES_PER_COUNT: f32 = 0.07;
pub const MOUSE_SENSITIVITY: f32 = 1.10;
/// Effective gravity magnitude in m/s^2 (980 cm/s^2 engine default x GravityScale; `FNRT_GRAVITY_SCALE` is a `cmc::Tuning`, see `Controller::with_tuning`).
pub fn gravity() -> f32 {
    static G: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *G.get_or_init(|| Params::default().gravity_z().abs() * 0.01)
}
/// Time until touchdown on the baseplate for a body at `height` (m) moving at `vertical_speed` (m/s, up positive), 0 when grounded.
/// Used by the animation graph's landing prediction.
pub fn fall_time_left(height: f32, vertical_speed: f32) -> f32 {
    if height <= 0. { return 0.; }
    let g = gravity();
    (vertical_speed + (vertical_speed * vertical_speed + 2. * g * height).sqrt()) / g
}
/// PROVISIONAL character turn rates (deg/s). The original RotationRate is NOT serialized in any 1.8 package
/// (engine default would be 360 deg/s yaw), so these are placeholders, not extracted values.
pub const FACING_MOVING_RATE: f32 = 720.;
pub const TURN_IN_PLACE_RATE: f32 = 360.;
const TURN_IN_PLACE_START: f32 = 90.;
const TURN_IN_PLACE_STOP: f32 = 2.;
/// Signed shortest rotation (degrees, in (-180,180]) taking `from` to `to`.
pub fn shortest_angle(from: f32, to: f32) -> f32 { let d = (to - from).rem_euclid(360.); if d > 180. { d - 360. } else { d } }
/// Rotate `current` toward `target` by at most `rate*dt` degrees along the shortest arc.
pub fn turn_toward(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let d = shortest_angle(current, target); let step = (rate * dt.max(0.)).min(d.abs());
    (current + step * d.signum()).rem_euclid(360.)
}
/// Frame-rate independent exponential smoothing (replaces `(dt*rate).min(1)` lerps).
pub fn smooth_toward(current: f32, target: f32, rate: f32, dt: f32) -> f32 { current + (target - current) * (1. - (-rate * dt.max(0.)).exp()) }

#[derive(Clone, Copy, Default, PartialEq, Debug)]
struct Exported { position: Vec2, velocity: Vec2, height: f32, vertical_speed: f32, crouched: bool }

#[derive(Debug)]
pub struct Controller {
    /// Feet position on the baseplate plane, metres (x, world z - 12).
    pub position: Vec2,
    /// Horizontal velocity, m/s.
    pub velocity: Vec2,
    /// Feet height above the baseplate, metres. While walking it is the height of the floor surface under the feet (0 on the baseplate,
    /// the prop top when standing on one): use `airborne()` / `grounded()`, not `height > 0`, to decide whether the pawn is in the air.
    pub height: f32,
    /// Vertical velocity, m/s (up positive).
    pub vertical_speed: f32,
    /// Actual crouch state (`bIsCrouched`). Writing it from outside is treated as a crouch REQUEST (legacy callers); use `wants_crouch`.
    pub crouched: bool,
    /// Crouch input (`bWantsToCrouch`): the component crouches when it can and un-crouches when the key is released AND the standing capsule fits.
    pub wants_crouch: bool,
    pub running: bool,
    /// Character heading in the same degree convention as the camera yaw.
    pub facing: f32,
    turning: bool,
    /// Trunk/rock blockers in controller space (propcol.rs), refreshed by the caller each frame.
    pub blockers: Vec<Blocker>,
    /// Convex hulls of the real props near the pawn, in controller space (metres: x, world z - 12, up), refreshed by the caller each tick like `blockers`.
    pub hulls: Vec<fn_hull::Hull>,
    cmc: Cmc,
    exported: Exported,
    jump_key_prev: bool,
}
impl Default for Controller {
    fn default() -> Self {
        Controller { position: Vec2::ZERO, velocity: Vec2::ZERO, height: 0., vertical_speed: 0., crouched: false, wants_crouch: false, running: false, facing: 0., turning: false, blockers: vec![], hulls: vec![],
            cmc: Cmc::new(Params::default(), Scene::default(), Vec3::ZERO), exported: Exported::default(), jump_key_prev: false }
    }
}
impl Controller {
    /// A controller whose movement component uses `Params::default().with_tuning(t)` (`FNRT_GRAVITY_SCALE` / `FNRT_JUMPZ` experiments).
    pub fn with_tuning(t: &crate::cmc::Tuning) -> Self { let mut c = Controller::default(); c.cmc.p = Params::default().with_tuning(t); c }
    /// `fall_time_left` for THIS controller's gravity (same bits as the free function unless a gravity tuning is active).
    pub fn fall_time_left(&self) -> f32 {
        let (height, vertical_speed) = (self.air_height(), self.vertical_speed);
        if height <= 0. { return 0.; }
        let g = self.cmc.p.gravity_z().abs() * 0.01;
        (vertical_speed + (vertical_speed * vertical_speed + 2. * g * height).sqrt()) / g
    }
    /// The pawn is off the ground (`MOVE_Falling`).
    pub fn airborne(&self) -> bool { self.cmc.is_falling() }
    pub fn grounded(&self) -> bool { !self.cmc.is_falling() }
    /// Height above the ground while airborne (never 0 in the air), 0 while grounded - also on a prop top. Feed this, not `height`, to code that
    /// treats `> 0` as "airborne" (animation graph, combat upper-body rule, jump stage).
    pub fn air_height(&self) -> f32 { if self.airborne() { self.height.max(1e-4) } else { 0. } }
    /// `jump` (the key state of THIS frame) is a fresh press: true on the rising edge, before `step` consumes it.
    pub fn jump_edge(&self, jump: bool) -> bool { jump && !self.jump_key_prev }
    /// Read access to the movement component (tests, `--movement-test` logging).
    pub fn component(&self) -> &Cmc { &self.cmc }
    pub fn component_mut(&mut self) -> &mut Cmc { &mut self.cmc }
    /// Number of jumps the component has started (`DoJump` succeeded) since creation.
    pub fn jumps_started(&self) -> u32 { self.cmc.jumped }
    pub fn landings(&self) -> u32 { self.cmc.landed }
    /// Current standing/crouched capsule half height (cm).
    pub fn capsule_half_height_cm(&self) -> f32 { self.cmc.half_height }

    /// Advance one frame: `input` is the world-space movement direction (|input| <= 1, x and controller-y axes), `jump` the jump KEY STATE
    /// (a rising edge is `ACharacter::Jump`, a falling edge `StopJumping`), `run` the sprint request, `dt` the frame time.
    pub fn step(&mut self, input: Vec2, jump: bool, run: bool, dt: f32) {
        let dt = dt.clamp(0., MAX_FRAME_DT);
        self.import_external_changes();
        self.cmc.scene.solids.clear();
        self.cmc.scene.solids.extend(Scene::from_blockers(&self.blockers).solids);
        self.cmc.scene.hulls = if self.hulls.is_empty() { vec![] } else { Scene::hulls_to_cm(&self.hulls) };
        let input = input.clamp_length_max(1.);
        self.cmc.wants_sprint = run;
        self.cmc.wants_crouch = self.wants_crouch;
        let f = self.facing.to_radians(); self.cmc.facing = Vec3::new(f.sin(), -f.cos(), 0.);
        // ASSUMED input layer (not engine code): a jump press whose crouch REQUEST is already off (key released / toggle cancelled on this very press) stands the capsule up
        // first. UE4.16 `CanJumpInternal` refuses while `bIsCrouched` (Character.cpp:265) and `CheckJumpInput` runs before `PerformMovement` un-crouches, so without this the press would be lost.
        if jump && !self.jump_key_prev { if self.cmc.is_crouched && !self.cmc.wants_crouch { self.cmc.un_crouch(); } self.cmc.jump(); }
        if !jump && self.jump_key_prev { self.cmc.stop_jumping(); }
        self.jump_key_prev = jump;
        self.cmc.tick(input, dt);
        self.running = run && !self.cmc.is_crouched && input.length_squared() > 0.;
        self.export();
    }
    /// Position to render. The simulation already runs at the frame rate, so this is the simulated position.
    pub fn render_position(&self) -> Vec2 { self.position }
    pub fn render_height(&self) -> f32 { self.height }
    fn export(&mut self) {
        let c = &self.cmc;
        self.position = Vec2::new(c.loc.x, c.loc.y) * 0.01;
        self.velocity = Vec2::new(c.velocity.x, c.velocity.y) * 0.01;
        let h = c.feet_height() * 0.01; self.height = if h.abs() < 1e-5 { 0. } else { h };
        self.vertical_speed = c.velocity.z * 0.01;
        self.crouched = c.is_crouched;
        self.exported = Exported { position: self.position, velocity: self.velocity, height: self.height, vertical_speed: self.vertical_speed, crouched: self.crouched };
    }
    /// Callers may still assign `position`, `velocity`, `height`, `vertical_speed` or `crouched` directly (spawn, tests, clamp_to_box):
    /// whatever differs from what the last step exported is imported into the component as a teleport / request.
    fn import_external_changes(&mut self) {
        let s = self.exported;
        if self.position != s.position { self.cmc.loc.x = self.position.x * 100.; self.cmc.loc.y = self.position.y * 100.; let hh = self.cmc.half_height; self.cmc.teleport_to_feet(Vec3::new(self.cmc.loc.x, self.cmc.loc.y, self.cmc.loc.z - hh)); }
        if self.height != s.height {
            let hh = self.cmc.half_height; let feet = Vec3::new(self.cmc.loc.x, self.cmc.loc.y, self.height * 100.);
            self.cmc.loc = feet + Vec3::new(0., 0., hh);
            if self.height > 0. { self.cmc.start_falling_externally(); } else { self.cmc.teleport_to_feet(feet); }
        }
        if self.velocity != s.velocity { self.cmc.velocity.x = self.velocity.x * 100.; self.cmc.velocity.y = self.velocity.y * 100.; }
        if self.vertical_speed != s.vertical_speed { self.cmc.velocity.z = self.vertical_speed * 100.; }
        if self.crouched != s.crouched { self.wants_crouch = self.crouched; }
    }
    /// Keep the simulation inside the world box (so the controller cannot overshoot an
    /// invisible clamp and feel sticky); cancels velocity into the wall.
    pub fn clamp_to_box(&mut self, min: Vec2, max: Vec2) {
        self.position = self.position.clamp(min, max);
        if self.position.x <= min.x && self.velocity.x < 0. || self.position.x >= max.x && self.velocity.x > 0. { self.velocity.x = 0.; }
        if self.position.y <= min.y && self.velocity.y < 0. || self.position.y >= max.y && self.velocity.y > 0. { self.velocity.y = 0.; }
    }
    /// Update character heading from the camera yaw (variable frame dt, rate-based so frame-rate
    /// independent). Moving: track camera quickly (strafe-style, as the animation set assumes).
    /// Idle: hold heading until the camera is >90 deg away, then turn in place until aligned.
    pub fn update_facing(&mut self, camera_yaw: f32, dt: f32) {
        let delta = shortest_angle(self.facing, camera_yaw).abs();
        let moving = self.velocity.length_squared() > 0.01;
        if moving || self.airborne() { self.turning = false; self.facing = turn_toward(self.facing, camera_yaw, FACING_MOVING_RATE, dt); return; }
        if delta > TURN_IN_PLACE_START { self.turning = true; }
        if self.turning { self.facing = turn_toward(self.facing, camera_yaw, TURN_IN_PLACE_RATE, dt); if shortest_angle(self.facing, camera_yaw).abs() <= TURN_IN_PLACE_STOP { self.turning = false; } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn shortest_angle_wraps() { assert_eq!(shortest_angle(350., 10.), 20.); assert_eq!(shortest_angle(10., 350.), -20.); assert_eq!(shortest_angle(0., 180.), 180.); assert!(shortest_angle(90., 90.).abs() < 1e-6); assert!((shortest_angle(0., 540.) - 180.).abs() < 1e-4); }
    #[test] fn turn_toward_takes_short_arc_and_never_overshoots() { assert!((turn_toward(359., 1., 100., 0.01) - 0.).abs() < 1e-3 || (turn_toward(359., 1., 100., 0.01) - 360.).abs() < 1e-3); assert_eq!(turn_toward(10., 12., 720., 1.), 12.); let mut a = 10.; for _ in 0..100 { a = turn_toward(a, 350., 360., 1. / 60.); } assert!((a - 350.).abs() < 1e-3); }
    #[test] fn idle_character_holds_heading_then_turns_in_place() { let mut c = Controller::default(); c.update_facing(60., 1. / 60.); assert_eq!(c.facing, 0.); c.update_facing(120., 1. / 60.); assert!(c.facing > 0. && c.facing <= 6.0001); for _ in 0..120 { c.update_facing(120., 1. / 60.); } assert!((c.facing - 120.).abs() < 1e-3); }
    #[test] fn moving_character_tracks_camera_across_wrap() { let mut c = Controller { facing: 350., velocity: Vec2::X, ..Default::default() }; c.update_facing(10., 1. / 60.); assert!(c.facing > 350. || c.facing < 10.); let mut seen_mid = false; for _ in 0..30 { c.update_facing(10., 1. / 60.); if c.facing > 20. && c.facing < 340. { seen_mid = true; } } assert!(!seen_mid, "must never swing the long way round"); assert!((shortest_angle(c.facing, 10.)).abs() < 1e-3); }
    #[test] fn facing_rate_is_frame_rate_independent() { let mut r = vec![]; for rate in [30., 60., 240.] { let mut c = Controller { velocity: Vec2::X, ..Default::default() }; for _ in 0..(rate * 0.1) as u32 { c.update_facing(179., 1. / rate); } r.push(c.facing); } assert!((r[0] - r[2]).abs() < 1e-3 && (r[1] - r[2]).abs() < 1e-3); }
    #[test] fn smoothing_is_frame_rate_independent() { let mut r = vec![]; for rate in [40., 60., 240.] { let mut v = 0.; for _ in 0..(rate * 0.25) as u32 { v = smooth_toward(v, 1., 12., 1. / rate); } r.push(v); } assert!((r[0] - r[2]).abs() < 1e-4 && (r[1] - r[2]).abs() < 1e-4); }
    #[test] fn mouse_scale_is_original_default() { assert!((MOUSE_DEGREES_PER_COUNT * MOUSE_SENSITIVITY - 0.077).abs() < 1e-6); }
    #[test] fn clamp_to_box_stops_sticky_overshoot() {
        let mut c = Controller::default(); c.velocity = Vec2::new(5., 0.);
        for _ in 0..600 { c.step(Vec2::X, false, false, 1. / 60.); c.clamp_to_box(Vec2::splat(-10.), Vec2::splat(10.)); }
        assert_eq!(c.position.x, 10.); assert_eq!(c.velocity.x, 0.);
        c.step(-Vec2::X, false, false, 1. / 60.); assert!(c.position.x < 10., "must leave the wall immediately");
    }
    #[test] fn fall_time_left_matches_the_ballistic_formula() {
        let g = gravity();
        assert!((fall_time_left(0., 0.)).abs() < 1e-6); assert!((fall_time_left(1e-4, 6.) - 2. * 6. / g).abs() < 1e-3);
        let apex = 6. * 6. / (2. * g); assert!((fall_time_left(apex, 0.) - (2. * apex / g).sqrt()).abs() < 1e-3);
    }
}

