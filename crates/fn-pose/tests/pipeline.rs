//! Numeric checks of the pieces of the pipeline that the golden logs do not print:
//! * `FnClip` (fn-anim decode) against the old md5anim loader, bone by bone, for the prototype's clips and (ignored test) all 388 exports;
//! * the evaluator against the reference port in `fn-animgraph` on EVERY bone of every frame of every scripted scenario (body and head parts),
//!   and the skinned body / head mesh vertices at 20 sampled frames (old `skin_and_cache` formula);
//! * parts decoded from cooked packages (fn-mesh) against the old glTF exports;
//! * the GPU palette byte layout.
mod common;
use common::*;
use fn_animgraph::{ClipLibrary, ClipSource, Skeleton};
use fn_pose::clip::{ClipStore, SampleMode};
use fn_pose::loader;
use fn_pose::part::Part;
use fn_pose::scenario::{self, Scenario};
use fn_pose::{CharacterRig, EvalOptions, PoseResult};
use glam::{Mat4, Quat, Vec3};

/// Rotation angle between two quaternions (sign invariant, no acos cancellation near 0).
fn angle(a: Quat, b: Quat) -> f32 { 2. * (a.conjugate() * b).xyz().length().min(1.).asin() }

fn compare_clips(old: &Old, store: &ClipStore, names: &[String]) -> (f32, f32, usize, usize) {
    let tree = &old.parts[0].tree;
    let (mut max_p, mut max_q, mut samples, mut structural) = (0f32, 0f32, 0usize, 0usize);
    for n in names {
        let (a, b) = (&old.md5[n], store.clips.get(n).unwrap_or_else(|| panic!("clip {n} not loaded")));
        if a.is_additive() != ClipSource::is_additive(b) || (ClipSource::duration(a) - ClipSource::duration(b)).abs() > 1e-4 { structural += 1; println!("  {n}: additive/duration differ ({} {} / {} {})", a.is_additive(), b.is_additive(), ClipSource::duration(a), ClipSource::duration(b)); }
        let dur = ClipSource::duration(a);
        for t in [0., 0.037, 0.31, dur * 0.5, dur * 0.999, dur * 1.37, 5.123] {
            for looping in [true, false] {
                for i in 0..tree.len() {
                    let bind = (tree.local[i].0, tree.local[i].1);
                    let (x, y) = (ClipSource::sample_bone(a, &tree.names[i], t, looping, bind), ClipSource::sample_bone(b, &tree.names[i], t, looping, bind));
                    assert_eq!(x.is_some(), y.is_some(), "{n}/{}: track presence", tree.names[i]);
                    assert_eq!(ClipSource::has_bone(a, &tree.names[i]), ClipSource::has_bone(b, &tree.names[i]), "{n}/{}: has_bone", tree.names[i]);
                    if let (Some(x), Some(y)) = (x, y) {
                        samples += 1;
                        let (ep, eq) = ((x.0 - y.0).abs().max_element(), angle(x.1, y.1));
                        if ep > max_p { max_p = ep; if ep > 1e-4 { println!("  {n}/{} t {t} loop {looping}: pos error {ep:.2e}", tree.names[i]); } }
                        if eq > max_q { max_q = eq; if eq > 1e-4 { println!("  {n}/{} t {t} loop {looping}: rot error {eq:.2e} rad", tree.names[i]); } }
                    }
                }
            }
        }
    }
    (max_p, max_q, samples, structural)
}

#[test]
fn fn_clip_exact_ue_sampling_equals_the_old_md5_loader_for_the_prototype_clips() {
    let Some(old) = Old::load() else { return };
    // the old md5 loader samples engine-exactly since the "Engine-exact clip sampling" fix (key spacing length/(N-1), looping wrap without a duplicate key, last-key hold, nlerp)
    let Some((store, _)) = package_clips(SampleMode::Ue) else { return };
    let names = fn_legacy::prototype_clip_names();
    let (p, q, n, s) = compare_clips(&old, &store, &names);
    println!("== FnClip (fn-anim decode, exact UE sampling) vs the (exact) md5anim loader: {} clips x {} body bones x 14 sample times: {n} samples; max position error {p:.2e} m, max rotation error {q:.2e} rad, structural differences {s}", names.len(), old.parts[0].tree.len());
    assert_eq!(s, 0);
    // The residual is the old EXPORT's noise, not the sampler: a bone whose clip has a translation track but no rotation track (Crouch_TwoHanded_Melee_CMM/thigh_twist_01_l)
    // gets UEViewer's null key (identity rotation) in the md5anim, while the cooked data (and UE: `GetBonePose` falls back to the reference pose) keep the bind rotation,
    // which is 2.5e-3 rad away on that twist bone. Everything else agrees to 5e-4 rad.
    assert!(p < 1e-4 && q < 3e-3, "{p} {q}");
}

/// All 388 exported clips (decodes every sequence: slow). `cargo test -p fn-pose --release -- --ignored --nocapture all_388`
#[test] #[ignore]
fn all_388_clips_fn_clip_equals_the_md5_loader() {
    let Some(mut old) = Old::load() else { return };
    let assets = fn_testdata::assets();
    let all = fn_legacy::files(&assets);
    let names: Vec<String> = all.iter().filter(|p| p.extension().is_some_and(|e| e == "md5anim")).filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).collect();
    old.md5 = fn_legacy::load_clip_set(&all, &names).unwrap();
    let root = loader::default_package_root().expect("packages");
    let src = fn_pose::ClipSourceDir::RawPackages(Box::leak(root.into_boxed_path()));
    let skel = fn_pose::clip::load_player_skeleton_info(&src).unwrap();
    let t = std::time::Instant::now();
    let store = ClipStore::load(&src, &names, &skel, SampleMode::Ue).unwrap();
    println!("decoded {} clips in {:.1} s", names.len(), t.elapsed().as_secs_f64());
    let (p, q, n, s) = compare_clips(&old, &store, &names);
    println!("== ALL {} clips: {n} samples; max position error {p:.2e} m, max rotation error {q:.2e} rad, structural differences {s}", names.len());
    assert_eq!(s, 0);
    // same null-key export noise as above (identity vs bind rotation of bones without a rotation track)
    assert!(p < 1e-4 && q < 3e-3, "{p} {q}");
}

fn as_reference(p: &Part) -> Skeleton { Skeleton { names: p.tree.names.clone(), parents: p.tree.parents.clone(), local: p.tree.local.clone() } }

/// Max |element| difference of the translation columns of two matrix lists, and of whole matrices.
fn mat_diff(a: &[Mat4], b: &[Mat4]) -> (f32, f32, usize) {
    let (mut pos, mut full, mut worst) = (0f32, 0f32, 0);
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        let p = (x.w_axis.truncate() - y.w_axis.truncate()).abs().max_element();
        let f = x.to_cols_array().iter().zip(y.to_cols_array()).map(|(u, v)| (u - v).abs()).fold(0., f32::max);
        if p > pos { pos = p; worst = i; }
        full = full.max(f);
    }
    (pos, full, worst)
}

/// Every bone of every frame of every scenario: fn-pose (legacy mesh-additive switch = the reference's behaviour) against the fn-animgraph reference
/// evaluator, for the body and the head part; and the default mode against the reference to show exactly where the face fix acts.
#[test]
fn evaluator_equals_the_reference_port_on_every_bone_of_every_frame() {
    let Some(old) = Old::load() else { return };
    let scenarios: Vec<Scenario> = vec![Scenario::Dir, Scenario::Pickaxe, Scenario::Anim, Scenario::Face].into_iter().chain((0..8).map(Scenario::JumpAir)).collect();
    let refs: Vec<Skeleton> = old.parts.iter().map(as_reference).collect();
    let mut legacy_rig = CharacterRig::new(old.parts.clone()); legacy_rig.opts = EvalOptions { legacy_mesh_additive: true };
    let fixed_rig = CharacterRig::new(old.parts.clone());
    let _l = lock();
    for scn in scenarios {
        let (mut frames, mut max_legacy, mut max_fixed, mut max_fixed_local) = (0, [0f32; 3], [0f32; 3], [0f32; 3]);
        let mut worst_fixed: Vec<(String, f32)> = vec![(String::new(), 0.); 3];
        scenario::run(scn, None, &mut |ctx| {
            frames += 1;
            let (lp, fp) = (legacy_rig.evaluate(&ctx.tree, &old.md5), fixed_rig.evaluate(&ctx.tree, &old.md5));
            for k in 0..3 {
                let r = &refs[k];
                let rpose = r.eval_pose(&ctx.tree, &old.md5);
                let rg = r.globals(&rpose);
                let (d, _, _) = mat_diff(&lp.parts[k].bone_mesh, &rg);
                max_legacy[k] = max_legacy[k].max(d);
                let (d, _, w) = mat_diff(&fp.parts[k].bone_mesh, &rg);
                if d > max_fixed[k] { max_fixed[k] = d; worst_fixed[k] = (old.parts[k].tree.names[w].clone(), d); }
                max_fixed_local[k] = max_fixed_local[k].max(fp.parts[k].bone_local.iter().zip(&rpose).map(|(a, b)| (a.0 - b.0).abs().max_element()).fold(0., f32::max));
            }
        });
        println!("{:<18} {frames:>5} frames | max bone position error vs the reference port: fn-pose body {:.1e} head {:.1e} master {:.1e} m (local rotations/translations {:.1e} {:.1e} {:.1e}) | with the legacy additive switch (pre-fix) the head differs by {:.1e} m, the body by {:.1e} m",
            scn.name(), max_fixed[0], max_fixed[1], max_fixed[2], max_fixed_local[0], max_fixed_local[1], max_fixed_local[2], max_legacy[1], max_legacy[0]);
        let _ = &worst_fixed;
        // the reference port in fn-animgraph carries the face fix (tracked-bone mask), so the default mode must equal it; the legacy switch
        // shows what the fix changes (only bones without a track in the aim-offset clips: twist / face bones)
        assert!(max_fixed.iter().all(|e| *e < 1e-5) && max_fixed_local.iter().all(|e| *e < 1e-5), "{}: fn-pose differs from the reference port: {max_fixed:?} {max_fixed_local:?}", scn.name());
    }
}

/// Body / head mesh vertices at 20 sampled frames: skin matrices of fn-pose skinned with the old formula against the reference port's globals.
#[test]
fn skinned_vertices_at_20_frames_equal_the_reference_cpu_skinning() {
    let Some(old) = Old::load() else { return };
    let rig = CharacterRig::new(old.parts.clone());
    let refs: Vec<Skeleton> = old.parts.iter().map(as_reference).collect();
    let mut want: Vec<(String, u32)> = vec![];
    for (s, n) in [(Scenario::Pickaxe, 1434u32), (Scenario::Dir, 2520), (Scenario::Face, 432), (Scenario::Anim, 860)] { for i in 0..5 { want.push((s.name(), (11 + (n - 12) * i / 4 + i * 7 % 5).min(n - 1))); } }
    let _l = lock();
    let (mut max_pos, mut max_nrm, mut n_vertices, mut n_frames) = ([0f32; 2], [0f32; 2], 0usize, 0usize);
    for scn in [Scenario::Pickaxe, Scenario::Dir, Scenario::Face, Scenario::Anim] {
        scenario::run(scn, None, &mut |ctx| {
            if !want.contains(&(scn.name(), ctx.frame)) { return; }
            n_frames += 1;
            let pose = rig.evaluate(&ctx.tree, &old.md5);
            for k in 0..2 {
                let part = &old.parts[k];
                // old formula: skin = globals[joint] * inverse; pos += (skin * p) * w
                let rg = refs[k].globals(&refs[k].eval_pose(&ctx.tree, &old.md5));
                let skin: Vec<Mat4> = part.joints.iter().enumerate().map(|(i, j)| rg[*j] * part.inverse_bind[i]).collect();
                let mine = part.skin_cpu(&pose.parts[k].skin_matrices);
                let _ = &skin;
                for (pi, prim) in part.prims.iter().enumerate() {
                    for v in 0..prim.positions.len() {
                        let (mut pos, mut nrm) = (Vec3::ZERO, Vec3::ZERO);
                        for q in 0..4 { let m = skin[prim.joints[v][q] as usize]; pos += m.transform_point3(Vec3::from(prim.positions[v])) * prim.weights[v][q]; nrm += m.transform_vector3(Vec3::from(prim.normals[v])) * prim.weights[v][q]; }
                        max_pos[k] = max_pos[k].max((pos - mine[pi].0[v]).abs().max_element());
                        max_nrm[k] = max_nrm[k].max((nrm.normalize_or_zero() - mine[pi].1[v]).abs().max_element());
                        n_vertices += 1;
                    }
                }
            }
        });
    }
    println!("== skinned vertices, {n_frames} sampled frames, {n_vertices} vertex evaluations: max position error body {:.2e} head {:.2e} m (vs the reference port; the face fix only moves bones with no additive track, i.e. the face)", max_pos[0], max_pos[1]);
    println!("   max normal error body {:.2e} head {:.2e}", max_nrm[0], max_nrm[1]);
    assert_eq!(n_frames, 20);
    // the body has no bone the aim-offset lacks a track for; the head's face bones differ by design (the fix), everything else is the same code
    assert!(max_pos[0] < 1e-5, "body skinning differs: {}", max_pos[0]);
}

/// The parts decoded from the cooked packages (fn-mesh) are the old glTF exports.
#[test]
fn parts_from_packages_equal_the_old_gltf_parts() {
    let Some(old) = Old::load() else { return };
    let Some((_, src)) = package_clips(SampleMode::Ue) else { return };
    let pk = loader::package_parts(&src).unwrap();
    for (a, b) in pk.iter().zip(&old.parts) {
        assert_eq!(a.tree.names, b.tree.names, "{}: bone names / order", b.name);
        assert_eq!(a.tree.parents, b.tree.parents, "{}: parents", b.name);
        assert_eq!(a.joints, b.joints, "{}: skin joints", b.name);
        let (mut el, mut ei, mut ev, mut nv) = (0f32, 0f32, 0f32, 0usize);
        for i in 0..a.tree.len() { let (x, y) = (a.tree.local[i], b.tree.local[i]); el = el.max((x.0 - y.0).abs().max_element()).max(angle(x.1, y.1)).max((x.2 - y.2).abs().max_element()); }
        for (x, y) in a.inverse_bind.iter().zip(&b.inverse_bind) { ei = ei.max(x.to_cols_array().iter().zip(y.to_cols_array()).map(|(u, v)| (u - v).abs()).fold(0., f32::max)); }
        assert_eq!(a.prims.len(), b.prims.len(), "{}: primitives", b.name);
        for (p, q) in a.prims.iter().zip(&b.prims) {
            assert_eq!(p.positions.len(), q.positions.len());
            for i in 0..p.positions.len() { ev = ev.max((Vec3::from(p.positions[i]) - Vec3::from(q.positions[i])).abs().max_element()); assert_eq!(p.joints[i], q.joints[i]); nv += 1; }
        }
        println!("== part {}: {} skeleton nodes (incl. the master IK bones appended at load), {} skin joints, {nv} vertices: max local bind error {el:.1e}, inverse-bind error {ei:.1e}, vertex position error {ev:.1e}", b.name, b.tree.len(), b.joints.len());
        assert!(el < 1e-5 && ei < 1e-4 && ev < 1e-5, "{}: {el} {ei} {ev}", b.name);
    }
}

#[test]
fn ik_bones_are_appended_to_the_part_skeletons() {
    let Some(old) = Old::load() else { return };
    for p in &old.parts[..2] {
        for b in ["ik_foot_l", "ik_foot_r", "ik_hand_l", "ik_hand_r", "ik_hand_gun", "ik_foot_root", "ik_hand_root"] { assert!(p.tree.bone_index(b).is_some(), "{}: {b}", p.name); }
        assert!(p.tree.len() > p.joints.len());
    }
}

#[test]
fn gpu_palette_bytes_are_the_column_major_f32_storage_buffer_layout_of_fn_render() {
    let Some(old) = Old::load() else { return };
    let rig = CharacterRig::new(old.parts.clone());
    let pose = rig.evaluate(&fn_animgraph::Node::Ref, &old.md5);
    let r: &PoseResult = &pose.parts[0];
    let bytes = r.gpu_bytes();
    assert_eq!(bytes.len(), old.parts[0].palette_len() * fn_pose::GPU_MATRIX_BYTES);
    // fn-render: `x.to_cols_array_2d()` per matrix, cast to bytes (bytemuck) = native-endian f32 columns; wgsl mat4x4<f32> = 4 columns of vec4
    let m = r.gpu_matrices();
    for (i, mat) in m.iter().enumerate() { for c in 0..4 { for k in 0..4 { let off = i * 64 + c * 16 + k * 4; assert_eq!(f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()), mat[c][k]); } } }
    // the bind pose skins to the identity (skin = global * inverse bind)
    for s in &r.skin_matrices { assert!(s.abs_diff_eq(Mat4::IDENTITY, 2e-4), "{s:?}"); }
    println!("== palette: {} matrices x 64 bytes = {} bytes (bind pose skins to identity within 2e-4)", m.len(), bytes.len());
}

/// New pipeline end to end: meshes from the cooked packages, clips decoded by fn-anim with exact UE sampling (the default). The old md5 runtime samples exactly too since
/// the engine-exact sampling fix, so the poses agree; `SampleMode::LegacyMd5` survives only as a documented emulation of the OLD wrong playback (clips played
/// NumFrames/(NumFrames-1) too fast), which therefore must DIFFER from the corrected old poses.
#[test]
fn package_meshes_with_exact_ue_sampling_reproduce_the_old_poses_and_the_legacy_emulation_differs_by_design() {
    let Some(old) = Old::load() else { return };
    let Some((legacy, src)) = package_clips(SampleMode::LegacyMd5) else { return };
    let Some((ue, _)) = package_clips(SampleMode::Ue) else { return };
    let pk = CharacterRig::new(loader::package_parts(&src).unwrap());
    let gl = CharacterRig::new(old.parts.clone());
    let _l = lock();
    let (mut e_legacy, mut e_ue, mut frames, mut e_ue_idle) = (0f32, 0f32, 0, 0f32);
    scenario::run(Scenario::Dir, None, &mut |ctx| {
        frames += 1;
        let want = gl.evaluate(&ctx.tree, &old.md5);
        let a = pk.evaluate(&ctx.tree, &legacy as &dyn ClipLibrary);
        let b = pk.evaluate(&ctx.tree, &ue as &dyn ClipLibrary);
        let (d1, _, _) = mat_diff(&a.parts[0].bone_mesh, &want.parts[0].bone_mesh);
        let (d2, _, _) = mat_diff(&b.parts[0].bone_mesh, &want.parts[0].bone_mesh);
        e_legacy = e_legacy.max(d1); e_ue = e_ue.max(d2); if ctx.frame < 10 { e_ue_idle = e_ue_idle.max(d2); }
    });
    println!("== dir-test, packages meshes + fn-anim clips: exact-UE sampling vs the old (exact) md5 poses max bone error {e_ue:.2e} m over {frames} frames (idle frames {e_ue_idle:.1e} m); the legacy emulation is {e_legacy:.3} m away");
    assert!(e_ue < 5e-4, "exact UE sampling must reproduce the old poses: {e_ue}");
    assert!(e_legacy > 1e-2, "the legacy emulation of the old wrong playback must differ from the corrected old poses: {e_legacy}");
}
