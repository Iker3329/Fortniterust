//! Bone hierarchy used by the evaluator: names (lower-case), parents, local bind transforms (translation, rotation, scale).
//!
//! This is the data of the old `assets::Model` (`names`, `parents`, `local`) plus the helpers the pose code needs. Units and axes are the
//! old glTF export's: metres, Y up, +Z forward, right = -X (see `fn_mesh::gltf` for the conversion from UE space).
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;

/// Local (translation, rotation) of one bone.
pub type BoneXform = (Vec3, Quat);
/// One pose: a local transform per skeleton bone.
pub type Pose = Vec<BoneXform>;

#[derive(Clone, Debug)]
pub struct BoneTree {
    pub names: Vec<String>,
    pub parents: Vec<Option<usize>>,
    /// Local bind transform (translation, rotation, scale).
    pub local: Vec<(Vec3, Quat, Vec3)>,
    index: HashMap<String, usize>,
}

impl BoneTree {
    pub fn new(names: Vec<String>, parents: Vec<Option<usize>>, local: Vec<(Vec3, Quat, Vec3)>) -> BoneTree {
        let mut t = BoneTree { names, parents, local, index: HashMap::new() };
        t.reindex();
        t
    }
    /// From the fn-animgraph skeleton type (same fields).
    pub fn from_animgraph(s: &fn_animgraph::Skeleton) -> BoneTree { BoneTree::new(s.names.clone(), s.parents.clone(), s.local.clone()) }
    fn reindex(&mut self) {
        self.index.clear();
        for (i, n) in self.names.iter().enumerate() { self.index.entry(n.clone()).or_insert(i); }
    }
    pub fn len(&self) -> usize { self.names.len() }
    pub fn is_empty(&self) -> bool { self.names.is_empty() }
    /// Index of a bone by (case-insensitive) name; the first bone of that name (the old `position` lookup).
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        if let Some(i) = self.index.get(name) { return Some(*i); }
        self.index.get(&name.to_lowercase()).copied()
    }
    /// Local bind pose (the reference pose of the mesh skeleton).
    pub fn bind_pose(&self) -> Pose { self.local.iter().map(|l| (l.0, l.1)).collect() }
    pub fn identity_pose(&self) -> Pose { vec![(Vec3::ZERO, Quat::IDENTITY); self.names.len()] }

    /// Component-space (position, rotation) of every bone (the model has unit scales). Parents precede children.
    pub fn component(&self, pose: &[BoneXform]) -> Pose {
        let mut c: Pose = Vec::with_capacity(pose.len());
        for i in 0..pose.len() {
            let (p, q) = pose[i];
            c.push(match self.parents[i] { Some(j) => { let (pp, pq) = c[j]; (pp + pq * p, (pq * q).normalize()) } None => (p, q) });
        }
        c
    }
    /// Local transform of bone `i` for a component-space transform, given the component transform of its parent.
    pub fn local_of(&self, i: usize, comp: &[BoneXform], world: BoneXform) -> BoneXform {
        match self.parents[i] {
            Some(j) => { let (pp, pq) = comp[j]; let inv = pq.conjugate(); (inv * (world.0 - pp), (inv * world.1).normalize()) }
            None => world,
        }
    }
    /// Mesh-space rotation of every bone.
    pub fn mesh_rotations(&self, pose: &[BoneXform]) -> Vec<Quat> {
        let mut m: Vec<Quat> = Vec::with_capacity(pose.len());
        for i in 0..pose.len() { m.push(match self.parents[i] { Some(j) => (m[j] * pose[i].1).normalize(), None => pose[i].1 }); }
        m
    }
    /// BlendDepth mask (`FAnimationRuntime::CreateMaskWeights`): for a filter bone with depth d the bone at k levels below it gets
    /// `min(1,(k+1)/d)` (d > 0), 1 (d = 0) or `max(0, 1-(k+1)/|d|)` (d < 0); later filters overwrite earlier ones; bones outside every
    /// filter get 0.
    pub fn mask_weights(&self, filters: &[(&str, i32)]) -> Vec<f32> {
        let n = self.names.len();
        let mut w = vec![0f32; n];
        for (bone, depth) in filters {
            let Some(root) = self.bone_index(bone) else { continue };
            let mut level: Vec<Option<usize>> = vec![None; n];
            level[root] = Some(0);
            for i in 0..n {
                if i != root {
                    if let Some(j) = self.parents[i] { if let Some(l) = level[j] { level[i] = Some(l + 1); } }
                }
            }
            for i in 0..n {
                if let Some(k) = level[i] {
                    let k1 = (k + 1) as f32;
                    w[i] = match *depth { 0 => 1., d if d > 0 => (k1 / d as f32).min(1.), d => (1. - k1 / (-d) as f32).max(0.) };
                }
            }
        }
        w
    }
    /// Character-local global matrices of a pose (the old `Model::apply_pose` hierarchy propagation, without the vertex skinning):
    /// `globals[i]` = parent chain of `from_scale_rotation_translation(local scale, rotation, translation)`. Parents precede children.
    pub fn globals(&self, pose: &[BoneXform]) -> Vec<Mat4> {
        let mut g = Vec::with_capacity(self.names.len());
        self.globals_into(pose, &mut g);
        g
    }
    pub fn globals_into(&self, pose: &[BoneXform], out: &mut Vec<Mat4>) {
        let n = self.names.len();
        out.clear();
        out.reserve(n);
        for i in 0..n {
            let local = Mat4::from_scale_rotation_translation(self.local[i].2, pose[i].1, pose[i].0);
            let g = match self.parents[i] { Some(j) => out[j] * local, None => local };
            out.push(g);
        }
    }
    /// Appends every bone of `master` that `self` lacks (parents first; a bone whose parent is unknown is skipped): the old
    /// `add_missing_master_bones`. The part meshes of 1.8 follow the master mesh `Fortnite_M_Avg_Player`, whose seven IK bones
    /// (`ik_foot_*`, `ik_hand_*`) the animation clips and the AnimBP's TwoBoneIK / HandIKRetargeting nodes work on. Returns how many
    /// bones were added. Bind transforms come from the master; the appended bones have no skin weights.
    pub fn append_missing_from(&mut self, master: &BoneTree) -> usize {
        let before = self.names.len();
        loop {
            let mut added = false;
            for i in 0..master.names.len() {
                let name = &master.names[i];
                if name.is_empty() || self.index.contains_key(name) { continue; }
                let Some(pi) = master.parents[i] else { continue };
                let Some(parent) = self.bone_index(&master.names[pi]) else { continue };
                self.names.push(name.clone());
                self.parents.push(Some(parent));
                self.local.push(master.local[i]);
                self.index.insert(name.clone(), self.names.len() - 1);
                added = true;
            }
            if !added { break; }
        }
        self.names.len() - before
    }
}
