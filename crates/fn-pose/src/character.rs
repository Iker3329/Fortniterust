//! A character = several [`Part`]s evaluated against the same pose tree, plus attach sockets.
//!
//! The old runtime evaluates the pose tree once per part (`Model::draw_pose`: `eval_pose` + `apply_pose` on each part's own skeleton: the
//! part's glTF nodes + the master's IK bones, with the part's own local bind transforms), then reads sockets off the body. [`CharacterRig`]
//! does the same and returns, per part, a [`PoseResult`].
//!
//! # GPU upload
//! `fn-render` skins with `bones[joints[k]]` from a `var<storage, read> bones: array<mat4x4<f32>>` of `joint-slot` order: one column-major
//! `f32x16` (64 bytes) per skin joint (`Part::palette_len()` matrices), `skin_matrices[k] = bone_mesh[joints[k]] * inverse_bind[k]` in
//! character space; the object matrix (character -> world) is separate. `Renderer::update_model(id, model, &pose.skin_matrices)` takes the
//! slice as is; [`PoseResult::write_gpu_bytes`] / [`PoseResult::gpu_bytes`] produce the identical byte layout for a direct
//! `queue.write_buffer` (the buffer is `create_model(prims, part.palette_len())` sized).
use crate::eval::{EvalOptions, PoseEvaluator};
use crate::part::Part;
use crate::skel::BoneXform;
use fn_animgraph::{ClipLibrary, Node};
use glam::Mat4;

/// Weapon_r socket yaw in glTF space (UE `RightHand` socket Yaw = -90 converted: UE yaw is left-handed about +Z; in the handedness-flipped
/// glTF export the same socket is +90 about Y).
pub const PICK_SOCKET_YAW: f32 = std::f32::consts::FRAC_PI_2;
/// Bytes of one GPU bone matrix (`mat4x4<f32>`).
pub const GPU_MATRIX_BYTES: usize = 64;

#[cfg(test)]
mod original_socket_tests {
    use super::*;
    #[test]
    fn socket_conversion_preserves_units_axes_and_refuses_unrecovered_rotators() {
        let mut s=fn_mesh::Socket {name:"RightHand".into(),bone:"weapon_r".into(),location:[100.,200.,300.],rotation:[0.,0.,0.],scale:[1.,2.,3.]};
        let d=SocketDef::from_original_yaw_socket(0,&s).unwrap();
        assert_eq!(d.local.transform_point3(glam::Vec3::ZERO),glam::Vec3::new(1.,3.,2.));
        assert_eq!(d.local.transform_vector3(glam::Vec3::ONE),glam::Vec3::new(1.,3.,2.));
        s.rotation[0]=1.;assert!(SocketDef::from_original_yaw_socket(0,&s).is_err());
        s.rotation=[0.,0.,1.];assert!(SocketDef::from_original_yaw_socket(0,&s).is_err());
        s.rotation=[0.;3];s.location[0]=f32::NAN;assert!(SocketDef::from_original_yaw_socket(0,&s).is_err());
    }
}

/// Per-bone result of one part.
#[derive(Clone, Debug, Default)]
pub struct PoseResult {
    /// Local (translation, rotation) of every skeleton bone of the part (incl. the appended IK bones), after the whole pose tree.
    pub bone_local: Vec<BoneXform>,
    /// Character-local matrix of every skeleton bone (`parent chain of from_scale_rotation_translation(bind scale, rotation, translation)`).
    pub bone_mesh: Vec<Mat4>,
    /// `bone_mesh[joint] * inverse_bind` per skin joint slot: the GPU bone palette.
    pub skin_matrices: Vec<Mat4>,
    /// `TwoBoneIk` diagnostics of this part's evaluation: [FK-IK gap right hand, left hand, residual after the solve right, left] in metres.
    pub ik_stats: [f32; 4],
}

impl PoseResult {
    /// The palette as `[[f32; 4]; 4]` columns (`bytemuck`-castable; what `fn-render` writes).
    pub fn gpu_matrices(&self) -> Vec<[[f32; 4]; 4]> { self.skin_matrices.iter().map(|m| m.to_cols_array_2d()).collect() }
    /// Appends the palette in the storage-buffer byte layout (little-endian f32, column-major, 64 bytes per matrix).
    pub fn write_gpu_bytes(&self, out: &mut Vec<u8>) {
        out.reserve(self.skin_matrices.len() * GPU_MATRIX_BYTES);
        for m in &self.skin_matrices { for f in m.to_cols_array() { out.extend_from_slice(&f.to_le_bytes()); } }
    }
    pub fn gpu_bytes(&self) -> Vec<u8> { let mut v = Vec::new(); self.write_gpu_bytes(&mut v); v }
    /// Character-local position of a bone (needs the part for the name lookup).
    pub fn bone_position(&self, part: &Part, name: &str) -> Option<glam::Vec3> { let i = part.tree.bone_index(name)?; Some(self.bone_mesh.get(i)?.transform_point3(glam::Vec3::ZERO)) }
}

/// An attach point: `local` relative to `bone` of part `part`.
#[derive(Clone, Debug)]
pub struct SocketDef {
    pub name: String,
    pub part: usize,
    pub bone: String,
    pub local: Mat4,
}
impl SocketDef {
    /// Original socket data, restricted to the yaw-only orientation recovered
    /// for RightHand. Refuse an unsupported rotator rather than inventing its
    /// conversion or silently falling back to a guessed offset.
    pub fn from_original_yaw_socket(part: usize, socket: &fn_mesh::Socket) -> Result<SocketDef,String> {
        if socket.rotation[0]!=0. || socket.rotation[2]!=0. {
            return Err(format!("{}: unsupported non-yaw original socket rotation",socket.name));
        }
        if socket.location.iter().chain(socket.rotation.iter()).chain(socket.scale.iter()).any(|v|!v.is_finite()) {
            return Err(format!("{}: non-finite original socket transform",socket.name));
        }
        let position=glam::Vec3::from(fn_mesh::gltf::ue_to_gltf_position(socket.location));
        let scale=glam::Vec3::from(fn_mesh::gltf::ue_to_gltf_direction(socket.scale));
        // Same yaw matrix path as the existing calibrated RightHand socket;
        // going through a quaternion changes the floating-point coefficients.
        let rotation=Mat4::from_rotation_y(-socket.rotation[1].to_radians());
        Ok(SocketDef { name:socket.name.clone(),part,bone:socket.bone.clone(),local:Mat4::from_translation(position)*rotation*Mat4::from_scale(scale) })
    }
    /// The `RightHand` socket of the player skeleton (bone `weapon_r`, zero translation, yaw -90 UE = +90 about Y here): where SK_Pick attaches.
    pub fn right_hand(part: usize) -> SocketDef { SocketDef { name: "RightHand".into(), part, bone: "weapon_r".into(), local: Mat4::from_rotation_y(PICK_SOCKET_YAW) } }
}

/// All parts of a character evaluated together.
#[derive(Clone, Debug, Default)]
pub struct CharacterPose {
    pub parts: Vec<PoseResult>,
    /// `(socket name, character-local matrix)` for every socket whose bone exists.
    pub sockets: Vec<(String, Mat4)>,
    /// `TwoBoneIk` diagnostics of the last part evaluated: [FK-IK gap right, left, residual right, left] in metres.
    pub ik_stats: [f32; 4],
    /// Clip names the pose tree asked for that the library lacked.
    pub missing_clips: Vec<String>,
}
impl CharacterPose {
    pub fn socket(&self, name: &str) -> Option<Mat4> { self.sockets.iter().find(|s| s.0 == name).map(|s| s.1) }
}

pub struct CharacterRig {
    pub parts: Vec<Part>,
    pub sockets: Vec<SocketDef>,
    pub opts: EvalOptions,
}

impl CharacterRig {
    pub fn new(parts: Vec<Part>) -> CharacterRig { CharacterRig { parts, sockets: vec![SocketDef::right_hand(0)], opts: EvalOptions::default() } }

    /// Evaluates the tree on every part and builds the skin matrices and sockets.
    pub fn evaluate(&self, tree: &Node, clips: &dyn ClipLibrary) -> CharacterPose {
        let mut out = CharacterPose::default();
        self.evaluate_into(tree, clips, &mut out);
        out
    }

    /// Like [`evaluate`](Self::evaluate) but reuses the allocations of `out`.
    pub fn evaluate_into(&self, tree: &Node, clips: &dyn ClipLibrary, out: &mut CharacterPose) {
        out.parts.resize_with(self.parts.len(), PoseResult::default);
        out.missing_clips.clear();
        for (part, res) in self.parts.iter().zip(out.parts.iter_mut()) {
            let ev = PoseEvaluator::new(&part.tree, clips, self.opts);
            res.bone_local = ev.eval_pose(tree);
            part.tree.globals_into(&res.bone_local, &mut res.bone_mesh);
            part.skin_matrices_into(&res.bone_mesh, &mut res.skin_matrices);
            res.ik_stats = ev.ik_stats.get();
            out.ik_stats = res.ik_stats; // like the old thread-local `HAND_IK_STATS`: the last part drawn wins (the pickaxe-test log prints the head's)
            for m in ev.missing.borrow().iter() { if !out.missing_clips.contains(m) { out.missing_clips.push(m.clone()); } }
        }
        out.sockets.clear();
        for s in &self.sockets {
            let (Some(part), Some(res)) = (self.parts.get(s.part), out.parts.get(s.part)) else { continue };
            if let Some(i) = part.tree.bone_index(&s.bone) { out.sockets.push((s.name.clone(), res.bone_mesh[i] * s.local)); }
        }
    }
}
