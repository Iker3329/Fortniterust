//! Optional pose evaluation for the harness: the soldier skeleton and the prototype's clips loaded through `fn-legacy` (old md5anim /
//! glTF exports), so the animation graph's pose tree can be evaluated headlessly and bone transforms logged / hashed.
use fn_animgraph::{Node, Pose, Skeleton};
use fn_legacy::Md5Clip;
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::path::Path;

pub struct Rig { pub skeleton: Skeleton, pub clips: HashMap<String, Md5Clip> }
/// One evaluated frame: the local pose and the character-local global matrices.
pub struct Posed { pub pose: Pose, pub globals: Vec<Mat4> }

impl Rig {
    /// Loads the skeleton and every clip the prototype uses from an exported `assets` directory (the old tree's `assets/`).
    pub fn load(assets: &Path) -> Result<Rig, String> {
        let all = fn_legacy::files(assets);
        if all.is_empty() { return Err(format!("no files below {}", assets.display())); }
        let skeleton = fn_legacy::load_soldier_skeleton(&all)?;
        let clips = fn_legacy::load_clip_set(&all, &fn_legacy::prototype_clip_names())?;
        Ok(Rig { skeleton, clips })
    }
    pub fn eval(&self, node: &Node) -> Posed { let pose = self.skeleton.eval_pose(node, &self.clips); self.posed(pose) }
    /// The character-local matrices of an already evaluated (e.g. tick-interpolated) local pose.
    pub fn posed(&self, pose: Pose) -> Posed { let globals = self.skeleton.globals(&pose); Posed { pose, globals } }
    pub fn bone(&self, p: &Posed, name: &str) -> Option<Vec3> { self.skeleton.bone_position(&p.globals, name) }
}

/// FNV-1a over the exact bit patterns of the local pose (translation + rotation of every bone): equal iff the pose is bit-identical.
pub fn pose_hash(pose: &[(Vec3, glam::Quat)]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut put = |v: f32| { for b in v.to_bits().to_le_bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); } };
    for (p, q) in pose { put(p.x); put(p.y); put(p.z); put(q.x); put(q.y); put(q.z); put(q.w); }
    h
}
/// Same hash over arbitrary f32 values (used for the movement state).
pub fn f32_hash(values: &[f32]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in values { for b in v.to_bits().to_le_bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); } }
    h
}
