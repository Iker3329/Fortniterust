//! AIM PITCH / RELAXED measurement: the held AR's barrel pitch in character space follows the camera pitch through the 1.8 AimOffset
//! (`AO_AR_NT_CMM`, RotationOffsetBlendSpace nodes fed AdjustedAimPitch). Needs the 1.8 install (`FN_TEST_ROOT`).
//! Run: `cargo test --release -p fn-app aim_pitch -- --nocapture`.
#![cfg(test)]
use crate::gun_fit::principal_axis;
use crate::loader::Source;
use crate::scene::{pawn_matrix, Character};
use fn_animgraph::Node;
use fn_game::GameSim;
use fn_pose::loadout::{Resources, ITEMS};
use fn_pose::{CharacterRig, ClipSourceDir, ClipStore, SampleMode};
use glam::{Vec2, Vec3};
use std::sync::Arc;

const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

#[test]
fn aim_pitch() {
    let root = std::env::var_os("FN_TEST_ROOT").map(std::path::PathBuf::from).unwrap_or_else(|| DEFAULT_ROOT.into());
    let dir = crate::assets::find_paks_dir(&root).expect("paks dir");
    let set = Arc::new(fn_pak::PakSet::open_install(&dir).unwrap());
    let src = ClipSourceDir::Pak(&set);
    let res = Resources::load(&src).unwrap();
    let mut parts = fn_pose::loader::package_parts(&src).unwrap();
    parts.truncate(2);
    let mut rig = CharacterRig::new(parts);
    if let Some(m) = crate::br_character::load_for(&Source::Paks(dir.clone()), Some(&set)) { rig = CharacterRig { parts: m.rig_parts, ..rig }; }
    rig.sockets = vec![fn_pose::SocketDef::from_original_yaw_socket(0, &res.player_hand_socket).unwrap()];
    let skel = fn_pose::clip::load_player_skeleton_info(&src).unwrap();
    let mut store = ClipStore::load(&src, &fn_legacy::prototype_clip_names(), &skel, SampleMode::Ue).unwrap();
    res.install_player_clips(&mut store);
    for n in ["AR_NonTargeted_CU_CMM", "AR_NonTargeted_CD_CMM", "M_NoWep_Relaxed_AO_CC", "M_NoWep_Relaxed_AO_CU", "M_NoWep_Relaxed_AO_CD", "M_AssaultRifle_Relaxed", "M_AssaultRifle_NonTargeted"] {
        match store.clips.get(n) { Some(c) => println!("clip {n:<28} tracked bones {:3} additive {:?} frames {}", c.num_bones(), c.additive_kind(), c.num_frames()), None => println!("clip {n:<28} MISSING from the store") }
    }
    {   // which bones do the AO clips rotate (degrees of the sampled mesh-space rotation offset)?
        use fn_animgraph::ClipSource;
        for n in ["AR_NonTargeted_CU_CMM", "M_NoWep_Relaxed_AO_CU", "M_NoWep_Relaxed_AO_CD"] {
            let c = &store.clips[n]; let mut s = String::new();
            for b in ["pelvis", "spine_01", "spine_02", "spine_03", "clavicle_r", "upperarm_r", "hand_r", "neck_01", "head"] {
                if let Some(x) = c.sample_bone(b, 0., false, (Vec3::ZERO, glam::Quat::IDENTITY)) { s += &format!(" {b} {:.1}", x.1.to_axis_angle().1.to_degrees()); }
            }
            println!("AO clip {n:<24}:{s}");
        }
    }
    let mut ch = Character::new(rig, Box::new(store));

    // the assault rifle (item 0): vertices in gun space, barrel axis = principal axis oriented toward the Muzzle socket
    let item = &res.items[0];
    let part = item.mesh.clone().unwrap();
    let gun_clips = res.weapon_clips(0).unwrap();
    let grig = CharacterRig::new(vec![part.clone()]);
    let gpose = grig.evaluate(&Node::Ref, &gun_clips);
    let gp = &gpose.parts[0];
    let verts: Vec<Vec3> = part.skin_cpu(&gp.skin_matrices).iter().flat_map(|(p, _)| p.iter().copied()).collect();
    let json = fn_fx::data::PackageJson::load(&set, ITEMS[0]).unwrap();
    let def = json.exports.iter().find(|e| e.class.ends_with("ItemDefinition")).unwrap();
    let mesh_long = fn_fx::data::long_package(def.props.get("WeaponMeshOverride").and_then(|v| v.as_str()).unwrap()).to_owned();
    let smesh = fn_mesh::SkeletalMesh::parse(&fn_pose::loader::package(&src, &mesh_long).unwrap()).unwrap();
    let muzzle_sock = smesh.sockets.iter().find(|s| s.name == "Muzzle").expect("Muzzle socket");
    let muzzle = gp.bone_mesh[part.tree.bone_index(&muzzle_sock.bone).unwrap()].transform_point3(Vec3::from(fn_mesh::gltf::ue_to_gltf_position(muzzle_sock.location)));
    let (centroid, axis) = principal_axis(&verts);
    let barrel = if axis.dot((muzzle - centroid).normalize()) < 0. { -axis } else { axis };

    let mut report = |label: &str, targeting: bool, pitch: f32, idle_secs: f32| -> (f32, f32) {
        let mut sim = GameSim::new();
        sim.world.props.clear();
        sim.yaw = 0.;
        sim.pitch = pitch;
        sim.set_held(Some(("AssaultRifle", targeting)));
        // the native relaxed update: the rifle was last fired long ago (LastFireTime -10); `idle_secs` of ticking at 30 Hz decides hip / relaxed
        let (dt, mut now) = (1. / 30., 0f32);
        for _ in 0..(idle_secs / dt).round() as usize { now += dt; sim.set_weapon_clock(Some(fn_animgraph::WeaponClock { world_time: now, last_fire_time: -10., equipping: false, reloading: false, charging: false })); sim.step_anim(Vec2::ZERO, dt); }
        sim.arms.clear();
        ch.evaluate(&mut sim);
        let world = pawn_matrix(&sim);
        let gun_to_world = world * ch.pose.socket("RightHand").unwrap();
        let b = world.inverse().transform_vector3(gun_to_world.transform_vector3(barrel).normalize());
        let (yaw, bp) = ((-b.x).atan2(b.z).to_degrees(), b.y.asin().to_degrees());
        let v = &sim.anim.vars;
        println!("{label:<10} camera pitch {pitch:6.1} | aim_pitch {:6.1} adj {:6.1} ubao {:.0} fbao {:.0} relaxed1 {} | barrel yaw {yaw:7.2} pitch {bp:7.2} deg (char space)", v.aim_pitch, v.adjusted_aim_pitch, v.upper_body_aim_offset_alpha, v.full_body_aim_offset_alpha, v.b_is_relaxed_level1);
        (yaw, bp)
    };
    // before the fix (no AO data for AO_AR_NT_CMM): barrel pitch 1.07 / 1.07 / 1.07 deg at camera pitch -60 / 0 / +60 (hip and ADS alike).
    let mut hip_pitch0 = 0f32;
    for (label, targeting, idle) in [("hip", false, 1.0f32), ("targeting", true, 1.0), ("relaxed", false, 6.0)] {
        let r: Vec<(f32, f32)> = [-60f32, 0., 60.].iter().map(|p| report(label, targeting, *p, idle)).collect();
        println!("  {label}: barrel pitch -60 -> {:.2}, 0 -> {:.2}, +60 -> {:.2} (spread {:.2} deg)", r[0].1, r[1].1, r[2].1, r[2].1 - r[0].1);
        if label == "relaxed" {
            // RelaxedAimOffset = M_NoWep_Relaxed_AO keys only pelvis / spine / clavicle_r / neck / head (no hand, IK or upperarm track): the lowered gun stays put, the head tracks.
            assert!((r[2].1 - r[0].1).abs() < 0.5 && r[1].1 < hip_pitch0 - 15., "relaxed: lowered ({:.1} deg vs hip {hip_pitch0:.1}) and not pitched by the AO: {r:?}", r[1].1);
        } else {
            hip_pitch0 = r[1].1;
            assert!(r[0].1 < r[1].1 - 30. && r[1].1 < r[2].1 - 30., "{label}: the held weapon pitches with the camera: {r:?}");
            assert!(r[2].1 - r[0].1 > 90., "{label}: spread {:.1}", r[2].1 - r[0].1);
        }
    }
}
