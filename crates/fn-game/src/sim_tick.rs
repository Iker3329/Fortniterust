//! Fixed-rate gameplay simulation (`SIM_HZ`, default 30 Hz) with render-time interpolation. README section "Fixed 30 Hz tick".
//!
//! WHAT RUNS WHERE
//!  * every FRAME (display rate, vsync, any fps): window/input polling, mouse + stick look (camera yaw/pitch ARE input), HUD/UI/menus, fps counter, harvestable
//!    shake decay, and the render itself, which blends the last two ticks with `alpha` (character position/height/facing, camera smoothing state, bone poses).
//!  * every TICK (`1 / SIM_HZ` s, driven by [`TickClock`]): the movement controller (UE `CharacterMovementComponent` port: walk, jump, fall, crouch), the
//!    pickaxe swing montage timer and its hit trace, the animation graph variables (state machines, blend weights, BlendList cross-fades, montage clocks),
//!    the camera smoothing (origin lag, shoulder/crouch offset, boom collision and penetration blend) and the pose evaluation that is later interpolated.
//!  * `FNRT_SIM_HZ=0` (or `SIM_HZ` set to 0) restores the old per-frame mode bit for bit: one "tick" per frame with the real frame time, alpha = 1.
//!
//! The evidence for the default is in `reports/tick-rate-evidence.json` (1.8 has a 30 Hz dedicated-server tick cap and per-frame client prediction; the
//! 30 Hz lock of this single-player build is the user's explicit direction, not something a 1.8 file pins).
#![allow(dead_code)]
use fn_animgraph::{AnimState,JumpState,PawnSample};
use fn_camera::{self as camera,CamState,Camera};
use fn_movement::{self as movement,Controller};
use glam::{Quat,Vec2,Vec3};
use std::fmt::Write as _;

/// Default simulation rate in Hz. Override per run with `FNRT_SIM_HZ=<hz>`; `0` = simulate once per rendered frame (the old behaviour).
pub const SIM_HZ: u32 = 30;
/// Most ticks one frame may run before the remaining backlog is dropped (spiral-of-death guard: a 5 tick catch-up is 167 ms of game time at 30 Hz).
pub const MAX_TICKS_PER_FRAME: u32 = 5;
/// Positions further apart than this between two ticks are a teleport (respawn, R key): the renderer snaps instead of gliding across the map.
pub const TELEPORT_M: f32 = 3.0;
/// Float slack when comparing the accumulator with the step (30 f32 frames of 1/30 s must give exactly one second of ticks).
const EPS: f64 = 1e-6;

/// `FNRT_SIM_HZ` parsing: unset/empty/garbage -> [`SIM_HZ`]; `0` -> per-frame mode; otherwise the rate (clamped to 1..=1000).
pub fn parse_sim_hz(v: Option<&str>) -> u32 {
    match v.map(str::trim) { None | Some("") => SIM_HZ, Some(s) => match s.parse::<u32>() { Ok(0) => 0, Ok(h) => h.clamp(1, 1000), Err(_) => SIM_HZ } }
}
/// The simulation rate of this run (read once from `FNRT_SIM_HZ`).
pub fn sim_hz() -> u32 {
    static H: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *H.get_or_init(|| parse_sim_hz(std::env::var("FNRT_SIM_HZ").ok().as_deref()))
}

/// What one frame has to do: run `ticks` simulation ticks of `step` seconds, then draw the state `alpha` (0..1) of the way from the previous tick to the last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Advance { pub ticks: u32, pub step: f32, pub alpha: f32, pub dropped: f32 }

/// Fixed-step accumulator ("fix your timestep"): frame time in, whole ticks out, the remainder becomes the interpolation alpha.
#[derive(Clone, Debug)]
pub struct TickClock { hz: u32, acc: f64, pub total_ticks: u64, pub dropped_total: f64 }
impl Default for TickClock { fn default() -> Self { TickClock::new(sim_hz()) } }
impl TickClock {
    pub fn new(hz: u32) -> Self { TickClock { hz, acc: 0., total_ticks: 0, dropped_total: 0. } }
    pub fn hz(&self) -> u32 { self.hz }
    pub fn step_secs(&self) -> f64 { if self.hz == 0 { 0. } else { 1. / self.hz as f64 } }
    /// Seconds of frame time not yet consumed by a tick.
    pub fn remainder(&self) -> f64 { self.acc }
    /// Starts with `secs` already accumulated (test hook: sweeps the tick phase against the frame phase).
    pub fn with_phase(mut self, secs: f64) -> Self { self.acc = secs.max(0.); self }
    /// Feeds one frame of `dt` seconds. Per-frame mode (hz 0): exactly one tick of the real `dt`, alpha 1.
    pub fn advance(&mut self, dt: f32) -> Advance {
        let dt = (dt as f64).max(0.);
        if self.hz == 0 { self.total_ticks += 1; return Advance { ticks: 1, step: dt as f32, alpha: 1., dropped: 0. }; }
        let h = self.step_secs();
        self.acc += dt;
        let mut ticks = 0u32;
        while self.acc + EPS >= h && ticks < MAX_TICKS_PER_FRAME { self.acc -= h; ticks += 1; }
        let mut dropped = 0.;
        if self.acc + EPS >= h { // hitch beyond the cap: drop the whole ticks, keep the fraction so the phase stays continuous
            let whole = ((self.acc + EPS) / h).floor(); dropped = whole * h; self.acc -= dropped;
        }
        if self.acc < 0. { self.acc = 0.; }
        self.total_ticks += ticks as u64; self.dropped_total += dropped;
        Advance { ticks, step: h as f32, alpha: ((self.acc / h) as f32).clamp(0., 1.), dropped: dropped as f32 }
    }
}

/// A key state that cannot be lost between two ticks: `take` reports "down now OR pressed since the last take" (a 10 ms tap that starts and ends between
/// two ticks still reaches the simulation as one tick of key-down; a held key stays down, so the controller still sees exactly one rising edge).
#[derive(Clone, Copy, Debug, Default)]
pub struct Latched { now: bool, pending: bool }
impl Latched {
    /// Once per frame with the key state polled this frame.
    pub fn sample(&mut self, down: bool) { self.now = down; self.pending |= down; }
    /// Once per tick; consumes the pending press.
    pub fn take(&mut self) -> bool { let r = self.now || self.pending; self.pending = false; r }
    pub fn now(&self) -> bool { self.now }
}

/// Shortest-arc interpolation of an angle in degrees (result in [0,360)).
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 { (a + movement::shortest_angle(a, b) * t).rem_euclid(360.) }
/// Bone pose blend: translation lerp, rotation slerp (normalised). Falls back to `b` when the skeletons differ in size.
pub fn blend_pose(a: &[(Vec3, Quat)], b: &[(Vec3, Quat)], t: f32) -> Vec<(Vec3, Quat)> {
    if a.len() != b.len() { return b.to_vec(); }
    if t >= 1. { return b.to_vec(); }
    a.iter().zip(b).map(|(x, y)| (x.0.lerp(y.0, t), x.1.slerp(y.1, t).normalize())).collect()
}

/// Everything the renderer needs from one tick.
#[derive(Clone, Debug, Default)]
pub struct Snapshot { pub pos: Vec2, pub height: f32, pub facing: f32, pub cam: CamState, pub poses: Vec<Vec<(Vec3, Quat)>> }
impl Snapshot {
    /// The state `t` of the way from `a` to `b`; a teleport between the two (> [`TELEPORT_M`]) shows `b` outright.
    pub fn blend(a: &Snapshot, b: &Snapshot, t: f32) -> Snapshot {
        if t >= 1. || (a.pos - b.pos).length() > TELEPORT_M || (a.height - b.height).abs() > TELEPORT_M { return b.clone(); }
        Snapshot { pos: a.pos.lerp(b.pos, t), height: a.height + (b.height - a.height) * t, facing: lerp_angle(a.facing, b.facing, t), cam: CamState::lerp(&a.cam, &b.cam, t),
            poses: if a.poses.len() == b.poses.len() { a.poses.iter().zip(&b.poses).map(|(p, q)| blend_pose(p, q, t)).collect() } else { b.poses.clone() } }
    }
}
/// The last two tick snapshots.
#[derive(Clone, Debug, Default)]
pub struct Pair { pub prev: Snapshot, pub cur: Snapshot }
impl Pair {
    pub fn new(s: Snapshot) -> Pair { Pair { prev: s.clone(), cur: s } }
    pub fn push(&mut self, s: Snapshot) { self.prev = std::mem::replace(&mut self.cur, s); }
    /// Drops the interpolation history (teleport / reset): the next frame shows `cur` exactly.
    pub fn snap(&mut self) { self.prev = self.cur.clone(); }
    pub fn at(&self, alpha: f32) -> Snapshot { Snapshot::blend(&self.prev, &self.cur, alpha) }
}

/// The simulation bookkeeping of the real `App` (the game loop in main.rs owns the actual controller / animation / camera objects).
#[derive(Debug, Default)]
pub struct AppSim {
    pub clock: TickClock, pub jump: Latched, pub attack: Latched,
    /// Frame-level held input, sampled every frame and read by every tick of that frame.
    pub forward: f32, pub side: f32, pub crouch_on: bool, pub run_on: bool,
    /// `None` until the first tick has run (render-only capture paths never tick and draw the controller directly).
    pub snaps: Option<Pair>, pub alpha: f32,
    /// What the last `App::update` decided to draw (snapshots blended by `alpha`); `render` uses it when present.
    pub draw: Option<Snapshot>,
    pub last: Option<Advance>, pub frame_ticks: u32, pub ticks_total: u64,
}

// ------------------------------------------------------------------------------------------------------------------------------
// Headless twin of the tick wiring (no window / GL / models): controller + animation graph + camera on the fixed step with interpolation.
// ------------------------------------------------------------------------------------------------------------------------------
/// One frame of live input (the keys as polled by the frame) plus the mouse look.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameIn { pub forward: bool, pub run: bool, pub jump: bool, pub crouch: bool, pub yaw: f32, pub pitch: f32 }
/// State after one tick (what a regression log records).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickRow { pub n: u64, pub t: f32, pub pos: Vec2, pub height: f32, pub vz: f32, pub air: bool, pub jumps: u32, pub state: JumpState, pub mix: f32, pub facing: f32, pub eye: Vec3, pub crouched: bool }
/// What the frame draws.
#[derive(Clone, Copy, Debug)]
pub struct Frame { pub ticks: u32, pub alpha: f32, pub pos: Vec2, pub height: f32, pub facing: f32, pub eye: Vec3, pub hide_pawn: bool }

pub struct FixedSim { pub clock: TickClock, pub c: Controller, pub a: AnimState, pub cam: Camera, pub jump: Latched, pub snaps: Pair, pub rows: Vec<TickRow>, yaw: f32, pitch: f32 }
impl FixedSim {
    pub fn new(hz: u32) -> Self {
        FixedSim { clock: TickClock::new(hz), c: Controller::default(), a: AnimState::new(), cam: Camera::default(), jump: Latched::default(), snaps: Pair::default(), rows: vec![], yaw: 0., pitch: 0. }
    }
    pub fn ticks(&self) -> u64 { self.rows.len() as u64 }
    /// One simulation tick, wired like `App::sim_tick`: controller -> animation graph -> camera smoothing -> snapshot.
    fn tick(&mut self, h: f32, i: &FrameIn, jump: bool) {
        let (s, co) = ((i.yaw.to_radians()).sin(), (i.yaw.to_radians()).cos());
        let dir = if i.forward { Vec2::new(s, -co) } else { Vec2::ZERO };
        self.c.wants_crouch = i.crouch && !self.c.jump_edge(jump);
        self.c.step(dir, jump, i.run, h);
        let c = &self.c;
        let sample = PawnSample { velocity: (c.velocity.x, c.velocity.y), input: (dir.x, dir.y), facing_deg: i.yaw, camera_yaw_deg: i.yaw, camera_pitch_deg: i.pitch, height: c.air_height(), vertical_speed: c.vertical_speed,
            fall_time_left: movement::fall_time_left(c.air_height(), c.vertical_speed), crouched: c.crouched, running: c.running, melee_swinging: false };
        self.a.step_sample(&sample, h);
        self.c.facing = self.a.visual_yaw(i.yaw);
        let p = self.c.render_position();
        let feet = Vec3::new(p.x, self.c.render_height(), 12. + p.y);
        self.cam.advance(&camera::MELEE, camera::Input { feet, crouched: self.c.crouched, yaw_deg: i.yaw, pitch_deg: i.pitch, dt: h }, camera::trace_ground);
        let snap = Snapshot { pos: p, height: self.c.render_height(), facing: self.c.facing, cam: self.cam.state(), poses: vec![] };
        if self.rows.is_empty() { self.snaps = Pair::new(snap); } else { self.snaps.push(snap); }
        let v = Camera::compose(&camera::MELEE, &self.cam.state(), feet, i.yaw, i.pitch, camera::trace_ground);
        let n = self.rows.len() as u64 + 1;
        self.rows.push(TickRow { n, t: n as f32 * h, pos: p, height: self.c.height, vz: self.c.vertical_speed, air: self.c.airborne(), jumps: self.c.jumps_started(), state: self.a.jump.state, mix: self.a.jump.pose().mix, facing: self.c.facing, eye: v.eye, crouched: self.c.crouched });
    }
    /// One display frame. `script` (when given) supplies the input of every tick from the tick index (deterministic at any fps); without it the tick
    /// reads the frame's live input (jump through the latch).
    pub fn frame(&mut self, dt: f32, live: FrameIn, script: Option<&dyn Fn(u64) -> FrameIn>) -> Frame {
        self.jump.sample(live.jump); self.yaw = live.yaw; self.pitch = live.pitch;
        let adv = self.clock.advance(dt);
        for _ in 0..adv.ticks {
            match script { Some(s) => { let i = s(self.ticks()); self.tick(adv.step, &i, i.jump); } None => { let j = self.jump.take(); self.tick(adv.step, &live, j); } }
        }
        let s = if self.rows.is_empty() { Snapshot::default() } else { self.snaps.at(adv.alpha) };
        let feet = Vec3::new(s.pos.x, s.height, 12. + s.pos.y);
        let v = Camera::compose(&camera::MELEE, &s.cam, feet, live.yaw, live.pitch, camera::trace_ground);
        Frame { ticks: adv.ticks, alpha: adv.alpha, pos: s.pos, height: s.height, facing: s.facing, eye: v.eye, hide_pawn: v.hide_pawn }
    }
}

/// Runs `seconds` of display frames at `render_hz` with a per-TICK script; returns the tick log (identical at every `render_hz`) and the frames drawn.
pub fn run_scripted(sim_hz: u32, render_hz: f32, seconds: f32, script: &dyn Fn(u64) -> FrameIn) -> (Vec<TickRow>, Vec<Frame>) {
    let mut s = FixedSim::new(sim_hz); let dt = 1. / render_hz; let n = (seconds * render_hz).round() as usize; let mut frames = Vec::with_capacity(n);
    for _ in 0..n { frames.push(s.frame(dt, FrameIn::default(), Some(script))); }
    (s.rows, frames)
}
/// Runs with LIVE input defined on wall time (`key(t)` is the key state polled at the start of the frame that begins at wall time t).
pub fn run_live(sim_hz: u32, render_hz: f32, seconds: f32, phase: f64, key: &dyn Fn(f32) -> FrameIn) -> (FixedSim, Vec<(f32, Frame)>) {
    let mut s = FixedSim::new(sim_hz); s.clock = TickClock::new(sim_hz).with_phase(phase); let dt = 1. / render_hz; let n = (seconds * render_hz).round() as usize; let mut frames = Vec::with_capacity(n);
    for k in 0..n { let t = k as f32 * dt; let f = s.frame(dt, key(t), None); frames.push((t, f)); }
    (s, frames)
}

/// Key press -> liftoff latency of a standing jump at `render_hz`, swept over `phases` tick/frame phase offsets. Returns per phase (poll -> sim state airborne,
/// poll -> first frame with a drawn height > 0, poll -> first frame with a drawn height >= 1 cm), all in ms.
pub fn latency_sweep(sim_hz: u32, render_hz: f32, phases: usize) -> Vec<(f32, f32, f32)> {
    let h = if sim_hz == 0 { 0. } else { 1. / sim_hz as f64 }; let dt = 1. / render_hz; let press = 1.0f32;
    (0..phases).map(|p| {
        let phase = h * p as f64 / phases as f64;
        let (sim, frames) = run_live(sim_hz, render_hz, 2.0, phase, &|t| FrameIn { jump: t >= press - 1e-4 && t < press + 0.1, ..Default::default() });
        let k0 = frames.iter().position(|(t, _)| *t >= press - 1e-4).unwrap();
        let t0 = frames[k0].0;
        let _ = dt;
        // time of the first frame (update + draw happen together) that shows the effect, measured from the frame that polled the key
        let first = |th: f32| frames.iter().skip(k0).find(|(_, f)| f.height >= th).map_or(f32::NAN, |(t, _)| (t - t0) * 1000.);
        // sim state airborne: first frame whose ticks include a tick row with air set
        let mut seen = 0usize; let mut air_ms = f32::NAN;
        for (t, f) in frames.iter() { let before = seen; seen += f.ticks as usize; if f.ticks > 0 && *t >= t0 - 1e-6 && air_ms.is_nan() && sim.rows[before..seen].iter().any(|r| r.air) { air_ms = (t - t0) * 1000.; } }
        (air_ms, first(1e-5), first(0.01))
    }).collect()
}

fn stats(v: &[f32]) -> (f32, f32, f32) { let n = v.len().max(1) as f32; (v.iter().cloned().fold(f32::MAX, f32::min), v.iter().sum::<f32>() / n, v.iter().cloned().fold(f32::MIN, f32::max)) }

/// `--sim-tick-test`: numbers for the README (headless; same physics as the game).
pub fn run() -> (String, usize) {
    let mut s = String::new(); let mut failed = 0usize;
    let _ = writeln!(s, "--sim-tick-test: fixed {SIM_HZ} Hz gameplay tick with render interpolation (this run: SIM_HZ {}; FNRT_SIM_HZ=0 = per-frame)\n", sim_hz());
    // 1. jump apex / airtime against the continuous (240 fps per-frame) result
    let g = movement::gravity(); let v0 = 6.0f32 * movement::params::jump_frame_scale(1. / 30.); /* the 30 Hz pawn tick is above the 0.03 s DoJump threshold (exe 0x6d4f20) */ let (ideal_apex, ideal_air) = (v0 * v0 / (2. * g), 2. * v0 / g);
    let jump_only = |t: u64| FrameIn { jump: (30..33).contains(&t), ..Default::default() };
    let (rows0, _) = run_scripted(0, 240., 3., &|t| FrameIn { jump: (240..264).contains(&t), ..Default::default() });
    let cont_apex = rows0.iter().map(|r| r.height).fold(0., f32::max);
    let (rows, _) = run_scripted(30, 144., 3., &jump_only);
    let apex = rows.iter().map(|r| r.height).fold(0., f32::max);
    let first_air = rows.iter().position(|r| r.air).unwrap(); let land = rows.iter().skip(first_air + 1).position(|r| !r.air).map(|i| i + first_air + 1).unwrap();
    let air_time = (land - first_air) as f32 / 30.;
    let air0 = { let a = rows0.iter().position(|r| r.air).unwrap(); let l = rows0.iter().skip(a + 1).position(|r| !r.air).unwrap() + a + 1; (l - a) as f32 / 240. };
    let ok_apex = (apex - ideal_apex).abs() < 0.03; let ok_air = (air_time - ideal_air).abs() <= 1. / 30. + 0.012; // vs the decoded closed form incl. the capsule hover (~2 cm / ~9 ms) and tick quantisation; the 240 fps per-frame run has no frame-delta scale and is shown for reference only
    let _ = writeln!(s, "Jump (JumpZ 600, g {g:.2} m/s^2): ideal apex {ideal_apex:.4} m airtime {ideal_air:.4} s\n  continuous (per-frame, 240 fps): apex {cont_apex:.4} m, airtime {air0:.4} s\n  30 Hz tick                      : apex {apex:.4} m (diff {:.2} cm), airtime {air_time:.4} s (diff {:.1} ms, tick {:.1} ms)\n  [{}] apex within 3 cm of the decoded closed form, [{}] airtime within one tick (+12 ms hover)", (apex - cont_apex) * 100., (air_time - air0) * 1000., 1000. / 30., if ok_apex { "PASS" } else { "FAIL" }, if ok_air { "PASS" } else { "FAIL" });
    failed += (!ok_apex) as usize + (!ok_air) as usize;
    // 2. determinism across display rates (script by tick index)
    let script = |t: u64| FrameIn { forward: t >= 10, run: t >= 20 && t < 120, jump: (60..63).contains(&t) || (100..103).contains(&t), crouch: (150..180).contains(&t), yaw: (t as f32 * 1.5) % 360., pitch: -12., ..Default::default() };
    let reference = run_scripted(30, 60., 8., &script).0;
    let _ = writeln!(s, "\nDeterminism: the same per-tick script at 30/60/144/240 fps display rate ({} ticks, compared field by field, bit exact):", reference.len());
    for hz in [30., 60., 144., 240.] { let rows = run_scripted(30, hz, 8., &script).0; let same = rows == reference; failed += (!same) as usize; let _ = writeln!(s, "  {hz:>5} fps: {} ticks, identical to the 60 fps run: {}  [{}]", rows.len(), same, if same { "PASS" } else { "FAIL" }); }
    // 3. input latency
    let _ = writeln!(s, "\nInput latency (jump key polled at a frame start -> effect), swept over 48 tick/frame phases; the key itself is pressed up to one frame earlier:");
    for hz in [60., 144., 240.] {
        let l = latency_sweep(30, hz, 48);
        let (a0, a1, a2) = (stats(&l.iter().map(|x| x.0).collect::<Vec<_>>()), stats(&l.iter().map(|x| x.1).collect::<Vec<_>>()), stats(&l.iter().map(|x| x.2).collect::<Vec<_>>()));
        let _ = writeln!(s, "  {hz:>3} fps: sim state airborne min/mean/max {:.1}/{:.1}/{:.1} ms | first drawn height > 0: {:.1}/{:.1}/{:.1} ms | drawn height >= 1 cm: {:.1}/{:.1}/{:.1} ms", a0.0, a0.1, a0.2, a1.0, a1.1, a1.2, a2.0, a2.1, a2.2);
        let ok = a0.2 <= 1000. / 30. + 1000. / hz + 1.; failed += (!ok) as usize;
    }
    let l0 = latency_sweep(0, 144., 4); let _ = writeln!(s, "  per-frame mode (SIM_HZ=0) at 144 fps for comparison: airborne after {:.1} ms (the same frame)", l0[0].0);
    // 4. smoothness at 144 Hz: per-frame drawn position deltas while sprinting, interpolated vs the raw tick state
    let (_, frames) = run_live(30, 144., 4., 0., &|_| FrameIn { forward: true, run: true, ..Default::default() });
    let deltas: Vec<f32> = frames.windows(2).filter(|w| w[0].0 > 2.0).map(|w| (w[1].1.pos - w[0].1.pos).length() * 100.).collect();
    let (dmin, dmean, dmax) = stats(&deltas); let zero = deltas.iter().filter(|d| **d < 1e-3).count();
    let mut raw = FixedSim::new(30); let mut raw_pos = vec![]; for k in 0..(4. * 144.) as usize { raw.frame(1. / 144., FrameIn { forward: true, run: true, ..Default::default() }, None); if k as f32 / 144. > 2.0 { raw_pos.push(raw.rows.last().unwrap().pos); } }
    let rd: Vec<f32> = raw_pos.windows(2).map(|w| (w[1] - w[0]).length() * 100.).collect(); let (rmin, rmean, rmax) = stats(&rd); let rzero = rd.iter().filter(|d| **d < 1e-3).count();
    let ok = dmax - dmin < 0.25 * dmean && zero == 0; failed += (!ok) as usize;
    let _ = writeln!(s, "\nSmoothness at 144 fps, sprint at steady speed ({:.2} m/s = {:.3} cm per frame ideal):\n  interpolated draw : per-frame delta min {dmin:.3} mean {dmean:.3} max {dmax:.3} cm, zero-delta frames {zero}/{}  [{}]\n  raw tick state    : per-frame delta min {rmin:.3} mean {rmean:.3} max {rmax:.3} cm, zero-delta frames {rzero}/{} (what the screen would show without interpolation)", dmean * 144. / 100., dmean, deltas.len(), if ok { "PASS" } else { "FAIL" }, rd.len());
    let (_, fr) = run_live(30, 144., 4., 0., &|_| FrameIn { forward: true, run: true, ..Default::default() });
    let ed: Vec<f32> = fr.windows(2).filter(|w| w[0].0 > 2.0).map(|w| (w[1].1.eye - w[0].1.eye).length() * 100.).collect(); let (emin, emean, emax) = stats(&ed);
    let _ = writeln!(s, "  camera eye (interpolated smoothing state): per-frame delta min {emin:.3} mean {emean:.3} max {emax:.3} cm");
    let _ = writeln!(s, "\n{} failed check(s).", failed);
    (s, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn feed(c: &mut TickClock, dts: &[f32]) -> (u32, f32) { let mut n = 0; let mut a = 0.; for d in dts { let r = c.advance(*d); n += r.ticks; a = r.alpha; } (n, a) }

    // ---- env / constants ----
    #[test] fn default_is_30_hz_and_zero_means_per_frame() {
        assert_eq!(SIM_HZ, 30);
        assert_eq!(parse_sim_hz(None), 30); assert_eq!(parse_sim_hz(Some("")), 30); assert_eq!(parse_sim_hz(Some("junk")), 30); assert_eq!(parse_sim_hz(Some("-3")), 30);
        assert_eq!(parse_sim_hz(Some("0")), 0); assert_eq!(parse_sim_hz(Some(" 60 ")), 60); assert_eq!(parse_sim_hz(Some("100000")), 1000);
        assert_eq!(MAX_TICKS_PER_FRAME, 5);
    }

    // ---- accumulator ----
    #[test] fn exact_frames_give_exact_ticks() {
        let mut c = TickClock::new(30); let r = c.advance(1. / 30.); assert_eq!(r.ticks, 1); assert!(r.alpha < 1e-3);
        for hz in [30., 60., 120., 144., 240.] { let mut c = TickClock::new(30); let n = feed(&mut c, &vec![1. / hz; hz as usize]).0; assert_eq!(n, 30, "{hz} fps: one second of frames = 30 ticks"); }
    }
    #[test] fn a_second_of_ticks_at_every_rate_over_a_minute() {
        for hz in [30., 59.94, 60., 75., 100., 144., 165., 240., 360.] { let mut c = TickClock::new(30); let frames = (hz * 60.) as usize; let n = feed(&mut c, &vec![1. / hz; frames]).0; let expect = (frames as f64 / hz as f64 * 30.).floor() as u32; assert!((n as i64 - expect as i64).abs() <= 1, "{hz} fps: {n} ticks vs {expect}"); }
    }
    #[test] fn fractional_dt_accumulates_without_drift() {
        let mut c = TickClock::new(30);
        // 7 ms frames: 30 ticks need 1000 ms -> after 143 frames (1.001 s) exactly 30 ticks, remainder 1 ms
        let (n, _) = feed(&mut c, &vec![0.007; 143]); assert_eq!(n, 30); assert!((c.remainder() - 0.001).abs() < 1e-6, "{}", c.remainder());
    }
    #[test] fn frames_faster_than_the_tick_run_zero_ticks_and_alpha_grows() {
        let mut c = TickClock::new(30); let mut last = -1.; let mut zero = 0;
        for _ in 0..4 { let r = c.advance(1. / 144.); if r.ticks == 0 { zero += 1; assert!(r.alpha > last, "alpha grows between ticks"); last = r.alpha; } }
        assert_eq!(zero, 4, "4 frames of 6.9 ms are 27.8 ms < 33.3 ms: no tick yet"); assert!((last - 4. / 144. * 30.).abs() < 1e-3);
        let r = c.advance(1. / 144.); assert_eq!(r.ticks, 1); assert!(r.alpha < 0.25);
    }
    #[test] fn alpha_is_the_fraction_between_ticks() {
        let mut c = TickClock::new(30); let r = c.advance(1. / 60.); assert_eq!((r.ticks, (r.alpha * 100.).round() as i32), (0, 50));
        let r = c.advance(1. / 120.); assert_eq!((r.ticks, (r.alpha * 100.).round() as i32), (0, 75));
        let r = c.advance(1. / 120.); assert_eq!(r.ticks, 1); assert!(r.alpha < 1e-3);
        for _ in 0..1000 { let r = c.advance(0.0123); assert!((0. ..=1.).contains(&r.alpha)); }
    }
    #[test] fn a_hitch_catches_up_then_the_cap_drops_the_rest() {
        let mut c = TickClock::new(30);
        let r = c.advance(0.1); assert_eq!((r.ticks, r.dropped), (3, 0.), "100 ms = 3 ticks, all simulated");
        let mut c = TickClock::new(30); let r = c.advance(0.25); assert_eq!(r.ticks, 5, "spiral-of-death clamp: at most 5 ticks per frame"); assert!((r.dropped - 2. / 30.).abs() < 1e-6, "0.25 s = 7.5 ticks: 5 run, 2 dropped, half a tick carried: {r:?}");
        assert!((r.alpha - 0.5).abs() < 1e-3, "the fraction survives the clamp"); assert!((c.dropped_total - 2. / 30.).abs() < 1e-6);
        let mut c = TickClock::new(30); let r = c.advance(10.); assert_eq!(r.ticks, 5); assert!(r.alpha >= 0. && r.alpha <= 1.); assert!(c.remainder() < 1. / 30. + 1e-9, "backlog is bounded after a 10 s stall");
        let r = c.advance(1. / 30.); assert_eq!(r.ticks, 1, "the very next normal frame is a normal frame");
    }
    #[test] fn exactly_five_ticks_is_not_a_clamp() { let mut c = TickClock::new(30); let r = c.advance(5. / 30.); assert_eq!((r.ticks, r.dropped), (5, 0.)); }
    #[test] fn zero_and_negative_dt_run_no_ticks() { let mut c = TickClock::new(30); for d in [0., -1., f32::NAN.max(0.)] { let r = c.advance(d); assert_eq!(r.ticks, 0); } }
    #[test] fn per_frame_mode_is_one_tick_of_the_real_frame_time() {
        let mut c = TickClock::new(0); for d in [0.004f32, 0.0167, 0.1] { let r = c.advance(d); assert_eq!((r.ticks, r.step, r.alpha, r.dropped), (1, d, 1., 0.)); }
    }
    #[test] fn other_rates_work() { let mut c = TickClock::new(60); assert_eq!(feed(&mut c, &vec![1. / 144.; 144]).0, 60); let mut c = TickClock::new(20); let r = c.advance(0.05); assert_eq!(r.ticks, 1); }

    // ---- latch ----
    #[test] fn a_tap_between_two_ticks_is_not_lost() {
        let mut l = Latched::default(); l.sample(true); l.sample(false); // pressed and released within one tick interval
        assert!(l.take(), "the press survives"); assert!(!l.take(), "and is delivered once");
        l.sample(true); assert!(l.take()); l.sample(true); assert!(l.take(), "a held key stays down"); l.sample(false); assert!(!l.take());
    }

    // ---- interpolation helpers ----
    #[test] fn angle_lerp_takes_the_short_arc() {
        assert!((lerp_angle(350., 10., 0.5) - 0.).abs() < 1e-3 || (lerp_angle(350., 10., 0.5) - 360.).abs() < 1e-3);
        assert!((lerp_angle(10., 350., 0.25) - 5.).abs() < 1e-3); assert!((lerp_angle(0., 90., 0.) - 0.).abs() < 1e-6); assert!((lerp_angle(0., 90., 1.) - 90.).abs() < 1e-4);
        for i in 0..=20 { let a = lerp_angle(170., 190., i as f32 / 20.); assert!(a >= 170. - 1e-3 && a <= 190. + 1e-3, "never the long way: {a}"); }
    }
    #[test] fn pose_blend_endpoints_and_midpoint() {
        let a = vec![(Vec3::ZERO, Quat::IDENTITY), (Vec3::X, Quat::IDENTITY)]; let b = vec![(Vec3::new(2., 0., 0.), Quat::from_rotation_y(1.)), (Vec3::X, Quat::IDENTITY)];
        assert_eq!(blend_pose(&a, &b, 1.), b); let m = blend_pose(&a, &b, 0.5); assert!((m[0].0 - Vec3::X).length() < 1e-6); assert!(m[0].1.angle_between(Quat::from_rotation_y(0.5)) < 1e-5 && (m[0].1.length() - 1.).abs() < 1e-5);
        assert!((blend_pose(&a, &b, 0.)[0].0 - a[0].0).length() < 1e-6); assert_eq!(blend_pose(&a, &b[..1], 0.5), b[..1].to_vec(), "mismatched skeletons fall back to the newest pose");
    }
    #[test] fn snapshot_blend_interpolates_and_snaps_on_teleport() {
        let a = Snapshot { pos: Vec2::ZERO, height: 0., facing: 350., ..Default::default() }; let b = Snapshot { pos: Vec2::new(0.2, 0.), height: 0.4, facing: 10., ..Default::default() };
        let m = Snapshot::blend(&a, &b, 0.5); assert!((m.pos.x - 0.1).abs() < 1e-6 && (m.height - 0.2).abs() < 1e-6 && (m.facing < 1e-3 || m.facing > 359.999));
        let far = Snapshot { pos: Vec2::new(100., 0.), ..Default::default() }; assert_eq!(Snapshot::blend(&a, &far, 0.3).pos, far.pos, "teleport shows the new position at once");
        let mut p = Pair::new(a.clone()); p.push(b.clone()); assert_eq!(p.at(0.).pos, a.pos); assert_eq!(p.at(1.).pos, b.pos); p.snap(); assert_eq!(p.at(0.).pos, b.pos);
    }

    // ---- the headless twin ----
    fn jump_script(t: u64) -> FrameIn { FrameIn { jump: (30..33).contains(&t), ..Default::default() } }
    #[test] fn jump_apex_and_airtime_at_30_hz_equal_the_continuous_result_within_one_tick() {
        let g = movement::gravity();
        let (cont, _) = run_scripted(0, 240., 3., &|t| FrameIn { jump: (240..264).contains(&t), ..Default::default() });
        let apex_c = cont.iter().map(|r| r.height).fold(0., f32::max); let a = cont.iter().position(|r| r.air).unwrap(); let l = cont.iter().skip(a + 1).position(|r| !r.air).unwrap() + a + 1; let air_c = (l - a) as f32 / 240.;
        for hz in [30., 60., 144., 240.] {
            let (rows, _) = run_scripted(30, hz, 3., &jump_script);
            let apex = rows.iter().map(|r| r.height).fold(0., f32::max);
            // The 30 Hz tick is the pawn tick: its 0.0333 s delta is above the 0.03 s threshold of UFortMovementComp_Character::DoJump (exe 0x6d4f20), so JumpZ is scaled by
            // jump_frame_scale(1/30) = 1.0589 whatever the display rate is (the 240 fps per-frame run `apex_c` has no scale: 60 Hz and faster do not trigger it).
            let v0 = 6. * movement::params::jump_frame_scale(1. / 30.);
            assert!(apex > apex_c + 0.02, "{hz} fps display: the 30 Hz tick jump {apex} must be higher than the unscaled continuous one {apex_c}");
            let a = rows.iter().position(|r| r.air).unwrap(); let l = rows.iter().skip(a + 1).position(|r| !r.air).unwrap() + a + 1; let air = (l - a) as f32 / 30.;
            assert!(air + 1e-3 >= air_c, "airtime {air} vs {air_c}");
            assert!((apex - v0 * v0 / (2. * g)).abs() < 0.03, "(the capsule's floor hover adds ~2 cm) apex {apex} vs decoded {}", v0 * v0 / (2. * g));
            assert!((air - 2. * v0 / g).abs() <= 1. / 30. + 0.012, "airtime {air} vs decoded {} (capsule hover adds ~9 ms, the touchdown is quantised to a tick)", 2. * v0 / g);
        }
    }
    #[test] fn tick_state_is_bit_identical_at_every_display_rate() {
        let script = |t: u64| FrameIn { forward: t >= 10, run: (20..120).contains(&t), jump: (60..63).contains(&t) || (100..103).contains(&t), crouch: (150..180).contains(&t), yaw: (t as f32 * 1.5) % 360., pitch: -12., ..Default::default() };
        let reference = run_scripted(30, 60., 8., &script).0; assert_eq!(reference.len(), 240);
        for hz in [30., 144., 240., 100., 59.94] { let rows = run_scripted(30, hz, 8., &script).0; assert!(rows.len() >= 239 && rows.len() <= 241, "{hz}: {}", rows.len()); assert_eq!(&rows[..239], &reference[..239], "{hz} fps display rate changed the simulation"); }
        // sanity: the scenario exercised jumping, running, crouching
        assert!(reference.iter().any(|r| r.air), "no jump"); assert!(reference.iter().any(|r| r.crouched), "no crouch"); assert!(reference.last().unwrap().pos.length() > 3., "end pos {:?}", reference.last().unwrap().pos);
    }
    #[test] fn wall_time_input_gives_the_same_jump_within_one_tick() {
        // The key is down for 100 ms starting at wall time 1.0 s, sampled by the frame: 30/60/144/240 fps must all jump once, at the same tick +-1.
        let mut first = vec![];
        for hz in [30., 60., 144., 240.] { let (s, _) = run_live(30, hz, 3., 0., &|t| FrameIn { jump: t >= 1.0 - 1e-4 && t < 1.1, ..Default::default() }); assert_eq!(s.c.jumps_started(), 1, "{hz} fps"); first.push(s.rows.iter().position(|r| r.air).unwrap() as i64); }
        assert!(first.iter().all(|f| (f - first[0]).abs() <= 1), "liftoff tick per display rate: {first:?}");
    }
    #[test] fn a_one_frame_tap_at_240_fps_still_jumps() {
        // a 4 ms press (one frame) falls between two 33 ms ticks in 31 of 32 frames: only the latch delivers it
        for press_frame in 238..250usize { let mut s = FixedSim::new(30); for k in 0..720usize { s.frame(1. / 240., FrameIn { jump: k == press_frame, ..Default::default() }, None); } assert_eq!(s.c.jumps_started(), 1, "tap at frame {press_frame}"); }
    }
    #[test] fn air_presses_and_key_mashing_at_30_hz_never_rejump_in_the_air() {
        let edges = |rows: &[TickRow]| rows.windows(2).filter(|w| !w[0].air && w[1].air).count() as u32 + rows.first().map_or(0, |r| r.air as u32);
        for hz in [60., 144., 240.] {
            let (rows, _) = run_scripted(30, hz, 6., &|t| FrameIn { jump: t == 10 || [12, 15, 18, 20].contains(&t), ..Default::default() });
            assert_eq!(rows.last().unwrap().jumps, 1, "{hz} fps: presses during the flight are ignored (JumpMaxCount 1, no buffering)"); assert_eq!(edges(&rows), 1);
            let (rows, _) = run_scripted(30, hz, 8., &|t| FrameIn { jump: t >= 10 && t % 2 == 0, ..Default::default() });
            let last = rows.last().unwrap(); assert!(last.jumps >= 2 && last.jumps == edges(&rows), "{hz} fps: only ground jumps count: {} jumps, {} takeoffs", last.jumps, edges(&rows));
            for w in rows.windows(2) { if w[0].air && w[1].air && w[0].height > 0. { assert!(w[1].vz <= w[0].vz + 1e-4, "vertical speed never increases in the air"); } }
        }
    }
    #[test] fn input_latency_is_at_most_one_tick_plus_a_frame() {
        for hz in [60., 144., 240.] { let l = latency_sweep(30, hz, 32); let worst = l.iter().map(|x| x.0).fold(0., f32::max); assert!(worst <= 1000. / 30. + 1000. / hz + 1., "{hz} fps: worst key->airborne {worst} ms");
            assert!(l.iter().all(|x| x.1.is_finite() && x.1 <= 1000. / 30. + 2000. / hz + 1.), "{hz}: first drawn motion {l:?}");
            assert!(l.iter().any(|x| x.0 > 10.), "the latency sweep must see the tick delay: {l:?}"); }
        let l0 = latency_sweep(0, 144., 4); assert!(l0.iter().all(|x| x.0 <= 1000. / 144. + 0.01), "per-frame mode: same frame {l0:?}");
    }
    #[test] fn drawn_motion_is_smooth_at_144_fps_but_the_raw_tick_state_is_not() {
        let (_, frames) = run_live(30, 144., 4., 0., &|_| FrameIn { forward: true, run: true, ..Default::default() });
        let d: Vec<f32> = frames.windows(2).filter(|w| w[0].0 > 2.0).map(|w| (w[1].1.pos - w[0].1.pos).length()).collect(); let mean = d.iter().sum::<f32>() / d.len() as f32;
        assert!(mean * 144. > 5.4 && mean * 144. < 5.6, "sprint speed 5.5 m/s: {}", mean * 144.);
        assert!(d.iter().all(|x| (x - mean).abs() < 0.12 * mean), "interpolated frame deltas stay within 12% of the mean");
        let mut raw = FixedSim::new(30); let mut prev = None; let mut zero = 0; let mut n = 0;
        for k in 0..576 { raw.frame(1. / 144., FrameIn { forward: true, run: true, ..Default::default() }, None); if k > 288 { let p = raw.rows.last().unwrap().pos; if let Some(q) = prev { n += 1; let d: Vec2 = p - q; if d.length() < 1e-6 { zero += 1; } } prev = Some(p); } }
        assert!(zero * 10 > n * 6, "the un-interpolated state repeats the same position on {zero}/{n} frames (that is the stutter the interpolation removes)");
    }
    #[test] fn camera_eye_is_smooth_too_and_follows_current_yaw_every_frame() {
        let (_, fr) = run_live(30, 144., 3., 0., &|t| FrameIn { forward: true, run: true, yaw: t * 90., pitch: -10., ..Default::default() });
        let e: Vec<f32> = fr.windows(2).filter(|w| w[0].0 > 1.5).map(|w| (w[1].1.eye - w[0].1.eye).length()).collect();
        let mean = e.iter().sum::<f32>() / e.len() as f32; assert!(e.iter().all(|x| *x < 3. * mean + 0.01), "no eye pops: max {} mean {mean}", e.iter().cloned().fold(0., f32::max));
    }
    #[test] fn per_frame_mode_matches_the_old_per_frame_controller() {
        // SIM_HZ=0 must equal stepping Controller directly with the frame dt (the pre-change behaviour).
        let mut s = FixedSim::new(0); let mut c = Controller::default();
        for k in 0..600usize { let jump = (100..110).contains(&k); let f = FrameIn { forward: k > 20, run: true, jump, ..Default::default() }; s.frame(1. / 144., f, None); c.step(if f.forward { Vec2::new(0., -1.) } else { Vec2::ZERO }, jump, true, 1. / 144.); }
        assert_eq!(s.c.position, c.position); assert_eq!(s.c.height, c.height); assert_eq!(s.c.vertical_speed, c.vertical_speed);
    }
}
