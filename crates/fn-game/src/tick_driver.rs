//! The fixed-rate game loop the front-end (`fn-app`) calls once per RENDERED frame: [`TickDriver`] wraps a [`GameSim`] with the old tick wiring of
//! `App::update` / `App::sim_tick` (README "Fixed 30 Hz tick"): frame time in, `floor(acc / (1/SIM_HZ))` simulation ticks out (at most 5 per frame), input taps
//! latched so none is lost between two ticks, the last two tick snapshots blended by the interpolation `alpha` (character position / height / facing, camera
//! smoothing state, bone poses) and the camera view composed every frame from the interpolated state with the CURRENT mouse-look yaw / pitch.
//!
//! [`SimClock`] is the bare accumulator (`sim_tick::TickClock`) for callers that run their own world; [`TickDriver`] is `SimClock` + `GameSim` + latches + snapshots.
//!
//! ```ignore
//! // fn-app, once per frame (after polling the keyboard / mouse / pad into a FrameInput):
//! let rep = driver.frame_real(&input, raw_dt, &mut |sim| /* evaluate the pose tree per model */ poses_of(sim));   // `raw_dt` is clamped to MAX_DELTA_TIME (0.1 s)
//! // then draw `driver.sim` (x/y/z, cam_view are the INTERPOLATED state) and `driver.drawn().unwrap().poses` (already blended by `rep.alpha`).
//! ```
//! `hz == 0` (`FNRT_SIM_HZ=0`, [`TickDriver::per_frame`]) is the old per-frame mode bit for bit: one tick of the real frame time per frame, alpha = 1.
use crate::sim::{FrameInput, GameSim, TickInput};
use crate::sim_tick::{Advance, Latched, Pair, Snapshot, TickClock};
use glam::{Quat, Vec3};

/// The simulation accumulator under its API name: `advance(frame_dt)` -> [`Advance`] (`ticks`, `step`, `alpha`, `dropped`).
pub type SimClock = TickClock;

/// Pose per skinned model (translation, rotation per bone): what a tick snapshot carries for the renderer.
pub type Poses = Vec<Vec<(Vec3, Quat)>>;

/// What one [`TickDriver::frame`] did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameReport {
    /// Frame time that was simulated (after the `MAX_DELTA_TIME` clamp for `frame_real`).
    pub dt: f32,
    /// The accumulator's answer: ticks run this frame, their step, the interpolation alpha, the dropped backlog.
    pub advance: Advance,
}
impl FrameReport { pub fn ticks(&self) -> u32 { self.advance.ticks } pub fn alpha(&self) -> f32 { self.advance.alpha } }

pub struct TickDriver {
    pub sim: GameSim,
    pub clock: SimClock,
    jump: Latched, attack: Latched,
    snaps: Option<Pair>,
    draw: Option<Snapshot>,
    /// Last frame's [`FrameReport`] and the running tick count (the old `AppSim.last` / `ticks_total`).
    pub last: Option<FrameReport>,
    pub ticks_total: u64,
}
impl TickDriver {
    /// A driver at `hz` ticks per second (`0` = per-frame mode).
    pub fn new(sim: GameSim, hz: u32) -> Self { TickDriver { sim, clock: SimClock::new(hz), jump: Latched::default(), attack: Latched::default(), snaps: None, draw: None, last: None, ticks_total: 0 } }
    /// The rate of this run: `FNRT_SIM_HZ` (unset = [`crate::sim_tick::SIM_HZ`] = 30, `0` = per frame).
    pub fn from_env(sim: GameSim) -> Self { Self::new(sim, crate::sim_tick::sim_hz()) }
    /// The old once-per-frame mode.
    pub fn per_frame(sim: GameSim) -> Self { Self::new(sim, 0) }
    pub fn hz(&self) -> u32 { self.clock.hz() }

    /// One rendered frame with a REAL frame time `raw_dt` (clamped to [`crate::time_audit::MAX_DELTA_TIME`], the old `App::update` rule). `poser` evaluates the
    /// pose tree of the sim after every tick (e.g. `|g| models.iter().map(|m| m.eval_pose(&g.pose_tree(), &clips)).collect()`); pass `&mut |_| Vec::new()` when
    /// the caller draws without bone poses.
    pub fn frame_real(&mut self, inp: &FrameInput, raw_dt: f32, poser: &mut dyn FnMut(&GameSim) -> Poses) -> FrameReport { self.frame(inp, crate::time_audit::clamp_frame_dt(raw_dt), poser) }

    /// One rendered frame of exactly `dt` seconds (scripted runs, tests): `begin_frame` -> latch -> N x `tick` (+ snapshot) -> interpolate -> compose the view.
    pub fn frame(&mut self, inp: &FrameInput, dt: f32, poser: &mut dyn FnMut(&GameSim) -> Poses) -> FrameReport {
        if inp.selection_changed { self.attack = Latched::default(); } // a tap latched for the outgoing item is dropped with its swing
        let held = self.sim.begin_frame(inp, dt);
        self.jump.sample(held.jump); self.attack.sample(held.attack);
        let adv = self.clock.advance(dt);
        for _ in 0..adv.ticks {
            let t = TickInput { jump: self.jump.take(), attack: self.attack.take(), ..held };
            self.sim.tick(&t, adv.step);
            let mut snap = self.sim.snapshot(); snap.poses = poser(&self.sim);
            match &mut self.snaps { Some(pair) => pair.push(snap), None => self.snaps = Some(Pair::new(snap)) }
        }
        if let Some(pair) = &self.snaps {
            let s = pair.at(adv.alpha);
            self.sim.apply_view(&s);
            self.draw = Some(s);
        }
        if inp.reset { self.sim.respawn(); self.snap(); }
        self.sim.frames += 1;
        self.ticks_total = self.clock.total_ticks;
        let rep = FrameReport { dt, advance: adv };
        self.last = Some(rep); rep
    }
    /// A teleport (respawn, scripted reset): drop the interpolation history so the next frame shows the new state outright.
    pub fn snap(&mut self) { self.snaps = None; self.draw = None; }
    /// The interpolated state drawn by the last frame (`None` before the first tick or after a [`TickDriver::snap`]). A renderer that finds no snapshot evaluates
    /// the pose tree itself (render-only capture paths).
    pub fn drawn(&self) -> Option<&Snapshot> { self.draw.as_ref() }
    /// Facing (degrees) to draw the character with: the interpolated one when a snapshot exists.
    pub fn drawn_facing(&self) -> f32 { self.draw.as_ref().map_or(self.sim.controller.facing, |d| d.facing) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn none(_: &GameSim) -> Poses { Vec::new() }
    fn script(tick: u64) -> FrameInput { // per-TICK script so every display rate sees the same input stream
        FrameInput { forward: if tick >= 10 { 1. } else { 0. }, sprint_key: (20..120).contains(&tick), jump: (60..63).contains(&tick) || (100..103).contains(&tick), force_crouch: (150..180).contains(&tick), set_yaw: Some((tick as f32 * 1.5) % 360.), set_pitch: Some(-12.), ..Default::default() }
    }
    /// Runs `seconds` of frames at `fps`, feeding the script by the number of ticks simulated so far; returns the pawn after every tick.
    fn run(hz: u32, fps: f32, seconds: f32) -> Vec<(u64, [u32; 6])> {
        let mut d = TickDriver::new(GameSim::new(), hz); let mut rows = vec![]; let dt = 1. / fps;
        for _ in 0..(seconds * fps).round() as usize {
            let before = d.clock.total_ticks; let inp = script(before);
            d.frame(&inp, dt, &mut none);
            for n in before..d.clock.total_ticks { let c = &d.sim.controller; let _ = n; rows.push((d.clock.total_ticks, [c.position.x.to_bits(), c.position.y.to_bits(), c.height.to_bits(), c.vertical_speed.to_bits(), c.velocity.x.to_bits(), c.velocity.y.to_bits()])); }
        }
        rows
    }
    #[test] fn per_frame_mode_is_exactly_gamesim_step() {
        let mut a = GameSim::new(); let mut d = TickDriver::per_frame(GameSim::new());
        for i in 0..400u32 {
            let inp = FrameInput { forward: if i > 20 { 1. } else { 0. }, jump: (100..110).contains(&i), sprint_key: i > 50, yaw_delta: 0.7, attack: (200..260).contains(&i), ..Default::default() };
            a.step(&inp, 1. / 60.); d.frame(&inp, 1. / 60., &mut none);
            assert_eq!((a.x.to_bits(), a.y.to_bits(), a.z.to_bits()), (d.sim.x.to_bits(), d.sim.y.to_bits(), d.sim.z.to_bits()), "frame {i}");
            assert_eq!(format!("{:?}", a.cam_view), format!("{:?}", d.sim.cam_view), "frame {i}");
            assert_eq!((a.swing.stage, a.swing.time.to_bits(), a.world.hits), (d.sim.swing.stage, d.sim.swing.time.to_bits(), d.sim.world.hits));
        }
    }
    #[test] fn thirty_hz_runs_one_tick_per_two_frames_at_60_fps_and_alpha_alternates() {
        let mut d = TickDriver::new(GameSim::new(), 30); let mut ticks = vec![];
        for _ in 0..10 { let r = d.frame(&FrameInput { forward: 1., ..Default::default() }, 1. / 60., &mut none); ticks.push((r.ticks(), (r.alpha() * 100.).round() as i32)); }
        assert_eq!(ticks.iter().map(|t| t.0).sum::<u32>(), 5, "{ticks:?}"); assert!(ticks.iter().all(|t| t.1 == 0 || t.1 == 50), "{ticks:?}");
    }
    #[test] fn tick_state_is_bit_identical_at_30_60_144_240_fps() {
        let reference = run(30, 60., 6.);
        assert_eq!(reference.len(), 180);
        for fps in [30., 144., 240., 100., 59.94] {
            let rows = run(30, fps, 6.);
            assert!(rows.len() >= 179 && rows.len() <= 181, "{fps} fps: {}", rows.len());
            let n = 179.min(rows.len());
            assert_eq!(&rows[..n].iter().map(|r| r.1).collect::<Vec<_>>(), &reference[..n].iter().map(|r| r.1).collect::<Vec<_>>(), "{fps} fps changed the simulation");
        }
        // the script exercised running and jumping
        assert!(f32::from_bits(reference.last().unwrap().1[0]).abs() + f32::from_bits(reference.last().unwrap().1[1]).abs() > 5.);
    }
    #[test] fn a_one_frame_tap_at_240_fps_still_jumps_and_a_held_key_jumps_once() {
        for press in 238..250usize {
            let mut d = TickDriver::new(GameSim::new(), 30);
            for k in 0..720usize { d.frame(&FrameInput { jump: k == press, ..Default::default() }, 1. / 240., &mut none); }
            assert_eq!(d.sim.controller.jumps_started(), 1, "tap at frame {press}");
        }
        let mut d = TickDriver::new(GameSim::new(), 30); for k in 0..480usize { d.frame(&FrameInput { jump: (60..300).contains(&k), ..Default::default() }, 1. / 120., &mut none); }
        assert_eq!(d.sim.controller.jumps_started(), 1, "holding the key is one jump (one rising edge)");
    }
    #[test] fn interpolation_is_smooth_at_144_fps_and_the_view_follows_the_current_yaw() {
        let mut d = TickDriver::new(GameSim::new(), 30); let mut xs = vec![];
        for k in 0..(4. * 144.) as usize { d.frame(&FrameInput { forward: 1., sprint_key: true, yaw_delta: 0.3, ..Default::default() }, 1. / 144., &mut none); if k as f32 / 144. > 2. { xs.push((d.sim.x, d.sim.z)); } }
        let ds: Vec<f32> = xs.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).collect(); let mean = ds.iter().sum::<f32>() / ds.len() as f32;
        assert!(ds.iter().all(|x| (x - mean).abs() < 0.12 * mean), "interpolated frame deltas stay within 12% of the mean {mean}");
        assert!((d.sim.cam_view.yaw_deg - d.sim.yaw).abs() < 1e-3, "the view is composed with this frame's yaw");
    }
    #[test] fn a_teleport_is_never_interpolated_and_snap_drops_the_history() {
        let mut d = TickDriver::new(GameSim::new(), 30);
        for _ in 0..60 { d.frame(&FrameInput { forward: 1., ..Default::default() }, 1. / 60., &mut none); }
        assert!(d.drawn().is_some());
        d.frame(&FrameInput { reset: true, ..Default::default() }, 1. / 60., &mut none); assert!(d.drawn().is_none(), "respawn drops the interpolation history");
        assert_eq!(d.sim.controller.position, glam::Vec2::ZERO);
    }
    #[test] fn poses_from_the_poser_are_blended_by_alpha() {
        let mut d = TickDriver::new(GameSim::new(), 30); let mut n = 0.;
        let mut poser = |_: &GameSim| { n += 1.; vec![vec![(Vec3::new(n, 0., 0.), Quat::IDENTITY)]] };
        d.frame(&FrameInput::default(), 1. / 30., &mut poser); d.frame(&FrameInput::default(), 1. / 30., &mut poser); // two ticks -> prev x=1, cur x=2
        let r = d.frame(&FrameInput::default(), 1. / 60., &mut poser); assert_eq!(r.ticks(), 0);
        assert!((d.drawn().unwrap().poses[0][0].0.x - 2.).abs() < 1e-5 || (d.drawn().unwrap().poses[0][0].0.x - 1.5).abs() < 1e-4, "{:?}", d.drawn().unwrap().poses);
    }
    #[test] fn a_swing_never_carries_into_a_newly_selected_item_and_non_melee_slots_do_not_swing() {
        let mut d = TickDriver::new(GameSim::new(), 30);
        for _ in 0..20 { d.frame(&FrameInput { attack: true, ..Default::default() }, 1. / 60., &mut none); }
        assert!(d.sim.swing.stage > 0, "swinging");
        d.frame(&FrameInput { attack: true, selection_changed: true, non_melee: true, ..Default::default() }, 1. / 60., &mut none);
        assert_eq!(d.sim.swing.stage, 0, "the outgoing swing is dropped on selection"); assert!(!d.sim.anim.equip.active);
        for _ in 0..60 { d.frame(&FrameInput { attack: true, non_melee: true, ..Default::default() }, 1. / 60., &mut none); }
        assert_eq!(d.sim.swing.stage, 0, "a non-melee item ignores the attack key");
        for _ in 0..10 { d.frame(&FrameInput { attack: true, ..Default::default() }, 1. / 60., &mut none); }
        assert!(d.sim.swing.stage > 0, "back on the pickaxe the attack works again");
    }
    #[test] fn real_frames_are_clamped_to_max_delta_time() {
        let mut d = TickDriver::new(GameSim::new(), 30); let r = d.frame_real(&FrameInput::default(), 5., &mut none);
        assert_eq!(r.dt, crate::time_audit::MAX_DELTA_TIME); assert_eq!(r.ticks(), 3, "0.1 s = 3 ticks");
    }
}
