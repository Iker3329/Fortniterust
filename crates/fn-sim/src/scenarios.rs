//! The scripted scenarios of the old prototype's test modes, run headlessly against `fn_game::GameSim`:
//!
//! | old flag               | here                         | output (same file names / line formats as the old exe where it wrote them)           |
//! |------------------------|------------------------------|---------------------------------------------------------------------------------------|
//! | `--movement-test`      | [`series::run_movement_test`]| `movement-test.txt`                                                                   |
//! | `--jump-air-test`      | [`series::run_jump_air_test`]| `jump-air-test.txt`                                                                   |
//! | `--camera-test`        | [`camera_test`]              | `camera-test.txt`                                                                     |
//! | `--camera-prop-test`   | [`camera_prop_test`]         | `camprop-before.txt` / `camprop-after.txt`                                            |
//! | `--pickaxe-test`       | [`pick_run`] (`PickMode::Pickaxe`) | `pickaxe-frames.txt`, `pickaxe-test.txt` (capture lines without the mesh-based grip numbers) |
//! | `--anim-test`          | [`pick_run`] (`PickMode::Anim`)    | same                                                                              |
//! | `--dir-test`           | [`pick_run`] (`PickMode::Dir`)     | same + `dir-frames.txt`                                                           |
//! | `--camera-video`       | [`camera_video`]             | `camera-video-v2.txt` (without the `size WxH` suffix; nothing is rendered)             |
//! | `--jump-feel-test`     | [`jump_feel_test`]           | `jump-feel-test.txt` (headless in the old exe too)                                     |
//! | `--face-test [--legacy]` | [`crate::face_test::face_test`] | `face-test[-legacy].txt` (the numbers; the old run's close-up PNGs need a renderer)  |
//! | `--time-test`          | [`crate::time_test::time_test`] | `time-audit-*.txt` on a VIRTUAL clock (the old run measured real wall time: no golden) |
//!
//! Every scenario also writes `<name>-frames.txt`: one line per frame with the pawn / animation / camera state and (when the clip assets
//! are available) a bit-exact hash of the evaluated pose, so two builds can be compared frame by frame (`fn-sim compare`).
use crate::pick_script as pick;
use crate::rig::{f32_hash, pose_hash, Rig};
use crate::video_script::{self, Step as VStep};
use fn_game::{animation_stage, horizontal_step, spawn, FrameInput, GameSim, Poses, TickDriver, EYE};
use fn_game::combat::{Harvestable, Kind};
use fn_game::movement::{smooth_toward, Controller};
use glam::{Vec2, Vec3};
use std::fmt::Write as _;
use std::path::Path;

pub const DT: f32 = 1. / 60.;

/// `--jump-feel-test` (old jump_feel.rs, ported into `fn_game::jump_feel`): latency / timeline / frame-rate-independence / gravity table log
/// and its failed-check count (always 0, the numeric assertions are unit tests).
pub fn jump_feel_test() -> (String, usize) { fn_game::jump_feel::run() }

/// Result of a scenario: the files it produced (name, content).
#[derive(Default)]
pub struct Outputs { pub files: Vec<(String, String)> }
impl Outputs {
    pub fn add(&mut self, name: &str, text: String) { self.files.push((name.to_owned(), text)); }
    pub fn write_all(&self, dir: &Path) -> std::io::Result<()> { std::fs::create_dir_all(dir)?; for (n, t) in &self.files { std::fs::write(dir.join(n), t)?; } Ok(()) }
}

/// One line of the generic per-frame log.
pub fn frame_line(f: u32, g: &GameSim, pose_hash: Option<u64>) -> String {
    let c = &g.controller; let v = &g.cam_view;
    let state = f32_hash(&[c.position.x, c.position.y, c.height, c.vertical_speed, c.velocity.x, c.velocity.y, g.yaw, g.pitch, v.eye.x, v.eye.y, v.eye.z]);
    format!("f{f:05} pos {:.5} {:.5} {:.5} vel {:.5} {:.5} vz {:.5} crouched {} air {} yaw {:.3} pitch {:.3} facing {:.3} | {} | eye {:.5} {:.5} {:.5} boom {:.4} hide {} | swing {} {:.4} hits {} wood {} | state {:016x} pose {}",
        g.x, g.y, g.z, c.velocity.x, c.velocity.y, c.vertical_speed, c.crouched as u8, c.airborne() as u8, g.yaw, g.pitch, c.facing, g.anim.describe(),
        v.eye.x, v.eye.y, v.eye.z, v.blocked_pct, v.hide_pawn as u8, g.swing.stage, g.swing.time, g.world.hits, g.world.wood, state,
        pose_hash.map_or("-".to_owned(), |h| format!("{h:016x}"))) + &terrain_suffix(g)
}
/// TERRAIN: with a landscape, the frame line ends with ` | ground <y under the feet> feet-ground <cm>` (absent on the flat plate, so flat logs are unchanged).
fn terrain_suffix(g: &GameSim) -> String {
    if g.terrain().is_none() { return String::new(); }
    let f = g.feet(); let gy = g.ground_y(f.x, f.z);
    format!(" | ground {gy:.5} feet-ground {:.2}cm", (f.y - gy) * 100.)
}

// ------------------------------------------------------------------------------------------------------------------------------
// --pickaxe-test / --anim-test / --dir-test (App::update driven by pick_test::step, fixed 60 Hz)
// ------------------------------------------------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickMode { Pickaxe, Anim, Dir }
impl PickMode { pub fn name(self) -> &'static str { match self { PickMode::Pickaxe => "pickaxe-test", PickMode::Anim => "anim-test", PickMode::Dir => "dir-test" } } }

/// Runs the scripted series in PER-FRAME mode (the old `FNRT_SIM_HZ=0`). `rig` (clips + skeleton) enables the bone columns; without it only the
/// movement/animation-state logs are written.
pub fn pick_run(mode: PickMode, rig: Option<&Rig>, max_frames: Option<u32>) -> Outputs { pick_run_hz(mode, rig, max_frames, 0) }

/// Evaluates the pose tree of `g` for the rig (one model: the soldier body), as the renderer side does after every tick.
pub fn rig_poses(rig: Option<&Rig>, g: &GameSim) -> Poses { match rig { Some(r) => vec![r.eval(&g.pose_tree()).pose], None => vec![] } }

/// The pose a frame draws: the tick-blended pose when the driver has a snapshot, else (render-only path) the current graph state.
fn drawn_pose(rig: &Rig, d: &TickDriver) -> crate::rig::Posed {
    match d.drawn().and_then(|s| s.poses.first()) { Some(p) => rig.posed(p.clone()), None => rig.eval(&d.sim.pose_tree()) }
}

/// `pick_run` at a simulation rate: `hz` ticks per second (`0` = per frame, `30` = the old exe's default fixed tick) with the frame loop of the old
/// `App::update` (`fn_game::TickDriver`): the logged pose is the interpolation of the last two tick poses, exactly what the old renderer skinned.
pub fn pick_run_hz(mode: PickMode, rig: Option<&Rig>, max_frames: Option<u32>, hz: u32) -> Outputs {
    pick::set_active(true); pick::set_anim_series(matches!(mode, PickMode::Anim | PickMode::Dir)); pick::set_dir_series(mode == PickMode::Dir);
    let mut d = TickDriver::new(GameSim::new(), hz); let mut out = Outputs::default();
    let (mut frames_log, mut pick_frames, mut dir_frames, mut caps) = (String::new(), String::new(), String::new(), String::new());
    let end = max_frames.unwrap_or_else(pick::end_frame).min(pick::end_frame());
    for f in 0..end {
        let ps = pick::step(f);
        if f == pick::clear_frame() { d.sim.world.props.clear(); } // empty baseplate for the jump/sprint/crouch series
        if f == pick::target_frame() { // back to the spawn facing -Z, a tree 1.8 m ahead so the scripted swings are deterministic
            d.sim.controller = Controller::default(); d.sim.to_spawn(); d.snap(); // scripted reset: no interpolation across the teleport
            d.sim.world.props.push(Harvestable::new(Kind::Tree, Vec3::new(0., 0., 12. - 1.8)));
        }
        let inp = FrameInput { forward: ps.forward, side: ps.side, script_jump: ps.jump, force_crouch: ps.crouch, force_run: ps.run, attack: ps.attack,
            set_yaw: ps.yaw.map(|y| (spawn().3 + y).rem_euclid(360.)), set_pitch: ps.pitch, equip: pick::equip(f), ..Default::default() };
        d.frame(&inp, DT, &mut |g| rig_poses(rig, g));
        let g = &d.sim;
        let posed = rig.map(|r| drawn_pose(r, &d));
        let h = posed.as_ref().map(|p| pose_hash(&p.pose));
        let _ = writeln!(frames_log, "{}", frame_line(f, &g, h));
        if let (Some(r), Some(p)) = (rig, &posed) {
            if let (Some(wr), Some(hl), Some(hr), Some(pv)) = (r.bone(p, "weapon_r"), r.bone(p, "hand_l"), r.bone(p, "hand_r"), r.bone(p, "pelvis")) {
                let _ = writeln!(pick_frames, "{} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}", f, wr.x, wr.y, wr.z, hl.x, hl.y, hl.z, hr.x, hr.y, hr.z, pv.x, pv.y, pv.z);
                if pick::dir_series() {
                    if let (Some(fl), Some(fr)) = (r.bone(p, "foot_l"), r.bone(p, "foot_r")) {
                        let _ = writeln!(dir_frames, "f {} yaw {:.1} vel {:.0} | foot_l {:.3} {:.3} {:.3} foot_r {:.3} {:.3} {:.3} pelvis {:.3} {:.3} {:.3} | {}", f, g.yaw, g.controller.velocity.length() * 100., fl.x, fl.y, fl.z, fr.x, fr.y, fr.z, pv.x, pv.y, pv.z, g.anim.locomotion_debug());
                    }
                }
            }
        }
        // capture lines (the old render() incremented `frames` before consulting the script's capture table)
        let name = pick::capture(g.frames).map(|n| n.to_owned());
        if let Some(name) = name {
            let props: Vec<String> = g.world.props.iter().map(|p| format!("{:?}@{:.0}hp", p.kind, p.health.max(0.))).collect();
            let _ = writeln!(caps, "{name} 1280x720 frame {} swing_stage {} t {:.3} hits {} misses {} wood {} stone {} props [{}] grip_r NA height {:.3} speed {:.2} crouched {} anim [{}]", g.frames, g.swing.stage, g.swing.time, g.world.hits, g.world.misses, g.world.wood, g.world.stone, props.join(","), g.controller.height, g.controller.velocity.length(), g.controller.crouched, g.anim.describe());
        }
    }
    let n = mode.name();
    out.add(&format!("{n}-frames.txt"), frames_log); out.add("pickaxe-test.txt", caps);
    if rig.is_some() { out.add("pickaxe-frames.txt", pick_frames); if mode == PickMode::Dir { out.add("dir-frames.txt", dir_frames); } }
    pick::set_active(false); pick::set_anim_series(false); pick::set_dir_series(false);
    out
}

// ------------------------------------------------------------------------------------------------------------------------------
// --camera-test (App::update with camera_script)
// ------------------------------------------------------------------------------------------------------------------------------
/// Scripted camera-test input for frame `f`: (yaw override, forward input).
pub fn camera_script(f: u32) -> (f32, f32) {
    let yaw = if f < 30 { 0. } else if f < 160 { 150. } else { (340. + (f - 160) as f32).rem_euclid(360.) };
    (yaw, if (100..260).contains(&f) { 1. } else { 0. })
}
pub fn camera_test(rig: Option<&Rig>) -> Outputs { camera_test_hz(rig, 0) }
/// `--camera-test` at a simulation rate (`FNRT_SIM_HZ`): `0` per frame, `30` the old exe's default.
pub fn camera_test_hz(rig: Option<&Rig>, hz: u32) -> Outputs {
    let mut d = TickDriver::new(GameSim::new(), hz); let mut out = Outputs::default(); let (mut log, mut frames_log) = (String::new(), String::new());
    for f in 0..260u32 { // the old run closed the window when the frame counter reached 260
        let (y, fwd) = camera_script(f);
        d.frame(&FrameInput { forward: fwd, set_yaw: Some(y), ..Default::default() }, DT, &mut |g| rig_poses(rig, g));
        let g = &d.sim;
        let _ = writeln!(log, "frame {} yaw {:.2} facing {:.2} pos {:.3} {:.3} speed {:.3} height {:.3}", f, g.yaw, g.controller.facing, g.x, g.z, g.controller.velocity.length(), g.y - EYE);
        let h = rig.map(|r| pose_hash(&drawn_pose(r, &d).pose));
        let _ = writeln!(frames_log, "{}", frame_line(f, g, h));
    }
    out.add("camera-test.txt", log); out.add("camera-test-frames.txt", frames_log); out
}

// ------------------------------------------------------------------------------------------------------------------------------
// --camera-video / --camera-prop-test (video_test::sim: a fixed step that drives the controller through its legacy request path)
// ------------------------------------------------------------------------------------------------------------------------------
/// Advances `g` by one fixed step exactly like the old `video_test::sim` (minus the HUD tick).
pub fn video_sim(g: &mut GameSim, s: &VStep, dt: f32) {
    g.frame_dt = dt;
    g.anim_time += dt;
    if s.equip { g.anim.equip.start(); }
    if s.plant_tree {
        g.controller = Controller::default(); g.to_spawn();
        g.world.props.push(Harvestable::new(Kind::Tree, Vec3::new(0., 0., 12. - 1.8)));
    }
    if let Some(fn_game::combat::Notify::TriggerAbility { .. }) = g.swing.step(s.attack, dt) {
        let f = g.controller.facing.to_radians();
        let _harvested = g.world.strike(Vec3::new(g.x, g.y - EYE, g.z), Vec3::new(f.sin(), 0., -f.cos())); // a damaged prop never takes the Bounce section (see GameSim::tick)
    }
    g.world.update(dt);
    g.yaw = s.yaw.rem_euclid(360.);
    g.pitch = fn_game::camera::clamp_pitch(s.pitch);
    if s.teleport { g.controller = Controller::default(); g.controller.position = Vec2::new(0., -246. - 12.); }
    let (dx, dz) = horizontal_step(s.forward, s.side, g.yaw, 1., 1.);
    g.controller.crouched = s.crouch && g.controller.grounded();
    g.refresh_blockers();
    g.controller.step(Vec2::new(dx, dz), s.jump, s.run, dt);
    g.crouch_camera = smooth_toward(g.crouch_camera, if g.controller.crouched { 0.5 } else { 0. }, 12., dt);
    let stage = animation_stage(g.controller.air_height(), g.controller.vertical_speed, g.jump_stage, g.state_time);
    if stage != g.jump_stage { g.state_time = 0.; g.jump_stage = stage; } else { g.state_time += dt; }
    let target = (g.controller.velocity.length() / 1.5).min(1.);
    g.locomotion_blend = smooth_toward(g.locomotion_blend, target, 12., dt);
    g.step_anim(Vec2::new(dx, dz), dt);
    let (lo, hi) = g.world_box(); g.controller.clamp_to_box(lo - Vec2::new(0., 12.), hi - Vec2::new(0., 12.)); // TERRAIN: the landscape bounds, else the plate
    g.controller.facing = g.anim.visual_yaw(g.yaw);
    let p = g.controller.render_position();
    g.x = p.x; g.z = 12. + p.y; g.y = EYE + g.controller.render_height();
    g.update_camera(dt);
    g.frames += 1;
}

pub fn camera_video(rig: Option<&Rig>) -> Outputs {
    let mut g = GameSim::new(); let mut out = Outputs::default();
    let mut log = String::from("frame yaw pitch pos_x pos_z speed height crouched eye_x eye_y eye_z boom swing_stage hits wood\n"); let mut frames_log = String::new();
    for f in 0..video_script::FRAMES {
        let s = video_script::step(f); video_sim(&mut g, &s, DT);
        let v = g.cam_view;
        let _ = writeln!(log, "{f} {:.2} {:.2} {:.3} {:.3} {:.3} {:.3} {} {:.3} {:.3} {:.3} {:.3} {} {} {}", g.yaw, g.pitch, g.x, g.z, g.controller.velocity.length(), g.controller.height, g.controller.crouched, v.eye.x, v.eye.y, v.eye.z, v.blocked_pct, g.swing.stage, g.world.hits, g.world.wood);
        let h = rig.map(|r| pose_hash(&r.eval(&g.pose_tree()).pose));
        let _ = writeln!(frames_log, "{}", frame_line(f, &g, h));
    }
    out.add("camera-video-v2.txt", log); out.add("camera-video-frames.txt", frames_log); out
}

/// (name, props as (kind, x, z) in world metres, jump frame, yaw)
struct PropScene { name: &'static str, props: &'static [(fn_game::combat::Kind, f32, f32)], jump_at: Option<u32>, yaw: f32 }
const PROP_SCENES: &[PropScene] = &[
    // Trunk 2.2 m behind the player (right where the 2.8 m boom wants to go).
    PropScene { name: "trunk-behind", props: &[(Kind::Tree, 0.31, 14.2)], jump_at: Some(30), yaw: 0. },
    // Tree 1.4 m behind-right: jumping puts the head and the camera boom inside the canopy.
    PropScene { name: "jump-beside-tree", props: &[(Kind::Tree, 0.8, 13.4)], jump_at: Some(30), yaw: 0. },
    // Trunk just beyond the camera, canopy over the camera's resting position.
    PropScene { name: "canopy-over-camera", props: &[(Kind::Tree, 0.0, 14.9)], jump_at: Some(30), yaw: 0. },
    // Rock behind the player.
    PropScene { name: "rock-behind", props: &[(Kind::Rock, 0.31, 14.0)], jump_at: None, yaw: 0. },
];
/// `--camera-prop-test [--no-prop-collision]`: the same script with prop collision on ("after") or off ("before").
pub fn camera_prop_test(collision: bool) -> Outputs {
    fn_game::props::set_disabled(!collision);
    let tag = if collision { "after" } else { "before" };
    let mut g = GameSim::new();
    let mut log = format!("Camera prop collision test ({tag}: prop collision {}). Units metres, fixed 60 Hz. clr_solid = eye distance to nearest trunk/rock surface, clr_all also counts canopies (negative = eye inside). The probe radius is 0.08.\n", if collision { "ON" } else { "OFF" });
    for sc in PROP_SCENES {
        g.world.props.clear();
        for &(k, x, z) in sc.props { g.world.props.push(Harvestable::new(k, Vec3::new(x, 0., z))); }
        g.controller = Controller::default();
        g.to_spawn(); g.yaw = sc.yaw; g.pitch = -12.;
        g.cam.reset(); g.anim_time = 0.; g.jump_stage = 0; g.state_time = 0.;
        let _ = writeln!(log, "== scene {} props {:?} jump_at {:?}", sc.name, sc.props, sc.jump_at);
        let (mut min_all, mut min_solid, mut max_pop, mut last_len, mut hidden) = (f32::MAX, f32::MAX, 0f32, None::<f32>, 0);
        for f in 0..100u32 {
            let s = VStep { yaw: sc.yaw, pitch: -12., jump: sc.jump_at == Some(f), ..Default::default() };
            video_sim(&mut g, &s, DT);
            let v = g.cam_view;
            let ca = fn_game::props::clearance(&g.world.props, v.eye, true);
            let cs = fn_game::props::clearance(&g.world.props, v.eye, false);
            min_all = min_all.min(ca); min_solid = min_solid.min(cs);
            let len = (v.eye - v.safe).length();
            if let Some(l) = last_len { max_pop = max_pop.max((len - l).abs()); }
            last_len = Some(len);
            if v.hide_pawn { hidden += 1; }
            let _ = writeln!(log, "{} f{f:03} feet.y {:.3} eye ({:.3},{:.3},{:.3}) boom-kept {:.3} len {:.3} hide_pawn {} clr_solid {:.3} clr_all {:.3}", sc.name, g.y - EYE, v.eye.x, v.eye.y, v.eye.z, v.blocked_pct, len, v.hide_pawn, cs, ca);
        }
        let _ = writeln!(log, "SUMMARY {} min clr_solid {:.3} min clr_all {:.3} largest one-frame boom change {:.3} m, pawn hidden {} of 100 frames\n", sc.name, min_solid, min_all, max_pop, hidden);
    }
    fn_game::props::set_disabled(false);
    let mut out = Outputs::default(); out.add(&format!("camprop-{tag}.txt"), log); out
}

// ------------------------------------------------------------------------------------------------------------------------------
// --sim-tick-test and the fixed-tick determinism proof
// ------------------------------------------------------------------------------------------------------------------------------
/// `--sim-tick-test` (old sim_tick.rs `run`, ported into `fn_game::sim_tick`): accumulator / determinism / latency / smoothness numbers and its failed-check count.
pub fn sim_tick_test() -> (String, usize) { fn_game::sim_tick::run() }

/// Input of the determinism scenario as a function of the TICK index only: run-up, sprint, two jumps (one tap shorter than a tick at 240 fps), crouch, a yaw sweep.
pub fn determinism_script(tick: u64) -> FrameInput {
    FrameInput { forward: if tick >= 10 { 1. } else { 0. }, sprint_key: (20..120).contains(&tick), jump: (60..63).contains(&tick) || tick == 100, force_crouch: (150..180).contains(&tick),
        set_yaw: Some((tick as f32 * 1.5) % 360.), set_pitch: Some(-12.), attack: (200..260).contains(&tick), ..Default::default() }
}
/// One simulated tick as a log line: every simulation field (pawn, floor height, animation graph, camera smoothing state, swing) as exact bit hash plus readable columns.
fn tick_line(n: u64, g: &GameSim) -> String {
    let c = &g.controller; let cs = g.cam.state();
    let h = f32_hash(&[c.position.x, c.position.y, c.height, c.vertical_speed, c.velocity.x, c.velocity.y, c.facing, g.yaw, g.pitch, cs.origin.x, cs.origin.y, cs.origin.z, cs.vt.x, cs.vt.y, cs.vt.z, cs.penetration, g.swing.time, g.crouch_camera, g.locomotion_blend]);
    format!("tick {n:04} pos {:.5} {:.5} h {:.5} vz {:.4} air {} crouched {} facing {:.3} swing {} {:.4} hits {} wood {} | {} | state {h:016x}", c.position.x, c.position.y, c.height, c.vertical_speed, c.airborne() as u8, c.crouched as u8, c.facing, g.swing.stage, g.swing.time, g.world.hits, g.world.wood, g.anim.describe())
}
/// Runs the determinism script for `seconds` at a DISPLAY rate of `fps` frames per second on a `sim_hz` tick (`TickDriver`, the app's loop) and returns the per-tick log.
/// Frames at 30 fps or faster run at most one tick, so every display rate feeds the same input to the same tick.
pub fn tick_log(sim_hz: u32, fps: f32, seconds: f32) -> Vec<String> {
    let mut d = TickDriver::new(GameSim::new(), sim_hz); let mut lines = vec![]; let dt = 1. / fps;
    for _ in 0..(seconds * fps).round() as usize {
        let before = d.clock.total_ticks; let inp = determinism_script(before);
        d.frame(&inp, dt, &mut |_| Vec::new());
        for n in before..d.clock.total_ticks { lines.push(tick_line(n + 1, &d.sim)); }
    }
    lines
}
/// `tick-determinism`: the same per-tick script at 30 / 60 / 144 / 240 fps display rate. Writes one log per rate plus a summary; the bool is "every rate produced the
/// same simulation" (compared over the ticks all rates completed: the last partial tick of a run may be missing at some rates).
pub fn tick_determinism(seconds: f32) -> (Outputs, bool) {
    let rates = [30., 60., 144., 240.]; let logs: Vec<Vec<String>> = rates.iter().map(|&r| tick_log(30, r, seconds)).collect();
    let common = logs.iter().map(Vec::len).min().unwrap_or(0).saturating_sub(1); // drop the in-flight last tick
    let mut ok = common as f32 >= seconds * 30. - 2.; let mut out = Outputs::default(); let mut summary = format!("fixed 30 Hz tick, {seconds} s of the same per-tick script at four display rates; {common} ticks compared bit-exactly\n");
    for (i, r) in rates.iter().enumerate() {
        let same = logs[i][..common] == logs[0][..common]; ok &= same;
        let _ = writeln!(summary, "{r:>5} fps: {} ticks, identical to the 30 fps run: {}", logs[i].len(), same);
        out.add(&format!("tick-determinism-{}fps.txt", *r as u32), logs[i].join("\n") + "\n");
    }
    let _ = writeln!(summary, "result: {}", if ok { "DETERMINISTIC" } else { "MISMATCH" });
    out.add("tick-determinism.txt", summary); (out, ok)
}

/// Serialises the unit tests that flip process-wide switches (pick script flags, prop collision).
#[cfg(test)]
pub static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    fn lock() -> std::sync::MutexGuard<'static, ()> { TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner()) }
    fn text<'a>(o: &'a Outputs, name: &str) -> &'a str { &o.files.iter().find(|f| f.0 == name).unwrap_or_else(|| panic!("{name} missing")).1 }
    #[test] fn scenarios_are_deterministic() {
        let _g = lock();
        let a = pick_run(PickMode::Anim, None, Some(700)); let b = pick_run(PickMode::Anim, None, Some(700));
        assert_eq!(text(&a, "anim-test-frames.txt"), text(&b, "anim-test-frames.txt"));
        let a = camera_test(None); let b = camera_test(None); assert_eq!(text(&a, "camera-test.txt"), text(&b, "camera-test.txt"));
    }
    #[test] fn the_fixed_tick_simulation_is_bit_identical_at_30_60_144_240_fps() {
        let (o, ok) = tick_determinism(8.); assert!(ok, "{}", text(&o, "tick-determinism.txt"));
        let log = text(&o, "tick-determinism-240fps.txt");
        assert!(log.contains(" air 1 ") && log.contains(" crouched 1 ") && log.contains(" swing "), "the script exercises jump, crouch and the swing");
        assert!(log.lines().any(|l| l.contains("swing 0 0.0000")) && log.lines().any(|l| !l.contains("swing 0 0.0000")), "the swing montage ran (it hits nothing here: the pawn is running away from the props)");
    }
    #[test] fn thirty_hz_scenarios_are_deterministic_too() {
        let _g = lock();
        let a = pick_run_hz(PickMode::Pickaxe, None, Some(900), 30); let b = pick_run_hz(PickMode::Pickaxe, None, Some(900), 30);
        assert_eq!(text(&a, "pickaxe-test-frames.txt"), text(&b, "pickaxe-test-frames.txt"));
        let a = camera_test_hz(None, 30); let b = camera_test_hz(None, 30); assert_eq!(text(&a, "camera-test.txt"), text(&b, "camera-test.txt"));
    }
    #[test] fn camera_prop_collision_changes_the_camera_only_where_props_are() {
        let _g = lock();
        let before = camera_prop_test(false); let after = camera_prop_test(true);
        let (b, a) = (text(&before, "camprop-before.txt"), text(&after, "camprop-after.txt"));
        assert!(a.contains("SUMMARY trunk-behind") && b.contains("SUMMARY rock-behind"));
        let min_solid = |t: &str| t.lines().filter(|l| l.starts_with("SUMMARY trunk-behind")).filter_map(|l| l.split("min clr_solid ").nth(1)).filter_map(|s| s.split(' ').next()).filter_map(|s| s.parse::<f32>().ok()).next().unwrap();
        assert!(min_solid(a) > min_solid(b), "collision keeps the eye out of the trunk: before {} after {}", min_solid(b), min_solid(a));
        assert!(!fn_game::props::disabled(), "the global switch is restored");
    }
    #[test] fn camera_video_script_runs_to_the_end_and_harvests_the_planted_tree() {
        let _g = lock();
        let o = camera_video(None); let log = text(&o, "camera-video-v2.txt");
        assert_eq!(log.lines().count(), 1 + video_script::FRAMES as usize);
        let last = log.lines().last().unwrap().split(' ').collect::<Vec<_>>(); let wood: u32 = last[last.len() - 1].parse().unwrap(); assert!(wood > 0, "the held-fire section harvests the tree: {last:?}");
    }
    #[test] fn pickaxe_series_captures_every_named_state_once() {
        let _g = lock();
        let o = pick_run(PickMode::Pickaxe, None, None); let caps = text(&o, "pickaxe-test.txt");
        for name in ["pick-idle", "pick-jump", "pick-hit-bounce-start", "pick-harvest-held-3", "pick-equip-early", "pick-sprintjump-apex", "pick-crouch-exited"] { assert_eq!(caps.lines().filter(|l| l.starts_with(&format!("{name} "))).count(), 1, "{name}"); }
    }
}
