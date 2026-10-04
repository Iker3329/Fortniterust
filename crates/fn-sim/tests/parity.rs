//! Numeric parity of the new crates against the OLD prototype executable: every scripted scenario is re-run headlessly and its log must equal
//! the golden log the old exe wrote (testdata/old-logs = per-frame mode `FNRT_SIM_HZ=0`, testdata/old-logs-30hz = the exe's default fixed 30 Hz tick, see the
//! README next to them). Tolerance 0: the lines must be identical. Tests that need the exported
//! clip/skeleton assets (pose columns) skip with a note when the old tree's `assets` folder is not found (set FN_OLD_ROOT).
use fn_sim::compare::compare_text;
use fn_sim::rig::Rig;
use fn_sim::scenarios::{self, Outputs, PickMode};
use fn_sim::series;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(()); // the scripted scenarios flip process-wide switches (pick script, prop collision)
fn lock() -> MutexGuard<'static, ()> { LOCK.lock().unwrap_or_else(|e| e.into_inner()) }
fn golden(rel: &str) -> String { golden_in("old-logs", rel) }
fn golden30(rel: &str) -> String { golden_in("old-logs-30hz", rel) }
fn golden_in(dir: &str, rel: &str) -> String { let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "testdata", dir, rel].iter().collect(); std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("golden log {} missing: {e}", p.display())) }
fn file<'a>(o: &'a Outputs, name: &str) -> &'a str { &o.files.iter().find(|f| f.0 == name).unwrap_or_else(|| panic!("{name} not produced")).1 }
fn rig() -> Option<Rig> { match Rig::load(&fn_testdata::assets()) { Ok(r) => Some(r), Err(e) => { eprintln!("SKIPPED (no pose assets): {e}"); None } } }
fn same(what: &str, golden: &str, ours: &str, drop: Option<(&str, &str)>, skip_contains: Option<&str>) {
    let f = |t: &str| match skip_contains { Some(p) => t.lines().filter(|l| !l.contains(p)).collect::<Vec<_>>().join("\n"), None => t.to_owned() };
    let r = compare_text(&f(golden), &f(ours), 0., drop, 0);
    assert!(r.ok() && r.exact == r.compared, "{what}: {}\n{:#?}", r.summary(), r.first_mismatches);
}

/// The old exe's own `--movement-test` reports ONE failed check since the 1.8 verification audit made DirectionalSpeedMultiplierCurve a step curve: its series still expects the old
/// linearly interpolated strafe speed (410 * 0.725 = 297.2) while the pawn now strafes at 410 * 0.8 = 328 cm/s (the fn-movement unit tests pin 328). The Rust port is verbatim, so
/// it fails the same line; the parity requirement (identical log) holds, and this test pins that the ONLY failure is that stale expectation.
#[test] fn movement_test_log_equals_the_old_exe() {
    let _g = lock(); let (log, failed) = series::run_movement_test();
    let stale = log.lines().filter(|l| l.starts_with("[FAIL]")).collect::<Vec<_>>();
    assert_eq!(stale, ["[FAIL] run strafe 90 deg: 328.0 cm/s (expected 297.2)"], "only the stale strafe expectation of the old series may fail");
    assert_eq!(failed, 1);
    same("movement-test", &golden("movement-test/movement-test.txt"), &log, None, None);
}
#[test] fn sim_tick_test_log_equals_the_old_exe() { let _g = lock(); let (log, failed) = scenarios::sim_tick_test(); assert_eq!(failed, 0); same("sim-tick-test", &golden("sim-tick-test/sim-tick-test.txt"), &log, None, None); }
#[test] fn jump_air_test_log_equals_the_old_exe() { let _g = lock(); let (log, failed) = series::run_jump_air_test(); assert_eq!(failed, 0); same("jump-air-test", &golden("jump-air-test/jump-air-test.txt"), &log, None, None); }
#[test] fn jump_feel_test_log_equals_the_old_exe() { let _g = lock(); let (log, failed) = scenarios::jump_feel_test(); assert_eq!(failed, 0); same("jump-feel-test", &golden("jump-feel-test/jump-feel-test.txt"), &log, None, None); }
#[test] fn face_test_logs_equal_the_old_exe_fixed_and_legacy_pipeline() {
    let _g = lock();
    let rig = match fn_sim::face_test::FaceRig::load(&fn_testdata::assets()) { Ok(r) => r, Err(e) => { eprintln!("SKIPPED (no pose assets): {e}"); return } };
    same("face-test", &golden("face-test/face-test.txt"), file(&fn_sim::face_test::face_test(&rig, false), "face-test.txt"), None, None);
    same("face-test-legacy", &golden("face-test/face-test-legacy.txt"), file(&fn_sim::face_test::face_test(&rig, true), "face-test-legacy.txt"), None, None);
}
#[test] fn camera_test_log_equals_the_old_exe() { let _g = lock(); same("camera-test", &golden("camera-test/camera-test.txt"), file(&scenarios::camera_test(None), "camera-test.txt"), None, None); }
#[test] fn camera_video_log_equals_the_old_exe() {
    let _g = lock();
    // the old log appends `size WxH` of the rendered window to every frame line
    let g = golden("camera-video/camera-video-v2.txt"); let g: String = g.lines().map(|l| l.split(" size ").next().unwrap().to_owned()).collect::<Vec<_>>().join("\n");
    same("camera-video", &g, file(&scenarios::camera_video(None), "camera-video-v2.txt"), None, None);
}
#[test] fn camera_prop_collision_logs_equal_the_old_exe_with_and_without_collision() {
    let _g = lock();
    same("camprop-after", &golden("camera-prop-test/camprop-after.txt"), file(&scenarios::camera_prop_test(true), "camprop-after.txt"), None, None);
    same("camprop-before", &golden("camera-prop-test/camprop-before.txt"), file(&scenarios::camera_prop_test(false), "camprop-before.txt"), None, None);
}
#[test] fn dir_test_logs_equal_the_old_exe_frame_by_frame_including_bones_and_graph_state() {
    let _g = lock(); let Some(rig) = rig() else { return };
    let o = scenarios::pick_run(PickMode::Dir, Some(&rig), None);
    same("dir-frames", &golden("dir-test/dir-frames.txt"), file(&o, "dir-frames.txt"), None, None);
    same("dir pickaxe-frames", &golden("dir-test/pickaxe-frames.txt"), file(&o, "pickaxe-frames.txt"), None, None);
    same("dir captures", &golden("dir-test/pickaxe-test.txt"), file(&o, "pickaxe-test.txt"), Some(("grip_r", "height")), Some("pick-size"));
}
#[test] fn pickaxe_test_logs_equal_the_old_exe_frame_by_frame_including_swing_and_equip() {
    let _g = lock(); let Some(rig) = rig() else { return };
    let o = scenarios::pick_run(PickMode::Pickaxe, Some(&rig), None);
    same("pickaxe-frames", &golden("pickaxe-test/pickaxe-frames.txt"), file(&o, "pickaxe-frames.txt"), None, None);
    same("pickaxe captures", &golden("pickaxe-test/pickaxe-test.txt"), file(&o, "pickaxe-test.txt"), Some(("grip_r", "height")), Some("pick-size"));
}
#[test] fn pose_hash_log_is_deterministic_across_runs() {
    let _g = lock(); let Some(rig) = rig() else { return };
    let a = scenarios::pick_run(PickMode::Anim, Some(&rig), Some(900)); let b = scenarios::pick_run(PickMode::Anim, Some(&rig), Some(900));
    assert_eq!(file(&a, "anim-test-frames.txt"), file(&b, "anim-test-frames.txt"));
    assert!(file(&a, "anim-test-frames.txt").lines().all(|l| !l.ends_with("pose -")), "pose hashes present");
}
// ---- the old exe's DEFAULT mode: the fixed 30 Hz tick (FNRT_SIM_HZ unset). 60 fps scripted frames = one tick every other frame, poses blended by alpha ----
#[test] fn camera_test_log_at_30_hz_equals_the_old_exe() { let _g = lock(); same("camera-test 30 Hz", &golden30("camera-test/camera-test.txt"), file(&scenarios::camera_test_hz(None, 30), "camera-test.txt"), None, None); }
#[test] fn dir_test_logs_at_30_hz_equal_the_old_exe_frame_by_frame_including_interpolated_bones() {
    let _g = lock(); let Some(rig) = rig() else { return };
    let o = scenarios::pick_run_hz(PickMode::Dir, Some(&rig), None, 30);
    same("dir-frames 30 Hz", &golden30("dir-test/dir-frames.txt"), file(&o, "dir-frames.txt"), None, None);
    same("dir pickaxe-frames 30 Hz", &golden30("dir-test/pickaxe-frames.txt"), file(&o, "pickaxe-frames.txt"), None, None);
    same("dir captures 30 Hz", &golden30("dir-test/pickaxe-test.txt"), file(&o, "pickaxe-test.txt"), Some(("grip_r", "height")), Some("pick-size"));
}
#[test] fn pickaxe_test_logs_at_30_hz_equal_the_old_exe_frame_by_frame_including_swing_and_equip() {
    let _g = lock(); let Some(rig) = rig() else { return };
    let o = scenarios::pick_run_hz(PickMode::Pickaxe, Some(&rig), None, 30);
    same("pickaxe-frames 30 Hz", &golden30("pickaxe-test/pickaxe-frames.txt"), file(&o, "pickaxe-frames.txt"), None, None);
    same("pickaxe captures 30 Hz", &golden30("pickaxe-test/pickaxe-test.txt"), file(&o, "pickaxe-test.txt"), Some(("grip_r", "height")), Some("pick-size"));
}