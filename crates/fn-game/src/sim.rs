//! `GameSim`: the renderer-free, OS-free game loop of the old `App::update` / `App::update_camera` / `App::step_anim` (main.rs), driven by
//! a per-frame [`FrameInput`] snapshot. The front-end (fn-app) fills `FrameInput` from keyboard/mouse/gamepad through the user's bindings
//! and `fn_settings::ModeLatch`es; the headless harness (fn-sim) fills it from scripts. Everything after the input snapshot is exactly the
//! old order of operations, so the same input stream gives the same numbers.
//!
//! Coordinates (old main.rs): the pawn eye is `(x, y, z)` in world metres with `y = EYE + feet height`, `z = 12 + controller.y`; the
//! controller works in "controller space" (position.y = world z - 12).
use fn_animgraph::{AnimState, Montage, Node, PawnSample};
use fn_camera::{self as camera, Camera, View};
use fn_combat::{Notify, Swing, World};
use fn_movement::{cmc::Tuning, smooth_toward, Controller};
use fn_settings::ModeLatch;
use crate::sim_tick::Snapshot;
use glam::{Vec2, Vec3};

/// Half size (m) of the square baseplate.
pub const HALF: f32 = 256.0;
/// Eye height above the feet (m).
pub const EYE: f32 = 1.75;
/// World z of the controller origin (controller y = world z - 12).
pub const Z_OFFSET: f32 = 12.;
/// (x, eye y, z, yaw, pitch) of the spawn.
pub fn spawn() -> (f32, f32, f32, f32, f32) { (0.0, EYE, 12.0, 0.0, -12.0) }

/// The coarse jump animation stage the old HUD/locomotion code used: 1 rising, 2 apex, 3 falling, 4 landing one-shot (0.8333 s), 0 none.
pub fn animation_stage(height: f32, vertical_speed: f32, previous: u8, elapsed: f32) -> u8 {
    if height > 0. { if vertical_speed > 1. { 1 } else if vertical_speed < -1. { 3 } else { 2 } }
    else if (1..=3).contains(&previous) || (previous == 4 && elapsed < 0.833333) { 4 } else { 0 }
}
/// World-space step for held keys: `forward`/`side` in -1..1, `yaw` in degrees; |result| = `speed * dt` for any diagonal.
pub fn horizontal_step(forward: f32, side: f32, yaw: f32, speed: f32, dt: f32) -> (f32, f32) {
    let length = (forward * forward + side * side).sqrt().max(1.0);
    let angle = yaw.to_radians();
    ((forward * angle.sin() + side * angle.cos()) / length * speed * dt,
     (-forward * angle.cos() + side * angle.sin()) / length * speed * dt)
}
/// Locomotion blend between two adjacent of the eight source directions: (from, to, fraction).
pub fn direction_blend(forward: f32, side: f32) -> (&'static str, &'static str, f32) {
    // Source sample angles in reports/blendspace-samples.json, including +/-180 wrap.
    let sector = (side.atan2(forward) / std::f32::consts::FRAC_PI_4).rem_euclid(8.);
    let lo = sector.floor() as usize; let names = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    (names[lo % 8], names[(lo + 1) % 8], sector.fract())
}
/// Nearest of the eight direction names (diagnostics).
pub fn direction_name(forward: f32, side: f32) -> &'static str { let sector = (side.atan2(forward) / std::f32::consts::FRAC_PI_4).round() as i32; ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][sector.rem_euclid(8) as usize] }

/// One frame of player intent, after key bindings / gamepad merging (the old `App::update` locals).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameInput {
    /// Movement axes, -1..1 (W/S, D/A; keyboard + gamepad already merged and clamped).
    pub forward: f32, pub side: f32,
    /// Raw key states (`bindings.down(A_JUMP/A_CROUCH/A_SPRINT/A_FIRE)` | gamepad). Crouch/sprint go through the hold/toggle latches.
    pub jump: bool, pub crouch_key: bool, pub sprint_key: bool, pub attack: bool,
    /// Scripted additions OR'ed after the latches (the old `ps.jump` / `ps.crouch` / `ps.run`); real input leaves them false.
    pub script_jump: bool, pub force_crouch: bool, pub force_run: bool,
    /// Degrees added to yaw / pitch this frame (arrow keys, right stick rate * dt, mouse counts * sensitivity).
    pub yaw_delta: f32, pub pitch_delta: f32,
    /// Absolute camera override applied first (scripted camera): yaw in degrees (any value, wrapped), pitch in degrees.
    pub set_yaw: Option<f32>, pub set_pitch: Option<f32>,
    /// Start the Melee_Equip montage (pickaxe slot selected).
    pub equip: bool,
    /// The selected hotbar slot changed this frame: an outgoing melee montage / hit / equip is dropped (Swing::default(), equip off, latched attack cleared) so it is never carried into the new item.
    pub selection_changed: bool,
    /// A non-melee item is selected (!HudState::melee_equipped()): the attack key is ignored (no swing, no hit). Default false = the pickaxe is equipped.
    pub non_melee: bool,
    /// R key: back to the spawn.
    pub reset: bool,
}

/// A source of per-frame input (keyboard/gamepad adapter in the app, script in the harness).
pub trait InputSource { fn poll(&mut self, frame: u32, dt: f32) -> FrameInput; }
/// Closure adapter: `ScriptInput(|frame| FrameInput { .. })`.
pub struct ScriptInput<F: FnMut(u32) -> FrameInput>(pub F);
impl<F: FnMut(u32) -> FrameInput> InputSource for ScriptInput<F> { fn poll(&mut self, frame: u32, _dt: f32) -> FrameInput { (self.0)(frame) } }

/// Everything the old `App` owned that is game state (no GL objects, no window, no HUD).
pub struct GameSim {
    pub swing: Swing, pub world: World, pub controller: Controller, pub cam: Camera, pub cam_view: View, pub anim: AnimState,
    pub x: f32, pub y: f32, pub z: f32, pub yaw: f32, pub pitch: f32,
    pub crouch_camera: f32, pub anim_time: f32, pub locomotion_blend: f32, pub jump_stage: u8, pub state_time: f32,
    pub crouch_latch: ModeLatch, pub sprint_latch: ModeLatch, pub was_moving: bool,
    /// Input.CrouchMode == Toggle / Input.ToggleSprint (settings).
    pub crouch_toggle: bool, pub sprint_toggle: bool,
    pub frame_dt: f32, pub frames: u32,
    /// Movement experiment knobs (`FNRT_GRAVITY_SCALE` / `FNRT_JUMPZ`, see `fn_movement::cmc::Tuning`); applied to every controller this sim creates.
    pub tuning: Tuning,
    /// Montages driven from outside the pickaxe state (the front-end's weapon arms: equip / fire / reload / heal on the player's slots); appended to [`GameSim::montages`].
    pub arms: Vec<Montage>,
    /// ADS HOOK: hold / aim-down-sights camera modes, blend weight, pawn hiding and the targeting speed limit (see `ads_view`); default = the pickaxe camera.
    pub ads: crate::ads_view::AdsView,
}
impl Default for GameSim { fn default() -> Self { Self::new() } }
impl GameSim {
    /// The state `App` starts a run in (sandbox props, spawn pose).
    pub fn new() -> Self {
        let (x, y, z, yaw, pitch) = spawn();
        let mut g = GameSim { swing: Swing::default(), world: World::sandbox(), controller: Controller::default(), cam: Camera::default(), cam_view: View::default(), anim: AnimState::new(),
            x, y, z, yaw, pitch, crouch_camera: 0., anim_time: 0., locomotion_blend: 0., jump_stage: 0, state_time: 0.,
            crouch_latch: ModeLatch::default(), sprint_latch: ModeLatch::default(), was_moving: false, crouch_toggle: false, sprint_toggle: false, frame_dt: 0., frames: 0, tuning: Tuning::default(), arms: vec![], ads: Default::default() };
        g.settle_anim(); // the pawn starts standing idle (old App::settle_anim, run once before the first frame of every path)
        g
    }
    /// Runs the graph for 1.5 s of standing idle at the current camera yaw / pitch, so a render-only path (menus, captures that draw without stepping) shows the SAME idle
    /// pose the game does. A never-stepped AnimState is a different pose (jog lean additive at alpha 1, aim offset applied twice, camera pitch ignored).
    pub fn settle_anim(&mut self) { for _ in 0..90 { self.step_anim(Vec2::ZERO, 1. / 60.); } }
    /// A sim whose pawn uses the gravity / jump-speed tuning `t` (the front-end passes `Tuning::from_env()`; default = the recovered values).
    pub fn with_tuning(t: Tuning) -> Self { let mut g = Self::new(); g.tuning = t; g.controller = Controller::with_tuning(&t); g }
    /// A fresh controller carrying this sim's tuning (reset / respawn).
    pub fn new_controller(&self) -> Controller { Controller::with_tuning(&self.tuning) }
    pub fn to_spawn(&mut self) { (self.x, self.y, self.z, self.yaw, self.pitch) = spawn(); }
    /// Feet position in world metres (x, ground-relative y, z).
    pub fn feet(&self) -> Vec3 { Vec3::new(self.x, self.y - EYE, self.z) }
    /// Hands the player controller the current trunk/rock blockers (world z = controller y + 12).
    pub fn refresh_blockers(&mut self) {
        self.controller.blockers = fn_props::player_blockers(&self.world.props, Z_OFFSET);
        // real 1.8 props: their convex hulls within reach of the capsule (a prop is 1-9 hulls; the rest cannot be touched this tick)
        self.controller.hulls = fn_props::player_hulls(&self.world.props, Z_OFFSET, self.controller.position, 6.);
    }
    /// Athena melee camera (fn-camera): pivot/offsets/boom/collision against the baseplate and the live harvestables.
    pub fn update_camera(&mut self, dt: f32) {
        let feet = self.feet();
        let props = &self.world.props;
        let world = |a: Vec3, b: Vec3, r: f32| match (camera::trace_ground(a, b, r), fn_props::trace(props, a, b, r)) { (Some(x), Some(y)) => Some(x.min(y)), (x, y) => x.or(y) };
        self.cam_view = self.cam.update(self.ads.tick_mode(), camera::Input { feet, crouched: self.controller.crouched, yaw_deg: self.yaw, pitch_deg: self.pitch, dt }, world);
    }
    /// One animation-graph tick from the movement controller's state (the AnimBlueprint reads the pawn, never the keys): `input` is the
    /// movement input direction handed to `Controller::step` this frame.
    pub fn step_anim(&mut self, input: Vec2, dt: f32) {
        let c = &self.controller;
        let sample = PawnSample { velocity: (c.velocity.x, c.velocity.y), input: (input.x, input.y), facing_deg: self.yaw, camera_yaw_deg: self.yaw, camera_pitch_deg: self.pitch,
            height: c.air_height(), vertical_speed: c.vertical_speed, fall_time_left: c.fall_time_left(), crouched: c.crouched, running: c.running, melee_swinging: self.swing.stage > 0 };
        self.anim.step_sample(&sample, dt);
        self.controller.facing = self.anim.visual_yaw(self.yaw); // the visible body yaw is the graph's (Actor Yaw Offset), on every path that ticks the graph
    }
    /// Montages by slot: Melee_Equip on UpperBody; the pickaxe swing on both of its slot tracks (standing-still clip on FullBodyInPlace,
    /// in-motion clip on UpperBody; the graph's ShouldBypassFullbodyInPlace blend picks between them).
    pub fn montages(&self) -> Vec<Montage> {
        let mut m = vec![];
        if self.anim.equip.active { m.push(Montage { slot: "UpperBody", clip: "Melee_WideGrip_Equip_CMM".to_owned(), time: self.anim.equip.clip_time(), weight: self.anim.equip.weight(), blend: fn_animgraph::SlotBlend::Pose }); }
        if let (Some(standing), Some(moving)) = (self.swing.clip(false), self.swing.clip(true)) {
            let w = self.swing.weight();
            m.push(Montage { slot: "FullBodyInPlace", clip: standing.to_owned(), time: self.swing.time, weight: w, blend: fn_animgraph::SlotBlend::Pose });
            m.push(Montage { slot: "UpperBody", clip: moving.to_owned(), time: self.swing.time, weight: w, blend: fn_animgraph::SlotBlend::Pose });
        }
        m.extend(self.arms.iter().cloned());
        m
    }
    /// Selects the held item's core animation set (`None` = the pickaxe path).
    pub fn set_held(&mut self, held: Option<(&'static str, bool)>) { self.anim.set_held(held); }
    /// The held firearm's fire clock for the native relaxed update (see `fn_animgraph::WeaponClock`); `None` = pickaxe / consumable.
    pub fn set_weapon_clock(&mut self, clock: Option<fn_animgraph::WeaponClock>) { self.anim.set_weapon_clock(clock); }
    /// The AnimBlueprint's pose tree for this tick (evaluate with `fn_animgraph::Skeleton::eval_pose`).
    pub fn pose_tree(&self) -> Node { self.anim.plan(&self.montages()) }

    /// Advances one frame with a REAL frame time: the only clamp is `time_audit::MAX_DELTA_TIME` (UE `MaxDeltaTime` stand-in, 0.1 s; it used to
    /// be 50 ms, which ran the whole game in slow motion below 20 fps). Same as `step(inp, raw_dt.min(MAX_DELTA_TIME))`.
    pub fn step_real(&mut self, inp: &FrameInput, raw_dt: f32) { self.step(inp, crate::time_audit::clamp_frame_dt(raw_dt)); }

    // ---- the frame / tick split of the old `App::update` + `App::sim_tick` (see `crate::sim_tick`, README "Fixed 30 Hz tick") ----
    /// The per-FRAME half of the old `App::update` (everything that runs at display rate, whatever `SIM_HZ` is): equip montage trigger, camera yaw/pitch (input,
    /// not simulation), harvestable shake decay / regrow timer, and the crouch / sprint hold-or-toggle latches. Returns the held input the ticks of this frame use
    /// (`jump` and `attack` still raw: the caller latches them, see [`crate::tick_driver::TickDriver`]).
    pub fn begin_frame(&mut self, inp: &FrameInput, dt: f32) -> TickInput {
        self.frame_dt = dt;
        if inp.selection_changed { self.swing = Swing::default(); self.anim.equip.active = false; } // never carry an outgoing melee montage / hit into a newly selected hotbar item
        if inp.equip { self.anim.equip.start(); }
        self.anim_time += dt;
        if let Some(y) = inp.set_yaw { self.yaw = y.rem_euclid(360.); }
        if let Some(p) = inp.set_pitch { self.pitch = p; }
        self.world.update(dt); // cosmetic (shake, regrow): per frame so the wobble is not stepped at the tick rate
        self.yaw += inp.yaw_delta; self.pitch += inp.pitch_delta;
        self.yaw = self.yaw.rem_euclid(360.); self.pitch = camera::clamp_pitch(self.pitch);
        let (forward, side) = (inp.forward, inp.side);
        // Crouch/sprint: hold or toggle per settings (Input.CrouchMode / ToggleSprint).
        let moving = forward != 0. || side != 0.;
        let crouch_on = self.crouch_latch.update(inp.crouch_key, self.crouch_toggle, inp.jump);
        let run_on = self.sprint_latch.update(inp.sprint_key, self.sprint_toggle, self.was_moving && !moving);
        self.was_moving = moving;
        TickInput { forward, side, crouch: crouch_on || inp.force_crouch, run: run_on || inp.force_run, jump: inp.jump || inp.script_jump, attack: inp.attack && !inp.non_melee }
    }
    /// One fixed simulation tick of `h` seconds (the old `App::sim_tick`, minus the pose evaluation the renderer side adds): swing montage + its TriggerAbility hit
    /// trace, the movement controller (the CMC port), jump/fall stage, animation graph variables, camera smoothing + collision. Reads `self.yaw` / `self.pitch`.
    pub fn tick(&mut self, t: &TickInput, h: f32) {
        if let Some(Notify::TriggerAbility { .. }) = self.swing.step(t.attack, h) {
            // TriggerAbility notify: trace from the character along its facing; a hit takes the Bounce section.
            let f = self.controller.facing.to_radians();
            let p = self.controller.render_position();
            // HARVEST HOOK: the camera ray is the aim of the ability's WeakSpots line selection (fn_combat::harvest_fx)
            self.world.fx.aim = Some(fn_combat::harvest_fx::Aim { eye: self.cam_view.eye, dir: self.cam_view.forward });
            if self.world.strike(Vec3::new(p.x, self.controller.render_height(), Z_OFFSET + p.y), Vec3::new(f.sin(), 0., -f.cos())) { self.swing.blocked(); }
        }
        let (dx, dz) = horizontal_step(t.forward, t.side, self.yaw, 1., 1.);
        // Jump request (ACharacter::Jump/CheckJumpInput/CanJump): the controller sees the key STATE; a rising edge is one jump attempt that is dropped
        // at the end of that tick. CanJump refuses it while falling (JumpMaxCount 1), so an air press never re-triggers anything. The jump key also
        // cancels a crouch (ASSUMED), so the crouch request is dropped on the press tick.
        let crouch_wanted = t.crouch && !self.controller.jump_edge(t.jump);
        self.controller.wants_crouch = crouch_wanted;
        self.refresh_blockers();
        let sprint_blocked = self.ads.apply_speed(&mut self.controller); // ADS HOOK: targeting limits the speed stats to SpeedWalk and cancels sprint
        self.controller.step(Vec2::new(dx, dz), t.jump, t.run && !sprint_blocked, h);
        self.crouch_camera = smooth_toward(self.crouch_camera, if self.controller.crouched { 0.5 } else { 0. }, 12., h);
        let stage = animation_stage(self.controller.air_height(), self.controller.vertical_speed, self.jump_stage, self.state_time);
        if stage != self.jump_stage { self.state_time = 0.; self.jump_stage = stage; } else { self.state_time += h; }
        let target = (self.controller.velocity.length() / 1.5).min(1.);
        self.locomotion_blend = smooth_toward(self.locomotion_blend, target, 12., h);
        self.step_anim(Vec2::new(dx, dz), h);
        // Clamp the simulation itself (not just the camera) so the controller can't overshoot an invisible wall.
        self.controller.clamp_to_box(Vec2::new(-HALF + 0.5, -HALF + 0.5 - Z_OFFSET), Vec2::new(HALF - 0.5, HALF - 0.5 - Z_OFFSET));
        self.controller.facing = self.anim.visual_yaw(self.yaw); // visible body yaw from the graph (Actor Yaw Offset / turn-in-place)
        let p = self.controller.render_position(); let height = self.controller.render_height();
        self.advance_camera(Vec3::new(p.x, height, Z_OFFSET + p.y), h);
    }
    /// Camera collision world = baseplate + the live harvestables (trunks, canopies, rocks).
    fn camera_world<'a>(props: &'a [fn_combat::Harvestable]) -> impl Fn(Vec3, Vec3, f32) -> Option<f32> + 'a {
        move |a: Vec3, b: Vec3, r: f32| match (camera::trace_ground(a, b, r), fn_props::trace(props, a, b, r)) { (Some(x), Some(y)) => Some(x.min(y)), (x, y) => x.or(y) }
    }
    /// One camera smoothing tick (pivot lag, shoulder/crouch offset, boom feelers, penetration blend) at the simulation step.
    pub fn advance_camera(&mut self, feet: Vec3, dt: f32) {
        let world = Self::camera_world(&self.world.props);
        self.cam.advance(self.ads.tick_mode(), camera::Input { feet, crouched: self.controller.crouched, yaw_deg: self.yaw, pitch_deg: self.pitch, dt }, world);
    }
    /// What the renderer needs from the state after the last tick (position, floor height, visible facing, camera smoothing state); the driver adds poses.
    pub fn snapshot(&self) -> Snapshot {
        let p = self.controller.render_position();
        Snapshot { pos: p, height: self.controller.render_height(), facing: self.controller.facing, cam: self.cam.state(), poses: vec![] }
    }
    /// Shows `s` (usually an interpolation of two tick snapshots): pawn position x/y/z and the camera view composed from the interpolated smoothing state and
    /// THIS frame's yaw/pitch (mouse look is input, not simulation).
    pub fn apply_view(&mut self, s: &Snapshot) {
        self.x = s.pos.x; self.z = Z_OFFSET + s.pos.y; self.y = EYE + s.height;
        let feet = Vec3::new(s.pos.x, s.height, Z_OFFSET + s.pos.y);
        self.cam_view = self.ads.compose(&s.cam, feet, self.yaw, self.pitch, Self::camera_world(&self.world.props)); // ADS HOOK: hold view mixed with the ADS view (weight 0 = Camera::compose(MELEE))
    }
    /// The R key / a scripted reset: back to the spawn with a fresh controller.
    pub fn respawn(&mut self) { self.to_spawn(); self.controller = self.new_controller(); }

    /// Advances one frame in PER-FRAME mode (the old `FNRT_SIM_HZ=0`): one tick of the real frame time, drawn without interpolation. This is the path the harness
    /// (`fn-sim`) and the movement/jump logs use; the fixed-rate game loop is [`crate::tick_driver::TickDriver`]. `dt` is the frame time (the app clamps real
    /// frames to `MAX_DELTA_TIME` = 0.1 s; scripted runs use 1/60).
    pub fn step(&mut self, inp: &FrameInput, dt: f32) {
        let held = self.begin_frame(inp, dt);
        self.tick(&held, dt);
        let s = self.snapshot(); self.apply_view(&s);
        if inp.reset { self.respawn(); }
        self.frames += 1;
    }
}

/// Held input of one frame as the ticks of that frame read it (the old `AppSim.forward/side/crouch_on/run_on` + the latched `jump` / `attack`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TickInput { pub forward: f32, pub side: f32, pub crouch: bool, pub run: bool, pub jump: bool, pub attack: bool }
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn landing_is_one_shot_and_jump_can_interrupt() { assert_eq!(animation_stage(0., 0., 3, 0.1), 4); assert_eq!(animation_stage(0., 0., 4, 0.5), 4); assert_eq!(animation_stage(0., 0., 4, 0.84), 0); assert_eq!(animation_stage(0.1, 6., 4, 0.2), 1); }
    #[test] fn eight_direction_selection() { for (f, s, name) in [(1., 0., "N"), (1., 1., "NE"), (0., 1., "E"), (-1., 1., "SE"), (-1., 0., "S"), (-1., -1., "SW"), (0., -1., "W"), (1., -1., "NW")] { assert_eq!(direction_name(f, s), name); } }
    #[test] fn direction_blending_interpolates_and_wraps() { let angle = std::f32::consts::PI / 8.; let (a, b, t) = direction_blend(angle.cos(), angle.sin()); assert_eq!((a, b), ("N", "NE")); assert!((t - 0.5).abs() < 1e-6); let (a, b, t) = direction_blend(angle.cos(), -angle.sin()); assert_eq!((a, b), ("NW", "N")); assert!((t - 0.5).abs() < 1e-6); assert_eq!(direction_blend(-1., 0.).0, "S"); }
    #[test] fn diagonal_speed_matches_straight_speed() { let (x, z) = horizontal_step(1.0, 1.0, 0.0, 5.0, 1.0); assert!(((x * x + z * z).sqrt() - 5.0).abs() < 0.0001); }
    #[test] fn movement_follows_camera_heading() { let (x, z) = horizontal_step(1.0, 0.0, 90.0, 5.0, 1.0); assert!((x - 5.0).abs() < 0.0001 && z.abs() < 0.0001); }
    #[test] fn idle_does_not_move_and_spawn_is_on_plate() { assert_eq!(horizontal_step(0.0, 0.0, 50.0, 10.0, 1.0), (0.0, 0.0)); let (x, y, z, _, _) = spawn(); assert!(x.abs() < HALF && z.abs() < HALF && y == EYE); }

    fn run(frames: u32, f: impl Fn(u32) -> FrameInput) -> GameSim { let mut g = GameSim::new(); for i in 0..frames { g.step(&f(i), 1. / 60.); } g }
    /// The runtime feeds the held firearm's fire clock through `set_weapon_clock`: the graph reaches bIsRelaxedLevel1 after 5 s without a shot (and switches the upper-body
    /// pose to the relaxed one), a shot / ADS-free firing cancels it, putting the rifle away clears it (1.8 FortAnimInstance rule, see fn-animgraph AIM_RELAXED.md).
    #[test] fn weapon_clock_drives_the_relaxed_pose_and_a_shot_cancels_it() {
        use fn_animgraph::WeaponClock;
        let mut g = GameSim::new(); g.world.props.clear(); g.set_held(Some(("AssaultRifle", false)));
        let (dt, mut now, mut fire) = (1. / 30., 0f32, -10f32);
        let run = |g: &mut GameSim, now: &mut f32, fire: f32, secs: f32| { for _ in 0..(secs / dt).round() as usize { *now += dt; g.set_weapon_clock(Some(WeaponClock { world_time: *now, last_fire_time: fire, equipping: false, reloading: false, charging: false })); g.step_anim(Vec2::ZERO, dt); } };
        let relaxed = |g: &GameSim| { let mut v = vec![]; g.pose_tree().clip_names(&mut v); v.iter().any(|n| n == "M_AssaultRifle_Relaxed") };
        run(&mut g, &mut now, fire, 4.5);
        assert!(!g.anim.vars.b_is_relaxed_level1 && !relaxed(&g), "4.5 s: still ready (counter {})", g.anim.vars.relaxed_level_time_counter);
        run(&mut g, &mut now, fire, 1.0);
        assert!(g.anim.vars.b_is_relaxed_level1 && !g.anim.vars.b_is_relaxed_level2 && g.anim.vars.temp_is_relaxed_level1 && relaxed(&g), "5.5 s: relaxed level 1");
        fire = now; run(&mut g, &mut now, fire, 0.3);
        assert!(!g.anim.vars.b_is_relaxed_level1 && !relaxed(&g) && !g.anim.vars.b_recently_fired, "a shot cancels relaxed; bRecentlyFired lasts 0.2 s");
        run(&mut g, &mut now, fire, 15.5);
        assert!(g.anim.vars.b_is_relaxed_level2, "15 s after the last shot: level 2");
        g.set_weapon_clock(None); g.step_anim(Vec2::ZERO, dt);
        assert!(!g.anim.vars.b_is_relaxed_level1 && !g.anim.vars.b_is_relaxed_level2, "pickaxe in hand: never relaxed");
    }    #[test] fn walking_forward_moves_along_minus_z_at_the_run_speed_and_the_camera_follows() {
        let g = run(120, |_| FrameInput { forward: 1., ..Default::default() });
        assert!(g.z < 12. - 2., "forward is -Z: z {}", g.z); assert!((g.controller.velocity.length() - 4.1).abs() < 0.05, "{}", g.controller.velocity.length());
        assert!((g.cam_view.eye - g.feet()).length() > 1.5 && g.cam_view.eye.is_finite(), "third-person boom behind the pawn");
    }
    #[test] fn holding_fire_next_to_a_tree_harvests_it_and_bounces() {
        let mut g = GameSim::new(); g.world.props.clear(); g.world.props.push(fn_combat::Harvestable::new(fn_combat::Kind::Tree, Vec3::new(0., 0., 12. - 1.8)));
        for _ in 0..300 { g.step(&FrameInput { attack: true, ..Default::default() }, 1. / 60.); }
        assert!(g.world.hits >= 3 && g.world.wood > 0, "hits {} wood {}", g.world.hits, g.world.wood);
    }
    #[test] fn same_inputs_give_bit_identical_state() {
        let f = |i: u32| FrameInput { forward: if (30..200).contains(&i) { 1. } else { 0. }, side: if (100..140).contains(&i) { 1. } else { 0. }, jump: i == 80 || i == 150, sprint_key: (60..180).contains(&i), yaw_delta: if i < 90 { 1.5 } else { 0. }, ..Default::default() };
        let (a, b) = (run(300, f), run(300, f));
        assert_eq!((a.x.to_bits(), a.y.to_bits(), a.z.to_bits()), (b.x.to_bits(), b.y.to_bits(), b.z.to_bits())); assert_eq!(a.anim.describe(), b.anim.describe()); assert_eq!(format!("{:?}", a.cam_view), format!("{:?}", b.cam_view));
    }
    #[test] fn reset_returns_to_the_spawn() { let mut g = run(60, |_| FrameInput { forward: 1., ..Default::default() }); g.step(&FrameInput { reset: true, ..Default::default() }, 1. / 60.); assert_eq!((g.x, g.z, g.yaw), (0., 12., 0.)); assert_eq!(g.controller.position, Vec2::ZERO); }
    #[test] fn the_baseplate_edge_stops_the_pawn() { let g = run(60 * 100, |_| FrameInput { forward: 1., sprint_key: true, ..Default::default() }); assert!(g.z >= -HALF + 0.4 - 0.01, "z {}", g.z); }
}
