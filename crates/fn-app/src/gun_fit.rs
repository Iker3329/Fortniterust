//! GUN FIT: measures whether the held guns sit in the character's hands the way the 1.8 animations author them.
//! Places each firearm exactly as `Scene::update` does (`world * RightHand socket`, weapon skeleton at its reference pose) and reports hand -> gun distances in cm.
//! Needs the 1.8 install: `FN_TEST_ROOT` (or the default Downloads path). Run: `cargo test --release -p fn-app gun_fit -- --nocapture`.
#![cfg(test)]
use crate::loader::Source;
use crate::scene::{camera_for, pawn_matrix, Character};
use fn_animgraph::Node;
use fn_game::{FrameInput, GameSim};
use fn_pose::loadout::{Resources, ITEMS};
use fn_pose::{CharacterRig, ClipSourceDir, ClipStore, SampleMode};
use glam::{Mat4, Vec2, Vec3};
use std::sync::Arc;

const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

fn nearest(points: &[Vec3], p: Vec3) -> (f32, usize) {
    let mut best = (f32::MAX, 0);
    for (i, q) in points.iter().enumerate() { let d = q.distance(p); if d < best.0 { best = (d, i); } }
    best
}
/// Principal axis of a point cloud (power iteration on the covariance), unit length.
pub(crate) fn principal_axis(points: &[Vec3]) -> (Vec3, Vec3) {
    let c = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let mut cov = [[0f32; 3]; 3];
    for p in points { let d = (*p - c).to_array(); for i in 0..3 { for j in 0..3 { cov[i][j] += d[i] * d[j]; } } }
    let mut v = Vec3::new(1., 0.3, 0.2).normalize();
    for _ in 0..100 {
        let n = Vec3::new(cov[0][0] * v.x + cov[0][1] * v.y + cov[0][2] * v.z, cov[1][0] * v.x + cov[1][1] * v.y + cov[1][2] * v.z, cov[2][0] * v.x + cov[2][1] * v.y + cov[2][2] * v.z);
        v = n.normalize();
    }
    (c, v)
}
fn deg(a: Vec3, b: Vec3) -> f32 { a.normalize().dot(b.normalize()).clamp(-1., 1.).acos().to_degrees() }

/// Orthographic side view from the character's right (character space: +Z forward = screen right, +Y up), 4 px per cm: body bones grey, gun vertices orange,
/// hand_r red, hand_l blue, ik_hand_r / ik_hand_l thin rings, the gun's `handle` bone green.
fn side_view(file: &str, verts: &[Vec3], attach: Mat4, ch: &Character, handle: Option<Vec3>) {
    let (w, h, s) = (900u32, 700u32, 4.0f32);
    let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([245, 245, 245, 255]));
    let body = &ch.rig.parts[0];
    let bm = &ch.pose.parts[0].bone_mesh;
    let pos = |n: &str| bm[body.tree.bone_index(n).unwrap()].w_axis.truncate();
    let c = pos("hand_r");
    let to_px = |p: Vec3| -> (i32, i32) { (((p.z - c.z) * 100. * s + w as f32 * 0.4) as i32, (h as f32 * 0.5 - (p.y - c.y) * 100. * s) as i32) };
    let mut put = |x: i32, y: i32, col: [u8; 4], r: i32| { for dy in -r..=r { for dx in -r..=r { let (px, py) = (x + dx, y + dy); if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h { img.put_pixel(px as u32, py as u32, image::Rgba(col)); } } } };
    for (i, p) in body.tree.parents.iter().enumerate() {
        if let Some(p) = p { let (a, b) = (to_px(bm[*p].w_axis.truncate()), to_px(bm[i].w_axis.truncate())); for k in 0..=40 { let t = k as f32 / 40.; put(a.0 + ((b.0 - a.0) as f32 * t) as i32, a.1 + ((b.1 - a.1) as f32 * t) as i32, [120, 120, 120, 255], 0); } }
    }
    for v in verts { let (x, y) = to_px(attach.transform_point3(*v)); put(x, y, [240, 140, 20, 255], 0); }
    for (n, col, r) in [("hand_r", [220, 20, 20, 255], 5), ("hand_l", [20, 60, 220, 255], 5), ("ik_hand_r", [220, 20, 20, 255], 8), ("ik_hand_l", [20, 60, 220, 255], 8)] {
        let (x, y) = to_px(pos(n));
        if r == 5 { put(x, y, col, r); } else { for k in 0..64 { let a = k as f32 / 64. * std::f32::consts::TAU; put(x + (a.cos() * r as f32) as i32, y + (a.sin() * r as f32) as i32, col, 0); } }
    }
    if let Some(hd) = handle { let (x, y) = to_px(attach.transform_point3(hd)); put(x, y, [20, 160, 20, 255], 4); }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/gun-fit");
    std::fs::create_dir_all(&dir).unwrap();
    img.save(dir.join(file)).unwrap();
}

#[test]
fn gun_fit() {
    let root = std::env::var_os("FN_TEST_ROOT").map(std::path::PathBuf::from).unwrap_or_else(|| DEFAULT_ROOT.into());
    let dir = crate::assets::find_paks_dir(&root).expect("paks dir");
    let set = Arc::new(fn_pak::PakSet::open_install(&dir).unwrap());
    let src = ClipSourceDir::Pak(&set);
    let res = Resources::load(&src).unwrap();
    // the character exactly as the loader builds it
    let mut parts = fn_pose::loader::package_parts(&src).unwrap();
    parts.truncate(2);
    let mut rig = CharacterRig::new(parts);
    if let Some(m) = crate::br_character::load_for(&Source::Paks(dir.clone()), Some(&set)) { rig = CharacterRig { parts: m.rig_parts, ..rig }; }
    let hand = fn_pose::SocketDef::from_original_yaw_socket(0, &res.player_hand_socket).unwrap();
    rig.sockets = vec![hand];
    let skel = fn_pose::clip::load_player_skeleton_info(&src).unwrap();
    let mut store = ClipStore::load(&src, &fn_legacy::prototype_clip_names(), &skel, SampleMode::Ue).unwrap();
    res.install_player_clips(&mut store);
    let mut ch = Character::new(rig, Box::new(store));
    let names = ["AR", "Shotgun", "BoltSniper"];

    for g in 0..3 {
        let item = &res.items[g];
        let core: &'static str = match item.core_animation.as_deref().unwrap() { "AssaultRifle" => "AssaultRifle", "Rifle" => "Rifle", "Shotgun" => "Shotgun", o => panic!("{o}") };
        // ---- the weapon: skeleton, sockets, mesh
        let part = item.mesh.clone().unwrap();
        let gun_clips = res.weapon_clips(g).unwrap();
        let grig = CharacterRig::new(vec![part.clone()]);
        let gpose = grig.evaluate(&Node::Ref, &gun_clips);
        let gp = &gpose.parts[0];
        let skinned = part.skin_cpu(&gp.skin_matrices);
        let verts: Vec<Vec3> = skinned.iter().flat_map(|(p, _)| p.iter().copied()).collect();
        println!("\n=================== {} ({}) core={core}: {} vertices, {} weapon bones", names[g], ITEMS[g], verts.len(), part.tree.len());
        // weapon skeleton bones (character-of-the-gun space, reference pose; cm, glTF axes x, y(up), z) with parent
        for (i, n) in part.tree.names.iter().enumerate() {
            let p = gp.bone_mesh[i].w_axis.truncate() * 100.;
            let par = part.tree.parents[i].map(|j| part.tree.names[j].as_str()).unwrap_or("-");
            println!("  bone[{i:2}] {n:<22} parent {par:<16} pos_cm ({:7.2},{:7.2},{:7.2}) local_rot(xyzw) {:?} local_scale {:?}", p.x, p.y, p.z, part.tree.local[i].1.to_array().map(|v| (v * 1000.).round() / 1000.), part.tree.local[i].2.to_array());
        }
        // sockets: of the weapon mesh and of the weapon skeleton asset
        let json = fn_fx::data::PackageJson::load(&set, ITEMS[g]).unwrap();
        let def = json.exports.iter().find(|e| e.class.ends_with("ItemDefinition")).unwrap();
        let mesh_long = fn_fx::data::long_package(def.props.get("WeaponMeshOverride").and_then(|v| v.as_str()).unwrap()).to_owned();
        let mpkg = fn_pose::loader::package(&src, &mesh_long).unwrap();
        let smesh = fn_mesh::SkeletalMesh::parse(&mpkg).unwrap();
        println!("  mesh {mesh_long}; ref_skeleton root: pos {:?} rot {:?} scale {:?}", smesh.ref_skeleton[0].ref_pos, smesh.ref_skeleton[0].ref_rot, smesh.ref_skeleton[0].ref_scale);
        let mut sockets: Vec<(String, fn_mesh::Socket)> = smesh.sockets.iter().cloned().map(|s| ("mesh".to_owned(), s)).collect();
        if let Some(sp) = smesh.skeleton_path.as_deref() {
            if let Ok(pkg) = fn_pose::loader::package(&src, sp.split('.').next().unwrap()) {
                if let Ok(sk) = fn_mesh::Skeleton::parse(&pkg) { sockets.extend(sk.sockets.iter().cloned().map(|s| ("skeleton".to_owned(), s))); }
            }
        }
        let mut socket_pos: Vec<(String, Vec3)> = vec![];
        for (owner, s) in &sockets {
            let gl = Vec3::from(fn_mesh::gltf::ue_to_gltf_position(s.location));
            let b = part.tree.bone_index(&s.bone);
            let at = b.map(|i| gp.bone_mesh[i].transform_point3(gl));
            println!("  socket[{owner}] {:<18} bone {:<14} loc_ue {:?} rot_ue(p,y,r) {:?} scale {:?} -> gun-space cm {:?}", s.name, s.bone, s.location, s.rotation, s.scale, at.map(|p| (p * 100.).to_array().map(|v| (v * 10.).round() / 10.)));
            if let Some(a) = at { socket_pos.push((s.name.clone(), a)); }
        }
        // ---- barrel axis in gun space: the mesh's own `Muzzle` socket (UE: +X of the weapon = the barrel) from the vertex centroid, refined with the principal axis
        let (centroid, axis) = principal_axis(&verts);
        let muzzle = socket_pos.iter().find(|(n, _)| n == "Muzzle").map(|(_, p)| *p).expect("Muzzle socket");
        let barrel = if axis.dot((muzzle - centroid).normalize()) < 0. { -axis } else { axis };
        let handle = part.tree.bone_index("handle").map(|i| gp.bone_mesh[i].w_axis.truncate());
        println!("  vertex centroid (cm) {:?}, barrel axis (gun space) {:?}, Muzzle {:?}, handle bone {:?}", (centroid * 100.).to_array().map(|v| (v * 10.).round() / 10.), barrel.to_array().map(|v| (v * 1000.).round() / 1000.), (muzzle * 100.).to_array().map(|v| (v * 10.).round() / 10.), handle.map(|m| (m * 100.).to_array().map(|v| (v * 10.).round() / 10.)));

        // ---- one measurement of the current pose with the gun placed by `attach` (char-local matrix); returns [d_hand_r, d_hand_l, handle->hand_r, barrel yaw, barrel pitch]
        let measure = |label: &str, sim: &GameSim, ch: &Character, attach: Mat4, verbose: bool| -> [f32; 5] {
            let world = pawn_matrix(sim);
            let pose = &ch.pose;
            let body = &ch.rig.parts[0];
            let bp = |n: &str| -> Vec3 { let i = body.tree.bone_index(n).unwrap_or_else(|| panic!("bone {n}")); world.transform_point3(pose.parts[0].bone_mesh[i].w_axis.truncate()) };
            let gun_to_world = world * attach; // GUN FIT: with attach = the RightHand socket this is identical to Scene::update (`world * weapon_socket`)
            let wverts: Vec<Vec3> = verts.iter().map(|v| gun_to_world.transform_point3(*v)).collect();
            let barrel_w = gun_to_world.transform_vector3(barrel).normalize();
            let fwd_char = world.transform_vector3(Vec3::Z).normalize();
            let cam = camera_for(sim, 1280, 720);
            // barrel direction in character space (+Z forward, right = -X)
            let bc = world.inverse().transform_vector3(barrel_w);
            let (yaw, pitch) = ((-bc.x).atan2(bc.z).to_degrees(), bc.y.asin().to_degrees());
            let dr = nearest(&wverts, bp("hand_r")).0 * 100.;
            let dl = nearest(&wverts, bp("hand_l")).0 * 100.;
            let dh = handle.map_or(f32::NAN, |h| gun_to_world.transform_point3(h).distance(bp("hand_r")) * 100.);
            if verbose {
                println!("  -- {} / {label}: socket origin vs weapon_r bone {:.2} cm; speed2d {:.0}", names[g], (gun_to_world.w_axis.truncate() - bp("weapon_r")).length() * 100., sim.anim.vars.speed2d);
                for b in ["hand_r", "hand_l", "ik_hand_r", "ik_hand_l"] {
                    let (d, i) = nearest(&wverts, bp(b));
                    println!("     {b:<10} nearest gun vertex {:5.2} cm (vertex {i})", d * 100.);
                }
                println!("     hand_r -> handle bone {dh:.2} cm; hand_r in gun space {:?}; hand_l in gun space {:?} (cm)", (gun_to_world.inverse().transform_point3(bp("hand_r")) * 100.).to_array().map(|v| (v * 10.).round() / 10.), (gun_to_world.inverse().transform_point3(bp("hand_l")) * 100.).to_array().map(|v| (v * 10.).round() / 10.));
                println!("     barrel: angle to character forward {:.1} deg, to camera forward {:.1} deg; yaw {yaw:.1} (+right) pitch {pitch:.1} deg in character space", deg(barrel_w, fwd_char), deg(barrel_w, cam.forward));
            }
            [dr, dl, dh, yaw, pitch]
        };

        // ---- the player frames
        for frame in ["idle_hip", "idle_targeting", "jog"] {
            let mut sim = GameSim::new();
            sim.world.props.clear();
            sim.yaw = 0.; sim.pitch = 0.;
            sim.set_held(Some((core, frame == "idle_targeting")));
            if frame == "jog" { for _ in 0..80 { sim.step(&FrameInput { forward: 1., ..Default::default() }, 1. / 60.); } }
            else { for _ in 0..120 { sim.step_anim(Vec2::ZERO, 1. / 60.); } }
            sim.arms.clear();
            ch.evaluate(&mut sim);
            let socket = ch.pose.socket("RightHand").unwrap();
            measure(frame, &sim, &ch, socket, true);
            if frame == "idle_hip" || frame == "idle_targeting" { side_view(&format!("{}_{frame}_side.png", names[g]), &verts, socket, &ch, handle); }
            if frame == "idle_hip" {
                // sensitivity: the same pose with the socket yaw replaced (diagnostic only; nothing here feeds the runtime)
                let wr = ch.pose.parts[0].bone_mesh[ch.rig.parts[0].tree.bone_index("weapon_r").unwrap()];
                for a in [0f32, 90., 180., 270.] {
                    let m = measure("alt", &sim, &ch, wr * Mat4::from_rotation_y(a.to_radians()), false);
                    println!("     socket yaw {a:5.0} about glTF Y: hand_r {:5.1} cm, hand_l {:5.1} cm, hand_r->handle {:5.1} cm, barrel yaw {:7.1} pitch {:6.1}{}", m[0], m[1], m[2], m[3], m[4], if a == 90. { "   <- current RightHand socket" } else { "" });
                }
            }
            if frame == "jog" {
                // the run cycle: 60 further frames, summary of the same numbers
                let (mut lo, mut hi) = ([f32::MAX; 5], [f32::MIN; 5]);
                for _ in 0..60 {
                    sim.step(&FrameInput { forward: 1., ..Default::default() }, 1. / 60.);
                    sim.arms.clear();
                    ch.evaluate(&mut sim);
                    let s = ch.pose.socket("RightHand").unwrap();
                    let m = measure("cycle", &sim, &ch, s, false);
                    for k in 0..5 { lo[k] = lo[k].min(m[k]); hi[k] = hi[k].max(m[k]); }
                }
                println!("     jog cycle (60 frames): hand_r {:.2}..{:.2} cm, hand_l {:.2}..{:.2} cm, hand_r->handle {:.2}..{:.2} cm, barrel yaw {:.1}..{:.1} deg, pitch {:.1}..{:.1} deg", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], lo[3], hi[3], lo[4], hi[4]);
            }
        }
    }
}
