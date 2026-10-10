//! `fn-sim`: headless scenario runner and log comparator.
//!
//!   fn-sim movement-test | jump-air-test | jump-feel-test | sim-tick-test | tick-determinism | time-test | face-test [--legacy] | camera-test | camera-video | camera-prop-test [--no-prop-collision]
//!          | pickaxe-test | anim-test | dir-test | all     [--out DIR] [--assets DIR] [--frames N] [--hz N]
//!   `--hz N` = simulation rate of camera-test / pickaxe-test / anim-test / dir-test: 0 = per frame (the old FNRT_SIM_HZ=0, default here), 30 = the old exe's default tick.
//!   fn-sim compare A.txt B.txt [--tol 0.0006] [--drop FROM TO] [--skip-a N] [--skip-lines-containing TEXT]
//!
//! `--assets` points at an exported `assets` directory (default: the old prototype's, found like fn-testdata does); without clips the
//! pose columns and the pickaxe-frames / dir-frames logs are skipped. Exit code 0 = every check / comparison passed.
use fn_sim::{compare, rig::Rig, scenarios::{self, Outputs, PickMode}, series};
use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()) }
fn flag(args: &[String], name: &str) -> bool { args.iter().any(|a| a == name) }

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else { eprintln!("usage: fn-sim <scenario|compare|all> [options] (see crates/fn-sim/src/main.rs)"); std::process::exit(2) };
    if cmd == "compare" { std::process::exit(run_compare(&args)); }
    if cmd == "move-trace" {
        let (Some(plan), Some(out)) = (args.get(1), args.get(2)) else { eprintln!("usage: fn-sim move-trace PLAN OUT.csv [--fps 60] [--hz 30]"); std::process::exit(2) };
        let fps: f32 = arg(&args, "--fps").and_then(|s| s.parse().ok()).unwrap_or(60.);
        let hz: u32 = arg(&args, "--hz").and_then(|s| s.parse().ok()).unwrap_or(fn_game::SIM_HZ);
        let p = match std::fs::read_to_string(plan).map_err(|e| e.to_string()).and_then(|t| fn_sim::move_trace::Plan::parse(&t)) { Ok(p) => p, Err(e) => { eprintln!("{e}"); std::process::exit(2) } };
        let csv = fn_sim::move_trace::run(&p, fps, hz);
        if let Err(e) = std::fs::write(out, csv) { eprintln!("{e}"); std::process::exit(2) }
        println!("move-trace: {} frames at {fps} fps, sim {hz} Hz -> {out}", p.frames);
        return;
    }
    let out_dir = PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "sim-out".into()));
    let assets = arg(&args, "--assets").map(PathBuf::from).unwrap_or_else(fn_testdata::assets);
    let frames = arg(&args, "--frames").and_then(|s| s.parse().ok());
    let hz: u32 = arg(&args, "--hz").and_then(|s| s.parse().ok()).unwrap_or(0);
    let rig = if flag(&args, "--no-rig") { None } else { match Rig::load(&assets) { Ok(r) => Some(r), Err(e) => { eprintln!("note: no pose rig ({e}); bone columns skipped"); None } } };
    let names: Vec<&str> = if cmd == "all" { vec!["movement-test", "jump-air-test", "jump-feel-test", "sim-tick-test", "tick-determinism", "time-test", "face-test", "camera-test", "camera-video", "camera-prop-test", "pickaxe-test", "anim-test", "dir-test"] } else { vec![cmd] };
    let mut failed = 0;
    for n in names {
        let (outs, checks_failed): (Outputs, usize) = match n {
            "movement-test" | "jump-air-test" | "jump-feel-test" => { let (log, f) = match n { "movement-test" => series::run_movement_test(), "jump-air-test" => series::run_jump_air_test(), _ => scenarios::jump_feel_test() }; let mut o = Outputs::default(); o.add(&format!("{n}.txt"), log); (o, f) }
            "sim-tick-test" => { let (log, f) = scenarios::sim_tick_test(); let mut o = Outputs::default(); o.add("sim-tick-test.txt", log); (o, f) }
            "tick-determinism" => { let (o, ok) = scenarios::tick_determinism(8.); (o, (!ok) as usize) }
            "time-test" => (fn_sim::time_test::time_test(), 0),
            "face-test" => match fn_sim::face_test::FaceRig::load(&assets) { Ok(r) => { let mut o = fn_sim::face_test::face_test(&r, flag(&args, "--legacy")); if cmd == "all" { o.files.extend(fn_sim::face_test::face_test(&r, true).files); } (o, 0) } Err(e) => { eprintln!("face-test: {e}"); std::process::exit(2) } },
            "camera-test" => (scenarios::camera_test_hz(rig.as_ref(), hz), 0),
            "camera-video" => (scenarios::camera_video(rig.as_ref()), 0),
            "camera-prop-test" => { let mut o = scenarios::camera_prop_test(!flag(&args, "--no-prop-collision")); if cmd == "all" { o.files.extend(scenarios::camera_prop_test(false).files); } (o, 0) }
            "pickaxe-test" => (scenarios::pick_run_hz(PickMode::Pickaxe, rig.as_ref(), frames, hz), 0),
            "anim-test" => (scenarios::pick_run_hz(PickMode::Anim, rig.as_ref(), frames, hz), 0),
            "dir-test" => (scenarios::pick_run_hz(PickMode::Dir, rig.as_ref(), frames, hz), 0),
            other => { eprintln!("unknown scenario {other}"); std::process::exit(2) }
        };
        let dir = out_dir.join(n);
        match outs.write_all(&dir) { Ok(()) => println!("{n}: {} failed check(s); {} file(s) -> {}", checks_failed, outs.files.len(), dir.display()), Err(e) => { eprintln!("{n}: cannot write {}: {e}", dir.display()); std::process::exit(2) } }
        failed += checks_failed;
    }
    std::process::exit(if failed == 0 { 0 } else { 1 });
}

fn run_compare(args: &[String]) -> i32 {
    let (Some(a), Some(b)) = (args.get(1), args.get(2)) else { eprintln!("usage: fn-sim compare A B [--tol X] [--drop FROM TO] [--skip-a N]"); return 2 };
    let tol: f64 = arg(args, "--tol").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let drop = args.iter().position(|x| x == "--drop").and_then(|i| Some((args.get(i + 1)?.as_str(), args.get(i + 2)?.as_str())));
    let skip_a: usize = arg(args, "--skip-a").and_then(|s| s.parse().ok()).unwrap_or(0);
    let skip_contains = arg(args, "--skip-lines-containing");
    let (ta, tb) = match (std::fs::read_to_string(a), std::fs::read_to_string(b)) { (Ok(x), Ok(y)) => (x, y), (e1, e2) => { eprintln!("cannot read: {:?} {:?}", e1.err(), e2.err()); return 2 } };
    let filt = |t: String| match &skip_contains { Some(p) => t.lines().filter(|l| !l.contains(p.as_str())).collect::<Vec<_>>().join("\n"), None => t };
    let (ta, tb) = (filt(ta), filt(tb));
    let r = compare::compare_text(&ta, &tb, tol, drop, skip_a);
    println!("{}", r.summary());
    for (line, x, y, why) in &r.first_mismatches { println!("  line {line}: {why}\n    A: {}\n    B: {}", x.chars().take(220).collect::<String>(), y.chars().take(220).collect::<String>()); }
    if r.ok() { 0 } else { 1 }
}