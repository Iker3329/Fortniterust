//! The scripted scenarios of the old prototype's test modes (`--dir-test`, `--pickaxe-test`, `--anim-test`, `--face-test`, `--jump-air-test`)
//! driven headlessly through `fn_game::GameSim` / the jump-air twin, handing every frame's animation state and pose tree to a callback.
//! Same loops, same inputs, same order of operations as `fn_sim::scenarios` (which logs hashes / bone columns) and the old exe.
use fn_animgraph::{AnimState, JumpState, Node, PawnSample};
use fn_game::combat::{Harvestable, Kind};
use fn_game::movement::{self, Controller};
use fn_game::{spawn, FrameInput, GameSim};
use fn_sim::pick_script as pick;
use fn_sim::video_script::Step as VStep;
use glam::{Vec2, Vec3};
use std::sync::Mutex;

pub const DT: f32 = 1. / 60.;
/// The pick script flips process-wide switches; scenarios serialise on this.
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    Pickaxe,
    Anim,
    Dir,
    /// `--face-test`: 40 settle frames, then pitch +80..-80, yaw -75..75, a 3-turn spin (432 frames).
    Face,
    /// `--jump-air-test` scenario `i` (0..8) at 60 Hz.
    JumpAir(usize),
}
impl Scenario {
    pub fn parse(s: &str) -> Option<Scenario> {
        Some(match s {
            "pickaxe-test" | "pickaxe" => Scenario::Pickaxe,
            "anim-test" | "anim" => Scenario::Anim,
            "dir-test" | "dir" => Scenario::Dir,
            "face-test" | "face" => Scenario::Face,
            other => { let n = other.strip_prefix("jump-air-test:").or_else(|| other.strip_prefix("jump-air:"))?; Scenario::JumpAir(n.parse().ok()?) }
        })
    }
    pub fn name(&self) -> String {
        match self { Scenario::Pickaxe => "pickaxe-test".into(), Scenario::Anim => "anim-test".into(), Scenario::Dir => "dir-test".into(), Scenario::Face => "face-test".into(), Scenario::JumpAir(i) => format!("jump-air-test:{i}") }
    }
}

/// One simulated frame.
pub struct FrameCtx<'a> {
    pub frame: u32,
    pub anim: &'a AnimState,
    /// The full game state (absent for the jump-air twin, which has only a controller + graph).
    pub game: Option<&'a GameSim>,
    pub tree: Node,
    /// Screenshot name the old script took after this frame (pick scenarios).
    pub capture: Option<&'static str>,
    /// Face test: (phase name, scripted camera yaw offset, scripted pitch).
    pub face: Option<(&'static str, f32, f32)>,
}

pub fn frame_count(s: Scenario) -> u32 {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match s {
        Scenario::Pickaxe => { pick::set_active(true); pick::set_anim_series(false); pick::set_dir_series(false); let n = pick::end_frame(); pick::set_active(false); n }
        Scenario::Anim => { pick::set_active(true); pick::set_anim_series(true); pick::set_dir_series(false); let n = pick::end_frame(); pick::set_anim_series(false); pick::set_active(false); n }
        Scenario::Dir => { pick::set_active(true); pick::set_dir_series(true); let n = pick::end_frame(); pick::set_dir_series(false); pick::set_anim_series(false); pick::set_active(false); n }
        Scenario::Face => 40 + 180 + 180 + 72,
        Scenario::JumpAir(_) => 300,
    }
}

/// Runs the scenario (at most `max_frames` frames) and calls `f` after every frame. Frame numbering is the old log's (0-based loop index).
pub fn run(s: Scenario, max_frames: Option<u32>, f: &mut dyn FnMut(&FrameCtx)) {
    match s {
        Scenario::Pickaxe | Scenario::Anim | Scenario::Dir => run_pick(s, max_frames, f),
        Scenario::Face => run_face(max_frames, f),
        Scenario::JumpAir(i) => run_jump_air(i, max_frames, f),
    }
}

fn run_pick(mode: Scenario, max_frames: Option<u32>, f: &mut dyn FnMut(&FrameCtx)) {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    pick::set_active(true);
    pick::set_anim_series(matches!(mode, Scenario::Anim | Scenario::Dir));
    pick::set_dir_series(mode == Scenario::Dir);
    let mut g = GameSim::new();
    let end = max_frames.unwrap_or_else(pick::end_frame).min(pick::end_frame());
    for fr in 0..end {
        let ps = pick::step(fr);
        if fr == pick::clear_frame() { g.world.props.clear(); }
        if fr == pick::target_frame() {
            g.controller = Controller::default();
            g.to_spawn();
            g.world.props.push(Harvestable::new(Kind::Tree, Vec3::new(0., 0., 12. - 1.8)));
        }
        let inp = FrameInput { forward: ps.forward, side: ps.side, script_jump: ps.jump, force_crouch: ps.crouch, force_run: ps.run, attack: ps.attack,
            set_yaw: ps.yaw.map(|y| (spawn().3 + y).rem_euclid(360.)), set_pitch: ps.pitch, equip: pick::equip(fr), ..Default::default() };
        g.step(&inp, DT);
        let capture = pick::capture(g.frames);
        f(&FrameCtx { frame: fr, anim: &g.anim, game: Some(&g), tree: g.pose_tree(), capture, face: None });
    }
    pick::set_active(false);
    pick::set_anim_series(false);
    pick::set_dir_series(false);
}

/// The old `face_test::run` script: (phase, frames, script(frame fraction) -> (yaw offset deg, pitch deg)).
type Script = fn(f32) -> (f32, f32);
const PHASES: [(&str, u32, Script); 3] = [
    ("pitch", 180, |t| (0., 80. - 160. * t)),
    ("yaw", 180, |t| (-75. + 150. * t, -20.)),
    ("spin", 72, |t| (t * 1080., -35.)),
];

fn run_face(max_frames: Option<u32>, f: &mut dyn FnMut(&FrameCtx)) {
    let mut g = GameSim::new();
    g.to_spawn();
    let yaw0 = g.yaw;
    g.cam.reset();
    g.anim_time = 0.;
    for _ in 0..40 { fn_sim::scenarios::video_sim(&mut g, &VStep { yaw: yaw0, pitch: 0., ..Default::default() }, DT); }
    let mut frame = 0u32;
    for (name, n, script) in PHASES {
        for k in 0..n {
            if max_frames.is_some_and(|m| frame >= m) { return; }
            let (dyaw, pitch) = script(k as f32 / (n - 1) as f32);
            fn_sim::scenarios::video_sim(&mut g, &VStep { yaw: yaw0 + dyaw, pitch, ..Default::default() }, DT);
            f(&FrameCtx { frame, anim: &g.anim, game: Some(&g), tree: g.pose_tree(), capture: None, face: Some((name, dyaw, pitch)) });
            frame += 1;
        }
    }
}

/// Input of one jump-air frame (`fn_game::jump_air::In`).
pub type JumpIn = fn_game::jump_air::In;
/// The eight `--jump-air-test` scenarios of `fn_sim::series` (script by 60 Hz frame index).
pub fn jump_air_script(i: usize) -> Option<(&'static str, Box<dyn Fn(usize) -> JumpIn>)> {
    Some(match i {
        0 => ("press only at takeoff (f10)", Box::new(|f| JumpIn { jump: f == 10, ..Default::default() })),
        1 => ("air presses at f15 f30 f47 f60 f75 f80 f81 f82", Box::new(|f| JumpIn { jump: [10, 15, 30, 47, 60, 75, 80, 81, 82].contains(&f), ..Default::default() })),
        2 => ("key held from takeoff for 4 s (no auto re-jump)", Box::new(|f| JumpIn { jump: (10..250).contains(&f), ..Default::default() })),
        3 => ("key mashed every other frame (re-jumps only from the ground)", Box::new(|f| JumpIn { jump: f >= 10 && f % 2 == 0, ..Default::default() })),
        4 => ("press during landing recovery (f90 after takeoff f10)", Box::new(|f| JumpIn { jump: f == 10 || f == 90, ..Default::default() })),
        5 => ("press 2 frames before touchdown (f82), must not be buffered", Box::new(|f| JumpIn { jump: f == 10 || f == 82, ..Default::default() })),
        6 => ("crouch key held, press at f10 and f50 (the jump key cancels the crouch request)", Box::new(|f| JumpIn { crouch: f < 200, jump: f == 10 || f == 50, ..Default::default() })),
        7 => ("sprint jump with air presses", Box::new(|f| JumpIn { forward: true, run: true, jump: f == 30 || f == 45 || f == 60, ..Default::default() })),
        _ => return None,
    })
}

/// `fn_game::jump_air::simulate_full`'s loop (controller + AnimState wiring of `App::update`) at 60 Hz, exposing the graph every frame.
fn run_jump_air(i: usize, max_frames: Option<u32>, f: &mut dyn FnMut(&FrameCtx)) {
    let Some((_, script)) = jump_air_script(i) else { return };
    let dt = 1. / 60.;
    let (mut c, mut a) = (Controller::default(), AnimState::new());
    let _ = JumpState::Idle;
    for fr in 0..max_frames.unwrap_or(300).min(300) as usize {
        let inp = script(fr);
        c.wants_crouch = inp.crouch && !c.jump_edge(inp.jump);
        let dir = if inp.forward { Vec2::Y } else { Vec2::ZERO };
        c.step(dir, inp.jump, inp.run, dt);
        a.step_sample(&PawnSample { velocity: (c.velocity.x, c.velocity.y), input: (dir.x, dir.y), facing_deg: c.facing, camera_yaw_deg: c.facing, camera_pitch_deg: 0., height: c.air_height(), vertical_speed: c.vertical_speed,
            fall_time_left: movement::fall_time_left(c.air_height(), c.vertical_speed), crouched: c.crouched, running: c.running, melee_swinging: false }, dt);
        f(&FrameCtx { frame: fr as u32, anim: &a, game: None, tree: a.plan(&[]), capture: None, face: None });
    }
}
