//! `--face-test [--legacy]` (old face_test.rs, hand-ported): the character's head part is posed while the aim pitch sweeps +80..-80, the yaw
//! sweeps -75..75 and the camera spins fast (head tracking / the aim offset rotate the head). Per frame the log lists the head bone, the head
//! tracking angles and the largest displacement (relative to the head bone) of every face vertex (eyes, lids, brows, lips, jaw, teeth, hair)
//! and of every face bone; a rigid face reads 0. The old run rendered close-up PNGs through OpenGL; here nothing is rendered, the numbers
//! come from `fn_animgraph::SkinnedMesh` (the renderer-free `apply_pose`) and are identical to the old log line for line.
//! `legacy` = `MeshAdditiveMode::Legacy`, the pre-fix pose pipeline (bones without an additive track pinned to the mesh orientation).
use crate::scenarios::{video_sim, Outputs, DT};
use crate::video_script::Step;
use fn_animgraph::{MeshAdditiveMode, PosedMesh};
use fn_game::GameSim;
use glam::Vec3;
use std::fmt::Write as _;
use std::path::Path;

/// The head part + the prototype's clips (what `App` evaluates on `models[1]`).
pub struct FaceRig { pub head: fn_legacy::SkinnedPart, pub clips: std::collections::HashMap<String, fn_legacy::Md5Clip> }
impl FaceRig {
    pub fn load(assets: &Path) -> Result<FaceRig, String> {
        let all = fn_legacy::files(assets);
        if all.is_empty() { return Err(format!("no files below {}", assets.display())); }
        Ok(FaceRig { head: fn_legacy::load_head_model(&all)?, clips: fn_legacy::load_clip_set(&all, &fn_legacy::prototype_clip_names())? })
    }
}

type Script = fn(f32) -> (f32, f32);
/// (name, frames, script(frame fraction) -> (yaw offset deg, pitch deg)); the old run also captured PNGs every n frames.
const PHASES: [(&str, u32, Script); 3] = [
    ("pitch", 180, |t| (0., 80. - 160. * t)),         // +80 -> -80
    ("yaw", 180, |t| (-75. + 150. * t, -20.)),         // -75 -> +75 at pitch -20 (looking a little down)
    ("spin", 72, |t| (t * 1080., -35.)),               // 3 fast turns (~900 deg/s), looking down
];

/// Runs the sweeps and returns `face-test.txt` (or `face-test-legacy.txt`). `Err(summary)` never happens: the test asserts on the result.
pub fn face_test(rig: &FaceRig, legacy: bool) -> Outputs {
    let tag = if legacy { "face-test-legacy" } else { "face-test" };
    let mode = if legacy { MeshAdditiveMode::Legacy } else { MeshAdditiveMode::TrackedMask };
    let (sk, mesh) = (&rig.head.body.skeleton, &rig.head.mesh);
    let mut log = format!("Face test ({}). Units metres; dev = displacement of the skinned face vertices / bones relative to the head bone vs the first frame (rigid = 0).\n",
        if legacy { "LEGACY mesh-space additive: bones without a track pinned to mesh orientation" } else { "fixed: tracked-bone mask in mesh_additive" });
    let _ = writeln!(log, "head part skinned bones: {}", mesh.skinned_bone_names(sk).join(", "));
    // bones that must ride on the head: the head and everything below it (not the spine / neck / clavicles the neck verts blend with)
    let bones: Vec<String> = mesh.skinned_bone_names(sk).into_iter().filter(|b| !(b.starts_with("spine") || b.starts_with("neck") || b.starts_with("clavicle"))).collect();
    let mut g = GameSim::new(); g.to_spawn();
    let yaw0 = g.yaw;
    g.cam.reset(); g.anim_time = 0.;
    for _ in 0..40 { video_sim(&mut g, &Step { yaw: yaw0, pitch: 0., ..Default::default() }, DT); }
    let mut posed: Option<PosedMesh> = None; // `App::render` skips the pose update while the camera hides the pawn
    let mut reference: Vec<Vec3> = vec![]; let mut bone_ref: Vec<Vec3> = vec![];
    let (mut worst_v, mut worst_b) = (0f32, 0f32);
    let mut frame = 0u32;
    for (name, n, script) in PHASES {
        let _ = writeln!(log, "== phase {name}");
        for f in 0..n {
            let (dyaw, pitch) = script(f as f32 / (n - 1) as f32);
            video_sim(&mut g, &Step { yaw: yaw0 + dyaw, pitch, ..Default::default() }, DT);
            if !g.cam_view.hide_pawn { let pose = sk.eval_pose_with(&g.pose_tree(), &rig.clips, mode); posed = Some(mesh.pose(sk, &pose)); }
            let (snap, bpos, hp) = match &posed {
                Some(p) => (mesh.rigid_snapshot(sk, p, "head"), bones.iter().map(|b| sk.bone_in_frame(&p.globals, b, "head").unwrap_or_default()).collect::<Vec<_>>(), sk.bone_position(&p.globals, "head").unwrap_or_default()),
                None => (vec![], bones.iter().map(|_| Vec3::ZERO).collect(), Vec3::ZERO),
            };
            if reference.is_empty() { reference = snap.clone(); bone_ref = bpos.clone(); }
            let (mut dv, mut db, mut which) = (0f32, 0f32, "");
            if snap.len() == reference.len() { for (a, b) in snap.iter().zip(&reference) { dv = dv.max((*a - *b).length()); } }
            for (i, (a, b)) in bpos.iter().zip(&bone_ref).enumerate() { let d = (*a - *b).length(); if d > db { db = d; which = &bones[i]; } }
            worst_v = worst_v.max(dv); worst_b = worst_b.max(db);
            let v = &g.anim.vars;
            let _ = writeln!(log, "{name} f{f:03} ctrl yaw {:+.1} pitch {:+.1} | track yaw {:+.1} pitch {:+.1} | head bone ({:.3},{:.3},{:.3}) | face verts {} dev {:.5} m | worst face bone {} dev {:.5} m",
                dyaw, pitch, v.head_track_yaw, v.head_track_pitch, hp.x, hp.y, hp.z, snap.len(), dv, which, db);
            frame += 1;
        }
    }
    let _ = writeln!(log, "SUMMARY {tag}: largest face-vertex displacement relative to the head bone {worst_v:.5} m, largest face-bone displacement {worst_b:.5} m over {frame} frames");
    let mut out = Outputs::default(); out.add(&format!("{tag}.txt"), log); out
}

/// (largest face-vertex displacement, largest face-bone displacement) in metres from the SUMMARY line of a face-test log.
pub fn summary(log: &str) -> Option<(f32, f32)> {
    let l = log.lines().rev().find(|l| l.starts_with("SUMMARY"))?;
    let v = l.split("head bone ").nth(1)?.split(" m").next()?.parse().ok()?; let b = l.split("face-bone displacement ").nth(1)?.split(" m").next()?.parse().ok()?; Some((v, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn summary_parses_the_last_line() { assert_eq!(summary("x\nSUMMARY face-test: largest face-vertex displacement relative to the head bone 0.19519 m, largest face-bone displacement 0.20586 m over 432 frames\n"), Some((0.19519, 0.20586))); }
    #[test] fn the_fixed_pipeline_keeps_the_face_rigid_and_the_legacy_one_does_not() {
        let _g = crate::scenarios::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(rig) = FaceRig::load(&fn_testdata::assets()) else { eprintln!("SKIPPED (no pose assets)"); return };
        let (fixed, legacy) = (face_test(&rig, false), face_test(&rig, true));
        let (v, b) = summary(&fixed.files[0].1).unwrap(); assert!(v < 1e-3 && b < 1e-3, "face moved relative to the head bone: {v} {b}");
        let (lv, lb) = summary(&legacy.files[0].1).unwrap(); assert!(lv > 0.05 && lb > 0.05, "the legacy pipeline reproduces the detached face: {lv} {lb}");
    }
}
