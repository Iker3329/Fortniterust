//! Per-frame bone positions of the fn-pose evaluator against the golden logs of the OLD executable (`testdata/old-logs`), for the scripted
//! scenarios driven by `fn_game::GameSim` + `fn_animgraph`:
//!
//! * `--dir-test`     dir-frames.txt (foot_l, foot_r, pelvis; 3 decimals) and pickaxe-frames.txt (weapon_r, hand_l, hand_r, pelvis; 4 decimals)
//! * `--pickaxe-test` pickaxe-frames.txt + the capture lines of pickaxe-test.txt (grip distances, FK/IK gaps: they exercise the attach
//!                    socket, TwoBoneIK and HandIKRetargeting)
//!
//! The golden logs were regenerated from the old exe AFTER its engine-exact clip sampling fix, so both clip sources (old md5anim loader, fn-anim packages with `SampleMode::Ue`) are exact.
//! The logs print rounded values, so the comparison is (a) the printed text of OUR value equals the golden text, (b) the numeric difference
//! (<= half a print unit when (a) holds). Run with `cargo test -p fn-pose --release -- --nocapture` to see the numbers.
mod common;
use common::*;
use fn_pose::clip::SampleMode;
use fn_pose::diag;
use fn_pose::scenario::{self, Scenario};
use fn_pose::{CharacterRig, EvalOptions};
use glam::Vec3;

fn parse_floats(s: &str) -> Vec<f32> { s.split_whitespace().filter_map(|x| x.parse().ok()).collect() }

struct Golden { dir: Vec<[f32; 9]>, dir_text: Vec<String>, pick: Vec<(u32, [f32; 12])>, pick_text: Vec<String> }
fn load_golden(dir_test: bool) -> Golden {
    let (mut dir, mut dir_text) = (vec![], vec![]);
    if dir_test {
        for l in golden("dir-test/dir-frames.txt").lines() {
            // f N yaw .. vel .. | foot_l x y z foot_r x y z pelvis x y z | ...
            let mid = l.split('|').nth(1).unwrap_or("");
            let v = parse_floats(&mid.replace("foot_l", "").replace("foot_r", "").replace("pelvis", ""));
            let mut a = [0f32; 9]; a.copy_from_slice(&v[..9]); dir.push(a); dir_text.push(mid.trim().to_owned());
        }
    }
    let name = if dir_test { "dir-test/pickaxe-frames.txt" } else { "pickaxe-test/pickaxe-frames.txt" };
    let (mut pick, mut pick_text) = (vec![], vec![]);
    for l in golden(name).lines() {
        let v = parse_floats(l);
        let mut a = [0f32; 12]; a.copy_from_slice(&v[1..13]); pick.push((v[0] as u32, a)); pick_text.push(l.to_owned());
    }
    Golden { dir, dir_text, pick, pick_text }
}

fn run_against_logs(scn: Scenario, use_fnclips: bool) {
    let Some(old) = Old::load() else { return };
    let dir_test = scn == Scenario::Dir;
    let g = load_golden(dir_test);
    let fn_store = if use_fnclips { match package_clips(SampleMode::Ue) { Some(s) => Some(s.0), None => return } } else { None };
    let lib: &dyn fn_animgraph::ClipLibrary = match &fn_store { Some(s) => s, None => &old.md5 };
    let mut rig = CharacterRig::new(vec![old.parts[0].clone()]);
    rig.opts = EvalOptions::default();
    let body = &rig.parts[0];
    let bone = |p: &fn_pose::PoseResult, n: &str| p.bone_position(body, n).unwrap();
    let (mut e_pick, mut e_dir) = (Errs::default(), Errs::default());
    let (mut text_miss_pick, mut text_miss_dir, mut frames) = (0u32, 0u32, 0u32);
    let _l = lock();
    scenario::run(scn, None, &mut |ctx| {
        let pose = rig.evaluate(&ctx.tree, lib);
        let p = &pose.parts[0];
        let f = ctx.frame as usize;
        frames += 1;
        // pickaxe-frames.txt: weapon_r hand_l hand_r pelvis
        let mine = [bone(p, "weapon_r"), bone(p, "hand_l"), bone(p, "hand_r"), bone(p, "pelvis")];
        let want = &g.pick[f].1;
        let names = ["weapon_r", "hand_l", "hand_r", "pelvis"];
        let mut text = format!("{}", f);
        for (k, m) in mine.iter().enumerate() {
            e_pick.add(names[k], max_abs(*m, Vec3::new(want[3 * k], want[3 * k + 1], want[3 * k + 2])), ctx.frame);
            text += &format!(" {:.4} {:.4} {:.4}", m.x, m.y, m.z);
        }
        if text != g.pick_text[f] { text_miss_pick += 1; }
        if dir_test {
            let (fl, fr, pv) = (bone(p, "foot_l"), bone(p, "foot_r"), bone(p, "pelvis"));
            let w = &g.dir[f];
            e_dir.add("foot_l", max_abs(fl, Vec3::new(w[0], w[1], w[2])), ctx.frame);
            e_dir.add("foot_r", max_abs(fr, Vec3::new(w[3], w[4], w[5])), ctx.frame);
            e_dir.add("pelvis", max_abs(pv, Vec3::new(w[6], w[7], w[8])), ctx.frame);
            let t = format!("foot_l {:.3} {:.3} {:.3} foot_r {:.3} {:.3} {:.3} pelvis {:.3} {:.3} {:.3}", fl.x, fl.y, fl.z, fr.x, fr.y, fr.z, pv.x, pv.y, pv.z);
            if !g.dir_text[f].starts_with(&t) { text_miss_dir += 1; }
        }
    });
    let src = if use_fnclips { "fn-anim clips (decoded packages, exact UE sampling)" } else { "old md5anim clips" };
    println!("== {} with {src}: {frames} frames", scn.name());
    e_pick.report("pickaxe-frames");
    println!("  pickaxe-frames printed-text mismatches: {text_miss_pick} of {frames}");
    if dir_test { e_dir.report("dir-frames"); println!("  dir-frames printed-text mismatches: {text_miss_dir} of {frames}"); }
    assert_eq!(g.pick.len() as u32, frames, "frame count");
    // half a print unit (4 decimals) plus the f32 noise of the clip source
    // the md5 source reproduces the printed log exactly (half a print unit + f32 noise); the decoded packages differ from the UEViewer export by its own key noise
    // (worst at the last frames: 2.3e-4 m at the held last key of the end of the series, i.e. a quarter of a millimetre)
    let tol = if use_fnclips { 3e-4 } else { 6e-5 };
    assert!(e_pick.max() < tol, "pickaxe-frames max error {}", e_pick.max());
    if dir_test { assert!(e_dir.max() < 6e-4, "dir-frames max error {}", e_dir.max()); }
    if !use_fnclips { assert_eq!(text_miss_pick + text_miss_dir, 0, "the md5 source must print exactly the old log"); }
}

#[test] fn dir_test_matches_the_old_logs_with_the_md5_source() { run_against_logs(Scenario::Dir, false); }
#[test] fn pickaxe_test_matches_the_old_logs_with_the_md5_source() { run_against_logs(Scenario::Pickaxe, false); }
#[test] fn dir_test_matches_the_old_logs_with_fn_anim_clips() { run_against_logs(Scenario::Dir, true); }
#[test] fn pickaxe_test_matches_the_old_logs_with_fn_anim_clips() { run_against_logs(Scenario::Pickaxe, true); }

/// The capture lines of pickaxe-test.txt: grip distances to the SK_Pick handle (attach socket = weapon_r + RightHand yaw), the hands'
/// positions along the handle and the TwoBoneIK diagnostics (FK-IK gap before the solve, residual after it).
fn captures_match(use_fnclips: bool) {
    let Some(old) = Old::load() else { return };
    let fn_store = if use_fnclips { match package_clips(SampleMode::Ue) { Some(s) => Some(s.0), None => return } } else { None };
    let lib: &dyn fn_animgraph::ClipLibrary = match &fn_store { Some(s) => s, None => &old.md5 };
    // body + head like the old runtime: the logged IK diagnostics are those of the LAST part evaluated (the head)
    let rig = CharacterRig::new(vec![old.parts[0].clone(), old.parts[1].clone()]);
    let pick = fn_pose::loader::old_tree_pick_positions(&old.idx).unwrap();
    let axis = diag::handle_axis_xz(&pick);
    let gold: Vec<String> = golden("pickaxe-test/pickaxe-test.txt").lines().filter(|l| !l.contains("pick-size")).map(str::to_owned).collect();
    let _l = lock();
    let (mut n, mut worst) = (0, 0f32);
    let mut bad = vec![];
    scenario::run(Scenario::Pickaxe, None, &mut |ctx| {
        let Some(name) = ctx.capture else { return };
        let Some(line) = gold.iter().find(|l| l.starts_with(&format!("{name} "))) else { return };
        let pose = rig.evaluate(&ctx.tree, lib);
        let p = &pose.parts[0];
        let body = &rig.parts[0];
        let attach = pose.socket("RightHand").expect("RightHand socket");
        let bone = |b: &str| p.bone_position(body, b);
        let gr = diag::grip_axis_distance(attach, axis, diag::grip_center("r", bone).unwrap());
        let gl = diag::grip_axis_distance(attach, axis, diag::grip_center("l", bone).unwrap());
        let (ar, al, butt) = diag::hand_along_handle(bone, &pick, attach).unwrap();
        let s = pose.ik_stats;
        let mine = format!("grip_r {gr:.3}m grip_l {gl:.3}m along_r {ar:.3} along_l {al:.3} (butt {butt:.3} head~0.30) ik[fk-ik gap r {:.3} l {:.3}; residual r {:.4} l {:.4}]", s[0], s[1], s[2], s[3]);
        let goldpart = line.split("grip_r ").nth(1).and_then(|r| r.split(" height").next()).map(|r| format!("grip_r {r}")).unwrap_or_default();
        n += 1;
        // numeric deltas of the printed grip values
        let nums = |t: &str| { let mut v = vec![]; let mut cur = String::new(); for c in t.chars().chain(std::iter::once(' ')) { if c.is_ascii_digit() || c == '.' || c == '-' { cur.push(c); } else { if let Ok(x) = cur.parse::<f32>() { v.push(x); } cur.clear(); } } v };
        let (a, b) = (nums(&mine), nums(&goldpart));
        if a.len() == b.len() { for (x, y) in a.iter().zip(&b) { worst = worst.max((x - y).abs()); } }
        if mine != goldpart { bad.push(format!("{name}\n   mine {mine}\n   gold {goldpart}")); }
    });
    println!("== pickaxe-test capture lines ({}): {n} compared, {} printed-text mismatches, worst printed-value difference {worst:.4}", if use_fnclips { "fn-anim clips" } else { "md5 clips" }, bad.len());
    for b in bad.iter().take(6) { println!("{b}"); }
    assert!(n >= 20, "captures found: {n}");
    assert!(worst <= 0.0011, "grip / IK numbers differ by {worst}");
    if !use_fnclips { assert!(bad.is_empty(), "{} capture lines differ", bad.len()); }
}
#[test] fn pickaxe_captures_grip_ik_and_attach_socket_match_with_md5_clips() { captures_match(false); }
#[test] fn pickaxe_captures_grip_ik_and_attach_socket_match_with_fn_anim_clips() { captures_match(true); }
