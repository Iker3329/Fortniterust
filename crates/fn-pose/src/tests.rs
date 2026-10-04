//! Self-contained unit tests (synthetic skeletons / clips; no game data needed).
use crate::clip::{FnClip, SampleMode};
use crate::eval::{solve_two_bone_ik, EvalOptions, PoseEvaluator};
use crate::skel::BoneTree;
use crate::{CharacterRig, Part, PoseResult};
use fn_anim::fnanim::FnBone;
use fn_anim::{BoneTransform, FnAnim, SkeletonInfo, TranslationRetarget};
use fn_animgraph::{ClipLibrary, ClipSource, Node};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;

fn chain(names: &[&str], parents: &[Option<usize>], t: Vec3) -> BoneTree {
    BoneTree::new(names.iter().map(|s| s.to_string()).collect(), parents.to_vec(), names.iter().enumerate().map(|(i, _)| (if i == 0 { Vec3::ZERO } else { t }, Quat::IDENTITY, Vec3::ONE)).collect())
}
fn skel_info(n: usize) -> SkeletonInfo {
    SkeletonInfo { bone_names: (0..n).map(|i| format!("b{i}")).collect(), parents: (0..n as i32).map(|i| i - 1).collect(), ref_pose: vec![BoneTransform::IDENTITY; n], retarget: vec![TranslationRetarget::Animation; n], retarget_sources: vec![] }
}
fn fnanim(bones: Vec<FnBone>, frames: usize, length: f32, additive: bool) -> FnAnim {
    FnAnim { name: "t".into(), skeleton: String::new(), retarget_source: "None".into(), ref_pose_seq: String::new(), num_frames: frames, length, rate_scale: 1.0, frame_rate: frames as f32 / length, additive, additive_kind: additive as u8 * 2,
        ref_pose_type: 0, ref_frame_index: 0, retarget_base: false, root_motion: false, step: false, bones, notifies: vec![], sync_markers: vec![], curves: vec![] }
}
fn bone(name: &str, mode: TranslationRetarget, source: [f32; 3], pos: Option<Vec<[f32; 3]>>, rot: Option<Vec<[f32; 4]>>) -> FnBone { FnBone { name: name.into(), retarget: mode, retarget_source: source, pos, rot, scale: None } }
fn lib(c: FnClip) -> HashMap<String, FnClip> { let mut m = HashMap::new(); m.insert("c".to_string(), c); m }
fn quat_ue(axis_deg: [f32; 3]) -> [f32; 4] { let q = Quat::from_euler(glam::EulerRot::XYZ, axis_deg[0].to_radians(), axis_deg[1].to_radians(), axis_deg[2].to_radians()); [q.x, q.y, q.z, q.w] }

#[test]
fn clip_values_are_converted_to_the_gltf_space_of_the_old_loader() {
    // UE cm (10, 20, 30) -> (x, z, y) * 0.01 m; quaternion (x, y, z, w) -> (x, z, y, -w) up to sign
    let q = quat_ue([10., 20., 30.]);
    let fa = fnanim(vec![bone("root", TranslationRetarget::Animation, [0.; 3], Some(vec![[10., 20., 30.]]), Some(vec![q]))], 1, 0.1, false);
    let c = FnClip::from_fnanim(&fa, &skel_info(1), SampleMode::Ue);
    let (p, r) = c.sample_bone("root", 0., false, (Vec3::ONE, Quat::IDENTITY)).unwrap();
    assert!((p - Vec3::new(0.1, 0.3, 0.2)).length() < 1e-6, "{p:?}");
    let want = Quat::from_xyzw(q[0], q[2], q[1], -q[3]);
    assert!(r.dot(want).abs() > 1. - 1e-6, "{r:?} vs {want:?}");
    assert!(c.sample_bone("nope", 0., false, (Vec3::ZERO, Quat::IDENTITY)).is_none() && !c.has_bone("nope") && c.has_bone("root"));
}

#[test]
fn retarget_modes_follow_the_old_fit() {
    let b = |m| bone("x", m, [0., 0., 100.], Some(vec![[0., 0., 50.]]), Some(vec![[0., 0., 0., 1.]])); // source |s| = 1 m (glTF y), animated 0.5 m
    let skel = skel_info(2);
    let bind = (Vec3::new(0., 2., 0.), Quat::IDENTITY);
    let run = |mode, additive| {
        let mut fa = fnanim(vec![bone("root", TranslationRetarget::Animation, [0.; 3], None, None), b(mode)], 1, 0.1, additive);
        fa.retarget_base = false;
        FnClip::from_fnanim(&fa, &skel, SampleMode::Ue).sample_bone("x", 0., false, bind).unwrap().0
    };
    assert!((run(TranslationRetarget::Animation, false) - Vec3::new(0., 0.5, 0.)).length() < 1e-6, "animation keeps the animated translation");
    assert!((run(TranslationRetarget::AnimationRelative, false) - Vec3::new(0., 0.5, 0.)).length() < 1e-6, "AnimationRelative stays unimplemented (= Animation)");
    assert!((run(TranslationRetarget::AnimationScaled, false) - Vec3::new(0., 1.0, 0.)).length() < 1e-6, "scaled: 0.5 * |bind| / |source| = 0.5 * 2 / 1");
    assert_eq!(run(TranslationRetarget::Skeleton, false), bind.0, "skeleton: the target's bind translation");
    assert_eq!(run(TranslationRetarget::Skeleton, true), Vec3::ZERO, "additive skeleton mode: zero delta");
}

#[test]
fn ue_sampling_uses_n_minus_1_spacing_and_legacy_the_old_n_over_length_rate() {
    let pos: Vec<[f32; 3]> = (0..5).map(|f| [f as f32 * 10., 0., 0.]).collect();
    let fa = fnanim(vec![bone("root", TranslationRetarget::Animation, [0.; 3], Some(pos), Some(vec![[0., 0., 0., 1.]]))], 5, 1.0, false);
    let bind = (Vec3::ZERO, Quat::IDENTITY);
    let ue = FnClip::from_fnanim(&fa, &skel_info(1), SampleMode::Ue);
    let leg = FnClip::from_fnanim(&fa, &skel_info(1), SampleMode::LegacyMd5);
    let x = |c: &FnClip, t, looping| c.sample_bone("root", t, looping, bind).unwrap().0.x;
    assert!((x(&ue, 0.5, false) - 0.2).abs() < 1e-6, "t/length*(N-1) = frame 2");
    assert!((x(&leg, 0.5, false) - 0.2).abs() > 1e-3 && (x(&leg, 0.5, false) - 0.25).abs() < 1e-6, "old: t * N/length = frame 2.5");
    assert!((x(&ue, 1.25, true) - x(&ue, 0.25, true)).abs() < 1e-6, "looping wraps modulo the sequence length");
    assert!((x(&ue, 3.0, false) - 0.4).abs() < 1e-6, "non-looping holds the last key");
    assert!((ClipSource::duration(&ue) - 1.0).abs() < 1e-6);
}

fn tree4() -> BoneTree { chain(&["root", "spine", "head", "eye"], &[None, Some(0), Some(1), Some(2)], Vec3::new(0., 1., 0.)) }

#[test]
fn mesh_space_additive_keeps_bones_without_a_track_riding_on_their_parent() {
    // the additive clip keys root / spine / head only; `eye` has no track (like the face bones vs the aim-offset clips)
    let delta = quat_ue([0., 0., 0.]);
    let _ = delta;
    let rot = [Quat::from_rotation_z(0.0), Quat::from_rotation_z(0.0), Quat::from_rotation_x(0.5)].map(|q| [q.x, q.y, q.z, q.w]);
    let b = |n: &str, i: usize| bone(n, TranslationRetarget::Animation, [0.; 3], Some(vec![[0.; 3]]), Some(vec![rot[i]]));
    let fa = fnanim(vec![b("root", 0), b("spine", 1), b("head", 2)], 1, 0.1, true);
    let c = FnClip::from_fnanim(&fa, &skel_info(3), SampleMode::Ue);
    let libr = lib(c);
    let tree = tree4();
    let node = Node::ApplyMeshAdditive { base: Box::new(Node::Ref), add: Box::new(Node::clip("c", 0., false)), alpha: 1. };
    let eye = tree.bone_index("eye").unwrap();
    let head = tree.bone_index("head").unwrap();
    let mesh_rot = |opts| { let ev = PoseEvaluator::new(&tree, &libr, opts); let p = ev.eval_pose(&node); (tree.mesh_rotations(&p), p) };
    let (fixed, pose) = mesh_rot(EvalOptions::default());
    let (legacy, _) = mesh_rot(EvalOptions { legacy_mesh_additive: true });
    assert!(pose[eye].1.angle_between(Quat::IDENTITY) < 1e-6, "no delta: the eye's local pose is untouched");
    assert!(fixed[eye].angle_between(fixed[head]) < 1e-5, "fixed: the eye follows the head's mesh rotation");
    assert!(fixed[head].angle_between(Quat::IDENTITY) > 0.4, "the head did rotate");
    assert!(legacy[eye].angle_between(Quat::IDENTITY) < 1e-5, "legacy: the eye stayed pinned at its old mesh orientation");
}

#[test]
fn two_bone_ik_reaches_a_reachable_effector_and_clamps_an_unreachable_one() {
    let (root, joint, end) = (Vec3::ZERO, Vec3::new(0., 0., 1.), Vec3::new(0., 0., 2.));
    let target = Vec3::new(1., 0.5, 0.8);
    let (j, e) = solve_two_bone_ik(root, joint, end, Vec3::new(0., 3., 0.), target, false, 1., 1.);
    assert!((e - target).length() < 1e-5 && ((j - root).length() - 1.).abs() < 1e-4 && ((e - j).length() - 1.).abs() < 1e-4);
    let far = Vec3::new(0., 0., 5.);
    let (_, e) = solve_two_bone_ik(root, joint, end, Vec3::Y, far, false, 1., 1.);
    assert!((e - Vec3::new(0., 0., 2.)).length() < 1e-5, "clamped to the full reach");
}

#[test]
fn bone_tree_helpers() {
    let t = tree4();
    assert_eq!(t.bone_index("HEAD"), Some(2));
    let pose: Vec<_> = (0..4).map(|i| (if i == 0 { Vec3::ZERO } else { Vec3::Y }, Quat::from_rotation_x(0.1 * i as f32))).collect();
    let c = t.component(&pose);
    for i in 1..4 { let l = t.local_of(i, &c, c[i]); assert!((l.0 - pose[i].0).length() < 1e-5 && l.1.angle_between(pose[i].1) < 1e-5); }
    // BlendDepth: spine with depth 2 -> spine 1/2, head 1, eye 1; depth 0 -> all 1; negative depth fades out
    assert_eq!(t.mask_weights(&[("spine", 2)]), vec![0., 0.5, 1., 1.]);
    assert_eq!(t.mask_weights(&[("spine", 0)]), vec![0., 1., 1., 1.]);
    let w = t.mask_weights(&[("root", -2)]); assert_eq!(w, vec![0.5, 0., 0., 0.]);
    // master IK bones appended parents-first, only the missing ones
    let mut part = chain(&["root", "pelvis"], &[None, Some(0)], Vec3::Y);
    let master = chain(&["root", "pelvis", "ik_hand_gun", "ik_hand_l"], &[None, Some(0), Some(0), Some(2)], Vec3::X);
    assert_eq!(part.append_missing_from(&master), 2);
    assert_eq!(part.names, ["root", "pelvis", "ik_hand_gun", "ik_hand_l"]);
    assert_eq!(part.parents[3], Some(2));
}

#[test]
fn skin_matrices_gpu_layout_and_palette_order() {
    let tree = chain(&["root", "a", "b"], &[None, Some(0), Some(1)], Vec3::new(0., 1., 0.));
    // palette order = joint slots (here reversed), skin = global * inverse bind
    let part = Part { name: "p".into(), joints: vec![2, 1], inverse_bind: vec![Mat4::from_translation(Vec3::new(0., -2., 0.)), Mat4::from_translation(Vec3::new(0., -1., 0.))], prims: vec![], tree };
    let rig = CharacterRig::new(vec![part]);
    let pose = rig.evaluate(&Node::Ref, &HashMap::<String, FnClip>::new());
    let r: &PoseResult = &pose.parts[0];
    assert_eq!(r.skin_matrices.len(), 2);
    assert!(r.skin_matrices.iter().all(|m| m.abs_diff_eq(Mat4::IDENTITY, 1e-6)), "the bind pose skins to identity");
    assert_eq!(r.bone_mesh.len(), 3);
    let mut moved = r.clone();
    moved.skin_matrices[1] = Mat4::from_translation(Vec3::new(1., 2., 3.));
    let bytes = moved.gpu_bytes();
    assert_eq!(bytes.len(), 2 * 64);
    // column-major: translation is the 4th column = floats 12..15 of the matrix
    let f = |i: usize| f32::from_le_bytes(bytes[64 + i * 4..64 + i * 4 + 4].try_into().unwrap());
    assert_eq!((f(12), f(13), f(14), f(15)), (1., 2., 3., 1.));
    assert_eq!(moved.gpu_matrices()[1][3], [1., 2., 3., 1.]);
}

#[test]
fn missing_clips_are_reported_and_fall_back_to_the_bind_pose() {
    let tree = tree4();
    let empty = HashMap::<String, FnClip>::new();
    let ev = PoseEvaluator::new(&tree, &empty as &dyn ClipLibrary, EvalOptions::default());
    let p = ev.eval_pose(&Node::clip("nope", 0., true));
    assert_eq!(p, tree.bind_pose());
    assert_eq!(ev.missing.borrow().as_slice(), ["nope".to_string()]);
}
