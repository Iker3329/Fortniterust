//! Skeleton decoding checks (against the meshes that reference the skeletons and the old exports) and a scan of
//! every Skeleton / SkeletalMesh / StaticMesh export found in the extracted test data.
//! Gated on the test data existing; run with `-- --nocapture` for the tables.

mod common;
use common::*;
use fn_mesh::{Skeleton, SkeletalMesh, StaticMesh, TranslationRetarget};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// (skeleton package, mesh package that uses it, old export of the mesh)
const PAIRS: &[(&str, &str, &str)] = &[
    ("Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_Skeleton", "Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player", "Fortnite_M_Avg_Player"),
    (
        "Characters/Player/Male/Medium/Bodies/M_Med_Soldier_04/Meshes/SK_M_Med_Soldier_04_Skeleton",
        "Characters/Player/Male/Medium/Bodies/M_Med_Soldier_04/Meshes/SK_M_Med_Soldier_04",
        "SK_M_Med_Soldier_04",
    ),
];

#[test]
fn skeletons_decode_and_agree_with_their_meshes() {
    let (Some(td), Some(old)) = (testdata(), old_exports()) else {
        eprintln!("NOTE: test data or old exports missing; skipping skeleton verification");
        return;
    };
    println!("\nskeleton asset vs mesh RefSkeleton vs old gltf");
    for (skel_path, mesh_path, name) in PAIRS {
        let sp = td.join(skel_path);
        if !sp.with_extension("uasset").exists() {
            eprintln!("NOTE: {skel_path} not extracted; skipping");
            continue;
        }
        let skel = Skeleton::parse(&load_package(&sp)).unwrap();
        let mesh = SkeletalMesh::parse(&load_package(&td.join(mesh_path))).unwrap();
        // unique names, root has no parent, parents refer to existing bones
        let mut names: HashMap<&str, usize> = HashMap::new();
        for (i, b) in skel.bones.iter().enumerate() {
            assert!(names.insert(&b.name, i).is_none(), "duplicate bone {}", b.name);
            assert!(b.parent < skel.bones.len() as i32);
        }
        assert_eq!(skel.bones[0].parent, -1);
        let topo = skel.bones.iter().enumerate().all(|(i, b)| b.parent < i as i32);
        // every mesh bone is in the skeleton, with the same parent and reference pose
        let mut missing = 0;
        let mut bad_parent = 0;
        let mut pose_err = 0f32;
        for b in &mesh.ref_skeleton {
            let Some(&si) = names.get(b.name.as_str()) else {
                missing += 1;
                continue;
            };
            let sb = &skel.bones[si];
            let sp = if sb.parent < 0 { None } else { Some(skel.bones[sb.parent as usize].name.as_str()) };
            let mp = if b.parent < 0 { None } else { Some(mesh.ref_skeleton[b.parent as usize].name.as_str()) };
            if sp != mp {
                bad_parent += 1;
            }
            for c in 0..3 {
                pose_err = pose_err.max((sb.ref_pos[c] - b.ref_pos[c]).abs()).max((sb.ref_scale[c] - b.ref_scale[c]).abs());
            }
            // quaternions q and -q are the same rotation
            let dq: f32 = (0..4).map(|c| (sb.ref_rot[c] - b.ref_rot[c]).abs()).fold(0.0, f32::max);
            let dqn: f32 = (0..4).map(|c| (sb.ref_rot[c] + b.ref_rot[c]).abs()).fold(0.0, f32::max);
            pose_err = pose_err.max(dq.min(dqn));
        }
        // names/parents of the old gltf export of the mesh must exist in the skeleton too
        let dir = Path::new(mesh_path).parent().unwrap();
        let g = Gltf::load(&old.join(dir).join(name));
        let nodes = g.json["nodes"].as_array().unwrap();
        let mut old_parent: HashMap<String, String> = HashMap::new();
        for n in nodes {
            if let Some(ch) = n["children"].as_array() {
                for c in ch {
                    old_parent.insert(nodes[c.as_u64().unwrap() as usize]["name"].as_str().unwrap().to_string(), n["name"].as_str().unwrap().to_string());
                }
            }
        }
        let mut old_missing = 0;
        let mut old_bad_parent = 0;
        for n in &nodes[1..] {
            let nm = n["name"].as_str().unwrap();
            match names.get(nm) {
                None => old_missing += 1,
                Some(&si) => {
                    let sb = &skel.bones[si];
                    if sb.parent >= 0 && old_parent.get(nm).map(String::as_str) != Some(skel.bones[sb.parent as usize].name.as_str()) {
                        old_bad_parent += 1;
                    }
                }
            }
        }
        let mut modes: HashMap<String, usize> = HashMap::new();
        for b in &skel.bones {
            *modes.entry(format!("{:?}", b.retarget)).or_default() += 1;
        }
        println!(
            "{:<34} skeleton bones {:>3} (parents-before-children {}) | mesh {} bones: missing {} bad parent {} ref pose max err {:.1e} | old gltf {} bones: missing {} bad parent {} | sockets {} curves {} retarget srcs {} slot groups {} | retarget modes {:?}",
            skel.name,
            skel.bones.len(),
            topo,
            mesh.ref_skeleton.len(),
            missing,
            bad_parent,
            pose_err,
            nodes.len() - 1,
            old_missing,
            old_bad_parent,
            skel.sockets.len(),
            skel.curves.len(),
            skel.retarget_sources.len(),
            skel.slot_groups.len(),
            modes
        );
        assert_eq!(missing, 0);
        assert_eq!(bad_parent, 0);
        assert!(pose_err < 1e-3, "{}: mesh/skeleton reference pose differ by {pose_err}", skel.name);
        assert_eq!(old_missing, 0);
        assert_eq!(old_bad_parent, 0);
        for s in &skel.sockets {
            assert!(skel.bone_index_ci(&s.bone).is_some(), "socket {} on unknown bone {}", s.name, s.bone);
        }
        for r in &skel.retarget_sources {
            assert!(r.pose.len() <= skel.bones.len());
        }
    }
    // The player skeleton in this build has 266 bones (not 93); body skeleton 99, body mesh 92 (+1 mesh node = 93 gltf nodes).
    let p = td.join(PAIRS[0].0);
    let skel = Skeleton::parse(&load_package(&p)).unwrap();
    assert_eq!(skel.bones.len(), 266);
    assert_eq!(skel.sockets.len(), 50);
    assert_eq!(skel.bone_index("pelvis"), Some(1));
    assert_eq!(skel.bones[1].retarget, TranslationRetarget::AnimationScaled);
    assert_eq!(skel.bones[7].retarget, TranslationRetarget::Skeleton); // clavicle_l
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map_or(false, |x| x == "uasset") {
                out.push(p);
            }
        }
    }
}

#[test]
fn scan_every_mesh_export_in_testdata() {
    let Some(td) = testdata() else {
        eprintln!("NOTE: test data missing; skipping mesh scan");
        return;
    };
    let mut files = vec![];
    walk(&td, &mut files);
    let (mut packages, mut skel_ok, mut sk_ok, mut st_ok) = (0, 0, 0, 0);
    let mut failures = vec![];
    for f in &files {
        let rd = |e: &str| std::fs::read(f.with_extension(e)).unwrap_or_default();
        let Ok(pkg) = fn_ueio::Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")) else { continue };
        packages += 1;
        for (class, which) in [("Skeleton", 0), ("SkeletalMesh", 1), ("StaticMesh", 2)] {
            if pkg.find_export(class).is_none() {
                continue;
            }
            let rel = f.strip_prefix(&td).unwrap().display().to_string();
            let r: Result<String, String> = match which {
                0 => Skeleton::parse(&pkg).map(|s| format!("{} bones, {} sockets", s.bones.len(), s.sockets.len())).map_err(|e| e.to_string()),
                1 => SkeletalMesh::parse(&pkg)
                    .and_then(|m| {
                        // every LOD must also convert to glTF
                        for l in 0..m.lods.len() {
                            fn_mesh::gltf::skeletal_mesh_lod_to_gltf(&m, l, &m.name)?;
                        }
                        Ok(m)
                    })
                    .map(|m| format!("{} bones, {} LODs, {} verts LOD0", m.ref_skeleton.len(), m.lods.len(), m.lods[0].num_vertices))
                    .map_err(|e| e.to_string()),
                _ => StaticMesh::parse(&pkg)
                    .and_then(|m| {
                        for l in 0..m.lods.len() {
                            fn_mesh::gltf::static_mesh_lod_to_gltf(&m, l, &m.name)?;
                        }
                        Ok(m)
                    })
                    .map(|m| format!("{} LODs, {} verts LOD0", m.lods.len(), m.lods[0].positions.len()))
                    .map_err(|e| e.to_string()),
            };
            match r {
                Ok(s) => {
                    match which {
                        0 => skel_ok += 1,
                        1 => sk_ok += 1,
                        _ => st_ok += 1,
                    }
                    println!("OK   {class:<13} {rel}: {s}");
                }
                Err(e) => {
                    println!("FAIL {class:<13} {rel}: {e}");
                    failures.push(rel);
                }
            }
        }
    }
    println!("scanned {packages} packages: {skel_ok} Skeleton, {sk_ok} SkeletalMesh, {st_ok} StaticMesh parsed; {} failures", failures.len());
    assert!(failures.is_empty(), "failures: {failures:?}");
    assert!(sk_ok >= 5 && st_ok >= 2 && skel_ok >= 1);
}
