//! `--script <file|builtin:NAME>`: an input timeline replayed through the real game loop (fixed time step, no real devices, no mouse capture).
//!
//! ```text
//! # comment (also ';')
//! size 1280x720          client size of the window for this run
//! fps 60                 fixed frame rate (dt = 1/fps)
//! props sandbox|none|tree|real|realtree|realrock    sandbox (default) = the three procedural harvestables of the old runtime, none = empty baseplate, tree = one procedural tree 1.8 m ahead,
//!                        real = the REAL 1.8 props around the anchor (props.rs), realtree / realrock = one real Athena_Tree_Large_1 / Athena_Prop_Rocks_StoneLarge01 1.4 m ahead
//!                        (the real ones need the exported props; without them the sandbox stays)
//! end 900                last frame + 1 (default: last entry + 30)
//! 20-100 forward         held for frames 20..100   (`A-B`: A inclusive, B exclusive; a single number is one frame)
//! 65 jump                one frame
//! 300-375 forward sprint
//! 410-480 forward strafe=0.41421356        axis values: fwd=<-1..1> strafe=<-1..1> (absolute, added to the key axes)
//! 0-60 yaw=90 pitch=-10  absolute control yaw / pitch every frame      turn=<deg>  look=<deg>   per-frame yaw / pitch delta
//! 5 equip                start the Melee_Equip montage          5 reset      back to the spawn          9999 quit
//! 100 resize 1920x1080   resize the window client area
//! 10 shot idle           capture <out>/idle.png after frame 10 has been rendered
//! ```
//! Actions: `forward back left right jump crouch sprint attack` map to the same `FrameInput` fields the keyboard path fills (`jump`,
//! `crouch_key`, `sprint_key`, `attack`), so hold/toggle latches and the jump/crouch interplay are exercised exactly like real play.
use fn_game::{FrameInput, GameSim};

#[derive(Clone, Debug, PartialEq)]
pub enum Action { Slot(usize), Reload, Health(f32), Shield(f32), Give(usize, u32), Forward, Back, Left, Right, Jump, Crouch, Sprint, Attack, Equip, Reset, Quit, Fwd(f32), Strafe(f32), Yaw(f32), Pitch(f32), Turn(f32), Look(f32), Shot(String), Resize(u32, u32) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Props { Sandbox, None, Tree, Real, RealTree, RealRock }

#[derive(Clone, Debug)]
struct Entry { from: u32, to: u32, action: Action }

#[derive(Clone, Debug)]
pub struct Script { pub name: String, pub size: Option<(u32, u32)>, pub fps: f32, pub props: Props, end: Option<u32>, entries: Vec<Entry>, pub assume: bool, pub standin_gun: bool }

fn num(s: &str, what: &str, line: usize) -> Result<f32, String> { s.parse::<f32>().map_err(|_| format!("line {line}: bad number in {what:?}")).and_then(|v| if v.is_finite() { Ok(v) } else { Err(format!("line {line}: {what:?} is not finite")) }) }

impl Script {
    pub fn parse(name: &str, text: &str) -> Result<Script, String> {
        let mut s = Script { name: name.to_owned(), size: None, fps: 60., props: Props::Sandbox, end: None, entries: vec![], assume: false, standin_gun: false };
        for (n, raw) in text.lines().enumerate() {
            let n = n + 1;
            let line = raw.split(['#', ';']).next().unwrap_or("").trim();
            if line.is_empty() { continue; }
            let toks: Vec<&str> = line.split_whitespace().collect();
            match toks[0] {
                "size" => { let (w, h) = crate::cli::parse_size(toks.get(1).ok_or(format!("line {n}: size needs WxH"))?).map_err(|e| format!("line {n}: {e}"))?; s.size = Some(crate::platform::check_client_size(w, h, "size").map_err(|e| format!("line {n}: {e}"))?); }
                "fps" => { let f = num(toks.get(1).ok_or(format!("line {n}: fps needs a value"))?, "fps", n)?; if !(1. ..=1000.).contains(&f) { return Err(format!("line {n}: fps {f} out of range")); } s.fps = f; }
                "props" => s.props = match toks.get(1).copied() { Some("sandbox") => Props::Sandbox, Some("none") => Props::None, Some("tree") => Props::Tree, Some("real") => Props::Real, Some("realtree") => Props::RealTree, Some("realrock") => Props::RealRock, o => return Err(format!("line {n}: unknown props {o:?}")) },
                "assume" => s.assume = true, "standin-gun" => s.standin_gun = true,
                "end" => s.end = Some(toks.get(1).and_then(|v| v.parse().ok()).ok_or(format!("line {n}: end needs a frame number"))?),
                first => {
                    let (from, to) = match first.split_once('-') {
                        Some((a, b)) => (a.parse::<u32>().map_err(|_| format!("line {n}: bad frame {a:?}"))?, b.parse::<u32>().map_err(|_| format!("line {n}: bad frame {b:?}"))?),
                        None => { let a = first.parse::<u32>().map_err(|_| format!("line {n}: expected a frame or range, got {first:?}"))?; (a, a.checked_add(1).ok_or_else(|| format!("line {n}: frame {a} is too large"))?) }
                    };
                    if to <= from { return Err(format!("line {n}: empty range {first}")); }
                    let mut i = 1;
                    if toks.len() < 2 { return Err(format!("line {n}: no action after {first}")); }
                    while i < toks.len() {
                        let t = toks[i];
                        let action = match t {
                            "forward" => Action::Forward, "back" => Action::Back, "left" => Action::Left, "right" => Action::Right, "jump" => Action::Jump, "crouch" => Action::Crouch,
                            "sprint" => Action::Sprint, "attack" | "fire" => Action::Attack, "reload" => Action::Reload, "equip" => Action::Equip, "reset" => Action::Reset, "quit" => Action::Quit,
                            "shot" => { i += 1; Action::Shot(toks.get(i).ok_or(format!("line {n}: shot needs a name"))?.to_string()) }
                            "resize" => { i += 1; let (w, h) = crate::cli::parse_size(toks.get(i).ok_or(format!("line {n}: resize needs WxH"))?).map_err(|e| format!("line {n}: {e}"))?; let (w, h) = crate::platform::check_client_size(w, h, "resize").map_err(|e| format!("line {n}: {e}"))?; Action::Resize(w, h) }
                            _ => match t.split_once('=') {
                                Some(("fwd", v)) => Action::Fwd(num(v, t, n)?), Some(("strafe", v)) => Action::Strafe(num(v, t, n)?), Some(("yaw", v)) => Action::Yaw(num(v, t, n)?),
                                Some(("pitch", v)) => Action::Pitch(num(v, t, n)?), Some(("slot", v)) => Action::Slot(v.parse::<usize>().ok().filter(|k| (1..=6).contains(k)).ok_or(format!("line {n}: slot needs 1..6"))? - 1), Some(("health", v)) => Action::Health(num(v, t, n)?), Some(("shield", v)) => Action::Shield(num(v, t, n)?),
                                Some(("give", v)) => { let (name, qty) = v.split_once(':').ok_or(format!("line {n}: give needs <bandage|medkit|shields|purple>:<count>"))?; let idx = ["bandage", "medkit", "shields", "purple", "ammo_ar", "ammo_shotgun", "ammo_sniper" /* AMMO HOOK: 4.. = an ammo pickup for firearm idx-4 */].iter().position(|k| *k == name).ok_or(format!("line {n}: unknown healable / ammo {name:?}"))?; Action::Give(idx, qty.parse().map_err(|_| format!("line {n}: bad count"))?) } Some(("turn", v)) => Action::Turn(num(v, t, n)?), Some(("look", v)) => Action::Look(num(v, t, n)?),
                                _ => return Err(format!("line {n}: unknown action {t:?}")),
                            },
                        };
                        s.entries.push(Entry { from, to, action });
                        i += 1;
                    }
                }
            }
        }
        Ok(s)
    }
    pub fn end_frame(&self) -> u32 { self.end.unwrap_or_else(|| self.entries.iter().map(|e| e.to).max().unwrap_or(0).saturating_add(30)) }
    fn active(&self, frame: u32) -> impl Iterator<Item = &Action> + '_ { self.entries.iter().filter(move |e| (e.from..e.to).contains(&frame)).map(|e| &e.action) }
    /// The input of one frame (the keyboard-path fields of `FrameInput`).
    pub fn input_at(&self, frame: u32) -> FrameInput {
        let mut f = FrameInput::default();
        for a in self.active(frame) {
            match a {
                Action::Forward => f.forward += 1., Action::Back => f.forward -= 1., Action::Right => f.side += 1., Action::Left => f.side -= 1.,
                Action::Fwd(v) => f.forward += v, Action::Strafe(v) => f.side += v,
                Action::Jump => f.jump = true, Action::Crouch => f.crouch_key = true, Action::Sprint => f.sprint_key = true, Action::Attack => f.attack = true,
                Action::Equip => f.equip = true, Action::Reset => f.reset = true,
                Action::Yaw(v) => f.set_yaw = Some(*v), Action::Pitch(v) => f.set_pitch = Some(*v), Action::Turn(v) => f.yaw_delta += v, Action::Look(v) => f.pitch_delta += v,
                Action::Quit | Action::Shot(_) | Action::Resize(..) | Action::Slot(_) | Action::Reload | Action::Health(_) | Action::Shield(_) | Action::Give(..) => {}
            }
        }
        f.forward = f.forward.clamp(-1., 1.); f.side = f.side.clamp(-1., 1.);
        f
    }
    /// The arms-side input of one frame (slot=N selects, reload, health=N sets the player's health for the heal test); fire is `attack` / `fire`.
    pub fn arms_at(&self, frame: u32) -> crate::arms::ArmsInput {
        let mut a = crate::arms::ArmsInput::default();
        for act in self.active(frame) { match act { Action::Slot(i) => a.select = Some(*i), Action::Reload => a.reload_press = true, Action::Health(h) => a.set_health = Some(*h), Action::Shield(v) => a.set_shield = Some(*v), Action::Give(i, q) if *i >= 4 => a.give_ammo = Some((*i - 4, *q)), Action::Give(i, q) => a.give = Some((*i, *q)), _ => {} } }
        a
    }
    pub fn shots_at(&self, frame: u32) -> Vec<String> { self.active(frame).filter_map(|a| if let Action::Shot(n) = a { Some(n.clone()) } else { None }).collect() }
    pub fn resize_at(&self, frame: u32) -> Option<(u32, u32)> { self.active(frame).find_map(|a| if let Action::Resize(w, h) = a { Some((*w, *h)) } else { None }) }
    pub fn quit_at(&self, frame: u32) -> bool { self.active(frame).any(|a| matches!(a, Action::Quit)) }
}

/// What a scripted run needs from a script, whether it is a text timeline or a built-in scenario with world scripting (the pickaxe series).
pub trait Driver {
    fn name(&self) -> String;
    fn fps(&self) -> f32 { 60. }
    fn size(&self) -> Option<(u32, u32)> { None }
    fn end_frame(&self) -> u32;
    /// World set-up before `frame` is stepped (props planted / cleared).
    fn prepare(&mut self, _frame: u32, _g: &mut GameSim) {}
    /// How the real 1.8 props fill the world when the run starts (`props.rs`); `Off` keeps the sandbox / the script's own props (the parity scripts).
    fn real_props(&self) -> crate::props::Mode { crate::props::Mode::Off }
    fn input(&mut self, frame: u32) -> FrameInput;
    /// Arms input of `frame` (quickbar selection, reload, health set) for scripted weapon tests.
    fn arms(&mut self, _frame: u32) -> crate::arms::ArmsInput { Default::default() }
    fn assume_unrecovered(&self) -> bool { false }
    fn standin_gun(&self) -> bool { false }
    /// Screenshot names to capture after `frame` has been stepped and rendered (`g.frames` is already incremented).
    fn shots(&mut self, frame: u32, g: &GameSim) -> Vec<String>;
    fn resize(&self, _frame: u32) -> Option<(u32, u32)> { None }
    fn quit(&self, _frame: u32) -> bool { false }
}

pub fn apply_props(props: Props, g: &mut GameSim) {
    match props {
        Props::Sandbox | Props::Real | Props::RealTree | Props::RealRock => {} // the real ones are placed when the load finishes (`props::Library::populate`)
        Props::None => g.world.props.clear(),
        Props::Tree => { g.world.props.clear(); g.world.props.push(fn_combat::Harvestable::new(fn_combat::Kind::Tree, glam::Vec3::new(0., 0., 12. - 1.8))); }
    }
}

impl Driver for Script {
    fn name(&self) -> String { self.name.clone() }
    fn fps(&self) -> f32 { self.fps }
    fn size(&self) -> Option<(u32, u32)> { self.size }
    fn end_frame(&self) -> u32 { Script::end_frame(self) }
    fn prepare(&mut self, frame: u32, g: &mut GameSim) { if frame == 0 { apply_props(self.props, g); } }
    fn real_props(&self) -> crate::props::Mode { match self.props { Props::Real => crate::props::Mode::Layout, Props::RealTree => crate::props::Mode::One(crate::props::ONE_TREE_ID), Props::RealRock => crate::props::Mode::One(crate::props::ONE_ROCK_ID), _ => crate::props::Mode::Off } }
    fn input(&mut self, frame: u32) -> FrameInput { self.input_at(frame) }
    fn arms(&mut self, frame: u32) -> crate::arms::ArmsInput { self.arms_at(frame) }
    fn assume_unrecovered(&self) -> bool { self.assume }
    fn standin_gun(&self) -> bool { self.standin_gun }
    fn shots(&mut self, frame: u32, _g: &GameSim) -> Vec<String> { self.shots_at(frame) }
    fn resize(&self, frame: u32) -> Option<(u32, u32)> { self.resize_at(frame) }
    fn quit(&self, frame: u32) -> bool { self.quit_at(frame) }
}

/// The fn-sim `--pickaxe-test` series (idle, jog, sprint, crouch, jump, swing / hit / bounce / harvest, equip, ... then a resize sweep) driven
/// through the app: the inputs are `pick_script::step`, the world scripting is `pick_run`'s, so its frame log equals fn-sim's line for line.
#[cfg(feature = "sim-compare")]
pub struct PickDriver;
#[cfg(feature = "sim-compare")]
impl PickDriver {
    pub fn new() -> PickDriver { use fn_sim::pick_script as p; p::set_active(true); p::set_anim_series(false); p::set_dir_series(false); PickDriver }
}
#[cfg(feature = "sim-compare")]
impl Driver for PickDriver {
    fn name(&self) -> String { "pickaxe".into() }
    fn size(&self) -> Option<(u32, u32)> { Some((1280, 720)) }
    fn end_frame(&self) -> u32 { fn_sim::pick_script::end_frame() }
    fn prepare(&mut self, f: u32, g: &mut GameSim) {
        use fn_sim::pick_script as pick;
        if f == pick::clear_frame() { g.world.props.clear(); }
        if f == pick::target_frame() {
            g.controller = fn_game::movement::Controller::default(); g.to_spawn();
            g.world.props.push(fn_combat::Harvestable::new(fn_combat::Kind::Tree, glam::Vec3::new(0., 0., 12. - 1.8)));
        }
    }
    fn input(&mut self, f: u32) -> FrameInput {
        use fn_sim::pick_script as pick;
        let ps = pick::step(f);
        FrameInput { forward: ps.forward, side: ps.side, script_jump: ps.jump, force_crouch: ps.crouch, force_run: ps.run, attack: ps.attack,
            set_yaw: ps.yaw.map(|y| (fn_game::spawn().3 + y).rem_euclid(360.)), set_pitch: ps.pitch, equip: pick::equip(f), ..Default::default() }
    }
    fn shots(&mut self, _f: u32, g: &GameSim) -> Vec<String> {
        use fn_sim::pick_script as pick;
        if let Some(n) = pick::capture(g.frames) { return vec![n.to_owned()]; }
        if pick::size_capture(g.frames) { return vec!["pick-size".to_owned()]; } // the resolution sweep: the name gets the size appended
        vec![]
    }
    fn resize(&self, f: u32) -> Option<(u32, u32)> { fn_sim::pick_script::resize(f).map(|(w, h)| (w as u32, h as u32)) }
}

pub const SMOKE: &str = "# the old --smoke-test timeline at fixed 60 Hz: idle, jog, jump, landing, crouch, sprint, direction blend, swings, moving swing, crouched swing
size 1280x720
fps 60
props sandbox
0 equip
20-100 forward
65 jump
195-280 crouch
225-270 forward
300-375 forward sprint
410-480 forward strafe=0.41421356
490-545 attack
610-715 forward
635-690 attack
750-850 crouch
770-805 attack
10 shot idle
60 shot jog
85 shot jump
145 shot landing
210 shot crouch-idle
250 shot crouch-walk
350 shot sprint
465 shot direction-blend
507 shot swing
540 shot swing-two
570 shot swing-recovery
652 shot moving-swing
787 shot crouched-swing
870 shot scene
end 880
";
pub const WALK: &str = "# walk, turn, jump, crouch, sprint, swing next to a tree (short)
size 1280x720
fps 60
props tree
0 equip
10 shot start
30-150 forward
60 jump
90-150 sprint
150 shot walk-end
160-220 crouch
200 shot crouch
230-250 attack
250 shot swing
260-320 turn=1.5
320 shot turned
end 330
";

pub const WEAPONS_BODY: &str = "size 1280x720\nfps 60\nprops none\n5 slot=2\n40-100 fire\n120 reload\n280 slot=3\n320 fire\n420 fire\n470 reload\n660 slot=4\n700 fire\n760 reload\n960 slot=5\n965 health=40\n970 fire\n1300 slot=2\n";
/// Default policy: every behaviour the 1.8 data does not decide stays missing and is reported (`[arms] ... MISSING`). Ends with one capture of the held state.
pub const HEALING: &str = "# every healing consumable of the data through the real ability logic (use ring, montage, trigger, cost, effect, cooldown)
size 1280x720
fps 60
props none
5 slot=5
10 health=30 shield=20
15 fire
150 shot ring
400 give=medkit:1 health=30
410 fire
1100 fire
1300 give=shields:2 shield=10
1310 fire
1700 fire
1900 give=purple:1 health=10 shield=0
1910 fire
2500 fire
end 3000
";
pub fn weapons_text(assumed: bool) -> String { let head = if assumed { "assume\nstandin-gun\n" } else { "" }; format!("{head}{WEAPONS_BODY}{}end 1420\n", if assumed { "1400 shot held\n" } else { "" }) }

pub fn builtin(name: &str) -> Option<Result<Box<dyn Driver>, String>> {
    match name {
        "smoke" => Some(Script::parse("smoke", SMOKE).map(|s| Box::new(s) as Box<dyn Driver>)),
        "walk" => Some(Script::parse("walk", WALK).map(|s| Box::new(s) as Box<dyn Driver>)),
        "healing" => Some(Script::parse("healing", HEALING).map(|s| Box::new(s) as Box<dyn Driver>)),
        "weapons" => Some(Script::parse("weapons", &weapons_text(false)).map(|s| Box::new(s) as Box<dyn Driver>)),
        "weapons-assumed" => Some(Script::parse("weapons-assumed", &weapons_text(true)).map(|s| Box::new(s) as Box<dyn Driver>)),
        #[cfg(feature = "sim-compare")]
        "pickaxe" => Some(Ok(Box::new(PickDriver::new()))),
        _ => None,
    }
}

/// `file` or `builtin:NAME`.
pub fn load(spec: &str) -> Result<Box<dyn Driver>, String> {
    if let Some(name) = spec.strip_prefix("builtin:") { return builtin(name).unwrap_or_else(|| Err(format!("unknown built-in script {name:?} (smoke, walk, pickaxe, weapons, weapons-assumed, healing)"))); }
    let text = std::fs::read_to_string(spec).map_err(|e| format!("cannot read script {spec}: {e}"))?;
    Ok(Box::new(Script::parse(spec, &text)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn a_single_frame_at_u32_max_is_an_error_not_a_wrapping_range() {
        let e = Script::parse("t", "4294967295 jump\n").unwrap_err(); assert!(e.contains("too large") && e.contains("line 1"), "{e}");
        let s = Script::parse("t", "4294967294 jump\n").unwrap(); assert_eq!(s.end_frame(), u32::MAX, "end_frame saturates instead of wrapping");
        assert!(Script::parse("t", "5-4294967295 forward\n").is_ok());
    }
    #[test] fn oversized_or_tiny_script_windows_are_rejected_with_the_limit_in_the_message() {
        for text in ["10 resize 9000x500", "10 resize 1280x9000", "10 resize 99999x1", "size 9000x720", "size 100x100", "5 resize 639x360"] {
            let e = Script::parse("t", text).unwrap_err(); assert!(e.contains("line 1") && (e.contains("8192") || e.contains("minimum")), "{text}: {e}");
        }
        assert!(Script::parse("t", "10 resize 8192x8192\nsize 640x360").is_ok());
    }
    #[test] fn parses_ranges_axes_and_options() {
        let s = Script::parse("t", "size 800x600\nfps 30\nprops none\n# c\n10-20 forward sprint ; trailing\n15 jump shot a\n5-8 strafe=0.5 yaw=90\n40 resize 640x360\nend 100").unwrap();
        assert_eq!((s.size, s.fps, s.props, s.end_frame()), (Some((800, 600)), 30., Props::None, 100));
        let f = s.input_at(15); assert_eq!(f.forward, 1.); assert!(f.sprint_key && f.jump && !f.attack); assert_eq!(s.shots_at(15), vec!["a".to_owned()]);
        assert!(s.input_at(9).forward == 0. && s.input_at(20).forward == 0., "20 is exclusive");
        let g = s.input_at(6); assert_eq!((g.side, g.set_yaw), (0.5, Some(90.))); assert_eq!(s.resize_at(40), Some((640, 360))); assert_eq!(s.resize_at(41), None);
    }
    #[test] fn forward_back_cancel_and_axes_clamp() { let s = Script::parse("t", "0-5 forward back\n0-5 fwd=1 forward").unwrap(); assert_eq!(s.input_at(1).forward, 1.); }
    #[test] fn end_defaults_to_the_last_entry_plus_30() { assert_eq!(Script::parse("t", "5-10 forward").unwrap().end_frame(), 40); assert_eq!(Script::parse("t", "").unwrap().end_frame(), 30); }
    #[test] fn bad_lines_are_reported_with_their_number() {
        for (text, needle) in [("foo bar", "line 1"), ("10-5 jump", "empty range"), ("5", "no action"), ("0 sprintt", "unknown action"), ("0 yaw=x", "bad number"), ("size 5", "expected WxH"), ("fps 0", "out of range"), ("props lots", "unknown props"), ("0 shot", "needs a name"), ("0 resize", "needs WxH")] {
            let e = Script::parse("t", text).unwrap_err(); assert!(e.contains(needle), "{text:?} -> {e}");
        }
    }
    #[test] fn builtin_scripts_parse_and_have_captures() {
        for n in ["smoke", "walk"] { let mut d = builtin(n).unwrap().unwrap(); let g = GameSim::new(); let shots: usize = (0..d.end_frame()).map(|f| d.shots(f, &g).len()).sum(); assert!(shots >= 5, "{n}"); assert!(d.end_frame() > 100); }
        assert!(builtin("nope").is_none());
    }
    #[test] fn smoke_script_walks_jumps_crouches_sprints_and_swings_in_the_simulation() {
        let mut d = builtin("smoke").unwrap().unwrap(); let mut g = GameSim::new();
        let (mut max_h, mut crouched, mut ran, mut swung, mut min_z) = (0f32, false, false, false, g.z);
        for f in 0..d.end_frame() { d.prepare(f, &mut g); g.step(&d.input(f), 1. / 60.); max_h = max_h.max(g.controller.air_height()); crouched |= g.controller.crouched; ran |= g.controller.running; swung |= g.swing.stage > 0; min_z = min_z.min(g.z); }
        assert!(max_h > 0.5 && crouched && ran && swung && min_z < g.feet().z + 1., "jump {max_h} crouch {crouched} run {ran} swing {swung}");
        assert!(g.world.hits + g.world.misses > 0, "swings landed somewhere");
    }
    #[test] fn weapon_script_actions_map_to_arms_input() {
        let s = Script::parse("t", "assume\nstandin-gun\n5 slot=2\n7 reload health=40\n9 fire\n").unwrap();
        assert!(s.assume && s.standin_gun);
        assert_eq!(s.arms_at(5).select, Some(1)); let a = s.arms_at(7); assert!(a.reload_press && a.set_health == Some(40.)); assert!(s.input_at(9).attack);
        assert!(Script::parse("t", "5 slot=7").is_err());
        for assumed in [false, true] { let t = weapons_text(assumed); let s = Script::parse("w", &t).unwrap(); assert_eq!(s.assume, assumed); assert_eq!(s.end_frame(), 1420); }
    }    #[test] fn tree_props_plant_one_tree_ahead() { let mut g = GameSim::new(); apply_props(Props::Tree, &mut g); assert_eq!(g.world.props.len(), 1); apply_props(Props::None, &mut g); assert!(g.world.props.is_empty()); }
}
