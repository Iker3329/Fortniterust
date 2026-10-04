//! The `--face-test` of the old runtime: the head part (SK_M_MED_Head_01_ATH) under a pitch sweep +80..-80, a yaw sweep and a fast spin.
//! Per frame: head bone position, head-tracking angles, and the largest displacement (relative to the head bone, vs the first frame) of every
//! face vertex and face bone. With the tracked-bone mask a rigid face reads 0; the pre-fix mesh-space additive (`--legacy`) detaches it by 20 cm.
//! Compared line by line with the golden logs of the old exe (`testdata/old-logs/face-test/face-test[-legacy].txt`).
mod common;
use common::*;
use fn_pose::diag;
use fn_pose::scenario::{self, Scenario};
use fn_pose::{CharacterRig, EvalOptions};
use glam::Vec3;
use std::fmt::Write as _;

struct Run { lines: Vec<String>, head_pos: Vec<Vec3>, dev_v: Vec<f32>, dev_b: Vec<f32>, track: Vec<(f32, f32)>, worst_v: f32, worst_b: f32, n_verts: usize, rigid_everywhere: bool }

fn run_face(old: &Old, legacy: bool) -> Run {
    let mut rig = CharacterRig::new(vec![old.parts[1].clone()]);
    rig.opts = EvalOptions { legacy_mesh_additive: legacy };
    let head = &rig.parts[0];
    let skinned = head.skinned_bone_names();
    let bones: Vec<String> = skinned.iter().filter(|b| !(b.starts_with("spine") || b.starts_with("neck") || b.starts_with("clavicle"))).cloned().collect();
    let mut r = Run { lines: vec![], head_pos: vec![], dev_v: vec![], dev_b: vec![], track: vec![], worst_v: 0., worst_b: 0., n_verts: 0, rigid_everywhere: true };
    let (mut reference, mut bone_ref): (Vec<Vec3>, Vec<Vec3>) = (vec![], vec![]);
    let _l = lock();
    scenario::run(Scenario::Face, None, &mut |ctx| {
        let pose = rig.evaluate(&ctx.tree, &old.md5);
        let p = &pose.parts[0];
        let skin = rig.parts[0].skin_cpu(&p.skin_matrices);
        let snap = diag::rigid_snapshot(&rig.parts[0], &p.bone_mesh, &skin, "head");
        let bpos: Vec<Vec3> = bones.iter().map(|b| diag::bone_in_frame(&rig.parts[0], &p.bone_mesh, b, "head").unwrap_or_default()).collect();
        if reference.is_empty() { reference = snap.clone(); bone_ref = bpos.clone(); }
        let (mut dv, mut db, mut which) = (0f32, 0f32, "");
        if snap.len() == reference.len() { for (a, b) in snap.iter().zip(&reference) { dv = dv.max((*a - *b).length()); } }
        for (i, (a, b)) in bpos.iter().zip(&bone_ref).enumerate() { let d = (*a - *b).length(); if d > db { db = d; which = &bones[i]; } }
        r.worst_v = r.worst_v.max(dv); r.worst_b = r.worst_b.max(db);
        let hp = rig.parts[0].tree.bone_index("head").map(|h| p.bone_mesh[h].transform_point3(Vec3::ZERO)).unwrap_or_default();
        let v = &ctx.anim.vars;
        let (name, dyaw, pitch) = ctx.face.unwrap();
        let mut l = String::new();
        let _ = write!(l, "{name} f{:03} ctrl yaw {:+.1} pitch {:+.1} | track yaw {:+.1} pitch {:+.1} | head bone ({:.3},{:.3},{:.3}) | face verts {} dev {:.5} m | worst face bone {} dev {:.5} m",
            ctx.frame - match name { "pitch" => 0, "yaw" => 180, _ => 360 }, dyaw, pitch, v.head_track_yaw, v.head_track_pitch, hp.x, hp.y, hp.z, snap.len(), dv, which, db);
        r.lines.push(l); r.head_pos.push(hp); r.dev_v.push(dv); r.dev_b.push(db); r.track.push((v.head_track_yaw, v.head_track_pitch)); r.n_verts = snap.len();
    });
    r.rigid_everywhere = r.worst_v < 1e-3 && r.worst_b < 1e-3;
    r
}

/// (head bone, track yaw, track pitch, face verts, dev, worst-bone dev) parsed from a golden line.
fn parse(l: &str) -> Option<(Vec3, f32, f32, usize, f32, f32)> {
    let num = |s: &str| s.trim().parse::<f32>().ok();
    let track = l.split("track yaw ").nth(1)?; let (ty, rest) = track.split_once(" pitch ")?; let (tp, _) = rest.split_once(" |")?;
    let hb = l.split("head bone (").nth(1)?; let (hb, _) = hb.split_once(')')?; let h: Vec<f32> = hb.split(',').filter_map(|x| x.parse().ok()).collect();
    let fv = l.split("face verts ").nth(1)?; let (n, rest) = fv.split_once(" dev ")?; let (dv, _) = rest.split_once(" m")?;
    let wb = l.split("worst face bone").nth(1)?; let (_, d) = wb.rsplit_once(" dev ")?; let (d, _) = d.split_once(" m")?;
    Some((Vec3::new(h[0], h[1], h[2]), num(ty)?, num(tp)?, n.trim().parse().ok()?, num(dv)?, num(d)?))
}

fn compare(old: &Old, legacy: bool) {
    let name = if legacy { "face-test/face-test-legacy.txt" } else { "face-test/face-test.txt" };
    let gold = golden(name);
    let glines: Vec<&str> = gold.lines().filter(|l| l.starts_with("pitch f") || l.starts_with("yaw f") || l.starts_with("spin f")).collect();
    let r = run_face(old, legacy);
    assert_eq!(glines.len(), r.lines.len(), "frame count");
    let (mut e_head, mut e_track, mut e_dv, mut e_db, mut text_miss, mut n_miss_bone_name) = (0f32, 0f32, 0f32, 0f32, 0u32, 0u32);
    for (g, m) in glines.iter().zip(&r.lines) {
        let (gp, gy, gpi, gn, gdv, gdb) = parse(g).expect("golden line");
        let (mp, my, mpi, mn, mdv, mdb) = parse(m).expect("own line");
        e_head = e_head.max((gp - mp).abs().max_element()); e_track = e_track.max((gy - my).abs()).max((gpi - mpi).abs());
        e_dv = e_dv.max((gdv - mdv).abs()); e_db = e_db.max((gdb - mdb).abs());
        assert_eq!(gn, mn, "face vertex count");
        if g != m { text_miss += 1; if g.split("worst face bone").nth(1).and_then(|s| s.split(" dev ").next()) != m.split("worst face bone").nth(1).and_then(|s| s.split(" dev ").next()) { n_miss_bone_name += 1; } }
    }
    println!("== face-test{}: {} frames, face vertices {}", if legacy { " --legacy (pre-fix mesh additive)" } else { "" }, r.lines.len(), r.n_verts);
    println!("  head bone position max error {e_head:.2e} m (printed to 1e-3), track angles {e_track:.2e} deg (printed to 0.1)");
    println!("  face-vertex rigidity (dev vs head bone): ours max {:.5} m; golden max {:.5} m; max |diff| per frame {e_dv:.2e} m", r.worst_v, glines.iter().filter_map(|l| parse(l)).map(|x| x.4).fold(0., f32::max));
    println!("  face-bone rigidity: ours max {:.5} m; max |diff| per frame {e_db:.2e} m", r.worst_b);
    println!("  lines identical to the old log: {} of {} ({} differ in the worst-bone name only because it is chosen among bones at float noise 0.00000)", r.lines.len() as u32 - text_miss, r.lines.len(), n_miss_bone_name);
    assert!(e_head <= 1.1e-3, "head bone position");
    assert!(e_track <= 0.11, "head track angles");
    assert!(e_dv <= if legacy { 2e-4 } else { 2e-5 } && e_db <= if legacy { 2e-4 } else { 2e-5 }, "deviation numbers: {e_dv} {e_db}");
    if legacy { assert!(r.worst_v > 0.15, "legacy additive must detach the face ({})", r.worst_v); } else { assert!(r.worst_v < 1e-4 && r.worst_b < 1e-4, "the face must stay rigid: {} {}", r.worst_v, r.worst_b); }
}

#[test] fn face_vertices_stay_rigid_and_match_the_old_log() { let Some(old) = Old::load() else { return }; compare(&old, false); }
#[test] fn legacy_mesh_additive_reproduces_the_detached_face_of_the_old_log() { let Some(old) = Old::load() else { return }; compare(&old, true); }
