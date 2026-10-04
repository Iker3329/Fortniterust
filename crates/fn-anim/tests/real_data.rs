//! Integration tests against the real Fortnite 1.8 packages and the old UEViewer md5anim exports.
//!
//! The test data lives outside the repository; every test is skipped (with a message) when it is missing:
//! * `FN_TESTDATA` (default `%LOCALAPPDATA%\Temp\fn-testdata`): extracted raw `FortniteGame/Content/...` packages.
//! * `FN_OLD_MD5` : directory with the 388 old `.md5anim` files (default: the old FortniteRuntime tree).
//! * `FN_OLD_REPORTS`: the old runtime's `reports` directory.

use fn_anim::md5;
use fn_anim::verify::{compare_all_frames, Diff};
use fn_anim::{AnimSequence, BlendSpace, FnAnim, Precision, SkeletonInfo};
use fn_ueio::Package;
use std::path::{Path, PathBuf};

const OLD_ROOT: &str = r"C:\Users\Brady\Documents\Codex\2026-10-02\im-seeing-titkoks-where-people-are-2\FortniteRuntime";

fn testdata() -> Option<PathBuf> {
    let p = std::env::var_os("FN_TESTDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("Temp").join("fn-testdata"));
    if p.join("FortniteGame/Content/Animation").is_dir() {
        Some(p)
    } else {
        eprintln!("SKIPPED: test data {} not found", p.display());
        None
    }
}

fn old_md5_dir() -> Option<PathBuf> {
    let p = std::env::var_os("FN_OLD_MD5").map(PathBuf::from).unwrap_or_else(|| Path::new(OLD_ROOT).join(r"assets\animation\Game\Characters\Player\Male\Male_Avg_Base\Fortnite_M_Avg_Player_Skeleton"));
    if p.is_dir() {
        Some(p)
    } else {
        eprintln!("SKIPPED: old md5anim dir {} not found", p.display());
        None
    }
}

fn old_reports() -> Option<PathBuf> {
    let p = std::env::var_os("FN_OLD_REPORTS").map(PathBuf::from).unwrap_or_else(|| Path::new(OLD_ROOT).join("reports"));
    if p.is_dir() {
        Some(p)
    } else {
        eprintln!("SKIPPED: old reports dir {} not found", p.display());
        None
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map(|x| x == "uasset").unwrap_or(false) {
                out.push(p);
            }
        }
    }
}

fn load(f: &Path) -> Package {
    let rd = |e: &str| std::fs::read(f.with_extension(e)).unwrap_or_default();
    Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")).unwrap_or_else(|e| panic!("{}: {e}", f.display()))
}

fn skeleton(root: &Path) -> SkeletonInfo {
    SkeletonInfo::from_package(&load(&root.join("FortniteGame/Content/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_Skeleton.uasset"))).unwrap()
}

fn all_sequences(root: &Path) -> Vec<AnimSequence> {
    let mut files = Vec::new();
    walk(&root.join("FortniteGame/Content/Animation"), &mut files);
    files.sort();
    files
        .iter()
        .filter_map(|f| {
            let pkg = load(f);
            pkg.find_export("AnimSequence").map(|_| AnimSequence::parse(&pkg).unwrap_or_else(|e| panic!("{}: {e}", f.display())))
        })
        .collect()
}

#[test]
fn skeleton_parses() {
    let Some(root) = testdata() else { return };
    let s = skeleton(&root);
    assert_eq!(s.num_bones(), 266);
    assert_eq!(s.bone_names[0], "Root");
    assert_eq!(s.parents[0], -1);
    assert_eq!(s.bone_names[1], "pelvis");
    for (i, &p) in s.parents.iter().enumerate().skip(1) {
        assert!(p >= 0 && (p as usize) < i, "bone {i} parent {p}");
    }
    assert_eq!(s.ref_pose.len(), 266);
    assert_eq!(s.retarget.len(), 266);
    assert_eq!(s.retarget_sources.len(), 18);
    assert!(s.find_retarget_source("MPR_SK_M_MALE_Base_Skeleton").is_some());
    // BoneTree modes as counted independently by fn-mesh: 227 Skeleton, 22 AnimationScaled, 13 Animation, 4 AnimationRelative
    let count = |m: fn_anim::TranslationRetarget| s.retarget.iter().filter(|&&x| x == m).count();
    assert_eq!(count(fn_anim::TranslationRetarget::Skeleton), 227);
    assert_eq!(count(fn_anim::TranslationRetarget::AnimationScaled), 22);
    assert_eq!(count(fn_anim::TranslationRetarget::Animation), 13);
    assert_eq!(count(fn_anim::TranslationRetarget::AnimationRelative), 4);
    // rotations are unit quaternions
    for t in &s.ref_pose {
        let q = t.rotation;
        assert!(((q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]) - 1.0).abs() < 1e-3);
    }
}

/// Every one of the 388 clips: same frame / joint count, identical metadata and (here) byte-identical md5anim text.
#[test]
fn all_clips_match_old_md5anim_exports() {
    let (Some(root), Some(md5dir)) = (testdata(), old_md5_dir()) else { return };
    let skel = skeleton(&root);
    let seqs = all_sequences(&root);
    assert_eq!(seqs.len(), 388);
    let mut total = Diff::default();
    let mut identical = 0;
    for seq in &seqs {
        let old_bytes = std::fs::read(md5dir.join(format!("{}.md5anim", seq.name))).unwrap_or_else(|_| panic!("no md5anim for {}", seq.name));
        let old = md5::parse_md5anim(&String::from_utf8_lossy(&old_bytes)).unwrap();
        let new = md5::export(seq, &skel);
        assert_eq!(new.num_frames, old.num_frames, "{} frame count", seq.name);
        assert_eq!(new.frames.len(), old.frames.len(), "{} frames", seq.name);
        assert_eq!(new.bone_names, old.bone_names, "{} joints", seq.name);
        assert_eq!(new.additive, old.additive, "{} additive", seq.name);
        assert_eq!(new.retarget_base, old.retarget_base, "{} retarget-base", seq.name);
        assert_eq!(new.track_flags, old.track_flags, "{} track flags", seq.name);
        assert!((new.frame_rate - old.frame_rate).abs() <= old.frame_rate * 1e-5, "{} rate", seq.name);
        for (a, b) in new.retarget.iter().zip(&old.retarget) {
            let (a, b) = (a.unwrap(), b.unwrap());
            assert_eq!(a.0, b.0, "{} retarget mode", seq.name);
            for k in 0..3 {
                assert!((a.1[k] - b.1[k]).abs() < 1e-5, "{} retarget source", seq.name);
            }
        }
        let d = compare_all_frames(&new, &old);
        // old files are rounded to 6 / 10 decimals
        assert!(d.max_pos_cm < 2e-6, "{}: pos error {}", seq.name, d.max_pos_cm);
        assert!(d.max_xyz < 1e-9, "{}: quat component error {}", seq.name, d.max_xyz);
        total.merge(&d);
        let mut buf = Vec::new();
        md5::write_md5anim(seq, &skel, &mut buf).unwrap();
        if buf == old_bytes {
            identical += 1;
        }
    }
    eprintln!("388 clips: max pos err {:.2e} cm, max quat component err {:.2e}, byte-identical {identical}/388", total.max_pos_cm, total.max_xyz);
    assert_eq!(identical, 388);
}

/// Sampled poses (UE nlerp) vs old frames interpolated with slerp: positions agree, rotations within the
/// nlerp/slerp difference; with `RotationBlend::Slerp` they agree to rounding.
#[test]
fn sampled_poses_match_old_frames() {
    let (Some(root), Some(md5dir)) = (testdata(), old_md5_dir()) else { return };
    let skel = skeleton(&root);
    let seqs = all_sequences(&root);
    let mut tested = 0;
    const PICK: &[&str] = &["M_Jog_NoWep_N", "M_Jog_NoWep_NE", "M_Jog_NoWep_E", "M_Jog_NoWep_SE", "M_Jog_NoWep_S", "M_Jog_NoWep_SW", "M_Jog_NoWep_W", "M_Jog_NoWep_NW", "M_Jog_NoWep_N_Core", "M_Jog_NoWep_NE_Core", "M_Jog_NoWep_E_Core", "M_Jog_NoWep_SE_Core", "M_Jog_NoWep_S_Core", "M_Jog_NoWep_SW_Core", "M_Jog_NoWep_W_Core", "M_Jog_NoWep_NW_Core", "Sprint_AR", "Sprint_Default", "Sprint_Sword", "Sprint_TwoHanded", "CrouchSprint_NoWeapon", "M_JumpApex_ARDS_Loop_Additive", "M_JumpFall_ARDS_Additive", "M_JumpLand_ARDS_Additive", "M_JumpUp_ARDS_Additive", "JumpLand_NoWep_CMM_Additive", "JumpLand_NoWep_CMM_Additive_Core", "JumpUp_NoWep_CMM_Additive", "TwoHanded_Loop_Swing1", "TwoHanded_Loop_Swing2", "TwoHanded_Loop_Swing1_Bounce", "TwoHanded_Loop_Swing2_InMotion", "Crouch_NoWeapon_E", "Crouch_NoWeapon_E_Core", "Crouch_NoWep_CMM", "Crouch_NoWeapon_SW_Core", "Commando_Male_Body_Idle", "Idle_Noise", "Idle_Turn_Left", "Idle_Turn_Right_180"];
    for seq in seqs.iter().filter(|s| PICK.contains(&s.name.as_str())) {
        let old = md5::parse_md5anim(&std::fs::read_to_string(md5dir.join(format!("{}.md5anim", seq.name))).unwrap()).unwrap();
        let (mut nl, mut sl) = (Diff::default(), Diff::default());
        let nf = seq.num_frames as f32;
        for k in 0..20 {
            let f = if k == 0 { 0.0 } else if k == 1 { nf - 1.0 } else { ((k as f32 * 0.6180339 * (nf - 1.0)) % (nf - 1.0)) + 0.37 * (k % 3) as f32 }.clamp(0.0, nf - 1.0);
            fn_anim::verify::compare_sampled(seq, &skel, &old, f, k, fn_anim::RotationBlend::Nlerp, &mut nl);
            fn_anim::verify::compare_sampled(seq, &skel, &old, f, k, fn_anim::RotationBlend::Slerp, &mut sl);
        }
        assert!(nl.max_pos_cm < 1e-3 && sl.max_pos_cm < 1e-3, "{} pos {} {}", seq.name, nl.max_pos_cm, sl.max_pos_cm);
        assert!(sl.max_xyz < 1e-4, "{} slerp xyz {}", seq.name, sl.max_xyz);
        assert!(nl.max_xyz < 0.02, "{} nlerp xyz {}", seq.name, nl.max_xyz);
        tested += 1;
    }
    assert!(tested >= 30, "only {tested} clips tested");
}

#[test]
fn pose_semantics() {
    let Some(root) = testdata() else { return };
    let skel = skeleton(&root);
    let seqs = all_sequences(&root);
    let core = seqs.iter().find(|s| s.name == "M_Jog_NoWep_N_Core").expect("clip");
    let add = seqs.iter().find(|s| s.name == "M_Jog_NoWep_N").expect("clip");
    assert!(!core.is_additive() && add.is_additive());
    let p = core.pose_at(0.0, &skel);
    assert_eq!(p.len(), 266);
    // bones without a track keep the reference pose in a non-additive clip, identity in an additive one
    let untracked: Vec<usize> = (0..266).filter(|&b| core.find_track(b).is_none()).collect();
    for &b in untracked.iter().take(10) {
        assert_eq!(p[b], skel.ref_pose[b]);
    }
    let pa = add.pose_at(0.0, &skel);
    for b in (0..266).filter(|&b| add.find_track(b).is_none()).take(10) {
        assert_eq!(pa[b], fn_anim::BoneTransform::ADDITIVE_IDENTITY);
    }
    // time mapping: last frame at t = length, clamped beyond
    let last = core.pose_at(core.length, &skel);
    assert_eq!(last, core.sample_frame(core.num_frames - 1, &skel));
    assert_eq!(core.pose_at(core.length * 10.0, &skel), last);
    assert_eq!(core.pose_at(-1.0, &skel), core.sample_frame(0, &skel));
    // bake_frames == per-frame sampling
    let baked = core.bake_frames(&skel);
    assert_eq!(baked.len(), core.num_frames);
    assert_eq!(baked[3], core.sample_frame(3, &skel));
    // half-way between two frames is the nlerp / lerp of the neighbours
    let mid = core.pose_at_frame(3.5, &skel);
    for b in [1usize, 2, 10] {
        for k in 0..3 {
            let e = (baked[3][b].translation[k] + baked[4][b].translation[k]) * 0.5;
            assert!((mid[b].translation[k] - e).abs() < 1e-3);
        }
    }
}

#[test]
fn metadata_matches_old_reports() {
    let (Some(root), Some(reports)) = (testdata(), old_reports()) else { return };
    let seqs = all_sequences(&root);
    let find = |n: &str| seqs.iter().find(|s| s.name == n);
    // animation-metadata.json: AdditiveAnimType, RefPoseType, RetargetSource, RefPoseSeq, RefFrameIndex, SequenceLength, RateScale
    let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(reports.join("animation-metadata.json")).unwrap()).unwrap();
    let mut checked = 0;
    for m in meta.as_array().unwrap() {
        let Some(seq) = find(m["name"].as_str().unwrap()) else { continue };
        for line in m["metadata"].as_array().unwrap() {
            let line = line.as_str().unwrap().trim();
            let (k, v) = line.split_once(" = ").unwrap();
            match k {
                "AdditiveAnimType" => assert!(v.starts_with(match seq.additive.kind {
                    fn_anim::AdditiveAnimType::None => "AAT_None",
                    fn_anim::AdditiveAnimType::LocalSpaceBase => "AAT_LocalSpaceBase",
                    fn_anim::AdditiveAnimType::RotationOffsetMeshSpace => "AAT_RotationOffsetMeshSpace",
                }), "{} {line}", seq.name),
                "RefPoseType" => assert!(v.starts_with(match seq.additive.base {
                    fn_anim::RefPoseType::None => "ABPT_None",
                    fn_anim::RefPoseType::RefPose => "ABPT_RefPose",
                    fn_anim::RefPoseType::AnimScaled => "ABPT_AnimScaled",
                    fn_anim::RefPoseType::AnimFrame => "ABPT_AnimFrame",
                }), "{} {line}", seq.name),
                "RetargetSource" => assert_eq!(v, seq.retarget_source, "{}", seq.name),
                "RefPoseSeq" => {
                    let want = v.rsplit('.').next().unwrap().trim_end_matches('\'');
                    assert_eq!(seq.additive.ref_pose_seq.as_deref(), Some(want), "{}", seq.name);
                }
                "RefFrameIndex" => assert_eq!(seq.additive.ref_frame_index.to_string(), v, "{}", seq.name),
                "SequenceLength" => assert!((seq.length as f64 - v.parse::<f64>().unwrap()).abs() < 1e-4, "{} {line}", seq.name),
                "RateScale" => assert!((seq.rate_scale as f64 - v.parse::<f64>().unwrap()).abs() < 1e-6, "{} {line}", seq.name),
                _ => {}
            }
        }
        checked += 1;
    }
    assert!(checked >= 5, "only {checked} metadata entries matched a clip");
    // animation-blendspaces.json "clips": frames, length, rate scale, additive, ref pose, sync markers
    let bs: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(reports.join("animation-blendspaces.json")).unwrap()).unwrap();
    let mut clips = 0;
    for c in bs["clips"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let seq = find(name).unwrap_or_else(|| panic!("clip {name} not found"));
        assert_eq!(seq.num_frames as i64, c["numFrames"].as_i64().unwrap(), "{name}");
        assert!((seq.length as f64 - c["length"].as_f64().unwrap()).abs() < 1e-6, "{name}");
        assert!((seq.rate_scale as f64 - c["rateScale"].as_f64().unwrap()).abs() < 1e-6, "{name}");
        assert_eq!(seq.additive.kind != fn_anim::AdditiveAnimType::None, c["additive"].as_str().unwrap() != "AAT_None", "{name}");
        assert_eq!(seq.additive.ref_pose_seq.as_deref(), c["refPoseSeq"].as_str(), "{name}");
        let markers = c["markers"].as_array().unwrap();
        assert_eq!(seq.sync_markers.len(), markers.len(), "{name} markers");
        for (m, o) in seq.sync_markers.iter().zip(markers) {
            assert_eq!(m.name, o[0].as_str().unwrap(), "{name}");
            assert!((m.time as f64 - o[1].as_f64().unwrap()).abs() < 1e-6, "{name}");
        }
        clips += 1;
    }
    assert!(clips > 20, "only {clips} report clips");
}

#[test]
fn notifies_and_curves() {
    let Some(root) = testdata() else { return };
    let seqs = all_sequences(&root);
    let with_notifies: Vec<_> = seqs.iter().filter(|s| !s.notifies.is_empty()).collect();
    assert!(with_notifies.len() > 50, "{} clips with notifies", with_notifies.len());
    let mut steps = 0;
    for s in &with_notifies {
        for n in &s.notifies {
            assert!(n.trigger_time() >= -1e-3 && n.trigger_time() <= s.length + 1e-3, "{} notify {} at {} (length {})", s.name, n.notify_name, n.trigger_time(), s.length);
            if n.notify_class.as_deref().map(|c| c.contains("FootStep")).unwrap_or(false) {
                steps += 1;
            }
        }
    }
    assert!(steps > 100, "{steps} footstep notifies");
    let with_curves = seqs.iter().filter(|s| !s.curves.is_empty()).count();
    assert!(with_curves > 100, "{with_curves} clips with curves");
    for s in seqs.iter().filter(|s| !s.curves.is_empty()).take(30) {
        for c in &s.curves {
            assert!(!c.name.is_empty(), "{} unnamed curve", s.name);
            let v = c.eval(0.0);
            assert!(v.is_finite());
        }
    }
}

#[test]
fn fnanim_roundtrip_and_sizes() {
    let (Some(root), md5dir) = (testdata(), old_md5_dir()) else { return };
    let skel = skeleton(&root);
    let seqs = all_sequences(&root);
    let (mut md5_bytes, mut f32_bytes, mut q16_bytes) = (0usize, 0usize, 0usize);
    let mut worst_q16_pos = 0.0f32;
    let mut worst_q16_rot = 0.0f32;
    for seq in &seqs {
        let mut a = Vec::new();
        seq.write_fnanim(&skel, Precision::F32, &mut a);
        let mut b = Vec::new();
        seq.write_fnanim(&skel, Precision::Q16, &mut b);
        f32_bytes += a.len();
        q16_bytes += b.len();
        if let Some(d) = &md5dir {
            md5_bytes += std::fs::metadata(d.join(format!("{}.md5anim", seq.name))).map(|m| m.len() as usize).unwrap_or(0);
        }
        let fa = AnimSequence::read_fnanim(&a).unwrap();
        assert_eq!(fa.num_frames, seq.num_frames);
        assert_eq!(fa.bones.len(), 266);
        assert_eq!(fa.additive, seq.is_additive());
        assert_eq!(fa.notifies.len(), seq.notifies.len());
        assert_eq!(fa.curves.len(), seq.curves.len());
        // defaults used by the consumer for absent channels
        let defaults: Vec<_> = if seq.is_additive() { vec![fn_anim::BoneTransform::ADDITIVE_IDENTITY; 266] } else { skel.ref_pose.clone() };
        let want = seq.bake_frames(&skel);
        let got = fa.bake_frames(&defaults);
        assert_eq!(want, got, "{}: lossless fnanim differs", seq.name);
        let fb = FnAnim::read(&b).unwrap();
        let gq = fb.bake_frames(&defaults);
        for (w, g) in want.iter().zip(&gq) {
            for (x, y) in w.iter().zip(g) {
                for k in 0..3 {
                    worst_q16_pos = worst_q16_pos.max((x.translation[k] - y.translation[k]).abs());
                }
                for k in 0..4 {
                    worst_q16_rot = worst_q16_rot.max((x.rotation[k] - y.rotation[k]).abs());
                }
            }
        }
    }
    eprintln!("sizes over {} clips: md5anim {} B, fnanim F32 {} B, fnanim Q16 {} B; Q16 worst error pos {worst_q16_pos:.5} cm rot {worst_q16_rot:.2e}", seqs.len(), md5_bytes, f32_bytes, q16_bytes);
    assert!(worst_q16_pos < 0.01, "q16 position error {worst_q16_pos}");
    assert!(worst_q16_rot < 5e-5, "q16 rotation error {worst_q16_rot}");
    if md5_bytes > 0 {
        assert!(f32_bytes * 15 < md5_bytes);
    }
}

#[test]
fn blendspaces_match_old_report() {
    let (Some(root), Some(reports)) = (testdata(), old_reports()) else { return };
    let bs: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(reports.join("animation-blendspaces.json")).unwrap()).unwrap();
    let mut n = 0;
    for b in bs["blendSpaces"].as_array().unwrap() {
        let src = b["source"].as_str().unwrap().trim_start_matches("../../../");
        let path = root.join(src);
        if !path.exists() {
            eprintln!("SKIPPED blend space {src}: not extracted");
            continue;
        }
        let space = BlendSpace::parse(&load(&path)).unwrap();
        space.validate().unwrap();
        assert_eq!(space.name, b["name"].as_str().unwrap());
        assert_eq!(format!("{:?}", space.kind), b["className"].as_str().unwrap());
        assert_eq!(space.samples.len(), b["samples"].as_array().unwrap().len());
        assert_eq!(space.grid.len(), b["grid"].as_array().unwrap().len());
        for (a, o) in space.axes.iter().zip(b["axes"].as_array().unwrap()) {
            assert_eq!(a.min as f64, o["min"].as_f64().unwrap());
            assert_eq!(a.max as f64, o["max"].as_f64().unwrap());
            assert_eq!(a.grid_divisions as i64, o["grid"].as_i64().unwrap());
            assert_eq!(a.interp_type, o["interpType"].as_str().unwrap());
        }
        assert!((space.target_weight_interpolation_speed_per_sec as f64 - b["targetWeightInterpolationSpeedPerSec"].as_f64().unwrap()).abs() < 1e-6);
        for (s, o) in space.samples.iter().zip(b["samples"].as_array().unwrap()) {
            assert_eq!(s.animation.as_deref(), o["anim"].as_str());
            assert_eq!(s.position[0] as f64, o["pos"][0].as_f64().unwrap());
            assert_eq!(s.position[1] as f64, o["pos"][1].as_f64().unwrap());
        }
        // every sample position evaluates to that sample's own animation
        for s in &space.samples {
            let w = space.animation_weights(s.position[0], s.position[1]);
            let own = w.iter().find(|(a, _)| Some(a.as_str()) == s.animation.as_deref()).map(|x| x.1).unwrap_or(0.0);
            assert!((own - 1.0).abs() < 1e-4, "{}: {:?} -> {w:?}", space.name, s.position);
        }
        n += 1;
    }
    assert!(n >= 10, "only {n} blend spaces found");
}

#[test]
fn melee_montage_parses() {
    let (Some(root), Some(reports)) = (testdata(), old_reports()) else { return };
    let path = root.join("FortniteGame/Content/Animation/Game/MainPlayer/Montages/Melee/PlayerMeleeCombo.uasset");
    if !path.exists() {
        eprintln!("SKIPPED: PlayerMeleeCombo not extracted");
        return;
    }
    let m = fn_anim::AnimMontage::parse(&load(&path)).unwrap();
    assert_eq!(m.name, "PlayerMeleeCombo");
    assert!((m.length - 2.866_633_4).abs() < 1e-5);
    assert!((m.blend_in.time - 0.08).abs() < 1e-6 && (m.blend_out.time - 0.15).abs() < 1e-6);
    assert!(!m.slots.is_empty() && m.slots[0].slot_name == "FullBodyInPlace");
    let names: Vec<&str> = m.sections.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Swing1") && names.contains(&"Swing2"), "{names:?}");
    assert_eq!(m.sections.iter().find(|s| s.name == "Swing1").unwrap().next_section, "Swing2");
    assert!(m.notifies.len() >= 10);
    // old report: segment list with StartPos / AnimEndTime
    let rep: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(reports.join("melee-montage.json")).unwrap()).unwrap();
    let all: Vec<&fn_anim::montage::AnimSegment> = m.slots.iter().flat_map(|s| s.segments.iter()).collect();
    let segs = rep["segments"].as_array().unwrap();
    assert_eq!(all.len(), segs.len());
    for (s, o) in all.iter().zip(segs) {
        assert_eq!(s.animation.as_deref(), o["reference"]["name"].as_str());
        for f in o["fields"].as_array().unwrap() {
            let Some(v) = f["value"].as_f64() else { continue };
            match f["name"].as_str().unwrap() {
                "StartPos" => assert!((s.start_pos as f64 - v).abs() < 1e-5),
                "AnimStartTime" => assert!((s.anim_start_time as f64 - v).abs() < 1e-5),
                "AnimEndTime" => assert!((s.anim_end_time as f64 - v).abs() < 1e-5),
                "AnimPlayRate" => assert!((s.play_rate as f64 - v).abs() < 1e-5),
                "LoopingCount" => assert_eq!(s.looping_count as f64, v),
                _ => {}
            }
        }
    }
    // every referenced animation exists as a decoded AnimSequence
    let seqs = all_sequences(&root);
    for s in all {
        let n = s.animation.as_deref().unwrap();
        assert!(seqs.iter().any(|q| q.name == n), "montage references missing clip {n}");
    }
}
/// The fn-mesh skeleton parser and this crate's own parser must agree (only with `--features mesh`).
#[cfg(feature = "mesh")]
#[test]
fn skeleton_matches_fn_mesh() {
    let Some(root) = testdata() else { return };
    let pkg = load(&root.join("FortniteGame/Content/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_Skeleton.uasset"));
    let mine = SkeletonInfo::from_package(&pkg).unwrap();
    let theirs = SkeletonInfo::from_mesh_skeleton(&fn_mesh::Skeleton::parse(&pkg).unwrap());
    assert_eq!(mine.bone_names, theirs.bone_names);
    assert_eq!(mine.parents, theirs.parents);
    assert_eq!(mine.ref_pose, theirs.ref_pose);
    assert_eq!(mine.retarget, theirs.retarget);
    assert_eq!(mine.retarget_sources.len(), theirs.retarget_sources.len());
    for (a, b) in mine.retarget_sources.iter().zip(&theirs.retarget_sources) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.pose, b.pose);
    }
}

/// The old crouch export carried the original retarget rules (old runtime test `exported_crouch_contains_original_retarget_rules`).
#[test]
fn crouch_retarget_metadata() {
    let Some(root) = testdata() else { return };
    let skel = skeleton(&root);
    let seqs = all_sequences(&root);
    let Some(seq) = seqs.iter().find(|s| s.name == "Crouch_NoWep_CMM") else {
        eprintln!("SKIPPED: Crouch_NoWep_CMM not found");
        return;
    };
    let m = md5::export(seq, &skel);
    assert_eq!(m.retarget.iter().filter(|r| r.is_some()).count(), 266);
    assert_eq!(m.retarget[1].unwrap().0, 2); // pelvis
    assert_eq!(m.retarget[2].unwrap().0, 2); // Spine_01
    assert!(!m.additive);
    assert_eq!(m.track_flags.iter().filter(|&&v| v == (false, false)).count(), 178);
}