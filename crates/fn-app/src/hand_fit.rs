//! HAND FIT: where do hand_l / hand_r sit relative to the held AR in every state (hip / targeting x aim pitch, relaxed, fire montage, after fire, reload),
//! plus a node-by-node bisection of the pose tree and a sway (yaw over time) measurement. Same setup as gun_fit.rs. Needs the 1.8 install (`FN_TEST_ROOT`).
//! Run: `cargo test --release -p fn-app hand_fit -- --nocapture`.
#![cfg(test)]
use crate::arms::{Arms, ArmsData, ArmsInput, Policy};
use crate::loader::Source;
use crate::scene::{pawn_matrix, Character};
use fn_animgraph::{Montage, Node};
use fn_game::GameSim;
use fn_pose::loadout::Resources;
use fn_pose::{CharacterRig, ClipSourceDir, ClipStore, SampleMode};
use glam::{Mat4, Vec2, Vec3};
use std::sync::Arc;

const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

fn nearest(points: &[Vec3], p: Vec3) -> f32 { points.iter().map(|q| q.distance(p)).fold(f32::MAX, f32::min) }
/// Closest point on triangle (Ericson). The guns are low-poly (vertices 20 cm apart on the hand guard), so surface distance, not nearest-vertex distance, is the grip metric.
fn tri_closest(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0. && d2 <= 0. { return a; }
    let bp = p - b; let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0. && d4 <= d3 { return b; }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0. && d1 >= 0. && d3 <= 0. { return a + ab * (d1 / (d1 - d3)); }
    let cp = p - c; let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0. && d5 <= d6 { return c; }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0. && d2 >= 0. && d6 <= 0. { return a + ac * (d2 / (d2 - d6)); }
    let va = d3 * d6 - d5 * d4;
    if va <= 0. && (d4 - d3) >= 0. && (d5 - d6) >= 0. { let w = (d4 - d3) / ((d4 - d3) + (d5 - d6)); return b + (c - b) * w; }
    let denom = 1. / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}
fn tri_dist(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f32 { (p - tri_closest(p, a, b, c)).length() }
fn closest_on(tris: &[[Vec3; 3]], p: Vec3) -> Vec3 { tris.iter().map(|t| tri_closest(p, t[0], t[1], t[2])).min_by(|a, b| a.distance(p).total_cmp(&b.distance(p))).unwrap() }fn surf(tris: &[[Vec3; 3]], p: Vec3) -> f32 { tris.iter().map(|t| tri_dist(p, t[0], t[1], t[2])).fold(f32::MAX, f32::min) }

struct Env { clip_names: Vec<String>, all_clip_names: Vec<String>, ch: Character, verts: Vec<Vec3>, tris: Vec<[Vec3; 3]>, all_tris: Vec<Vec<[Vec3; 3]>>, all_verts: Vec<(&'static str, Vec<Vec3>)>, data: ArmsData, ref_l: std::cell::Cell<Option<glam::Quat>> }

fn yaw_of(m: Mat4) -> f32 { let f = m.transform_vector3(Vec3::Z); (-f.x).atan2(f.z).to_degrees() }

impl Env {
    /// (hand_r cm, hand_l cm, left ik alpha, snap_weapon_lhand, hand_l -> ik_hand_l cm) of the current `ch.pose`.
    fn measure(&self, sim: &GameSim, pose: &fn_pose::CharacterPose) -> [f32; 6] {
        let world = pawn_matrix(sim);
        let body = &self.ch.rig.parts[0];
        let bp = |n: &str| world.transform_point3(pose.parts[0].bone_mesh[body.tree.bone_index(n).unwrap()].w_axis.truncate());
        let g = world * pose.socket("RightHand").unwrap();
        let wv: Vec<[Vec3; 3]> = self.tris.iter().map(|t| [g.transform_point3(t[0]), g.transform_point3(t[1]), g.transform_point3(t[2])]).collect();
        let v = &sim.anim.vars;
        let rel = glam::Quat::from_mat4(&(g.inverse() * world * pose.parts[0].bone_mesh[body.tree.bone_index("hand_l").unwrap()]).transpose().transpose());
        let rel = rel.normalize();
        if self.ref_l.get().is_none() { self.ref_l.set(Some(rel)); }
        let rot = rel.angle_between(self.ref_l.get().unwrap()).to_degrees();
        [surf(&wv, bp("hand_r")) * 100., surf(&wv, bp("hand_l")) * 100., v.left_hand_ik_alpha, v.snap_weapon_lhand, bp("hand_l").distance(bp("ik_hand_l")) * 100., rot]
    }
    fn line(&mut self, label: &str, sim: &mut GameSim) -> [f32; 6] {
        sim.arms.clear();
        self.ch.evaluate(sim);
        let m = self.measure(sim, &self.ch.pose);
        let flag = if m[1] > 6. { "  <== LEFT HAND OFF GUN" } else { "" };
        let fd = self.finger_dev(&self.ch.pose);
        println!("{label:<34} hand_r {:5.2} hand_l {:5.2} Lalpha {:.2} snap {:.2} |hand_l-ik| {:5.2} rotL {:5.1} | finger dev L {:5.1} R {:5.1} vs {}{flag}", m[0], m[1], m[2], m[3], m[4], m[5], fd.0, fd.1, fd.2);
        m
    }
}

/// peel one single-base node layer (None for leaves / mixes)
fn base(n: &Node) -> Option<(&'static str, &Node)> {
    Some(match n {
        Node::RotateRoot { base, .. } => ("RotateRoot", base),
        Node::AimOffset { base, .. } => ("AimOffset", base),
        Node::TwoBoneIk { base, ik, .. } => (ik.bone, base),
        Node::HandRetarget { base, .. } => ("HandRetarget", base),
        Node::CopyBone { base, .. } => ("CopyBone", base),
        Node::ModifyBone { base, bone, .. } => (bone, base),
        Node::HeadTrack { base, .. } => ("HeadTrack", base),
        Node::ApplyAdditive { base, .. } => ("ApplyAdditive", base),
        Node::ApplyMeshAdditive { base, .. } => ("ApplyMeshAdditive", base),
        Node::Layered { base, .. } => ("Layered(base)", base),
        _ => return None,
    })
}

fn dump_curves(res: &Resources) {
    println!("\n##### 0. sequences (player skeleton) carrying the hand curves");
    let mut rows: Vec<String> = vec![];
    for (path, s) in &res.sequences {
        for c in &s.source.curves {
            if ["LeftHandFK", "BothHandFK", "SnapWeapon_LHand"].contains(&c.name.as_str()) {
                let (lo, hi) = c.keys.iter().fold((f32::MAX, f32::MIN), |a, k| (a.0.min(k.value), a.1.max(k.value)));
                rows.push(format!("{:<18} keys {:2} {:.2}..{:.2}  {}  (skel {})", c.name, c.keys.len(), lo, hi, path.rsplit('/').next().unwrap_or(path), s.skeleton.rsplit('/').next().unwrap_or("")));
            }
        }
    }
    rows.sort();
    for r in rows { println!("  {r}"); }
}

fn build() -> Env {
    let root = std::env::var_os("FN_TEST_ROOT").map(std::path::PathBuf::from).unwrap_or_else(|| DEFAULT_ROOT.into());
    let dir = crate::assets::find_paks_dir(&root).expect("paks dir");
    let set = Arc::new(fn_pak::PakSet::open_install(&dir).unwrap());
    let src = ClipSourceDir::Pak(&set);
    let res = Resources::load(&src).unwrap();
    let mut parts = fn_pose::loader::package_parts(&src).unwrap();
    parts.truncate(2);
    let mut rig = CharacterRig::new(parts);
    if std::env::var_os("HF_NO_BR").is_none() { if let Some(m) = crate::br_character::load_for(&Source::Paks(dir.clone()), Some(&set)) { rig = CharacterRig { parts: m.rig_parts, ..rig }; } }
    rig.sockets = vec![fn_pose::SocketDef::from_original_yaw_socket(0, &res.player_hand_socket).unwrap()];
    let skel = fn_pose::clip::load_player_skeleton_info(&src).unwrap();
    let mut clip_list = fn_legacy::prototype_clip_names(); for n in fn_animgraph::weapon_clips() { if !clip_list.contains(&n) { clip_list.push(n); } } // every weapon class' clips (probes only)
    let mut store = ClipStore::load(&src, &clip_list, &skel, SampleMode::Ue).unwrap();
    res.install_player_clips(&mut store);
    {
        use fn_animgraph::ClipSource;
        println!("\n##### 0b. clips in the store with the hand curves (name: has LeftHandFK/BothHandFK/SnapWeapon_LHand, value at t=0 / mid / end)");
        let mut names: Vec<&String> = store.clips.keys().collect();
        names.sort();
        for n in names {
            let c = &store.clips[n];
            let has: Vec<&str> = ["LeftHandFK", "BothHandFK", "SnapWeapon_LHand"].into_iter().filter(|k| c.has_float_curve(k)).collect();
            if !has.is_empty() {
                let d = c.duration();
                println!("  {n:<70} {:?} {}", has, has.iter().map(|k| format!("{k}: {:.2}/{:.2}/{:.2}", c.sample_float_curve(k, 0., false), c.sample_float_curve(k, d * 0.5, false), c.sample_float_curve(k, d, false))).collect::<Vec<_>>().join(" "));
            }
        }
    }
    let mut clip_names: Vec<String> = store.clips.iter().filter(|(_, c)| { use fn_animgraph::ClipSource; !c.is_additive() }).map(|(k, _)| k.clone()).collect();
    clip_names.sort();
    let all_clip_names: Vec<String> = store.clips.keys().cloned().collect();
    let ch = Character::new(rig, Box::new(store));
    let part = res.items[0].mesh.clone().unwrap();
    let gun_clips = res.weapon_clips(0).unwrap();
    let gpose = CharacterRig::new(vec![part.clone()]).evaluate(&Node::Ref, &gun_clips);
    let verts: Vec<Vec3> = part.skin_cpu(&gpose.parts[0].skin_matrices).iter().flat_map(|(p, _)| p.iter().copied()).collect();
    dump_curves(&res);
    let mut all_verts: Vec<(&'static str, Vec<Vec3>)> = vec![];
    let mut all_tris: Vec<Vec<[Vec3; 3]>> = vec![];
    for g in 0..3 {
        let part = res.items[g].mesh.clone().unwrap();
        let gp = CharacterRig::new(vec![part.clone()]).evaluate(&Node::Ref, &res.weapon_clips(g).unwrap());
        let core: &'static str = match res.items[g].core_animation.as_deref().unwrap() { "AssaultRifle" => "AssaultRifle", "Rifle" => "Rifle", "Shotgun" => "Shotgun", o => panic!("{o}") };
        let sk = part.skin_cpu(&gp.parts[0].skin_matrices);
        all_verts.push((core, sk.iter().flat_map(|(p, _)| p.iter().copied()).collect()));
        all_tris.push(part.prims.iter().zip(sk.iter()).flat_map(|(pr, (pos, _))| pr.indices.chunks(3).filter(|c| c.len() == 3).map(move |c| [pos[c[0] as usize], pos[c[1] as usize], pos[c[2] as usize]])).collect());
    }
    let data = ArmsData::from_resources(&res).unwrap();
    Env { clip_names, all_clip_names, ch, verts, tris: all_tris[0].clone(), all_tris, all_verts, data, ref_l: std::cell::Cell::new(None) }
}

fn idle_sim(targeting: bool, pitch: f32, secs: f32) -> GameSim {
    let mut sim = GameSim::new();
    sim.world.props.clear();
    sim.yaw = 0.; sim.pitch = pitch;
    sim.set_held(Some(("AssaultRifle", targeting)));
    let (dt, mut now) = (1. / 60., 0f32);
    for _ in 0..(secs / dt).round() as usize { now += dt; sim.set_weapon_clock(Some(fn_animgraph::WeaponClock { world_time: now, last_fire_time: -10., equipping: false, reloading: false, charging: false })); sim.step_anim(Vec2::ZERO, dt); }
    sim
}

impl Env {
    /// (palm_l, mean fingertip_l, palm_r, mean fingertip_r) surface distances to the AR (cm) of the current `ch.pose`.
    fn grip(&self, sim: &GameSim) -> [f32; 4] {
        let world = pawn_matrix(sim);
        let body = &self.ch.rig.parts[0];
        let bp = |n: &str| world.transform_point3(self.ch.pose.parts[0].bone_mesh[body.tree.bone_index(n).unwrap()].w_axis.truncate());
        let g = world * self.ch.pose.socket("RightHand").unwrap();
        let wv: Vec<[Vec3; 3]> = self.all_tris[0].iter().map(|t| [g.transform_point3(t[0]), g.transform_point3(t[1]), g.transform_point3(t[2])]).collect();
        let one = |side: &str| -> (f32, f32) {
            let palm = ["index", "middle", "ring", "pinky"].iter().map(|f| bp(&format!("{f}_01_{side}"))).sum::<Vec3>() / 4.;
            (surf(&wv, palm) * 100., ["index", "middle", "ring", "pinky"].iter().map(|f| surf(&wv, bp(&format!("{f}_03_{side}")))).sum::<f32>() / 4. * 100.)
        };
        let (l, r) = (one("l"), one("r"));
        [l.0, l.1, r.0, r.1]
    }
}
#[test]
fn hand_fit() {
    use fn_animgraph::ClipSource;
    let mut env = build();
    println!("\n##### 0c. authored finger curl: mean angle (deg) of the left / right finger bones' local rotation from the bind pose, per pose clip");
    for c in ["M_AssaultRifle_Targeted", "M_AssaultRifle_NonTargeted", "M_AssaultRifle_Relaxed", "M_NoWep_Relaxed", "M_Melee_TwoHanded", "Idle_Turn_Left"] {
        let body = &env.ch.rig.parts[0];
        let cp = env.ch.rig.evaluate(&Node::clip(c, 0., false), &*env.ch.clips);
        let ang = |side: &str| -> (f32, usize) {
            let mut s = 0.; let mut n = 0;
            for (i, nm) in body.tree.names.iter().enumerate() { let nm = nm.to_lowercase(); if nm.ends_with(side) && ["thumb", "index", "middle", "ring", "pinky"].iter().any(|f| nm.starts_with(f)) { s += cp.parts[0].bone_local[i].1.angle_between(body.tree.local[i].1).to_degrees(); n += 1; } }
            (s / n.max(1) as f32, n)
        };
        let (l, r) = (ang("_l"), ang("_r"));
        println!("  {c:<28} left fingers {:5.1} deg ({} bones)  right fingers {:5.1} deg", l.0, l.1, r.0);
    }
    println!("\n##### 0c2. per-joint finger rotation from bind (deg) in M_AssaultRifle_Targeted / M_Melee_TwoHanded");
    for c in ["M_AssaultRifle_Targeted", "M_Melee_TwoHanded"] {
        let body = &env.ch.rig.parts[0];
        let cp = env.ch.rig.evaluate(&Node::clip(c, 0., false), &*env.ch.clips);
        for side in ["l", "r"] {
            let mut s = format!("  {c} {side}:");
            for f in ["thumb", "index", "middle", "ring", "pinky"] {
                for j in 1..=3 { let n = format!("{f}_0{j}_{side}"); if let Some(i) = body.tree.bone_index(&n) { s += &format!(" {f}{j} {:5.1}", cp.parts[0].bone_local[i].1.angle_between(body.tree.local[i].1).to_degrees()); } }
            }
            println!("{s}");
        }
    }
    println!("\n##### 0c3. left-finger curl (mean angle from bind over the left finger bones) for every non-additive store clip whose name mentions Assault / AR_ / Rifle / Equip / Idle: name, left, right");
    {
        let body = &env.ch.rig.parts[0];
        let lf: Vec<usize> = body.tree.names.iter().enumerate().filter(|(_, n)| { let n = n.to_lowercase(); n.ends_with("_l") && ["thumb", "index", "middle", "ring", "pinky"].iter().any(|f| n.starts_with(f)) }).map(|(i, _)| i).collect();
        let rf: Vec<usize> = body.tree.names.iter().enumerate().filter(|(_, n)| { let n = n.to_lowercase(); n.ends_with("_r") && ["thumb", "index", "middle", "ring", "pinky"].iter().any(|f| n.starts_with(f)) }).map(|(i, _)| i).collect();
        let names = env.clip_names.clone();
        for c in names {
            let pose = env.ch.rig.evaluate(&Node::clip(&c, 0., false), &*env.ch.clips);
            let m = |set: &Vec<usize>| set.iter().map(|&i| pose.parts[0].bone_local[i].1.angle_between(body.tree.local[i].1).to_degrees()).sum::<f32>() / set.len() as f32;
            let (l, r) = (m(&lf), m(&rf));
            let short = c.rsplit('/').next().unwrap_or(&c);
            if ["Assault", "AR_", "Rifle", "Equip", "Idle", "Reload", "Fire"].iter().any(|k| short.contains(k)) { println!("  {short:<60} left {l:5.1} right {r:5.1}"); }
        }
    }
    println!("\n##### 0g. weapon_r / weapon_l / ik bones: does the pose clip carry tracks, and how far is the local rotation from the bind pose (deg)");
    for c in ["M_AssaultRifle_Targeted", "M_AssaultRifle_NonTargeted", "Rifle_Targeted_CMM", "M_Melee_TwoHanded"] {
        let body = &env.ch.rig.parts[0];
        let cp = env.ch.rig.evaluate(&Node::clip(c, 0., false), &*env.ch.clips);
        let mut s = format!("  {c:<28}");
        for b in ["weapon_r", "weapon_l", "hand_r", "hand_l", "ik_hand_root", "ik_hand_gun", "ik_hand_r", "ik_hand_l", "lowerarm_l", "upperarm_l", "clavicle_l"] {
            let i = body.tree.bone_index(b).unwrap();
            let has = env.ch.clips.clip(c).map_or(false, |k| k.has_bone(b));
            s += &format!(" {b}[{}] {:5.1}", if has { "T" } else { "-" }, cp.parts[0].bone_local[i].1.angle_between(body.tree.local[i].1).to_degrees());
        }
        println!("{s}");
    }
    println!("\n##### 0h. additive clips: tracks on the arm / hand / IK bones and the sampled delta (t = 0 and mid): rotation deg, translation cm");
    for c in ["Idle_Noise", "Idle_Noise_AR_Downsights", "M_Jog_Melee_TH_Additive", "M_Jog_ARDS_Additive", "M_Locomotion2D_Walk_AR_Additive", "AssaultRifle_FrontClip_SideAction_FireSmall_CMM", "AO_AR_NT_CMM_CC", "M_Jog_N_Start_Additive"] {
        let key = env.all_clip_names.iter().find(|n| n.rsplit('/').next().unwrap_or(n).split('.').next().unwrap_or(n) == c).cloned();
        let Some(key) = key else { println!("  {c}: not in the store"); continue };
        let clip = env.ch.clips.clip(&key).unwrap();
        let mut s = format!("  {c:<26}");
        for b in ["spine_03", "clavicle_l", "upperarm_l", "lowerarm_l", "hand_l", "upperarm_r", "hand_r", "ik_hand_root", "ik_hand_gun", "ik_hand_r", "ik_hand_l"] {
            let has = clip.has_bone(b);
            let mid = clip.duration() * 0.5;
            let d = |t: f32| clip.sample_bone(b, t, false, (Vec3::ZERO, glam::Quat::IDENTITY)).map_or((0., 0.), |(p, q)| (q.angle_between(glam::Quat::IDENTITY).to_degrees(), p.length() * 100.));
            let (a0, am) = (d(0.), d(mid));
            s += &format!(" {b}[{}] {:4.1}/{:4.1}deg {:4.1}/{:4.1}cm", if has { "T" } else { "-" }, a0.0, am.0, a0.1, am.1);
        }
        println!("{s}");
    }
    println!("\n##### 0j. grip (surface distance, cm) per state: left palm / left mean fingertip | right palm / right mean fingertip; min..max over the sampled frames");
    {
        use fn_game::FrameInput;
        // (name, targeting, forward, crouch, run, secs, last_fire_time -> relaxed when -10 and secs>5 (equip pin ignored: weapon clock fed directly))
        for (name, targeting, fwd, crouch, run, secs) in [("idle hip (not relaxed)", false, 0f32, false, false, 3f32), ("idle targeting", true, 0., false, false, 3.), ("relaxed idle hip (> 5 s)", false, 0., false, false, 9.), ("jog hip", false, 1., false, false, 3.), ("walk targeting", true, 1., false, false, 3.), ("sprint", false, 1., false, true, 3.), ("crouch targeting", true, 0., true, false, 3.), ("crouch jog hip", false, 1., true, false, 3.)] {
            let mut sim = GameSim::new();
            sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
            let dt = 1. / 60.;
            let (mut lo, mut hi) = ([f32::MAX; 4], [f32::MIN; 4]);
            for f in 0..(secs / dt) as usize {
                let t = f as f32 * dt;
                // a shot at t = 0 for the non-relaxed rows keeps them out of the relaxed pose; the relaxed row last fired at -10
                let last = if name.starts_with("relaxed") { -10. } else { t };
                sim.set_held(Some(("AssaultRifle", targeting)));
                sim.set_weapon_clock(Some(fn_animgraph::WeaponClock { world_time: t, last_fire_time: last, equipping: false, reloading: false, charging: false }));
                sim.step(&FrameInput { forward: fwd, force_crouch: crouch, force_run: run, non_melee: true, ..Default::default() }, dt);
                sim.arms.clear();
                env.ch.evaluate(&mut sim);
                if t < 1.5 && !name.starts_with("relaxed") { continue; }
                if name.starts_with("relaxed") && t < 7. { continue; }
                let g = env.grip(&sim);
                for k in 0..4 { lo[k] = lo[k].min(g[k]); hi[k] = hi[k].max(g[k]); }
            }
            println!("  {name:<26} L palm {:.1}..{:.1} tips {:.1}..{:.1} | R palm {:.1}..{:.1} tips {:.1}..{:.1}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], lo[3], hi[3]);
        }
    }    println!("\n##### 0i. left palm / mean fingertip distance to the AR surface over 6 s of targeting idle (every 0.25 s), cm");
    {
        let mut sim = GameSim::new();
        sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
        sim.set_held(Some(("AssaultRifle", true)));
        let mut s = String::new(); let mut rowmax = (0f32, 0f32);
        for f in 0..360 {
            sim.set_weapon_clock(Some(fn_animgraph::WeaponClock { world_time: f as f32 / 60., last_fire_time: f as f32 / 60., equipping: false, reloading: false, charging: false }));
            sim.step_anim(Vec2::ZERO, 1. / 60.);
            sim.arms.clear();
            env.ch.evaluate(&mut sim);
            let world = pawn_matrix(&sim);
            let body = &env.ch.rig.parts[0];
            let bp = |n: &str| world.transform_point3(env.ch.pose.parts[0].bone_mesh[body.tree.bone_index(n).unwrap()].w_axis.truncate());
            let g = world * env.ch.pose.socket("RightHand").unwrap();
            let wv: Vec<[Vec3; 3]> = env.all_tris[0].iter().map(|t| [g.transform_point3(t[0]), g.transform_point3(t[1]), g.transform_point3(t[2])]).collect();
            let palm = ["index", "middle", "ring", "pinky"].iter().map(|f| bp(&format!("{f}_01_l"))).sum::<Vec3>() / 4.;
            let tips = ["index", "middle", "ring", "pinky"].iter().map(|f| surf(&wv, bp(&format!("{f}_03_l")))).sum::<f32>() / 4. * 100.;
            let pd = surf(&wv, palm) * 100.;
            rowmax = (rowmax.0.max(pd), rowmax.1.max(tips));
            if f % 15 == 0 { s += &format!(" {pd:.1}/{tips:.1}"); }
        }
        println!("  palm/tips:{s}\n  max palm {:.1} max tips {:.1}", rowmax.0, rowmax.1);
    }
    println!("\n##### 0f. rigid refit of the AR in the RightHand frame: which gun offset (translation cm in gun axes, yaw about gun Y deg) best seats BOTH hands (cost = sum of fingertip+palm surface distances)");
    {
        let mut sim = idle_sim(true, 0., 1.0);
        sim.arms.clear();
        env.ch.evaluate(&mut sim);
        let body = &env.ch.rig.parts[0];
        let sock = env.ch.pose.socket("RightHand").unwrap();
        let loc = |n: &str| -> Vec3 { sock.inverse().transform_point3(env.ch.pose.parts[0].bone_mesh[body.tree.bone_index(n).unwrap()].w_axis.truncate()) };
        // gun surface samples (gun space, metres): triangle vertices + edge midpoints + centroids
        let mut pts: Vec<Vec3> = vec![];
        for t in &env.all_tris[0] { pts.extend([t[0], t[1], t[2], (t[0] + t[1]) * 0.5, (t[1] + t[2]) * 0.5, (t[0] + t[2]) * 0.5, (t[0] + t[1] + t[2]) / 3.]); }
        let hand_pts = |side: &str| -> Vec<Vec3> {
            let mut v: Vec<Vec3> = ["thumb", "index", "middle", "ring", "pinky"].iter().map(|f| loc(&format!("{f}_03_{side}"))).collect();
            v.push(["index", "middle", "ring", "pinky"].iter().map(|f| loc(&format!("{f}_01_{side}"))).sum::<Vec3>() / 4.);
            v
        };
        let (lp, rp) = (hand_pts("l"), hand_pts("r"));
        let cost = |tr: Vec3, yaw: f32, set: &[Vec3]| -> f32 {
            // the gun moved by (T, yaw): equivalent to moving the hand points by the inverse
            let q = glam::Quat::from_rotation_y(yaw.to_radians());
            set.iter().map(|p| { let pl = q.conjugate() * (*p - tr); pts.iter().map(|g| g.distance(pl)).fold(f32::MAX, f32::min) }).sum::<f32>() * 100.
        };
        println!("  current: left {:.1} right {:.1}", cost(Vec3::ZERO, 0., &lp), cost(Vec3::ZERO, 0., &rp));
        let costq = |tr: Vec3, q: glam::Quat, set: &[Vec3]| -> f32 { set.iter().map(|p| { let pl = q.conjugate() * (*p - tr); pts.iter().map(|g| g.distance(pl)).fold(f32::MAX, f32::min) }).sum::<f32>() * 100. };
        for (axis_name, axis) in [("yaw (about gun Y)", Vec3::Y), ("pitch (about gun X)", Vec3::X), ("roll (about gun Z = barrel)", Vec3::Z)] {
            let mut s = format!("  {axis_name:<28}");
            for a in (-30..=30).step_by(6) { let q = glam::Quat::from_axis_angle(axis, (a as f32).to_radians()); s += &format!(" {a:+3}:{:4.0}/{:3.0}", costq(Vec3::ZERO, q, &lp), costq(Vec3::ZERO, q, &rp)); }
            println!("{s}   (left/right cost)");
        }
        for (axis_name, axis) in [("translate X", Vec3::X), ("translate Y", Vec3::Y), ("translate Z", Vec3::Z)] {
            let mut s = format!("  {axis_name:<28}");
            for a in (-10..=10).step_by(2) { s += &format!(" {a:+3}:{:4.0}/{:3.0}", costq(axis * (a as f32 * 0.01), glam::Quat::IDENTITY, &lp), costq(axis * (a as f32 * 0.01), glam::Quat::IDENTITY, &rp)); }
            println!("{s}   (cm; left/right cost)");
        }
        let mut best = (f32::MAX, Vec3::ZERO, 0f32);
        for yaw in (-20..=20).step_by(4) { for tx in (-8..=8).step_by(2) { for ty in (-8..=8).step_by(2) { for tz in (-8..=8).step_by(2) {
            let tr = Vec3::new(tx as f32, ty as f32, tz as f32) * 0.01;
            let c = cost(tr, yaw as f32, &lp) + cost(tr, yaw as f32, &rp);
            if c < best.0 { best = (c, tr, yaw as f32); }
        } } } }
        println!("  best both-hands fit: cost {:.1} at T ({:.0},{:.0},{:.0}) cm yaw {:.0} deg: left {:.1} right {:.1}", best.0, best.1.x * 100., best.1.y * 100., best.1.z * 100., best.2, cost(best.1, best.2, &lp), cost(best.1, best.2, &rp));
        let mut bl = (f32::MAX, Vec3::ZERO, 0f32);
        for yaw in (-20..=20).step_by(4) { for tx in (-10..=10).step_by(2) { for ty in (-10..=10).step_by(2) { for tz in (-10..=10).step_by(2) {
            let tr = Vec3::new(tx as f32, ty as f32, tz as f32) * 0.01;
            let c = cost(tr, yaw as f32, &lp);
            if c < bl.0 { bl = (c, tr, yaw as f32); }
        } } } }
        println!("  best left-hand-only fit: cost {:.1} at T ({:.0},{:.0},{:.0}) cm yaw {:.0} (right hand then {:.1})", bl.0, bl.1.x * 100., bl.1.y * 100., bl.1.z * 100., bl.2, cost(bl.1, bl.2, &rp));
    }
    println!("\n##### 0d. fingertip -> gun surface distance (cm), idle hip / targeting, left and right hand (tips = *_03 bones, thumb_03, + palm centre = mean of *_01 bases)");
    for (label, targeting, gi) in [("AR hip", false, 0usize), ("AR targeting", true, 0), ("Shotgun hip", false, 1), ("Shotgun targeting", true, 1), ("Sniper hip", false, 2)] {
        let mut sim = GameSim::new();
        sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
        sim.set_held(Some((env.all_verts[gi].0, targeting)));
        for f in 0..60 { sim.set_weapon_clock(Some(fn_animgraph::WeaponClock { world_time: f as f32 / 60., last_fire_time: -10., equipping: false, reloading: false, charging: false })); sim.step_anim(Vec2::ZERO, 1. / 60.); }
        sim.arms.clear();
        env.ch.evaluate(&mut sim);
        let world = pawn_matrix(&sim);
        let body = &env.ch.rig.parts[0];
        let bp = |n: &str| world.transform_point3(env.ch.pose.parts[0].bone_mesh[body.tree.bone_index(n).unwrap()].w_axis.truncate());
        let g = world * env.ch.pose.socket("RightHand").unwrap();
        let wv: Vec<[Vec3; 3]> = env.all_tris[gi].iter().map(|t| [g.transform_point3(t[0]), g.transform_point3(t[1]), g.transform_point3(t[2])]).collect();
        for side in ["l", "r"] {
            let mut s = format!("{label} hand_{side}: wrist {:5.1}", surf(&wv, bp(&format!("hand_{side}"))) * 100.);
            for f in ["thumb", "index", "middle", "ring", "pinky"] { s += &format!(" {f}_01 {:4.1} {f}_03 {:4.1}", surf(&wv, bp(&format!("{f}_01_{side}"))) * 100., surf(&wv, bp(&format!("{f}_03_{side}"))) * 100.); }
            let palm = ["index", "middle", "ring", "pinky"].iter().map(|f| bp(&format!("{f}_01_{side}"))).sum::<Vec3>() / 4.;
            s += &format!(" palm {:4.1}", surf(&wv, palm) * 100.);
            {   // palm / wrist / tips in gun space (cm; glTF axes: +Z = barrel, +Y up, +X = the gun's left) and the closest gun surface point of each
                let gi_inv = g.inverse();
                let gun_local = &env.all_tris[gi];
                let show = |n: &str, w: Vec3| { let pl = gi_inv.transform_point3(w); let c = closest_on(gun_local, pl); format!("{n} ({:.1},{:.1},{:.1}) -> surf ({:.1},{:.1},{:.1})", pl.x * 100., pl.y * 100., pl.z * 100., c.x * 100., c.y * 100., c.z * 100.) };
                s += &format!("\n      {} | {} | {} | {}", show("palm", palm), show("wrist", bp(&format!("hand_{side}"))), show("middle_03", bp(&format!("middle_03_{side}"))), show("thumb_03", bp(&format!("thumb_03_{side}"))));
            }
            println!("  {s}");
        }
    }
    println!("\n##### 0e. IK frames in the gun mesh frame (RightHand socket): position cm, rotation as euler XYZ deg (glTF axes), AR targeting idle");
    {
        let mut sim = idle_sim(true, 0., 1.0);
        sim.arms.clear();
        env.ch.evaluate(&mut sim);
        let body = &env.ch.rig.parts[0];
        let sock = env.ch.pose.socket("RightHand").unwrap();
        for n in ["ik_hand_root", "ik_hand_gun", "ik_hand_r", "ik_hand_l", "weapon_r", "weapon_l", "hand_r", "hand_l"] {
            let m = sock.inverse() * env.ch.pose.parts[0].bone_mesh[body.tree.bone_index(n).unwrap()];
            let (_, q, t) = m.to_scale_rotation_translation();
            let e = q.to_euler(glam::EulerRot::XYZ);
            println!("  {n:<13} pos ({:7.2},{:7.2},{:7.2}) euler XYZ ({:7.1},{:7.1},{:7.1})", t.x * 100., t.y * 100., t.z * 100., e.0.to_degrees(), e.1.to_degrees(), e.2.to_degrees());
        }
    }
    println!("\n##### 1. idle states (aim pitch -45 / 0 / +30)");
    for (name, targeting, secs) in [("targeting", true, 1.0f32), ("hip", false, 1.0), ("relaxed", false, 6.0)] {
        for pitch in [-45f32, 0., 30.] {
            let mut sim = idle_sim(targeting, pitch, secs);
            env.line(&format!("AR {name} pitch {pitch:+.0}"), &mut sim);
        }
    }

    println!("\n##### 2. bisection of the pose tree (AR hip / targeting, pitch 0 and +30): hand_l to gun at each node of the chain (outermost first)");
    for (targeting, pitch) in [(true, 0f32), (false, 30.)] {
        let mut sim = idle_sim(targeting, pitch, 1.0);
        sim.arms.clear();
        env.ch.evaluate(&mut sim);
        let tree = sim.pose_tree();
        let mut n = &tree; let mut depth = 0;
        println!("-- targeting {targeting} pitch {pitch}");
        loop {
            let pose = env.ch.rig.evaluate(n, &*env.ch.clips);
            let m = env.measure(&sim, &pose);
            let label = match base(n) { Some((l, _)) => l, None => match n { Node::Mix(_) => "Mix", Node::Clip { .. } => "Clip", Node::Ref => "Ref", _ => "?" } };
            let palm = |side: &str| -> f32 {
                let world = pawn_matrix(&sim);
                let body = &env.ch.rig.parts[0];
                let bp = |n: &str| world.transform_point3(pose.parts[0].bone_mesh[body.tree.bone_index(n).unwrap()].w_axis.truncate());
                let g = world * pose.socket("RightHand").unwrap();
                let wv: Vec<[Vec3; 3]> = env.tris.iter().map(|t| [g.transform_point3(t[0]), g.transform_point3(t[1]), g.transform_point3(t[2])]).collect();
                surf(&wv, ["index", "middle", "ring", "pinky"].iter().map(|f| bp(&format!("{f}_01_{side}"))).sum::<Vec3>() / 4.) * 100.
            };
            let ikgap = |side: &str| -> (f32, f32) {
                let body = &env.ch.rig.parts[0];
                let bm = |n: &str| pose.parts[0].bone_mesh[body.tree.bone_index(n).unwrap()];
                let (h, i) = (bm(&format!("hand_{side}")), bm(&format!("ik_hand_{side}")));
                (h.w_axis.truncate().distance(i.w_axis.truncate()) * 100., glam::Quat::from_mat4(&h).normalize().angle_between(glam::Quat::from_mat4(&i).normalize()).to_degrees())
            };
            let (gl, gr) = (ikgap("l"), ikgap("r"));
            println!("  [{depth:2}] below+incl {label:<16} hand_r {:5.2} hand_l {:5.2} | palm_r {:5.2} palm_l {:5.2} | FK-vs-IK-bone gap R {:5.2}cm/{:5.1}deg L {:5.2}cm/{:5.1}deg", m[0], m[1], palm("r"), palm("l"), gr.0, gr.1, gl.0, gl.1);
            match base(n) { Some((_, b)) => { n = b; depth += 1; } None => break }
        }
    }

    println!("\n##### 3. AR fire montage, hip / targeting, several montage times (aim pitch 0 and +30)");
    for (targeting, pitch) in [(false, 0f32), (true, 0.), (false, 30.), (true, 30.)] {
        let mut rows: Vec<(f32, [f32; 6])> = vec![];
        let mut fired = false;
        {
            let mut sim = GameSim::new();
            sim.world.props.clear(); sim.yaw = 0.; sim.pitch = pitch;
            let mut arms = Arms::new(env.data.clone(), Policy { assume_unrecovered: true });
            arms.update(0., &ArmsInput { select: Some(1), ..Default::default() });
            let dt = 1. / 60.;
            let mut since = 0f32;
            for frame in 0..200 {
                let fire = frame == 120 && !fired; if fire { fired = true; }
                arms.set_aiming(targeting);
                arms.update(dt, &ArmsInput { fire_held: fire, ..Default::default() });
                sim.set_held(arms.held_core().map(|(c, _)| (c, targeting)));
                sim.set_weapon_clock(arms.weapon_clock());
                sim.step_anim(Vec2::ZERO, dt);
                sim.arms = arms.montages();
                if fired { since += dt; }
                if fired && [0.0167f32, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.6].iter().any(|t| (since - t).abs() < dt * 0.5) {
                    let ms: Vec<Montage> = sim.arms.clone();
                    let m = env.measure_with_montages(&mut sim);
                    println!("AR {} pitch {pitch:+.0} t_since_shot {since:5.3} montages {:?}: hand_r {:5.2} hand_l {:5.2} Lalpha {:.2} snap {:.2} rotL {:5.1}{}", if targeting { "targeting" } else { "hip" }, ms.iter().map(|m| format!("{}:{}@{:.3}w{:.2}", m.slot, m.clip, m.time, m.weight)).collect::<Vec<_>>(), m[0], m[1], m[2], m[3], m[5], if m[1] > 6. { "  <== OFF" } else { "" });
                    rows.push((since, m));
                }
            }
        }
        let _ = rows;
    }

    println!("\n##### 4. after firing until the magazine is empty (hip / targeting), trigger on empty, 3 s later");
    for targeting in [false, true] {
        {
            let mut sim = GameSim::new();
            sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
            let mut arms = Arms::new(env.data.clone(), Policy { assume_unrecovered: true });
            arms.mags[0] = 3;
            arms.update(0., &ArmsInput { select: Some(1), ..Default::default() });
            let dt = 1. / 60.;
            let mut last_report = -1.0f32;
            for frame in 0..600 {
                let hold = (120..150).contains(&frame) || (frame > 180 && frame < 190);
                arms.set_aiming(targeting);
                arms.update(dt, &ArmsInput { fire_held: hold, ..Default::default() });
                sim.set_held(arms.held_core().map(|(c, _)| (c, targeting)));
                sim.set_weapon_clock(arms.weapon_clock());
                sim.step_anim(Vec2::ZERO, dt);
                sim.arms = arms.montages();
                let tsec = frame as f32 * dt;
                if frame >= 120 && tsec - last_report > 0.19 && (frame < 330 || frame % 60 == 0) {
                    last_report = tsec;
                    let ms = sim.arms.len();
                    let m = env.measure_with_montages(&mut sim);
                    println!("AR {} frame {frame} mag {} montages {ms}: hand_r {:5.2} hand_l {:5.2} Lalpha {:.2} snap {:.2} rotL {:5.1}{}", if targeting { "targeting" } else { "hip" }, arms.mags[0], m[0], m[1], m[2], m[3], m[5], if m[1] > 6. { "  <== OFF" } else { "" });
                }
            }
        }
    }

    println!("\n##### 5. reload montage (targeting false), several times");
    {
        let mut sim = GameSim::new();
        sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
        let mut arms = Arms::new(env.data.clone(), Policy { assume_unrecovered: true });
        arms.mags[0] = 0;
        arms.update(0., &ArmsInput { select: Some(1), give_ammo: Some((0, 90)), ..Default::default() });
        let dt = 1. / 60.;
        for frame in 0..600 {
            arms.update(dt, &ArmsInput { reload_press: frame == 120, ..Default::default() });
            sim.set_held(arms.held_core().map(|(c, _)| (c, false)));
            sim.set_weapon_clock(arms.weapon_clock());
            sim.step_anim(Vec2::ZERO, dt);
            sim.arms = arms.montages();
            if frame >= 118 && frame % 20 == 0 && frame < 560 {
                let ms = sim.arms.len();
                let m = env.measure_with_montages(&mut sim);
                println!("reload frame {frame} montages {ms} mag {}: hand_r {:5.2} hand_l {:5.2} Lalpha {:.2} snap {:.2} rotL {:5.1}{}", arms.mags[0], m[0], m[1], m[2], m[3], m[5], if m[1] > 6. { "  <== OFF" } else { "" });
            }
        }
    }

    println!("\n##### 7. full sim.step scenarios (real GameSim tick incl. turn-in-place / locomotion): hand_l / hand_r gun distance range, left IK alpha range, facing and aim state");
    {
        use fn_game::FrameInput;
        // (name, targeting, forward, side, crouch, yaw turn deg over 0.5 s at t=1 s, fire from t=1 s for 1 s, secs)
        for (name, targeting, fwd, side, crouch, turn, fire, secs) in [
            ("stand hip", false, 0f32, 0f32, false, 0f32, false, 5f32), ("stand targeting", true, 0., 0., false, 0., false, 5.),
            ("stand hip turn 90", false, 0., 0., false, 90., false, 6.), ("stand targeting turn 90", true, 0., 0., false, 90., false, 6.),
            ("stand targeting turn 30", true, 0., 0., false, 30., false, 6.), ("stand targeting fire", true, 0., 0., false, 0., true, 5.),
            ("walk targeting", true, 1., 0., false, 0., false, 4.), ("strafe targeting", true, 0., 1., false, 0., false, 4.), ("jog hip", false, 1., 0., false, 0., false, 4.), ("jog hip fire", false, 1., 0., false, 0., true, 4.),
            ("crouch targeting", true, 0., 0., true, 0., false, 4.), ("crouch walk targeting", true, 1., 0., true, 0., false, 4.),
        ] {
            let mut sim = GameSim::new();
            sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
            let mut arms = Arms::new(env.data.clone(), Policy { assume_unrecovered: true });
            arms.mags[0] = 30;
            arms.update(0., &ArmsInput { select: Some(1), ..Default::default() });
            let dt = 1. / 60.;
            let (mut dl, mut dr, mut al, mut fo, mut sw, mut yo, mut sp) = ((f32::MAX, f32::MIN), (f32::MAX, f32::MIN), (f32::MAX, f32::MIN), (f32::MAX, f32::MIN), (f32::MAX, f32::MIN), (f32::MAX, f32::MIN), 0u32);
            let mut last_dir = 0f32; let mut prev_f = 0f32;
            for frame in 0..(secs / dt) as usize {
                let t = frame as f32 * dt;
                let firing = fire && t >= 1. && t < 2.;
                arms.set_aiming(targeting);
                arms.update(dt, &ArmsInput { fire_held: firing, ..Default::default() });
                sim.set_held(arms.held_core().map(|(c, _)| (c, targeting)));
                sim.set_weapon_clock(arms.weapon_clock());
                let yd = if turn != 0. && t >= 1. && t < 1.5 { turn / (0.5 / dt) } else { 0. };
                sim.step(&FrameInput { forward: fwd, side, force_crouch: crouch, yaw_delta: yd, non_melee: true, ..Default::default() }, dt);
                sim.arms = arms.montages();
                env.ch.evaluate(&mut sim);
                if t < 0.9 { prev_f = sim.controller.facing; continue; }
                let m = env.measure(&sim, &env.ch.pose);
                dl = (dl.0.min(m[1]), dl.1.max(m[1])); dr = (dr.0.min(m[0]), dr.1.max(m[0])); al = (al.0.min(m[2]), al.1.max(m[2]));
                let f = sim.controller.facing; let d = ((f - prev_f + 540.) % 360.) - 180.; prev_f = f;
                if d.abs() > 0.02 { let dir = d.signum(); if last_dir != 0. && dir != last_dir { sp += 1; } last_dir = dir; }
                let rel = ((f - sim.yaw + 540.) % 360.) - 180.; fo = (fo.0.min(rel), fo.1.max(rel));
                sw = (sw.0.min(sim.anim.vars.aim_yaw), sw.1.max(sim.anim.vars.aim_yaw)); yo = (yo.0.min(sim.anim.vars.actor_yaw_offset), yo.1.max(sim.anim.vars.actor_yaw_offset));
            }
            println!("{name:<24} hand_l {:.2}..{:.2}{} hand_r {:.2}..{:.2} Lalpha {:.2}..{:.2} | facing-yaw {:.1}..{:.1} aim_yaw {:.1}..{:.1} actor_yaw_offset {:.1}..{:.1} facing direction reversals {sp}", dl.0, dl.1, if dl.1 > 6. { " <== OFF" } else { "" }, dr.0, dr.1, al.0, al.1, fo.0, fo.1, sw.0, sw.1, yo.0, yo.1);
        }
    }

    println!("\n##### 9. sprint / jump / fall / land / equip (real GameSim tick), AR: hand_l distance and rotL range over the run, per 0.25 s window max");
    {
        use fn_game::FrameInput;
        for (name, run, jump_at, secs, targeting) in [("sprint", true, -1f32, 4f32, false), ("jump standing", false, 1., 3., false), ("jump sprinting", true, 2., 4., false), ("jump targeting", false, 1., 3., true)] {
            let mut sim = GameSim::new();
            sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
            let mut arms = Arms::new(env.data.clone(), Policy { assume_unrecovered: true });
            arms.mags[0] = 30;
            arms.update(0., &ArmsInput { select: Some(1), ..Default::default() });
            let dt = 1. / 60.;
            let mut win = String::new(); let mut wmax = (0f32, 0f32); let mut worst = 0f32;
            for frame in 0..(secs / dt) as usize {
                let t = frame as f32 * dt;
                arms.set_aiming(targeting);
                arms.update(dt, &ArmsInput::default());
                sim.set_held(arms.held_core().map(|(c, _)| (c, targeting)));
                sim.set_weapon_clock(arms.weapon_clock());
                sim.step(&FrameInput { forward: if run || jump_at < 0. { 1. } else { 0. }, force_run: run, script_jump: jump_at >= 0. && t >= jump_at && t < jump_at + 0.1, non_melee: true, ..Default::default() }, dt);
                sim.arms = arms.montages();
                env.ch.evaluate(&mut sim);
                let m = env.measure(&sim, &env.ch.pose);
                if t > 0.9 { wmax = (wmax.0.max(m[1]), wmax.1.max(m[5])); worst = worst.max(m[1]); }
                if frame % 15 == 14 && t > 0.9 { win += &format!(" {:.1}/{:.0}", wmax.0, wmax.1); wmax = (0., 0.); }
            }
            println!("{name:<16} hand_l cm / rotL deg per 0.25 s:{win}   worst {worst:.1} cm{}", if worst > 6. { " <== OFF" } else { "" });
        }
    }

    println!("\n##### 8. aim sweep: camera pitch x actor_yaw_offset (-> AdjustedAimYaw), targeting / hip; hand_l distance (cm) / rotL (deg)");
    for targeting in [true, false] {
        for off in [-80f32, -40., 0., 40., 80.] {
            let mut row = String::new();
            for pitch in [-80f32, -60., -30., 0., 30., 60., 80.] {
                let mut sim = GameSim::new();
                sim.world.props.clear(); sim.yaw = 0.; sim.pitch = pitch;
                sim.set_held(Some(("AssaultRifle", targeting)));
                let dt = 1. / 60.;
                for f in 0..90 {
                    sim.set_weapon_clock(Some(fn_animgraph::WeaponClock { world_time: f as f32 * dt, last_fire_time: f as f32 * dt, equipping: false, reloading: false, charging: false }));
                    sim.anim.vars.actor_yaw_offset = off;
                    sim.step_anim(Vec2::ZERO, dt);
                }
                sim.arms.clear();
                env.ch.evaluate(&mut sim);
                let m = env.measure(&sim, &env.ch.pose);
                row += &format!(" | p{pitch:+.0}: {:5.1}/{:4.0}", m[1], m[5]);
            }
            println!("{} offset {off:+.0}{row}", if targeting { "targeting" } else { "hip      " });
        }
    }

    println!("\n##### 6. SWAY: idle with the gun, constant camera (yaw 0 pitch 0), 5 s at 60 Hz; yaw (deg, char space +right) of pelvis / spine_03 / weapon_r / barrel-ish (weapon_r +X), min..max");
    for (name, targeting) in [("hip", false), ("targeting", true)] {
        let mut sim = GameSim::new();
        sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
        sim.set_held(Some(("AssaultRifle", targeting)));
        let dt = 1. / 60.;
        let names = ["pelvis", "spine_01", "spine_03", "upperarm_r", "hand_r", "weapon_r", "hand_l"];
        let mut lo = vec![f32::MAX; names.len() + 2]; let mut hi = vec![f32::MIN; names.len() + 2];
        let mut series: Vec<f32> = vec![];
        for frame in 0..420 {
            let now = frame as f32 * dt;
            sim.set_weapon_clock(Some(fn_animgraph::WeaponClock { world_time: now, last_fire_time: -10., equipping: false, reloading: false, charging: false }));
            sim.step_anim(Vec2::ZERO, dt);
            sim.arms.clear();
            env.ch.evaluate(&mut sim);
            if frame < 120 { continue; }
            let world = pawn_matrix(&sim);
            let body = &env.ch.rig.parts[0];
            let mut vals = vec![];
            for n in names { let m = env.ch.pose.parts[0].bone_mesh[body.tree.bone_index(n).unwrap()]; vals.push(yaw_of(world * m)); }
            let g = world * env.ch.pose.socket("RightHand").unwrap();
            let c: Vec3 = env.verts.iter().copied().sum::<Vec3>() / env.verts.len() as f32; let _ = c;
            vals.push(sim.anim.vars.actor_yaw_offset); vals.push(sim.controller.facing);
            let _ = g;
            for (i, v) in vals.iter().enumerate() { lo[i] = lo[i].min(*v); hi[i] = hi[i].max(*v); }
            series.push(vals[5]);
        }
        let mut s = String::new();
        for (i, n) in names.iter().chain(["actor_yaw_offset", "facing"].iter()).enumerate() { s += &format!(" {n} {:.2}..{:.2}", lo[i], hi[i]); }
        println!("{name}: {s}");
        println!("  weapon_r yaw every 0.25 s: {:?}", series.iter().step_by(15).map(|v| (v * 100.).round() / 100.).collect::<Vec<_>>());
    }
}

impl Env {
    fn measure_with_montages(&mut self, sim: &mut GameSim) -> [f32; 6] {
        // `sim.arms` is kept (the montages of this frame)
        self.ch.evaluate(sim);
        let fd = self.finger_dev(&self.ch.pose);
        print!("[fingers L {:4.1} R {:4.1} vs {}] ", fd.0, fd.1, fd.2);
        self.measure(sim, &self.ch.pose)
    }
}


impl Env {
    /// Max angle (deg) any left / right finger bone's local rotation differs from the authored pose clip (best of Targeted / NonTargeted / Relaxed).
    fn finger_dev(&self, pose: &fn_pose::CharacterPose) -> (f32, f32, String) {
        let body = &self.ch.rig.parts[0];
        let fingers = |side: &str| -> Vec<usize> { body.tree.names.iter().enumerate().filter(|(_, n)| { let n = n.to_lowercase(); n.ends_with(side) && ["thumb", "index", "middle", "ring", "pinky"].iter().any(|f| n.starts_with(f)) }).map(|(i, _)| i).collect() };
        let (l, r) = (fingers("_l"), fingers("_r"));
        let mut best = (f32::MAX, f32::MAX, String::new());
        for c in ["M_AssaultRifle_Targeted", "M_AssaultRifle_NonTargeted", "M_AssaultRifle_Relaxed"] {
            let cp = self.ch.rig.evaluate(&Node::clip(c, 0., false), &*self.ch.clips);
            let dev = |set: &Vec<usize>| set.iter().map(|&i| pose.parts[0].bone_local[i].1.angle_between(cp.parts[0].bone_local[i].1).to_degrees()).fold(0f32, f32::max);
            let (a, b) = (dev(&l), dev(&r));
            if a + b < best.0 + best.1 || best.2.is_empty() { best = (a, b, c.to_owned()); }
        }
        best
    }
}












/// SWAY / LEFT-HAND probe (the before/after probe for the gun-in-hand fit). For each scenario (real GameSim tick, constant camera yaw 0 / pitch 0) it reports
///  * LH err: left hand (hand_l) pose in the gun frame (hand_r frame == RightHand socket, weapon_r is hand_r's child at identity) against the AUTHORED one, i.e.
///    `inverse(ik_hand_r) * ik_hand_l` of the M_AssaultRifle_Targeted clip: the grip offset the animators authored. cm / deg, min..max;
///  * palm: left palm centre to the gun surface (cm);
///  * torso yaw: yaw of the shoulder line (upperarm_l -> upperarm_r) and of the hip line (thigh_l -> thigh_r) in world, relative to the camera (deg), min..max and range;
///  * barrel yaw: yaw of the gun's +Z (barrel) relative to the camera, min..max.
/// Run: `cargo test --release -p fn-app hand_sway -- --nocapture`.
#[test]
fn hand_sway() {
    use fn_game::FrameInput;
    let mut env = build();
    let idx: std::collections::HashMap<String, usize> = env.ch.rig.parts[0].tree.names.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
    let ix = |n: &str| idx[n];
    let reference = {
        let cp = env.ch.rig.evaluate(&Node::clip("M_AssaultRifle_Targeted", 0., false), &*env.ch.clips);
        let m = &cp.parts[0].bone_mesh;
        m[ix("ik_hand_r")].inverse() * m[ix("ik_hand_l")]
    };
    let (rt, rq) = (reference.w_axis.truncate(), glam::Quat::from_mat4(&reference).normalize());
    let line_yaw = |world: Mat4, pose: &fn_pose::CharacterPose, l: &str, r: &str| -> f32 {
        let p = |n: &str| world.transform_point3(pose.parts[0].bone_mesh[ix(n)].w_axis.truncate());
        let right = p(r) - p(l);
        let fwd = Vec3::Y.cross(right).normalize();
        let rf = world.transform_vector3(Vec3::Z);
        let wrap = |a: f32| ((a + 540.) % 360.) - 180.;
        wrap((-fwd.x).atan2(fwd.z).to_degrees() - (-rf.x).atan2(rf.z).to_degrees())
    };
    println!("\n##### raw authored pose clips: barrel yaw / pitch (deg, relative to the mesh forward +Z) and shoulder-line yaw");
    for c in ["M_AssaultRifle_Targeted", "M_AssaultRifle_NonTargeted", "M_AssaultRifle_Relaxed", "M_Jog_AssaultRifle_N_Core", "Sprint_TwoHanded", "AR_Equip_CMM"] {
        let cp = env.ch.rig.evaluate(&Node::clip(c, 0., false), &*env.ch.clips);
        let g = cp.socket("RightHand").unwrap();
        let b = g.transform_vector3(Vec3::Z);
        let m = &cp.parts[0].bone_mesh;
        let right = m[ix("upperarm_r")].w_axis.truncate() - m[ix("upperarm_l")].w_axis.truncate();
        let fwd = Vec3::Y.cross(right);
        println!("  {c:<30} barrel yaw {:6.1} pitch {:6.1} | shoulder yaw {:6.1}", (-b.x).atan2(b.z).to_degrees(), b.y.asin().to_degrees(), (-fwd.x).atan2(fwd.z).to_degrees());
    }
    println!("\n##### raw authored loop clips over one cycle (no graph): shoulder-line / hip-line yaw range and barrel yaw range (deg, mesh space)");
    for c in ["M_Jog_AssaultRifle_N_Core", "M_Walk_AR_N_Core", "Sprint_TwoHanded"] {
        use fn_animgraph::ClipSource;
        let d = env.ch.clips.clip(c).map_or(1., |k| k.duration());
        let (mut sh, mut hp, mut by) = ((f32::MAX, f32::MIN), (f32::MAX, f32::MIN), (f32::MAX, f32::MIN));
        for i in 0..60 {
            let cp = env.ch.rig.evaluate(&Node::clip(c, d * i as f32 / 60., true), &*env.ch.clips);
            let m = &cp.parts[0].bone_mesh;
            let y = |l: &str, r: &str| { let f = Vec3::Y.cross(m[ix(r)].w_axis.truncate() - m[ix(l)].w_axis.truncate()); (-f.x).atan2(f.z).to_degrees() };
            let (s, h) = (y("upperarm_l", "upperarm_r"), y("thigh_l", "thigh_r"));
            let b = cp.socket("RightHand").unwrap().transform_vector3(Vec3::Z); let b = (-b.x).atan2(b.z).to_degrees();
            sh = (sh.0.min(s), sh.1.max(s)); hp = (hp.0.min(h), hp.1.max(h)); by = (by.0.min(b), by.1.max(b));
        }
        println!("  {c:<28} len {d:.2}s shoulder {:.1}..{:.1} ({:.1}) hip {:.1}..{:.1} ({:.1}) barrel {:.1}..{:.1}", sh.0, sh.1, sh.1 - sh.0, hp.0, hp.1, hp.1 - hp.0, by.0, by.1);
    }
    println!("\n##### SWAY / LEFT HAND (before/after probe): LH err = hand_l pose in the gun frame vs authored ik_hand_l-in-ik_hand_r frame");
    println!("scenario | LH err cm min..max | LH rot max deg | palm cm | shoulder yaw min..max (range) | hip yaw min..max (range) | barrel yaw min..max (all deg, relative to the pawn facing)");
    // (name, targeting, fwd, fire window, run, jump_at, secs, skip)
    for (name, targeting, fwd, fire, run, jump_at, secs, skip) in [
        ("idle ADS", true, 0f32, None, false, -1f32, 4f32, 1f32), ("idle hip (shot at 0.2 s)", false, 0., Some((0.2, 0.3)), false, -1., 4., 1.),
        ("relaxed idle (>5 s)", false, 0., None, false, -1., 9., 7.),
        ("fire hip 3 s", false, 0., Some((1f32, 4f32)), false, -1., 5., 1.6), ("fire ADS 3 s", true, 0., Some((1., 4.)), false, -1., 5., 1.6),
        ("equip (first 2 s)", false, 0., None, false, -1., 2., 0.),
        ("jog hip relaxed", false, 1., None, false, -1., 4., 1.), ("jog hip (shot at 0.5 s)", false, 1., Some((0.5, 0.6)), false, -1., 4., 1.5), ("jog hip fire", false, 1., Some((1., 4.)), false, -1., 5., 1.6),
        ("walk ADS", true, 1., None, false, -1., 4., 1.), ("sprint", false, 1., None, true, -1., 4., 1.),
        ("jump standing", false, 0., None, false, 1., 3., 0.5),
    ] {
        let mut sim = GameSim::new();
        sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
        let mut arms = Arms::new(env.data.clone(), Policy { assume_unrecovered: true });
        arms.mags[0] = 30;
        arms.update(0., &ArmsInput { select: Some(1), ..Default::default() });
        let dt = 1. / 60.;
        let (mut ep, mut er, mut pd, mut sh, mut hp, mut by) = ((f32::MAX, f32::MIN), 0f32, (f32::MAX, f32::MIN), (f32::MAX, f32::MIN), (f32::MAX, f32::MIN), (f32::MAX, f32::MIN));
        for frame in 0..(secs / dt) as usize {
            let t = frame as f32 * dt;
            let firing = fire.map_or(false, |(a, b)| t >= a && t < b);
            arms.set_aiming(targeting);
            arms.update(dt, &ArmsInput { fire_held: firing, ..Default::default() });
            sim.set_held(arms.held_core().map(|(c, _)| (c, targeting)));
            sim.set_weapon_clock(arms.weapon_clock());
            sim.step(&FrameInput { forward: fwd, force_run: run, script_jump: jump_at >= 0. && t >= jump_at && t < jump_at + 0.1, non_melee: true, ..Default::default() }, dt);
            sim.arms = arms.montages();
            env.ch.evaluate(&mut sim);
            if t < skip { continue; }
            let world = pawn_matrix(&sim);
            let pose = &env.ch.pose;
            let m = &pose.parts[0].bone_mesh;
            let rel = m[ix("hand_r")].inverse() * m[ix("hand_l")];
            let e = (rel.w_axis.truncate() - rt).length() * 100.;
            let r = glam::Quat::from_mat4(&rel).normalize().angle_between(rq).to_degrees();
            ep = (ep.0.min(e), ep.1.max(e)); er = er.max(r);
            let g = world * pose.socket("RightHand").unwrap();
            let bp = |n: &str| world.transform_point3(m[ix(n)].w_axis.truncate());
            let wv: Vec<[Vec3; 3]> = env.tris.iter().map(|t| [g.transform_point3(t[0]), g.transform_point3(t[1]), g.transform_point3(t[2])]).collect();
            let palm = ["index", "middle", "ring", "pinky"].iter().map(|f| bp(&format!("{f}_01_l"))).sum::<Vec3>() / 4.;
            let d = surf(&wv, palm) * 100.; pd = (pd.0.min(d), pd.1.max(d));
            let s = line_yaw(world, pose, "upperarm_l", "upperarm_r"); sh = (sh.0.min(s), sh.1.max(s));
            let h = line_yaw(world, pose, "thigh_l", "thigh_r"); hp = (hp.0.min(h), hp.1.max(h));
            let b = g.transform_vector3(Vec3::Z); let rf = world.transform_vector3(Vec3::Z); let b = ((((-b.x).atan2(b.z) - (-rf.x).atan2(rf.z)).to_degrees() + 540.) % 360.) - 180.; by = (by.0.min(b), by.1.max(b));
        }
        println!("{name:<22} | {:5.2}..{:5.2} | {:5.1} | {:4.1}..{:4.1} | {:7.1}..{:6.1} ({:5.1}) | {:7.1}..{:6.1} ({:5.1}) | {:7.1}..{:6.1}", ep.0, ep.1, er, pd.0, pd.1, sh.0, sh.1, sh.1 - sh.0, hp.0, hp.1, hp.1 - hp.0, by.0, by.1);
    }
}





/// BISECT: barrel yaw / shoulder yaw / right-hand FK-vs-IK gap at every peelable node of the pose tree, sprint and hip jog (fired so not relaxed).
#[test]
fn hand_bisect() {
    use fn_game::FrameInput;
    let mut env = build();
    let idx: std::collections::HashMap<String, usize> = env.ch.rig.parts[0].tree.names.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
    for (key, name, run, fire_recent) in ["AssaultRifle", "Rifle", "TacticalShotgun", "Pistol", "GrenadeLauncher", "RocketLauncher"].into_iter().flat_map(|k| [(k, "sprint", true, false), (k, "jog hip (fired 1 s ago)", false, true)]) {
        let mut sim = GameSim::new();
        sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
        let dt = 1. / 60.;
        for f in 0..180 {
            let t = f as f32 * dt;
            sim.set_held(Some((key, false)));
            sim.set_weapon_clock(Some(fn_animgraph::WeaponClock { world_time: t, last_fire_time: if fire_recent { t - 0.5 } else { -10. }, equipping: false, reloading: false, charging: false }));
            sim.step(&FrameInput { forward: 1., force_run: run, non_melee: true, ..Default::default() }, dt);
        }
        env.ch.evaluate(&mut sim);
        let tree = sim.pose_tree();
        println!("-- {key} {name}");
        let mut n = &tree; let mut depth = 0;
        loop {
            let cp = env.ch.rig.evaluate(n, &*env.ch.clips);
            let m = &cp.parts[0].bone_mesh;
            let g = cp.socket("RightHand").unwrap();
            let b = g.transform_vector3(Vec3::Z);
            let f = Vec3::Y.cross(m[idx["upperarm_r"]].w_axis.truncate() - m[idx["upperarm_l"]].w_axis.truncate());
            let gap = (m[idx["hand_r"]].w_axis.truncate() - m[idx["ik_hand_r"]].w_axis.truncate()).length() * 100.;
            let label = match base(n) { Some((l, _)) => l, None => match n { Node::Mix(_) => "Mix", Node::Clip { .. } => "Clip", Node::Ref => "Ref", _ => "?" } };
            println!("  [{depth:2}] {label:<18} barrel yaw {:6.1} pitch {:6.1} shoulder yaw {:6.1} hand_r-ik_hand_r {:5.1} cm", (-b.x).atan2(b.z).to_degrees(), b.y.asin().to_degrees(), (-f.x).atan2(f.z).to_degrees(), gap);
            if let Node::ApplyAdditive { add, alpha, .. } = n {
                let mut names = vec![]; add.clip_names(&mut names);
                let ap = env.ch.rig.evaluate(add, &*env.ch.clips);
                let mut s = format!("       additive alpha {alpha:.2} clips {names:?}; delta deg / cm:");
                for b in ["pelvis", "spine_01", "spine_02", "spine_03", "clavicle_r", "upperarm_r", "hand_r", "ik_hand_gun"] { let i = idx[b]; s += &format!(" {b} {:.1}/{:.1}", ap.parts[0].bone_local[i].1.angle_between(glam::Quat::IDENTITY).to_degrees(), ap.parts[0].bone_local[i].0.length() * 100.); }
                println!("{s}");
            }
            match base(n) { Some((_, b)) => { n = b; depth += 1; } None => break }
        }
    }
}

/// WEAPON SWEEP: the before / after probe of the per-weapon core animation wiring, for EVERY CoreAnims_Male key a Battle Royale weapon resolves to (`BR_WEAPON_CORE_ANIM` of anim_core.rs):
/// AssaultRifle (AR family, snipers, AMR, triple shot), Rifle (pump shotgun), TacticalShotgun (heavy / auto shotgun, Shredder), Pistol (pistols, revolver, PDW / SMG, Gatling, zapper: native
/// AFortWeaponRanged default), GrenadeLauncher, RocketLauncher, Melee (pickaxe / harvesting tool / building and edit tools: native AFortWeapon default), TrapPlacement (consumables, grenades, traps;
/// also the empty-hands key PaperBlueprint is the AnimBP default). Per scenario, over one stretch of ticks (real `GameSim`, camera yaw 0 / pitch 0, no Arms montages except the "equip" / "fire" rows
/// of the loadout guns):
///  * shoulder / hip yaw (deg, line upperarm_l->upperarm_r / thigh_l->thigh_r relative to the pawn facing): min..max (range);
///  * barrel yaw: yaw of the RightHand socket's +Z relative to the pawn facing (deg) min..max (range);
///  * LH err: left hand (hand_l) in the hand_r frame vs the AUTHORED offset of the weapon's own pose clip for that scenario (`inverse(ik_hand_r) * ik_hand_l`): cm min..max;
///  * palm: left palm to the gun surface (cm), only for the loadout guns that have a mesh.
/// Runs each scenario twice: `legacy` (only the first per-weapon pass's five additive spaces, everything else on the pickaxe's: the state before) and `new` (the full CoreAnims_Male wiring).
/// Run: `cargo test --release -p fn-app weapon_sweep -- --nocapture`.
#[test]
fn weapon_sweep() {
    use fn_game::FrameInput;
    let mut env = build();
    let idx: std::collections::HashMap<String, usize> = env.ch.rig.parts[0].tree.names.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
    let ix = |n: &str| idx[n];
    let wrap = |a: f32| ((a + 540.) % 360.) - 180.;
    let yaw_line = |world: Mat4, m: &[Mat4], l: &str, r: &str| -> f32 {
        let p = |n: &str| world.transform_point3(m[ix(n)].w_axis.truncate());
        let fwd = Vec3::Y.cross(p(r) - p(l)).normalize();
        let rf = world.transform_vector3(Vec3::Z);
        wrap((-fwd.x).atan2(fwd.z).to_degrees() - (-rf.x).atan2(rf.z).to_degrees())
    };
    // (key, mesh index into env.all_tris when the loadout has that gun)
    let mut mesh_of: std::collections::HashMap<&'static str, usize> = std::collections::HashMap::new();
    for (g, (k, _)) in env.all_verts.iter().enumerate() { mesh_of.entry(*k).or_insert(g); }
    let keys = ["AssaultRifle", "Rifle", "TacticalShotgun", "Pistol", "GrenadeLauncher", "RocketLauncher", "Melee", "TrapPlacement"];
    // scenario: (name, targeting, forward, crouch, run, recent fire, secs, skip)
    let scenarios: [(&str, bool, f32, bool, bool, bool, f32, f32); 11] = [
        ("idle ADS", true, 0., false, false, true, 4., 1.), ("idle hip (fired)", false, 0., false, false, true, 4., 1.), ("relaxed idle (>5 s)", false, 0., false, false, false, 9., 7.),
        ("jog hip (fired)", false, 1., false, false, true, 4., 1.5), ("jog relaxed (>5 s)", false, 1., false, false, false, 9., 7.), ("walk ADS", true, 1., false, false, true, 4., 1.),
        ("sprint", false, 1., false, true, true, 4., 1.), ("crouch idle hip", false, 0., true, false, true, 4., 1.), ("crouch walk hip", false, 1., true, false, true, 4., 1.),
        ("crouch walk ADS", true, 1., true, false, true, 4., 1.), ("crouch idle relaxed", false, 0., true, false, false, 9., 7.),
    ];
    let dt = 1. / 60.;
    println!("\n##### AUTHORED clips per weapon (no graph): barrel yaw / shoulder yaw range over one cycle (deg, mesh space; barrel = RightHand socket +Z), LH offset error of the clip's own hand_l in the hand_r frame vs its first frame (cm)");
    for key in keys {
        let set = fn_animgraph::core_anims(key).unwrap();
        for (what, clip, looped) in [("sprint", set.sprint, true), ("crouch sprint", set.crouch_sprint, true), ("targeting pose", set.targeting_pose, false), ("non-targeting pose", set.nontargeting_pose, false), ("relaxed pose", set.relaxed_pose, false), ("relaxed pose lvl2", set.relaxed_pose2, false)] {
            if clip.is_empty() { continue; }
            use fn_animgraph::ClipSource;
            let d = if looped { env.ch.clips.clip(clip).map_or(1., |k| k.duration()) } else { 0. };
            let n = if looped { 60 } else { 1 };
            let (mut sh, mut by, mut le) = ((f32::MAX, f32::MIN), (f32::MAX, f32::MIN), 0f32);
            let mut first: Option<Vec3> = None;
            for i in 0..n {
                let cp = env.ch.rig.evaluate(&Node::clip(clip, d * i as f32 / n as f32, looped), &*env.ch.clips);
                let m = &cp.parts[0].bone_mesh;
                let f = Vec3::Y.cross(m[ix("upperarm_r")].w_axis.truncate() - m[ix("upperarm_l")].w_axis.truncate()); let s = (-f.x).atan2(f.z).to_degrees(); sh = (sh.0.min(s), sh.1.max(s));
                let b = cp.socket("RightHand").unwrap().transform_vector3(Vec3::Z); let b = (-b.x).atan2(b.z).to_degrees(); by = (by.0.min(b), by.1.max(b));
                let r = (m[ix("hand_r")].inverse() * m[ix("hand_l")]).w_axis.truncate();
                if let Some(f0) = first { le = le.max((r - f0).length() * 100.); } else { first = Some(r); }
            }
            println!("  {key:<16} {what:<18} {clip:<34} shoulder {:6.1}..{:6.1} ({:5.1}) barrel {:6.1}..{:6.1} ({:5.1}) LH drift max {:5.2} cm", sh.0, sh.1, sh.1 - sh.0, by.0, by.1, by.1 - by.0, le);
        }
    }
    println!("\n##### AUTHORED composition (no graph): the weapon's pose clip + the forward (N) sample of its own additive blend space (Node::ApplyAdditive alpha 1) over one cycle: barrel / shoulder yaw range (deg)");
    for key in keys {
        use fn_animgraph::ClipSource;
        let set = fn_animgraph::core_anims(key).unwrap();
        let nt = |p: &'static str, q: &'static str| if p.is_empty() { q } else { p };
        let walk = if key == "Pistol" { "M_Locomotion2D_Walk_Pistol_Additive" } else { "M_Locomotion2D_Walk_AR_Additive" };
        let rel1 = if set.jog_additive_relaxed.is_empty() { set.jog_additive } else { set.jog_additive_relaxed };
        let rel2 = if set.jog_additive_relaxed2.is_empty() { rel1 } else { set.jog_additive_relaxed2 };
        let crel = if set.crouch_jog_additive_relaxed.is_empty() { set.crouch_jog_additive } else { set.crouch_jog_additive_relaxed };
        let cases: [(&str, &str, &str); 7] = [
            ("jog hip", nt(set.nontargeting_pose, set.targeting_pose), set.jog_additive), ("jog relaxed", nt(set.relaxed_pose, nt(set.nontargeting_pose, set.targeting_pose)), rel1),
            ("jog relaxed lvl2", nt(set.relaxed_pose2, nt(set.relaxed_pose, set.targeting_pose)), rel2), ("walk ADS", set.targeting_pose, walk),
            ("crouch walk hip", nt(set.crouch_nontargeting_pose, set.crouch_targeting_pose), set.crouch_jog_additive), ("crouch relaxed", nt(set.crouch_relaxed_pose, nt(set.crouch_nontargeting_pose, set.crouch_targeting_pose)), crel),
            ("crouch walk ADS", set.crouch_targeting_pose, nt(set.crouch_walk_additive, "")),
        ];
        for (what, pose, space) in cases {
            if pose.is_empty() || space.is_empty() { continue; }
            let Some(def) = fn_animgraph::blendspace(space) else { println!("  {key:<16} {what:<18} {space}: no data"); continue };
            let n_clip = def.samples.iter().filter(|s| s.pos[0] == 0.).max_by(|a, b| a.pos[1].total_cmp(&b.pos[1])).map(|s| s.anim).unwrap();
            let d = env.ch.clips.clip(n_clip).map_or(1., |k| k.duration());
            let (mut sh, mut by) = ((f32::MAX, f32::MIN), (f32::MAX, f32::MIN));
            for i in 0..60 {
                let node = Node::ApplyAdditive { base: Box::new(Node::clip(pose, 0., false)), add: Box::new(Node::clip(n_clip, d * i as f32 / 60., true)), alpha: 1. };
                let cp = env.ch.rig.evaluate(&node, &*env.ch.clips);
                let m = &cp.parts[0].bone_mesh;
                let f = Vec3::Y.cross(m[ix("upperarm_r")].w_axis.truncate() - m[ix("upperarm_l")].w_axis.truncate()); let s = (-f.x).atan2(f.z).to_degrees(); sh = (sh.0.min(s), sh.1.max(s));
                let b = cp.socket("RightHand").unwrap().transform_vector3(Vec3::Z); let b = (-b.x).atan2(b.z).to_degrees(); by = (by.0.min(b), by.1.max(b));
            }
            println!("  {key:<16} {what:<18} {pose:<34} + {space:<30} ({n_clip}) shoulder {:6.1}..{:6.1} ({:5.1}) barrel {:6.1}..{:6.1} ({:5.1})", sh.0, sh.1, sh.1 - sh.0, by.0, by.1, by.1 - by.0);
        }
    }
    println!("\n##### WEAPON SWEEP (before = legacy additive data, after = full CoreAnims_Male wiring); angles deg relative to the pawn facing, LH err = hand_l in the hand_r frame vs the weapon's authored pose clip (cm)");
    println!("{:<16} {:<22} {:<6} | shoulder min..max (range) | hip range | barrel min..max (range) | LH err cm min..max | palm cm", "core key", "scenario", "data");
    let mut summary: Vec<(String, f32, f32, f32, f32)> = vec![];
    for key in keys {
        let set = fn_animgraph::core_anims(key).unwrap_or_else(|| panic!("no CoreAnims_Male[{key}]"));
        let pose_for = |tg: bool, crouch: bool, relaxed: bool| -> &'static str {
            let (t, n, r) = if crouch { (set.crouch_targeting_pose, set.crouch_nontargeting_pose, set.crouch_relaxed_pose) } else { (set.targeting_pose, set.nontargeting_pose, set.relaxed_pose) };
            let n = if n.is_empty() { t } else { n };
            if tg { t } else if relaxed && !r.is_empty() { r } else { n }
        };
        for (name, tg, fwd, crouch, run, recent, secs, skip) in scenarios {
            let mut row: Vec<(bool, [f32; 12])> = vec![];
            for legacy in [true, false] {
                let mut sim = GameSim::new();
                sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
                sim.anim.legacy_additive_data = legacy;
                let (mut sh, mut hp, mut by, mut le, mut pd) = ((f32::MAX, f32::MIN), (f32::MAX, f32::MIN), (f32::MAX, f32::MIN), (f32::MAX, f32::MIN), (f32::MAX, f32::MIN));
                let mut reference: Option<(Vec3, glam::Quat)> = None;
                let relaxed_expected = !recent && !tg;
                let ref_pose = pose_for(tg, crouch, relaxed_expected);
                for frame in 0..(secs / dt) as usize {
                    let t = frame as f32 * dt;
                    sim.set_held(Some((key, tg)));
                    sim.set_weapon_clock(Some(fn_animgraph::WeaponClock { world_time: t, last_fire_time: if recent { t - 0.5 } else { -10. }, equipping: false, reloading: false, charging: false }));
                    sim.step(&FrameInput { forward: fwd, force_run: run, force_crouch: crouch, non_melee: true, ..Default::default() }, dt);
                    sim.arms.clear();
                    env.ch.evaluate(&mut sim);
                    if t < skip { continue; }
                    let world = pawn_matrix(&sim);
                    let pose = &env.ch.pose;
                    let m = &pose.parts[0].bone_mesh;
                    if reference.is_none() {
                        let cp = env.ch.rig.evaluate(&Node::clip(ref_pose, 0., false), &*env.ch.clips);
                        let r = cp.parts[0].bone_mesh[ix("ik_hand_r")].inverse() * cp.parts[0].bone_mesh[ix("ik_hand_l")];
                        reference = Some((r.w_axis.truncate(), glam::Quat::from_mat4(&r).normalize()));
                    }
                    let (rt, _rq) = reference.unwrap();
                    let rel = m[ix("hand_r")].inverse() * m[ix("hand_l")];
                    let e = (rel.w_axis.truncate() - rt).length() * 100.; le = (le.0.min(e), le.1.max(e));
                    let s = yaw_line(world, m, "upperarm_l", "upperarm_r"); sh = (sh.0.min(s), sh.1.max(s));
                    let h = yaw_line(world, m, "thigh_l", "thigh_r"); hp = (hp.0.min(h), hp.1.max(h));
                    let g = world * pose.socket("RightHand").unwrap();
                    let b = g.transform_vector3(Vec3::Z); let rf = world.transform_vector3(Vec3::Z);
                    let b = wrap(((-b.x).atan2(b.z) - (-rf.x).atan2(rf.z)).to_degrees()); by = (by.0.min(b), by.1.max(b));
                    if let Some(&gi) = mesh_of.get(key) {
                        let wv: Vec<[Vec3; 3]> = env.all_tris[gi].iter().map(|t| [g.transform_point3(t[0]), g.transform_point3(t[1]), g.transform_point3(t[2])]).collect();
                        let palm = ["index", "middle", "ring", "pinky"].iter().map(|f| world.transform_point3(m[ix(&format!("{f}_01_l"))].w_axis.truncate())).sum::<Vec3>() / 4.;
                        let d = surf(&wv, palm) * 100.; pd = (pd.0.min(d), pd.1.max(d));
                    }
                }
                let v = [sh.0, sh.1, hp.0, hp.1, by.0, by.1, le.0, le.1, pd.0, pd.1, 0., 0.];
                println!("{key:<16} {name:<22} {:<6} | {:7.1}..{:6.1} ({:5.1}) | {:5.1} | {:7.1}..{:6.1} ({:5.1}) | {:5.2}..{:5.2} | {}", if legacy { "before" } else { "after" }, v[0], v[1], v[1] - v[0], v[3] - v[2], v[4], v[5], v[5] - v[4], v[6], v[7],
                    if pd.0 == f32::MAX { "-".to_string() } else { format!("{:4.1}..{:4.1}", pd.0, pd.1) });
                row.push((legacy, v));
            }
            let (b, a) = (row[0].1, row[1].1);
            summary.push((format!("{key} / {name}"), (b[5] - b[4]) - (a[5] - a[4]), b[7] - a[7], (b[1] - b[0]) - (a[1] - a[0]), a[5] - a[4]));
        }
    }
    println!("\n##### delta (before - after): barrel yaw range, LH err max (cm), shoulder yaw range; and the after barrel range");
    for s in summary { println!("{:<44} barrel range -{:6.1}  LH err max -{:5.2}  shoulder range -{:6.1}   (after barrel range {:5.1})", s.0, s.1, s.2, s.3, s.4); }
}

/// EQUIP / FIRE sweep for the loadout guns (the ones with montages and a mesh): the left hand vs the gun in the first 2 s after equip and while firing from the hip / ADS, jogging.
/// Reports the left palm-to-gun distance and the LH error per 0.25 s window of the equip, plus the fire rows. Needs `Arms` (the 3 loadout items).
#[test]
fn equip_fire_sweep() {
    use fn_game::FrameInput;
    let mut env = build();
    let idx: std::collections::HashMap<String, usize> = env.ch.rig.parts[0].tree.names.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
    let ix = |n: &str| idx[n];
    let wrap = |a: f32| ((a + 540.) % 360.) - 180.;
    let dt = 1. / 60.;
    {   // the RAW authored equip clip (no graph, no IK): left hand offset in the hand_r frame vs the AR's authored grip (non-targeting pose), per 0.1 s
        use fn_animgraph::ClipSource;
        let cref = env.ch.rig.evaluate(&Node::clip("M_AssaultRifle_NonTargeted", 0., false), &*env.ch.clips);
        let r0 = (cref.parts[0].bone_mesh[ix("hand_r")].inverse() * cref.parts[0].bone_mesh[ix("hand_l")]).w_axis.truncate();
        let r0ik = (cref.parts[0].bone_mesh[ix("ik_hand_r")].inverse() * cref.parts[0].bone_mesh[ix("ik_hand_l")]).w_axis.truncate();
        let d = env.ch.clips.clip("AR_Equip_CMM").map_or(1., |k| k.duration());
        let mut s = format!("raw AR_Equip_CMM ({d:.2} s) LH offset error vs authored grip (cm) [FK hand_l / IK ik_hand_l]:");
        for i in 0..=((d / 0.1) as usize) {
            let cp = env.ch.rig.evaluate(&Node::clip("AR_Equip_CMM", (i as f32 * 0.1).min(d), false), &*env.ch.clips);
            let m = &cp.parts[0].bone_mesh;
            s += &format!(" {:.0}/{:.0}", ((m[ix("hand_r")].inverse() * m[ix("hand_l")]).w_axis.truncate() - r0).length() * 100., ((m[ix("ik_hand_r")].inverse() * m[ix("ik_hand_l")]).w_axis.truncate() - r0ik).length() * 100.);
        }
        println!("{s}");
    }
    for sel in 1..=3usize {
        for legacy in [true, false] {
            let mut sim = GameSim::new();
            sim.world.props.clear(); sim.yaw = 0.; sim.pitch = 0.;
            sim.anim.legacy_additive_data = legacy;
            let mut arms = Arms::new(env.data.clone(), Policy { assume_unrecovered: true });
            for m in arms.mags.iter_mut() { *m = 30; }
            arms.update(0., &ArmsInput { select: Some(sel), ..Default::default() });
            let Some((core, _)) = arms.held_core() else { continue };
            let gi = env.all_verts.iter().position(|(k, _)| *k == core).unwrap_or(0);
            let mut windows: Vec<(f32, f32, f32, f32, f32)> = vec![];
            let mut cur = (f32::MAX, f32::MIN, f32::MAX, f32::MIN, 0f32);
            for frame in 0..(5. / dt) as usize {
                let t = frame as f32 * dt;
                arms.set_aiming(false);
                arms.update(dt, &ArmsInput::default());
                sim.set_held(arms.held_core().map(|(c, _)| (c, false)));
                sim.set_weapon_clock(arms.weapon_clock());
                sim.step(&FrameInput { non_melee: true, ..Default::default() }, dt);
                sim.arms = arms.montages();
                env.ch.evaluate(&mut sim);
                let world = pawn_matrix(&sim);
                let pose = &env.ch.pose; let m = &pose.parts[0].bone_mesh;
                let g = world * pose.socket("RightHand").unwrap();
                let wv: Vec<[Vec3; 3]> = env.all_tris[gi].iter().map(|t| [g.transform_point3(t[0]), g.transform_point3(t[1]), g.transform_point3(t[2])]).collect();
                let palm = ["index", "middle", "ring", "pinky"].iter().map(|f| world.transform_point3(m[ix(&format!("{f}_01_l"))].w_axis.truncate())).sum::<Vec3>() / 4.;
                let d = surf(&wv, palm) * 100.;
                let b = g.transform_vector3(Vec3::Z); let rf = world.transform_vector3(Vec3::Z);
                let by = wrap(((-b.x).atan2(b.z) - (-rf.x).atan2(rf.z)).to_degrees());
                cur.0 = cur.0.min(d); cur.1 = cur.1.max(d); cur.2 = cur.2.min(by); cur.3 = cur.3.max(by); cur.4 = cur.4.max(env.measure(&sim, pose)[4]);
                if frame % 15 == 14 { windows.push(cur); cur = (f32::MAX, f32::MIN, f32::MAX, f32::MIN, 0.); }
                let _ = t;
            }
            println!("equip sweep slot {sel} ({core}) {}: palm->gun cm per 0.25 s window: {}", if legacy { "before" } else { "after" }, windows.iter().map(|w| format!("{:.0}..{:.0}", w.0, w.1)).collect::<Vec<_>>().join(" "));
            println!("    barrel yaw range per window: {}   |hand_l-ik_hand_l| max cm per window: {}", windows.iter().map(|w| format!("{:.0}", w.3 - w.2)).collect::<Vec<_>>().join(" "), windows.iter().map(|w| format!("{:.0}", w.4)).collect::<Vec<_>>().join(" "));
        }
    }
}