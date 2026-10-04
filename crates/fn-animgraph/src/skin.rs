//! Renderer-free CPU skinning of a [`Skeleton`] pose: the vertex half of the old `assets::Model::apply_pose` / `skin_and_cache`, plus the
//! face-rigidity diagnostics (`skinned_bone_names`, `bone_in_frame`, `rigid_snapshot`) of `--face-test`.
//!
//! A [`SkinnedMesh`] is plain data (glTF-style positions / normals / joints / weights per primitive plus the skin's joint list and inverse
//! bind matrices); whoever loads it (fn-legacy from glTF today, fn-mesh later) fills it. [`SkinnedMesh::pose`] runs the whole shared pipeline:
//! local pose -> hierarchy propagation ([`Skeleton::globals`]) -> skin matrices -> skinned positions / normals. A GPU renderer uses the
//! `globals` + [`SkinnedMesh::skin_matrices`] and skips the CPU vertex pass.
use crate::skeleton::{BoneXform, Skeleton};
use glam::{Mat4, Vec3};

/// One skinned primitive (same layout as the glTF attributes; `j` are indices into the skin's joint list).
#[derive(Clone, Debug, Default)]
pub struct SkinPrimitive {
    /// Rest positions (metres, mesh space).
    pub p: Vec<[f32; 3]>,
    /// Rest normals (may be empty when only positions are needed).
    pub n: Vec<[f32; 3]>,
    /// Joint indices per vertex (into [`SkinnedMesh::joints`]).
    pub j: Vec<[u16; 4]>,
    /// Joint weights per vertex.
    pub w: Vec<[f32; 4]>,
}

/// The skin of a mesh: skeleton bone index per skin joint, inverse bind matrices, primitives.
#[derive(Clone, Debug, Default)]
pub struct SkinnedMesh {
    /// Skeleton bone (node) index of every skin joint.
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
    pub primitives: Vec<SkinPrimitive>,
}

/// Result of [`SkinnedMesh::pose`]: the character-local bone matrices and the skinned vertices of every primitive.
#[derive(Clone, Debug, Default)]
pub struct PosedMesh {
    pub globals: Vec<Mat4>,
    pub positions: Vec<Vec<Vec3>>,
    /// Skinned normals (normalised, zero where the weights cancel); empty vectors for primitives without normals.
    pub normals: Vec<Vec<Vec3>>,
}

impl SkinnedMesh {
    /// `globals[joint bone] * inverse bind` for every skin joint.
    pub fn skin_matrices(&self, globals: &[Mat4]) -> Vec<Mat4> { self.joints.iter().enumerate().map(|(i, j)| globals[*j] * self.inverse_bind[i]).collect() }
    /// Linear-blend skinning of every primitive with the given skin matrices.
    pub fn skin(&self, skin: &[Mat4]) -> (Vec<Vec<Vec3>>, Vec<Vec<Vec3>>) {
        let (mut positions, mut normals) = (vec![], vec![]);
        for prim in &self.primitives {
            let blend = |i: usize, v: [f32; 3], vector: bool| { let mut out = Vec3::ZERO; for k in 0..4 { let m = skin[prim.j[i][k] as usize]; out += if vector { m.transform_vector3(Vec3::from(v)) } else { m.transform_point3(Vec3::from(v)) } * prim.w[i][k]; } out };
            positions.push(prim.p.iter().enumerate().map(|(i, p)| blend(i, *p, false)).collect());
            normals.push(prim.n.iter().enumerate().map(|(i, n)| blend(i, *n, true).normalize_or_zero()).collect());
        }
        (positions, normals)
    }
    /// The old `Model::apply_pose`: propagates the local `pose` down the hierarchy and skins the mesh.
    pub fn pose(&self, skeleton: &Skeleton, pose: &[BoneXform]) -> PosedMesh {
        let globals = skeleton.globals(pose);
        let (positions, normals) = self.skin(&self.skin_matrices(&globals));
        PosedMesh { globals, positions, normals }
    }
    /// `--face-test`: names of every bone some vertex is skinned to (weight > 0), sorted by bone index.
    pub fn skinned_bone_names(&self, skeleton: &Skeleton) -> Vec<String> {
        let mut used = std::collections::BTreeSet::new();
        for p in &self.primitives { for i in 0..p.p.len() { for k in 0..4 { if p.w[i][k] > 0. { used.insert(self.joints[p.j[i][k] as usize]); } } } }
        used.into_iter().map(|b| skeleton.names[b].clone()).collect()
    }
    /// Skinned positions (from `posed`) in the frame of bone `root` of every vertex skinned only to `root` and its descendants (face / eye / lid /
    /// jaw / hair vertices for `head`): rigid with the bone unless the hierarchy was propagated wrongly. Empty when `root` is unknown.
    pub fn rigid_snapshot(&self, skeleton: &Skeleton, posed: &PosedMesh, root: &str) -> Vec<Vec3> {
        let Some(r) = skeleton.names.iter().position(|n| n == root) else { return vec![] };
        let Some(g) = posed.globals.get(r) else { return vec![] };
        let inv = g.inverse();
        let under = |mut b: usize| loop { if b == r { return true; } match skeleton.parents[b] { Some(p) => b = p, None => return false } };
        let mut out = vec![];
        for (k, p) in self.primitives.iter().enumerate() { for i in 0..p.p.len() {
            if (0..4).all(|q| p.w[i][q] <= 0. || under(self.joints[p.j[i][q] as usize])) { out.push(inv.transform_point3(posed.positions[k][i])); } } }
        out
    }
}

impl Skeleton {
    /// Position of bone `bone` expressed in the frame of bone `frame`, for the `globals` of a pose (the old `Model::bone_in_frame`).
    pub fn bone_in_frame(&self, globals: &[Mat4], bone: &str, frame: &str) -> Option<Vec3> {
        let (b, f) = (self.names.iter().position(|n| n == bone)?, self.names.iter().position(|n| n == frame)?);
        Some((globals.get(f)?.inverse() * *globals.get(b)?).transform_point3(Vec3::ZERO))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;
    fn two_bone_chain() -> (Skeleton, SkinnedMesh) {
        let sk = Skeleton { names: vec!["root".into(), "arm".into()], parents: vec![None, Some(0)], local: vec![(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE), (Vec3::new(0., 1., 0.), Quat::IDENTITY, Vec3::ONE)] };
        let prim = SkinPrimitive { p: vec![[0., 0., 0.], [0., 1., 0.], [0., 2., 0.]], n: vec![[0., 0., 1.]; 3], j: vec![[0, 0, 0, 0], [1, 0, 0, 0], [1, 0, 0, 0]], w: vec![[1., 0., 0., 0.], [1., 0., 0., 0.], [1., 0., 0., 0.]] };
        (sk, SkinnedMesh { joints: vec![0, 1], inverse_bind: vec![Mat4::IDENTITY, Mat4::from_translation(Vec3::new(0., -1., 0.))], primitives: vec![prim] })
    }
    #[test] fn bind_pose_skins_to_the_rest_vertices() {
        let (sk, mesh) = two_bone_chain(); let posed = mesh.pose(&sk, &sk.bind_pose());
        for (a, b) in posed.positions[0].iter().zip(&mesh.primitives[0].p) { assert!((*a - Vec3::from(*b)).length() < 1e-6); }
        assert!((posed.normals[0][0] - Vec3::Z).length() < 1e-6);
    }
    #[test] fn rotating_the_parent_carries_every_descendant_vertex_rigidly() {
        let (sk, mesh) = two_bone_chain(); let mut pose = sk.bind_pose(); pose[0].1 = Quat::from_rotation_z(0.7);
        let posed = mesh.pose(&sk, &pose);
        // the vertices skinned to `arm` (descendant of `root`) are rigid in the root's frame; the one bound to the root itself is too
        let snap = mesh.rigid_snapshot(&sk, &posed, "root"); assert_eq!(snap.len(), 3);
        for (a, b) in snap.iter().zip(&mesh.primitives[0].p) { assert!((*a - Vec3::from(*b)).length() < 1e-5, "{a:?} vs {b:?}"); }
        // only the arm's own vertices count for root = arm
        assert_eq!(mesh.rigid_snapshot(&sk, &posed, "arm").len(), 2); assert!(mesh.rigid_snapshot(&sk, &posed, "nope").is_empty());
    }
    #[test] fn bone_in_frame_and_skinned_bone_names() {
        let (sk, mesh) = two_bone_chain(); let mut pose = sk.bind_pose(); pose[0].1 = Quat::from_rotation_z(1.0); let g = sk.globals(&pose);
        let p = sk.bone_in_frame(&g, "arm", "root").unwrap(); assert!((p - Vec3::new(0., 1., 0.)).length() < 1e-5, "{p:?}");
        assert_eq!(mesh.skinned_bone_names(&sk), vec!["root", "arm"]);
    }
}
