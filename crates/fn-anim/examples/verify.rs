//! Verify decoded animations against the old md5anim exports.
//! usage: verify <testdata root> <md5anim dir> [--clips] [--samples]
use fn_anim::md5;
use fn_anim::verify::{compare_all_frames, compare_sampled, Diff};
use fn_anim::{AnimSequence, RotationBlend, SkeletonInfo};
use fn_ueio::Package;
use std::path::{Path, PathBuf};

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

fn load(f: &Path) -> Option<Package> {
    let rd = |e: &str| std::fs::read(f.with_extension(e)).unwrap_or_default();
    Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")).ok()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = Path::new(&args[1]);
    let md5dir = Path::new(&args[2]);
    let show_clips = args.iter().any(|a| a == "--clips");
    let samples = args.iter().any(|a| a == "--samples");
    let skel_pkg = load(&root.join("FortniteGame/Content/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_Skeleton.uasset")).expect("skeleton");
    let skel = SkeletonInfo::from_package(&skel_pkg).expect("skeleton parse");
    println!("skeleton: {} bones, {} retarget sources, non-unit scales: {}", skel.num_bones(), skel.retarget_sources.len(), skel.ref_pose.iter().filter(|t| t.scale.iter().any(|s| (s - 1.0).abs() > 1e-4)).count());
    for s in &skel.retarget_sources {
        println!("  retarget source {} ({} bones)", s.name, s.pose.len());
    }
    let mut files = Vec::new();
    walk(&root.join("FortniteGame/Content/Animation"), &mut files);
    files.sort();

    struct Row {
        name: String,
        frames: usize,
        pos: f64,
        rot: f64,
        wpos: String,
        wrot: String,
        identical: bool,
        meta: Vec<String>,
        fc: bool,
        jc: bool,
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut missing = 0;
    let mut identical = 0;
    let mut interp_stats = std::collections::BTreeMap::<(bool, bool), usize>::new();
    let mut total = Diff::default();
    let mut sampled_rows: Vec<(String, Diff, Diff, bool, String, String)> = Vec::new();
    const PICK: &[&str] = &["M_Jog_NoWep_N", "M_Jog_NoWep_NE", "M_Jog_NoWep_E", "M_Jog_NoWep_SE", "M_Jog_NoWep_S", "M_Jog_NoWep_SW", "M_Jog_NoWep_W", "M_Jog_NoWep_NW", "M_Jog_NoWep_N_Core", "M_Jog_NoWep_NE_Core", "M_Jog_NoWep_E_Core", "M_Jog_NoWep_SE_Core", "M_Jog_NoWep_S_Core", "M_Jog_NoWep_SW_Core", "M_Jog_NoWep_W_Core", "M_Jog_NoWep_NW_Core", "Sprint_AR", "Sprint_Default", "Sprint_Sword", "Sprint_TwoHanded", "CrouchSprint_NoWeapon", "M_JumpApex_ARDS_Loop_Additive", "M_JumpFall_ARDS_Additive", "M_JumpLand_ARDS_Additive", "M_JumpUp_ARDS_Additive", "JumpLand_NoWep_CMM_Additive", "JumpLand_NoWep_CMM_Additive_Core", "JumpUp_NoWep_CMM_Additive", "TwoHanded_Loop_Swing1", "TwoHanded_Loop_Swing2", "TwoHanded_Loop_Swing1_Bounce", "TwoHanded_Loop_Swing2_InMotion", "Crouch_NoWeapon_E", "Crouch_NoWeapon_E_Core", "Crouch_NoWep_CMM", "Crouch_NoWeapon_SW_Core", "Commando_Male_Body_Idle", "Idle_Noise", "Idle_Turn_Left", "Idle_Turn_Right_180"];
    let pick = |n: &str| PICK.contains(&n);
    for f in files {
        let Some(pkg) = load(&f) else { continue };
        if pkg.find_export("AnimSequence").is_none() {
            continue;
        }
        let seq = match AnimSequence::parse(&pkg) {
            Ok(s) => s,
            Err(e) => {
                println!("PARSE FAIL {}: {e}", f.display());
                continue;
            }
        };
        let old_path = md5dir.join(format!("{}.md5anim", seq.name));
        let Ok(old_bytes) = std::fs::read(&old_path) else {
            missing += 1;
            println!("no md5 for {}", seq.name);
            continue;
        };
        let old = md5::parse_md5anim(&String::from_utf8_lossy(&old_bytes)).expect("old md5 parse");
        let new = md5::export(&seq, &skel);
        let mut buf = Vec::new();
        md5::write(&new, &mut buf).unwrap();
        let same = buf == old_bytes;
        let interp = seq.tracks.iter().any(|t| (t.rot.len() > 1 && t.rot.len() < seq.num_frames) || (t.pos.len() > 1 && t.pos.len() < seq.num_frames));
        *interp_stats.entry((interp, same)).or_default() += 1;
        if !same && args.iter().any(|a| a == "--diff") {
            let (a, b) = (String::from_utf8_lossy(&buf).into_owned(), String::from_utf8_lossy(&old_bytes).into_owned());
            let mut n = 0;
            let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
            for (la, lb) in a.lines().zip(b.lines()) {
                if la != lb {
                    let k = if la.starts_with("//") { "comment" } else if la.starts_with("frameRate") { "rate" } else { "data" };
                    *kinds.entry(k).or_default() += 1;
                    if n < 3 {
                        println!("DIFF {} new: {la}\n     old: {lb}", seq.name);
                    }
                    n += 1;
                }
            }
            if a.lines().count() != b.lines().count() {
                println!("DIFF {} line count {} vs {}", seq.name, a.lines().count(), b.lines().count());
            }
            if samples {
                println!("  kinds {kinds:?} total {n}");
            }
            // which formats do the differing joints use?
            let mut shown = 0;
            'outer: for (fi, (fn_, fo)) in new.frames.iter().zip(&old.frames).enumerate() {
                for (b, (jn, jo)) in fn_.iter().zip(fo).enumerate() {
                    if jn.1 != jo.1 && shown < 4 {
                        let t = seq.find_track(b);
                        println!(
                            "   joint f{fi} {} rot_fmt {:?} keys {} new {:?} old {:?}",
                            skel.bone_names[b],
                            t.and_then(|t| t.rot_format),
                            t.map(|t| t.rot.len()).unwrap_or(0),
                            jn.1,
                            jo.1
                        );
                        shown += 1;
                        if shown >= 4 {
                            break 'outer;
                        }
                    }
                }
            }
        }
        if same {
            identical += 1;
        }
        let fc = new.num_frames == old.num_frames && new.frames.len() == old.frames.len();
        let jc = new.bone_names.len() == old.bone_names.len() && new.bone_names == old.bone_names;
        let mut meta = Vec::new();
        if new.additive != old.additive {
            meta.push("additive".to_string());
        }
        if new.retarget_base != old.retarget_base {
            meta.push("retarget-base".to_string());
        }
        if new.track_flags != old.track_flags {
            meta.push("track flags".to_string());
        }
        if (new.frame_rate - old.frame_rate).abs() > old.frame_rate * 1e-5 {
            meta.push(format!("frameRate {} vs {}", new.frame_rate, old.frame_rate));
        }
        let mut max_rt = 0.0f32;
        for (a, b) in new.retarget.iter().zip(&old.retarget) {
            match (a, b) {
                (Some((m1, p1)), Some((m2, p2))) => {
                    if m1 != m2 {
                        meta.push("retarget mode".to_string());
                    }
                    for k in 0..3 {
                        max_rt = max_rt.max((p1[k] - p2[k]).abs());
                    }
                }
                (None, None) => {}
                _ => meta.push("retarget presence".to_string()),
            }
        }
        if max_rt > 1e-5 {
            meta.push(format!("retarget src diff {max_rt}"));
        }
        meta.dedup();
        let (d, ok) = if fc && jc {
            (compare_all_frames(&new, &old), true)
        } else {
            (Diff::default(), false)
        };
        if ok {
            total.merge(&d);
        }
        if samples && pick(&seq.name) && ok {
            let mut sd = Diff::default();
            let mut sd2 = Diff::default();
            let nf = seq.num_frames as f32;
            for k in 0..20 {
                let fr = match k {
                    0 => 0.0,
                    1 => nf - 1.0,
                    _ => ((k as f32 * 0.6180339 * (nf - 1.0)) % (nf - 1.0)) + 0.37 * (k % 3) as f32,
                }
                .clamp(0.0, nf - 1.0);
                compare_sampled(&seq, &skel, &old, fr, k, RotationBlend::Nlerp, &mut sd);
                compare_sampled(&seq, &skel, &old, fr, k, RotationBlend::Slerp, &mut sd2);
            }
            let (bp, br) = (skel.bone_names[sd2.worst_pos.1].clone(), skel.bone_names[sd2.worst_rot.1].clone());
            sampled_rows.push((seq.name.clone(), sd, sd2, seq.is_additive(), bp, br));
        }
        rows.push(Row {
            name: seq.name.clone(),
            frames: seq.num_frames,
            pos: d.max_pos_cm,
            rot: d.max_rot_deg,
            wpos: format!("f{} {}", d.worst_pos.0, skel.bone_names.get(d.worst_pos.1).cloned().unwrap_or_default()),
            wrot: format!("f{} {}", d.worst_rot.0, skel.bone_names.get(d.worst_rot.1).cloned().unwrap_or_default()),
            identical: same,
            meta,
            fc,
            jc,
        });
    }
    println!("clips verified: {} (missing md5: {missing}); frame+joint count equal: {}/{}", rows.len(), rows.iter().filter(|r| r.fc && r.jc).count(), rows.len());
    println!("byte-identical md5anim text: {identical}/{}", rows.len());
    println!("  by (has sparse-key interpolation, identical): {interp_stats:?}");
    println!("max over all frames of all clips: pos {:.6} cm, rot {:.6} deg, max |dxyz| {:.3e}; joints compared {}", total.max_pos_cm, total.max_rot_deg, total.max_xyz, total.joints_compared);
    let meta_bad: Vec<&Row> = rows.iter().filter(|r| !r.meta.is_empty()).collect();
    println!("clips with metadata mismatch: {}", meta_bad.len());
    for r in meta_bad.iter().take(10) {
        println!("  {} {:?}", r.name, r.meta);
    }
    rows.sort_by(|a, b| (b.pos.max(b.rot)).partial_cmp(&a.pos.max(a.rot)).unwrap());
    println!("worst clips (pos cm / rot deg):");
    for r in rows.iter().take(12) {
        println!("  {:50} frames {:3} pos {:.6} rot {:.6}  [{}] [{}] identical={}", r.name, r.frames, r.pos, r.rot, r.wpos, r.wrot, r.identical);
    }
    if show_clips {
        println!("all clips:");
        for r in &rows {
            println!("  {:50} frames {:3} pos {:.6} rot {:.6} identical={}", r.name, r.frames, r.pos, r.rot, r.identical);
        }
    }
    if samples {
        println!("sampled comparison ({} clips x 20 times): pos cm / rot deg", sampled_rows.len());
        for (n, d, d2, add, bp, br) in &sampled_rows {
            println!("  {:36} {} pos {:.6} cm [{}] | nlerp rot {:.4} deg | slerp rot {:.5} deg [{}] xyz {:.1e} (joints {})", n, if *add { "ADD " } else { "core" }, d.max_pos_cm, bp, d.max_rot_deg, d2.max_rot_deg, br, d2.max_xyz, d.joints_compared);
        }
    }
}
