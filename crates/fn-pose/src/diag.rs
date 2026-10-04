//! Diagnostics ported from the old `assets.rs` / `face_test.rs`: pickaxe grip measurements, face-vertex rigidity, pose hashing.
use crate::part::Part;
use glam::{Mat4, Vec2, Vec3};

/// Handle centre line (x,z in SK_Pick mesh space; the handle runs along Y): mean of the vertices of the shaft (y -0.5..0.2).
pub fn handle_axis_xz(positions: &[[f32; 3]]) -> Vec2 {
    let s: Vec<_> = positions.iter().filter(|p| p[1] > -0.5 && p[1] < 0.2).collect();
    let n = s.len().max(1) as f32;
    Vec2::new(s.iter().map(|p| p[0]).sum::<f32>() / n, s.iter().map(|p| p[2]).sum::<f32>() / n)
}
/// Wrap centre of one hand: mean of the four finger-base joints and the thumb base (`side` is "r" or "l").
pub fn grip_center(side: &str, bone: impl Fn(&str) -> Option<Vec3>) -> Option<Vec3> {
    let mut sum = Vec3::ZERO;
    for f in ["index_01", "middle_01", "ring_01", "pinky_01", "thumb_01"] { sum += bone(&format!("{f}_{side}"))?; }
    Some(sum / 5.)
}
/// Distance (m, perpendicular to the handle) from a hand's wrap centre to the handle axis, for a socket frame.
pub fn grip_axis_distance(attach: Mat4, axis: Vec2, center: Vec3) -> f32 {
    let p = attach.inverse().transform_point3(center);
    ((p.x - axis.x).powi(2) + (p.z - axis.y).powi(2)).sqrt()
}
/// Position of each hand's wrap centre ALONG the handle (metres, pick mesh Y) and the mesh's lowest Y: `(along_r, along_l, butt)`.
pub fn hand_along_handle(bone: impl Fn(&str) -> Option<Vec3>, pick_positions: &[[f32; 3]], attach: Mat4) -> Option<(f32, f32, f32)> {
    let inv = attach.inverse();
    let ymin = pick_positions.iter().map(|v| v[1]).fold(f32::MAX, f32::min);
    let r = grip_center("r", &bone)?;
    let l = grip_center("l", &bone)?;
    Some((inv.transform_point3(r).y, inv.transform_point3(l).y, ymin))
}

/// Skinned positions (in the frame of `root`) of every vertex skinned only to `root` and its descendants (face / eye / lid / jaw / hair
/// vertices for `head`), for the CPU-skinned prims `skinned` (from [`Part::skin_cpu`]): rigid with the bone unless the hierarchy was
/// propagated wrongly (the old `Model::rigid_snapshot`).
pub fn rigid_snapshot(part: &Part, globals: &[Mat4], skinned: &[(Vec<Vec3>, Vec<Vec3>)], root: &str) -> Vec<Vec3> {
    let Some(r) = part.tree.bone_index(root) else { return vec![] };
    let Some(g) = globals.get(r) else { return vec![] };
    let inv = g.inverse();
    let under = |mut b: usize| loop { if b == r { return true; } match part.tree.parents[b] { Some(p) => b = p, None => return false } };
    let mut out = vec![];
    for (k, p) in part.prims.iter().enumerate() {
        for i in 0..p.positions.len() {
            if (0..4).all(|q| p.weights[i][q] <= 0. || under(part.joints[p.joints[i][q] as usize])) { out.push(inv.transform_point3(skinned[k].0[i])); }
        }
    }
    out
}

/// Position of bone `bone` expressed in the frame of bone `frame`.
pub fn bone_in_frame(part: &Part, globals: &[Mat4], bone: &str, frame: &str) -> Option<Vec3> {
    let (b, f) = (part.tree.bone_index(bone)?, part.tree.bone_index(frame)?);
    Some((globals.get(f)?.inverse() * *globals.get(b)?).transform_point3(Vec3::ZERO))
}

/// FNV-1a over the exact bit patterns of the local pose: equal iff the pose is bit-identical (same function as `fn_sim::rig::pose_hash`).
pub fn pose_hash(pose: &[(Vec3, glam::Quat)]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut put = |v: f32| { for b in v.to_bits().to_le_bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); } };
    for (p, q) in pose { put(p.x); put(p.y); put(p.z); put(q.x); put(q.y); put(q.z); put(q.w); }
    h
}
