//! `--movement-test` and `--jump-air-test`: headless, deterministic time-series runs of the movement component (no window, no GL, no input injection).
//! They write plain-text logs next to the executable (`verification/movement-test.txt`, `verification/jump-air-test.txt`; `--out <dir>` overrides)
//! and exit 0 when every check passed, 1 otherwise. The numeric assertions live in `movement_tests.rs` (cargo test); these runs print the
//! SERIES behind them (position/velocity/mode per frame) so a human can read the arc, the braking curve, the step-up, the ledge and the air presses.
use fn_game::jump_air::{log_is_a_clean_walk, simulate_full, state_log, In};
use fn_movement::cmc::{Cmc, Mode, Params};
use fn_movement::params as mp;
use fn_movement::scene::{Frustum, Scene};
use glam::{Vec2, Vec3};
use std::fmt::Write as _;

struct Log { s: String, failed: usize, checks: usize }
impl Log {
    fn new(title: &str) -> Log { let mut l = Log { s: String::new(), failed: 0, checks: 0 }; let _ = writeln!(l.s, "{title}\nFortnite 1.8 / UE 4.16 movement port (units: cm, s; UE axes X,Y horizontal, Z up). Gravity -2800 x GravityScale {} (PROVENANCE: decrypted config, not independently verified; the exe default is -980).\n", Params::default().gravity_scale); l }
    fn h(&mut self, t: &str) { let _ = writeln!(self.s, "\n== {t}"); }
    fn row(&mut self, c: &Cmc, t: f32, extra: &str) {
        let _ = writeln!(self.s, "t {t:6.3}  x {:8.2} y {:8.2} feet {:7.2} | v {:7.1} {:7.1} {:7.1} | {} crouch {} floorDist {:5.2} {extra}", c.loc.x, c.loc.y, c.feet_height(), c.velocity.x, c.velocity.y, c.velocity.z,
            if c.mode == Mode::Walking { "walk" } else { "fall" }, c.is_crouched as u8, c.floor.floor_dist);
    }
    fn check(&mut self, what: &str, ok: bool, detail: String) { self.checks += 1; if !ok { self.failed += 1; } let _ = writeln!(self.s, "[{}] {what}: {detail}", if ok { "PASS" } else { "FAIL" }); }
    fn finish(mut self) -> (String, usize) { let _ = writeln!(self.s, "\n{} checks, {} failed.", self.checks, self.failed); (self.s, self.failed) }
}
fn cyl(x: f32, y: f32, z0: f32, z1: f32, r: f32) -> Frustum { Frustum { c: Vec2::new(x, y), z0, z1, r0: r, r1: r } }
fn spawn(scene: Vec<Frustum>, x: f32, y: f32, feet: f32) -> Cmc { Cmc::new(Params::default(), Scene { solids: scene, ..Default::default() }, Vec3::new(x, y, feet)) }
const FWD: Vec2 = Vec2::new(0., -1.);

pub fn run_movement_test() -> (String, usize) {
    let mut l = Log::new("--movement-test: time series of the UCharacterMovementComponent port");
    let g = -Params::default().gravity_z();
    // 1. standing jump, every frame at 60 Hz
    l.h("jump from rest, 60 Hz (press at t=0.100 s)");
    { let mut c = spawn(vec![], 0., 0., 0.); let dt = 1. / 60.; let base = c.feet_z(); let mut t = 0.; let (mut apex, mut apex_t, mut takeoff_t, mut land_t) = (0f32, 0f32, None::<f32>, None::<f32>); let mut vprev = 0.; let mut gs = vec![];
      for f in 0..130 { if f == 6 { c.jump(); } c.tick(Vec2::ZERO, dt); t += dt; if c.is_falling() { takeoff_t.get_or_insert(t); if c.feet_z() - base > apex { apex = c.feet_z() - base; apex_t = t; } if f > 7 && !(c.velocity.z == 0.) { gs.push((vprev - c.velocity.z) / dt); } } if takeoff_t.is_some() && land_t.is_none() && c.is_moving_on_ground() { land_t = Some(t); } vprev = c.velocity.z; if f % 3 == 0 || land_t.is_some() && f < 100 { l.row(&c, t, ""); } }
      let air = land_t.unwrap_or(0.) - takeoff_t.unwrap_or(0.) + dt; let gm = gs.iter().sum::<f32>() / gs.len().max(1) as f32;
      l.check("apex = v^2/2g", (apex - mp::jump_apex_cm()).abs() < 1.5, format!("{apex:.2} cm vs {:.2} cm (apex at t={apex_t:.3})", mp::jump_apex_cm()));
      l.check("airtime = 2v/g", (air - mp::jump_airtime()).abs() < 0.05, format!("{air:.3} s vs {:.3} s", mp::jump_airtime()));
      l.check("integrated gravity = 2800 cm/s^2 (PROVENANCE: decrypted config, not independently verified)", (gm - g).abs() < 1., format!("mean dv/dt while falling {gm:.1} cm/s^2")); }
    l.h("frame-rate sweep of the same jump (30 Hz carries the exe DoJump frame-delta scale 1.0589, 60 Hz and faster do not)");
    for hz in [30., 60., 144., 240.] { let mut c = spawn(vec![], 0., 0., 0.); let dt = 1. / hz; let b = c.feet_z(); c.jump(); let (mut apex, mut t, mut left) = (0f32, 0., false);
        for _ in 0..(3. * hz) as usize { c.tick(Vec2::ZERO, dt); t += dt; if c.is_falling() { left = true; apex = apex.max(c.feet_z() - b); } if left && c.is_moving_on_ground() { break; } }
        l.check(&format!("{hz:>3} Hz jump"), (apex - mp::jump_apex_cm_at(1. / hz)).abs() < 2800. / (hz * hz * 8.) + 0.1 && (t - mp::jump_airtime_at(1. / hz)).abs() < 1.5 / hz + 0.006, format!("apex {apex:.2} cm airtime {t:.3} s")); }
    // 2. run, release, brake
    l.h("accelerate 2 s then release (60 Hz): speed and braking");
    { let mut c = spawn(vec![], 0., 0., 0.); let dt = 1. / 60.; let mut t = 0.; for f in 0..120 { c.tick(FWD, dt); t += dt; if f % 6 == 5 { l.row(&c, t, ""); } }
      let y0 = c.loc.y; let v = -c.velocity.y; let mut stop_t = None; for f in 0..60 { c.tick(Vec2::ZERO, dt); t += dt; if f < 12 { l.row(&c, t, "(braking)"); } if c.velocity == Vec3::ZERO && stop_t.is_none() { stop_t = Some(f as f32 * dt); } }
      let d = y0 - c.loc.y; l.check("run speed = SpeedRun", (v - mp::SPEED_RUN).abs() < 0.5, format!("{v:.1} cm/s")); l.check("braking distance (ODE 21.8 cm, 60 Hz engine value ~16.3 cm)", d > 14. && d < 22., format!("{d:.2} cm in {:.3} s", stop_t.unwrap_or(-1.))); }
    // 3. step up
    l.h("walk into a 30 cm step and a 50 cm step (60 Hz, every 3rd frame near the contact)");
    for (hgt, expect) in [(30., true), (50., false)] {
        let mut c = spawn(vec![cyl(0., -200., 0., hgt, 100.)], 0., 0., 0.); let dt = 1. / 60.; let mut best = 0f32; let mut t = 0.;
        let _ = writeln!(l.s, "-- step height {hgt} cm");
        for f in 0..150 { c.tick(FWD, dt); t += dt; best = best.max(c.feet_height()); if (20..40).contains(&f) && f % 2 == 0 { l.row(&c, t, ""); } }
        l.check(&format!("step {hgt} cm {}", if expect { "climbed" } else { "blocked" }), if expect { (best - hgt).abs() < 0.3 } else { best < 3. }, format!("highest feet {best:.2} cm (MaxStepHeight 45)")); }
    // 4. slopes
    l.h("slope limit (WalkableFloorZ 0.71 = 44.77 degrees)");
    for ang in [30., 40., 44., 46., 50., 60.] { let r = 300.; let h = r * (ang as f32).to_radians().tan(); let mut c = spawn(vec![Frustum { c: Vec2::new(0., -300.), z0: 0., z1: h, r0: r, r1: 0. }], 0., 0., 0.); let mut best = 0f32; for _ in 0..360 { c.tick(FWD, 1. / 60.); best = best.max(c.feet_height()); }
        let climbed = best > 40.; let want = ang < 44.76; l.check(&format!("{ang} degree cone {}", if want { "climbed" } else { "not climbed" }), climbed == want, format!("highest feet {best:.1} cm of {h:.0} cm")); }
    // 5. ledge
    l.h("plateau 100 cm high, radius 150 cm: standing walks off, crouched stays (perch radius 20 cm)");
    { let mut c = spawn(vec![cyl(0., 0., 0., 100., 150.)], 0., 0., 100.); let dt = 1. / 60.; let mut t = 0.; let mut fell_at = None;
      for f in 0..240 { c.tick(Vec2::X, dt); t += dt; if c.is_falling() && fell_at.is_none() { fell_at = Some(t); } if f % 10 == 0 || fell_at.is_some() && f < 140 && f % 4 == 0 { l.row(&c, t, ""); } }
      l.check("standing pawn walked off and landed", c.fell_off_ledge >= 1 && c.landed >= 1 && c.feet_height() < 0.5, format!("left the edge at t={:.2} s, ended at {:.2} cm, landings {}", fell_at.unwrap_or(-1.), c.feet_height(), c.landed));
      let mut c = spawn(vec![cyl(0., 0., 0., 100., 150.)], 0., 0., 100.); c.wants_crouch = true; for _ in 0..240 { c.tick(Vec2::X, dt); }
      l.check("crouched pawn stays on the plateau", c.is_crouched && c.is_moving_on_ground() && c.loc.x > 150. && c.loc.x < 150.5 + mp::PERCH_RADIUS_THRESHOLD, format!("centre {:.2} cm from the axis, feet {:.2}", c.loc.x, c.feet_height())); }
    // 6. crouch under a ceiling
    l.h("ceiling slab 90..100 cm: crouch, walk under, release crouch (stays crouched), walk out (stands)");
    { let mut c = spawn(vec![Frustum { c: Vec2::ZERO, z0: 90., z1: 100., r0: 300., r1: 300. }], 600., 0., 0.); let dt = 1. / 60.; let mut t = 0.; c.wants_crouch = true;
      for f in 0..420 { if f == 200 { c.wants_crouch = false; } let input = if f < 20 { Vec2::ZERO } else if f < 200 { -Vec2::X } else if f < 260 { Vec2::ZERO } else { Vec2::X }; c.tick(input, dt); t += dt; if f % 20 == 0 { l.row(&c, t, &format!("halfHeight {:.1}", c.half_height)); } if f == 259 { l.check("uncrouch refused under the ceiling", c.is_crouched && c.loc.x < 300., format!("x {:.1} halfHeight {:.1}", c.loc.x, c.half_height)); } }
      l.check("stands up after leaving the slab", !c.is_crouched && (c.half_height - mp::CAPSULE_HALF_HEIGHT).abs() < 0.01, format!("x {:.1} halfHeight {:.1}", c.loc.x, c.half_height)); }
    // 7. directional speeds / sprint / crouch
    l.h("speed caps (3 s at 60 Hz): facing is -Y");
    for (name, sprint, crouch, dir, want) in [("run forward", false, false, FWD, 410.), ("sprint forward", true, false, FWD, 550.), ("crouch walk", false, true, FWD, 290.), ("run backwards", false, false, -FWD, 410. * 0.65), ("run strafe 90 deg", false, false, Vec2::X, 410. * 0.725)] {
        let mut c = spawn(vec![], 0., 0., 0.); c.wants_sprint = sprint; c.wants_crouch = crouch; for _ in 0..180 { c.tick(dir, 1. / 60.); } let v = c.velocity.length(); l.check(name, (v - want).abs() < 0.6, format!("{v:.1} cm/s (expected {want:.1})")); }
    // 8. fall from height and terminal velocity
    l.h("free fall from 200 m (terminal velocity 4000 cm/s)");
    { let mut c = spawn(vec![], 0., 0., 20000.); let dt = 1. / 60.; let mut t = 0.; let mut vmax = 0f32; for f in 0..(20 * 60) { c.tick(Vec2::ZERO, dt); t += dt; vmax = vmax.max(-c.velocity.z); if f % 30 == 0 { l.row(&c, t, ""); } if c.is_moving_on_ground() { break; } } l.check("terminal velocity", (vmax - 4000.).abs() < 1., format!("{vmax:.1} cm/s, landed at t={t:.2} s")); }
    // 9. air presses on the controller
    l.h("jump key presses in the air (component level: every press is attempted, none starts a jump)");
    { let mut c = spawn(vec![], 0., 0., 0.); let dt = 1. / 60.; let mut t = 0.; c.jump(); let mut presses = 0; let mut seen = vec![];
      for f in 0..120 { if c.is_falling() && f % 7 == 0 { c.jump(); presses += 1; seen.push((f, c.velocity.z)); } c.tick(Vec2::ZERO, dt); t += dt; if f < 20 || f % 7 == 0 { l.row(&c, t, &format!("jumps {} pressed {}", c.jumped, c.pressed_jump as u8)); } }
      l.check("air presses ignored", c.jumped == 1 && presses >= 3, format!("{presses} presses in the air, jumps started {}", c.jumped)); }
    l.finish()
}

/// `--jump-air-test`: the headless twin of App::update (controller + animation graph) over the air-press scenarios; prints the jump state log of each.
/// Scripts are written in 60 Hz frame indices; at other rates a script frame lasts `hz/60` render frames (a tap is a 16.7 ms key press).
pub fn run_jump_air_test() -> (String, usize) {
    let mut l = Log::new("--jump-air-test: jump key presses while airborne, crouched, during landing recovery (controller + AdditiveJumps machine, App::update wiring)");
    type Script = Box<dyn Fn(usize) -> In>;
    // (name, script, expected takeoffs; 0 = "at least 2, only ever from the ground")
    let scenarios: Vec<(&str, Script, u32)> = vec![
        ("press only at takeoff (f10)", Box::new(|f| In { jump: f == 10, ..Default::default() }), 1),
        ("air presses at f15 f30 f47 f60 f75 f80 f81 f82", Box::new(|f| In { jump: [10, 15, 30, 47, 60, 75, 80, 81, 82].contains(&f), ..Default::default() }), 1),
        ("key held from takeoff for 4 s (no auto re-jump)", Box::new(|f| In { jump: (10..250).contains(&f), ..Default::default() }), 1),
        ("key mashed every other frame (re-jumps only from the ground)", Box::new(|f| In { jump: f >= 10 && f % 2 == 0, ..Default::default() }), 0),
        ("press during landing recovery (f90 after takeoff f10)", Box::new(|f| In { jump: f == 10 || f == 90, ..Default::default() }), 2),
        ("press 2 frames before touchdown (f82), must not be buffered", Box::new(|f| In { jump: f == 10 || f == 82, ..Default::default() }), 1),
        ("crouch key held, press at f10 and f50 (the jump key cancels the crouch request)", Box::new(|f| In { crouch: f < 200, jump: f == 10 || f == 50, ..Default::default() }), 1),
        ("sprint jump with air presses", Box::new(|f| In { forward: true, run: true, jump: f == 30 || f == 45 || f == 60, ..Default::default() }), 1),
    ];
    for hz in [60., 144.] {
        l.h(&format!("{hz} Hz"));
        for (name, script, want) in &scenarios {
            let (rows, a) = simulate_full(hz, (300. * hz / 60.) as usize, |f| script(((f as f32) * 60. / hz + 1e-4).floor() as usize));
            let last = rows.last().unwrap();
            let _ = writeln!(l.s, "-- {name}\n   takeoffs {} jump-clip entries {} | state log: {}", last.takeoffs, last.jump_entries, state_log(&a));
            for w in rows.windows(2).filter(|w| w[0].state != w[1].state) { let _ = writeln!(l.s, "   f{:03} t{:.3} {:?}->{:?} h{:.3} vz{:+.2} jumpkey{}", w[1].frame, w[1].t, w[0].state, w[1].state, w[1].height, w[1].vz, w[1].inp.jump as u8); }
            let clean = log_is_a_clean_walk(&a);
            let takeoffs_ok = if *want == 0 { last.takeoffs >= 2 } else { last.takeoffs == *want };
            l.check(name, clean.is_ok() && takeoffs_ok && last.jump_entries == last.takeoffs,
                format!("takeoffs {} (expected {}), jump clip entries {}, state walk {}", last.takeoffs, if *want == 0 { ">=2".to_string() } else { want.to_string() }, last.jump_entries, clean.err().unwrap_or_else(|| "clean".into())));
        }
    }
    l.finish()
}

