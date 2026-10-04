//! Diagnostic: raw UE reference-pose differences of part skeletons vs the master (before the glTF exporter's scale bake).
use fn_pose::br_character::{read_part_def, DEFAULT_HERO, hero_part_packages, MASTER_MESH_PKG};
use fn_pose::{loader, ClipSourceDir};
use std::collections::HashMap;

#[test]
fn raw_ref_pose_deltas_vs_master() {
    let root = std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let Ok(set) = fn_pak::PakSet::open_install(root) else { return };
    let src = ClipSourceDir::Pak(&set);
    let mesh = |long: &str| fn_mesh::SkeletalMesh::parse(&loader::package(&src, long).unwrap()).unwrap();
    let master = mesh(MASTER_MESH_PKG);
    let mi: HashMap<&str, usize> = master.ref_skeleton.iter().enumerate().map(|(i, b)| (b.name.as_str(), i)).collect();
    for pp in hero_part_packages(&src, DEFAULT_HERO).unwrap() {
        let def = read_part_def(&src, &pp).unwrap();
        let m = mesh(def.skeletal_mesh.split('.').next().unwrap());
        let mut rows = vec![];
        let (mut n_scale, mut n_pos, mut n_rot) = (0, 0, 0);
        for b in &m.ref_skeleton {
            let Some(&j) = mi.get(b.name.as_str()) else { continue };
            let mb = &master.ref_skeleton[j];
            let dp = ((b.ref_pos[0] - mb.ref_pos[0]).powi(2) + (b.ref_pos[1] - mb.ref_pos[1]).powi(2) + (b.ref_pos[2] - mb.ref_pos[2]).powi(2)).sqrt();
            let ds = (b.ref_scale[0] - mb.ref_scale[0]).abs().max((b.ref_scale[1] - mb.ref_scale[1]).abs()).max((b.ref_scale[2] - mb.ref_scale[2]).abs());
            let dr = 1. - (b.ref_rot[0] * mb.ref_rot[0] + b.ref_rot[1] * mb.ref_rot[1] + b.ref_rot[2] * mb.ref_rot[2] + b.ref_rot[3] * mb.ref_rot[3]).abs();
            if dp > 0.01 { n_pos += 1; } if ds > 1e-4 { n_scale += 1; } if dr > 1e-5 { n_rot += 1; }
            rows.push((dp, ds, dr, b.name.clone(), b.ref_pos, mb.ref_pos, b.ref_scale, mb.ref_scale, b.retarget));
        }
        rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        eprintln!("== {} ({} bones; shared with master {}): bones with |dpos|>0.01cm {n_pos}, scale differs {n_scale}, rot differs {n_rot}", def.skeletal_mesh, m.ref_skeleton.len(), rows.len());
        for r in rows.iter().take(8) { eprintln!("  {:<22} dpos {:.3} cm  part {:?} master {:?}  scale part {:?} master {:?} dscale {:.4} drot {:.2e}", r.3, r.0, r.4, r.5, r.6, r.7, r.1, r.2); }
        // retarget modes (player USkeleton BoneTree) of the bones whose bind differs
        let skel = fn_pose::clip::load_player_skeleton_info(&src).unwrap();
        let mut counts: std::collections::BTreeMap<String, (usize, f32, String)> = Default::default();
        for r in rows.iter().filter(|r| r.0 > 0.5) {
            let mode = skel.find_bone(&r.3).map(|i| format!("{:?}", skel.retarget[i])).unwrap_or_else(|| "not in the player USkeleton".into());
            let e = counts.entry(mode).or_insert((0, 0., String::new()));
            e.0 += 1; if r.0 > e.1 { e.1 = r.0; e.2 = r.3.clone(); }
        }
        eprintln!("  bones with bind translation delta > 0.5 cm by player-skeleton retarget mode: {counts:?}");
        for n in ["thigh_twist_01_l", "foot_l", "pelvis", "upperarm_l", "hand_l", "spine_01"] { if let Some(i) = skel.find_bone(n) { eprintln!("    {n}: mode {:?}, skeleton asset ref translation {:?}", skel.retarget[i], skel.ref_pose[i].translation); } }
        let mut by_scale: Vec<_> = rows.iter().filter(|r| r.1 > 1e-4).collect();
        by_scale.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        for r in by_scale.iter().take(5) { eprintln!("  SCALE {:<22} part {:?} master {:?}", r.3, r.6, r.7); }
    }
}
