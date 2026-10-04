//! Real-data checks of the BR part AnimBPs (skipped when the 1.8 install is absent).
use fn_animgraph::Node;
use fn_pose::br_character::{BrCharacter, DEFAULT_HERO};
use fn_pose::part_anim::{copy_pose, PartAnim, PartAnimInputs, PartAnimRig, PNode};
use fn_pose::{CharacterPose, CharacterRig, ClipSourceDir, ClipStore, EvalOptions};

fn install() -> Option<fn_pak::PakSet> {
    let root = std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    fn_pak::PakSet::open_install(root).ok()
}

#[test]
fn decode_both_part_animbps_and_report() {
    let Some(set) = install() else { eprintln!("1.8 install not found: skipped"); return };
    let src = ClipSourceDir::Pak(&set);
    let ch = BrCharacter::load(&src, DEFAULT_HERO).unwrap();
    for p in &ch.parts {
        let Some(ac) = &p.def.anim_class else { continue };
        let a = PartAnim::load(&src, ac, p.def.kind).unwrap();
        eprintln!("== {:?} {}: skeleton {:?}", p.def.kind, a.graph.package, a.graph.target_skeleton);
        for (i, n) in a.graph.nodes.iter().enumerate() { eprintln!("  [{i}] {} = {:?}", a.graph.node_names[i], match n { PNode::Dynamics { params, input } => format!("Dynamics bone {} input {input} alpha {} grav {} ang_spring {}/{}", params.bound_bone, params.alpha, params.gravity_scale, params.angular_spring, params.angular_spring_constant), o => format!("{o:?}") }); }
        eprintln!("  chain: {:?}", a.graph.dynamics_chain().iter().map(|(_, p)| p.bound_bone.clone()).collect::<Vec<_>>());
        eprintln!("  missing {:?} unsupported {:?} notes {:?}", a.missing, a.graph.unsupported(), a.graph.notes);
        for (bone, _) in a.graph.dynamics_chain().iter().map(|(_, p)| (p.bound_bone.clone(), ())) { assert!(p.part.tree.bone_index(&bone).is_some(), "{bone} not in the part skeleton"); }
        let hi = PartAnimInputs { pawn_speed_2d: 600., can_play_custom_animations: false };
        let run = a.pin_values(&hi);
        eprintln!("  pins @600cm/s: {run:?}");
        match p.def.kind {
            fn_pose::br_character::PartKind::Body => { assert_eq!(a.graph.dynamics_chain().len(), 9); assert_eq!(run.iter().filter(|(_, m, _)| m == "Alpha").count(), 4); assert!(run.iter().all(|(_, _, v)| (*v - 0.33).abs() < 1e-5)); }
            fn_pose::br_character::PartKind::Head => { assert!(a.missing.is_empty(), "{:?}", a.missing); assert!(run.iter().any(|(_, m, v)| m == "bActiveValue" && *v == 0.)); }
            _ => {}
        }
    }
}

#[test]
fn master_copy_vs_part_bind_and_simulation_runs() {
    let Some(set) = install() else { return };
    let src = ClipSourceDir::Pak(&set);
    let ch = BrCharacter::load(&src, DEFAULT_HERO).unwrap();
    let master = ch.master.clone().unwrap();
    let rig = CharacterRig { parts: ch.rig_parts(), sockets: vec![fn_pose::SocketDef::right_hand(0)], opts: EvalOptions::default() };
    let anims: Vec<Option<PartAnim>> = ch.parts.iter().take(2).map(|p| p.def.anim_class.as_ref().map(|a| PartAnim::load(&src, a, p.def.kind).unwrap())).collect();
    let mut par = PartAnimRig::new(&rig, master, anims);
    for n in &par.notes { eprintln!("note: {n}"); }
    // clips for the jog
    let skel = fn_pose::clip::load_player_skeleton_info(&src).unwrap();
    let names: Vec<String> = ["M_Jog_AR_Core".to_string()].to_vec();
    let store = ClipStore::load(&src, &names, &skel, fn_pose::SampleMode::Ue);
    let tree = match &store { Ok(_) => Node::clip("M_Jog_AR_Core", 0.2, true), Err(e) => { eprintln!("jog clip not loaded: {e}"); Node::Ref } };
    let empty = ClipStore::new();
    let clips: &dyn fn_animgraph::ClipLibrary = match &store { Ok(s) => s, Err(_) => &empty };
    // difference between evaluating the tree on each part's own bind (old path) and the copy of the master pose (CopyPoseFromMesh)
    let old = rig.evaluate(&tree, clips);
    let mut new = CharacterPose::default();
    let inp = PartAnimInputs { pawn_speed_2d: 600., can_play_custom_animations: false };
    par.evaluate_into(&rig, &tree, clips, 1. / 60., &inp, &mut new);
    for (pi, part) in rig.parts.iter().enumerate() {
        let (mut worst, mut worst_bone) = (0f32, String::new());
        let copy = copy_pose(&part.tree, &fn_pose::PoseEvaluator::new(&par.master.tree, clips, EvalOptions::default()).eval_pose(&tree), &(0..part.tree.len()).map(|i| if i == 0 { None } else { par.master.tree.bone_index(&part.tree.names[i]) }).collect::<Vec<_>>());
        let (co, cn) = (part.tree.component(&old.parts[pi].bone_local), part.tree.component(&copy));
        for i in 1..part.tree.len() { let d = (co[i].0 - cn[i].0).length(); if d > worst { worst = d; worst_bone = part.tree.names[i].clone(); } }
        eprintln!("part {pi}: old-path vs master-copy max component-space bone distance {worst:.4} m at {worst_bone}");
    }
    // rest-pose deformation caused by following the master's bind (CopyPoseFromMesh) with the part's own inverse bind
    for (pi, part) in rig.parts.iter().enumerate() {
        let rest = PartAnimRig::new(&rig, par.master.clone(), vec![None, None]);
        let mut r = CharacterPose::default();
        let mut rr = rest; rr.evaluate_into(&rig, &Node::Ref, &ClipStore::new(), 1. / 60., &inp, &mut r);
        let skinned = part.skin_cpu(&r.parts[pi].skin_matrices);
        let mut d: Vec<(f32, usize, usize)> = vec![];
        for (si, prim) in part.prims.iter().enumerate() { for (vi, p) in prim.positions.iter().enumerate() { d.push(((skinned[si].0[vi] - glam::Vec3::from(*p)).length(), si, vi)); } }
        d.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let (n, p95) = (d.len(), d[d.len() / 20].0);
        let worst = d[0];
        let prim = &part.prims[worst.1];
        let bone = (0..4).max_by(|a, b| prim.weights[worst.2][*a].partial_cmp(&prim.weights[worst.2][*b]).unwrap()).map(|k| part.tree.names[part.joints[prim.joints[worst.2][k] as usize]].clone()).unwrap();
        eprintln!("part {pi} rest pose following the master bind: {n} vertices, max displacement {:.3} m (main bone {bone}), 5% worst >= {p95:.3} m", worst.0);
    }
    // run the simulation for a second at jog speed and report how far the dyn bones move from the animated pose
    for _ in 0..60 { par.evaluate_into(&rig, &tree, clips, 1. / 60., &inp, &mut new); }
    let body = &rig.parts[0];
    let animated = copy_pose(&body.tree, &fn_pose::PoseEvaluator::new(&par.master.tree, clips, EvalOptions::default()).eval_pose(&tree), &(0..body.tree.len()).map(|i| if i == 0 { None } else { par.master.tree.bone_index(&body.tree.names[i]) }).collect::<Vec<_>>());
    let ca = body.tree.component(&animated);
    let cs = body.tree.component(&new.parts[0].bone_local);
    for (i, n) in body.tree.names.iter().enumerate() { if n.starts_with("dyn_") { eprintln!("  {n}: moved {:.2} cm from the animated pose", (ca[i].0 - cs[i].0).length() * 100.); } }
    assert!(new.parts[0].skin_matrices.iter().all(|m| m.is_finite()) && new.parts[1].skin_matrices.iter().all(|m| m.is_finite()));
}
