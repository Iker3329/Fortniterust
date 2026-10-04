//! Skeleton + pose-tree evaluator: the GL-free half of the old `assets::Model` (`eval_pose`, `apply_pose` without the skinning,
//! `bone_position`, the two-bone IK solvers and the blend-depth masks), plus the traits that decouple it from any clip decoder.
//!
//! * [`ClipSource`]   one animation sequence. The old prototype implements it with its md5anim loader (crate `fn-legacy`); the
//!                    `fn-anim` decoder crate implements it for the cooked UAnimSequence data.
//! * [`ClipLibrary`]  name -> clip lookup (blanket implemented for `HashMap<String, impl ClipSource>`).
//! * [`Skeleton`]     bone names / parents / local bind transforms. Poses are `Vec<(Vec3 translation, Quat rotation)>` per bone, local space.
//!
//! Units and axes are those of the old prototype's glTF export: metres, Y up, +Z forward, right = -X.
use crate::graph::{Node, TwoBone};
use glam::{Mat4, Quat, Vec3, Vec4};
use std::collections::HashMap;
use std::sync::Mutex;

/// Local (translation, rotation) of one bone.
pub type BoneXform = (Vec3, Quat);
/// One pose: a local transform per skeleton bone.
pub type Pose = Vec<BoneXform>;

/// One animation sequence as the pose evaluator needs it.
pub trait ClipSource {
    /// Local transform of `bone` (lower-case name) at `t` seconds, already retargeted to the skeleton (the old `Clip::sample_at` followed by
    /// `Clip::fit(name, pose, bind)`; `bind` is the bone's local bind transform). `looping` wraps, otherwise the last frame holds.
    /// `None` when the clip has no track for the bone (the evaluator then uses the bind pose, or the identity delta for additive clips).
    fn sample_bone(&self, bone: &str, t: f32, looping: bool, bind: BoneXform) -> Option<BoneXform>;
    /// Additive clips: bones without a track stay the identity delta (everything else falls back to the bind pose).
    fn is_additive(&self) -> bool;
    /// Sequence length in seconds (frames / frame rate).
    fn duration(&self) -> f32;
    /// Whether the clip animates the bone (lower-case name).
    fn has_bone(&self, bone: &str) -> bool;
    /// A named UE float curve at this sequence time.  Curves missing from a
    /// sequence contribute their UE default (zero) when poses are blended.
    fn sample_float_curve(&self, _name: &str, _t: f32, _looping: bool) -> f32 { 0.0 }
    /// Whether the sequence carries the named float curve (UE marks it `bValid` in the evaluated `FBlendedCurve`, whatever its value).
    fn has_float_curve(&self, _name: &str) -> bool { false }
}

/// Name -> clip lookup used by [`Skeleton::eval_pose`].
pub trait ClipLibrary {
    fn clip(&self, name: &str) -> Option<&dyn ClipSource>;
}
impl<C: ClipSource> ClipLibrary for HashMap<String, C> {
    fn clip(&self, name: &str) -> Option<&dyn ClipSource> { self.get(name).map(|c| c as &dyn ClipSource) }
}

/// Evaluate a named float curve through the active pose tree.  Each blend
/// uses the same scalar child weights as its pose counterpart; control nodes
/// do not add a sequence and therefore just pass the curve through.
///
/// Every rule below is decoded from the 1.8 exe (see `CURVE_BLEND.md`, `parity/curve-blend-evidence/`).  The engine keeps a
/// per-curve `bValid` flag next to each value (`FCurveElement { f32 Value; u8 bValid }`); the helper returns `(value, valid)` and
/// the flags follow the exe (`valid |=` for accumulate / lerp / blend, copied for override).  A leaf clip's curve is valid when the
/// sequence carries it (`ClipSource::has_float_curve`), so an authored zero still overrides in the MaxWeight branch, as in the exe.
pub fn sample_blended_curve(node: &Node, clips: &dyn ClipLibrary, name: &str) -> f32 {
    /// `ZERO_ANIMWEIGHT_THRESH` (exe constant at RVA 0x4008e1c).
    const ZERO_W: f32 = 1e-5;
    fn mix(children: &[(Node, f32)], clips: &dyn ClipLibrary, name: &str) -> (f32, bool) {
        // FAnimationRuntime::BlendPosesTogether (RVA 0x1f89870 curve part): Override(c0, w0) then Accumulate(ci, wi) for wi > 1e-5, with
        // the weights used as given (no normalisation inside the engine function).  The port feeds raw graph weights, so it normalises
        // them first exactly like its pose path does (identical when the weights already sum to 1).  UNVERIFIED: which engine caller
        // normalises blend-space sample weights.
        let live: Vec<&(Node, f32)> = children.iter().filter(|(_, w)| *w > 0.).collect();
        let total: f32 = live.iter().map(|(_, w)| *w).sum();
        if live.is_empty() || total <= 1e-6 { return (0., false); }
        let (mut v, mut ok) = (0., false);
        for (i, (n, w)) in live.iter().map(|c| (&c.0, c.1 / total)).enumerate() {
            if i > 0 && w <= ZERO_W { continue; }
            let (cv, cok) = eval(n, clips, name);
            v += cv * w; ok |= cok;
        }
        (v, ok)
    }
    fn eval(node: &Node, clips: &dyn ClipLibrary, name: &str) -> (f32, bool) {
        match node {
            Node::Ref => (0., false),
            Node::Clip { name: clip, time, looping } => match clips.clip(clip) {
                Some(c) => (c.sample_float_curve(name, *time, *looping), c.has_float_curve(name)),
                None => (0., false),
            },
            Node::Mix(children) => mix(children, clips, name),
            // State-machine crossfade (RVA 0x1fdbd70 curve part): Override(c1, 1 - a) then, for a > 1e-5, Accumulate(c2, a):
            // value = (1 - a) * c1 + a * c2, valid = v1 || v2.  (TwoWayBlend uses FBlendedCurve::Lerp, RVA 0x1f9c230, same value.)
            Node::MixProfile { from, to, w, .. } => {
                let a = w.clamp(0., 1.);
                let (v1, ok1) = eval(from, clips, name);
                if a <= ZERO_W { return (v1 * (1. - a), ok1); }
                let (v2, ok2) = eval(to, clips, name);
                (v1 * (1. - a) + v2 * a, ok1 || ok2)
            }
            // FAnimNode_ApplyAdditive / ApplyMeshSpaceAdditive (RVA 0x1ab2c40 / 0x1fda7a0) -> AccumulateAdditivePose (0x1f84f30) ->
            // FBlendedCurve::Accumulate (0x1f848b0): Alpha <= 1e-5 passes the base through; otherwise base + additive * Alpha, valid |=.
            // The exe does not clamp Alpha.
            Node::ApplyAdditive { base, add, alpha } | Node::ApplyMeshAdditive { base, add, alpha } => {
                let (b, bok) = eval(base, clips, name);
                if !(*alpha > ZERO_W) { return (b, bok); }
                let (a, aok) = eval(add, clips, name);
                (b + a * *alpha, bok || aok)
            }
            // FAnimNode_RotationOffsetBlendSpace::Evaluate (RVA 0x1ab5160) calls AccumulateMeshSpaceRotationAdditiveToLocalPose
            // (0x1f852d0) directly; that function never touches the curves (FBlendedCurve::Accumulate is only reached through
            // AccumulateAdditivePose 0x1f84f30).  The output curve is exactly the base pose's curve.
            Node::AimOffset { base, .. } => eval(base, clips, name),
            // FAnimNode_LayeredBoneBlend::Evaluate (RVA 0x1ab3660) -> BlendPosesPerBoneFilter (0x1f87ed0 local / 0x1f888c0 mesh).  The
            // blend pose is only evaluated when its weight > 1e-5, otherwise its curve is the zero-initialised invalid one.  The curve
            // weight is the max over bones of the clamped per-bone weight (= the node weight when the filter has a full-weight bone).
            //  - BlendByWeight (2): Override(base, 1) then Accumulate(blend, w): base + blend * w, valid |=.
            //  - MaxWeight (0, the default branch): Override(base) then every VALID blend curve REPLACES the base value (no weight,
            //    no max()); a curve missing from the blend pose keeps the base value.
            //  - NormalizeByWeight (1) is not used by any 1.8 node and is not modelled.
            Node::Layered { base, blend, weight, curve_blend, .. } => {
                let w = weight.clamp(0., 1.);
                let (b, bok) = eval(base, clips, name);
                if !(w > ZERO_W) { return (b, bok); }
                let (x, xok) = eval(blend, clips, name);
                match curve_blend {
                    crate::CurveBlend::BlendByWeight => (b + x * w, bok || xok),
                    crate::CurveBlend::MaxWeight => if xok { (x, true) } else { (b, bok) },
                }
            }
            Node::RotateRoot { base, .. } | Node::TwoBoneIk { base, .. } | Node::HandRetarget { base, .. }
            | Node::CopyBone { base, .. } | Node::ModifyBone { base, .. } | Node::HeadTrack { base, .. } => eval(base, clips, name),
        }
    }
    eval(node, clips, name).0
}

/// Names of clips the evaluator was asked for but the library lacked (the old `assets::MISSING_OPTIONAL`, graph part only).
static MISSING: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Drains the missing-clip messages recorded since the last call.
pub fn take_missing_clips() -> Vec<String> { MISSING.lock().map(|mut m| std::mem::take(&mut *m)).unwrap_or_default() }
fn note_missing(message: String) { if let Ok(mut m) = MISSING.lock() { if !m.contains(&message) { m.push(message); } } }

thread_local! {
    /// Last `two_bone_ik` diagnostics: [raw FK-IK gap right, left, residual after solve right, left] in metres.
    pub static HAND_IK_STATS: std::cell::Cell<[f32; 4]> = const { std::cell::Cell::new([0.; 4]) };
}

/// Weapon_r socket yaw in glTF space (UE Yaw=-90 converted).
pub const PICK_SOCKET_YAW: f32 = std::f32::consts::FRAC_PI_2;

/// Analytic two-bone IK: returns the rotations (about the root joint, then about the new middle joint) that bring the end
/// `c` to `t` (clamped to reach) while keeping the middle joint `b` in its current bend plane; and the reach used.
pub fn two_bone(a: Vec3, b: Vec3, c: Vec3, t: Vec3) -> Option<(Quat, Quat, f32)> {
    let (l1, l2) = ((b - a).length(), (c - b).length()); if l1 < 1e-5 || l2 < 1e-5 { return None; }
    let to = t - a; let dist = to.length(); if dist < 1e-5 { return None; } let u = to / dist;
    let d = dist.min((l1 + l2) * 0.9999).max((l1 - l2).abs() * 1.0001 + 1e-5);
    let along = (l1 * l1 - l2 * l2 + d * d) / (2. * d); let h = (l1 * l1 - along * along).max(0.).sqrt();
    let ab = b - a; let mut pole = ab - u * ab.dot(u); if pole.length() < 1e-5 { pole = u.any_orthonormal_vector(); } let pole = pole.normalize();
    let new_b = a + u * along + pole * h;
    let rot1 = Quat::from_rotation_arc(ab.normalize(), (new_b - a).normalize());
    let c1 = a + rot1 * (c - a); let end_dir = (c1 - new_b).normalize(); let want = (a + u * d - new_b).normalize();
    Some((rot1, Quat::from_rotation_arc(end_dir, want), d))
}

/// `AnimationCore::SolveTwoBoneIK` (UE 4.16): positions of the joint and the end for the chain root -> joint -> end reaching `effector`
/// with the joint pulled toward `joint_target` (the plane through root, effector and joint target). With `allow_stretching` the limb
/// lengths scale by `(max_stretch-1) * clamp((reach/maxLen - start_stretch)/(max_stretch-start_stretch), 0, 1)`.
#[allow(clippy::too_many_arguments)]
pub fn solve_two_bone_ik(root: Vec3, joint: Vec3, end: Vec3, joint_target: Vec3, effector: Vec3, allow_stretching: bool, start_stretch: f32, max_stretch: f32) -> (Vec3, Vec3) {
    const SMALL: f32 = 1.0e-4;
    let mut lower = (end - joint).length(); let mut upper = (joint - root).length(); let mut max_limb = lower + upper;
    let delta = effector - root; let mut desired_length = delta.length();
    let dir = if desired_length < SMALL { desired_length = SMALL; Vec3::X } else { delta / desired_length };
    let jt = joint_target - root;
    let bend = if jt.length_squared() < SMALL { Vec3::Y } else if dir.cross(jt).length_squared() < SMALL { dir.any_orthonormal_vector() } else { (jt - dir * jt.dot(dir)).normalize() };
    if allow_stretching {
        let range = max_stretch - start_stretch;
        if range > SMALL && max_limb > SMALL { let scaling = (max_stretch - 1.) * (((desired_length / max_limb) - start_stretch) / range).clamp(0., 1.); if scaling > SMALL { lower *= 1. + scaling; upper *= 1. + scaling; max_limb *= 1. + scaling; } }
    }
    if desired_length > max_limb { return (root + dir * upper, root + dir * max_limb); }
    let two_ab = 2. * upper * desired_length;
    let mut cos = if two_ab != 0. { (upper * upper + desired_length * desired_length - lower * lower) / two_ab } else { 0. };
    let reverse = cos < 0.; cos = cos.clamp(-1., 1.);
    let angle = cos.acos(); let line = upper * angle.sin();
    let mut proj = (upper * upper - line * line).max(0.).sqrt(); if reverse { proj = -proj; }
    (root + dir * proj + bend * line, effector)
}

/// Handle centre line (x,z in SK_Pick mesh space; the handle runs along Y): mean of the vertices of the shaft (y -0.5..0.2).
pub fn handle_axis_xz(positions: &[[f32; 3]]) -> glam::Vec2 { let s: Vec<_> = positions.iter().filter(|p| p[1] > -0.5 && p[1] < 0.2).collect(); let n = s.len().max(1) as f32; glam::Vec2::new(s.iter().map(|p| p[0]).sum::<f32>() / n, s.iter().map(|p| p[2]).sum::<f32>() / n) }
/// Wrap centre of one hand: mean of the four finger-base joints and the thumb base (`side` is "r" or "l").
pub fn grip_center(side: &str, bone: impl Fn(&str) -> Option<Vec3>) -> Option<Vec3> { let mut sum = Vec3::ZERO; for f in ["index_01", "middle_01", "ring_01", "pinky_01", "thumb_01"] { sum += bone(&format!("{f}_{side}"))?; } Some(sum / 5.) }
/// Distance (m, perpendicular to the handle) from a hand's wrap centre to the handle axis, for a pose and socket frame.
pub fn grip_axis_distance(attach: Mat4, axis: glam::Vec2, center: Vec3) -> f32 { let p = attach.inverse().transform_point3(center); ((p.x - axis.x).powi(2) + (p.z - axis.y).powi(2)).sqrt() }

/// How `ApplyMeshAdditive` / `AimOffset` treat bones the additive clips have no track for (see [`Skeleton::mesh_additive`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshAdditiveMode {
    /// Current behaviour: untracked bones (face, eyes, jaw, twist bones) keep their local pose and ride on their deformed parent.
    #[default]
    TrackedMask,
    /// Pre-fix behaviour (`--face-test --legacy`): untracked bones were pinned to their mesh orientation, detaching the face from the head.
    Legacy,
}

/// Bone hierarchy with local bind transforms (translation, rotation, scale). Bone names are lower-case.
#[derive(Clone, Debug)]
pub struct Skeleton {
    pub names: Vec<String>,
    pub parents: Vec<Option<usize>>,
    pub local: Vec<(Vec3, Quat, Vec3)>,
}

impl Skeleton {
    pub fn bone_index(&self, name: &str) -> Option<usize> { let n = name.to_lowercase(); self.names.iter().position(|x| *x == n) }
    pub fn bone_count(&self) -> usize { self.names.len() }
    /// Local bind pose (the reference pose of the mesh skeleton).
    pub fn bind_pose(&self) -> Pose { self.local.iter().map(|l| (l.0, l.1)).collect() }
    pub fn identity_pose(&self) -> Pose { vec![(Vec3::ZERO, Quat::IDENTITY); self.names.len()] }
    /// One clip sampled on every bone (additive clips: bones without a track stay the identity delta, others the bind pose).
    pub fn clip_pose(&self, clip: &dyn ClipSource, t: f32, looping: bool) -> Pose {
        (0..self.names.len()).map(|i| { let bind = (self.local[i].0, self.local[i].1);
            match clip.sample_bone(&self.names[i], t, looping, bind) { Some(p) => p, None => if clip.is_additive() { (Vec3::ZERO, Quat::IDENTITY) } else { bind } } }).collect()
    }
    /// Component-space (position, rotation) of every bone (the model has unit scales).
    pub fn component(&self, pose: &[BoneXform]) -> Pose {
        let mut c: Pose = Vec::with_capacity(pose.len());
        for i in 0..pose.len() { let (p, q) = pose[i]; c.push(match self.parents[i] { Some(j) => { let (pp, pq) = c[j]; (pp + pq * p, (pq * q).normalize()) }, None => (p, q) }); }
        c
    }
    /// Local transform of bone `i` for a component-space transform, given the component transform of its parent.
    pub fn local_of(&self, i: usize, comp: &[BoneXform], world: BoneXform) -> BoneXform {
        match self.parents[i] { Some(j) => { let (pp, pq) = comp[j]; let inv = pq.conjugate(); (inv * (world.0 - pp), (inv * world.1).normalize()) }, None => world }
    }
    pub fn mesh_rotations(&self, pose: &[BoneXform]) -> Vec<Quat> {
        let mut m: Vec<Quat> = Vec::with_capacity(pose.len());
        for i in 0..pose.len() { m.push(match self.parents[i] { Some(j) => (m[j] * pose[i].1).normalize(), None => pose[i].1 }); } m
    }
    /// BlendDepth mask (`FAnimationRuntime::CreateMaskWeights`): for a filter bone with depth d the bone at k levels below it gets
    /// `min(1,(k+1)/d)` (d > 0), 1 (d = 0) or `max(0, 1-(k+1)/|d|)` (d < 0); later filters overwrite earlier ones; bones outside every
    /// filter get 0. (BlendDepth ramp as remembered from the engine; the three signs are what the serialized filters rely on.)
    pub fn mask_weights(&self, filters: &[(&str, i32)]) -> Vec<f32> {
        let n = self.names.len(); let mut w = vec![0f32; n];
        for (bone, depth) in filters {
            let Some(root) = self.bone_index(bone) else { continue };
            let mut level: Vec<Option<usize>> = vec![None; n]; level[root] = Some(0);
            for i in 0..n { if i != root { if let Some(j) = self.parents[i] { if let Some(l) = level[j] { level[i] = Some(l + 1); } } } }
            for i in 0..n { if let Some(k) = level[i] { let k1 = (k + 1) as f32; w[i] = match *depth { 0 => 1., d if d > 0 => (k1 / d as f32).min(1.), d => (1. - k1 / (-d) as f32).max(0.) }; } }
        }
        w
    }
    fn blend_pose(a: &BoneXform, b: &BoneXform, t: f32) -> BoneXform { (a.0.lerp(b.0, t), a.1.slerp(b.1, t)) }
    /// Evaluates a pose tree. `RefPose` / missing clips = bind pose (identity delta where the node is used additively).
    pub fn eval_pose(&self, node: &Node, clips: &dyn ClipLibrary) -> Pose { self.eval(node, clips, false, MeshAdditiveMode::TrackedMask) }
    /// [`eval_pose`](Self::eval_pose) with an explicit mesh-space additive mode (`MeshAdditiveMode::Legacy` reproduces the pre-face-fix pipeline
    /// of `--face-test --legacy`; it replaced the old global `assets::LEGACY_MESH_ADDITIVE` switch).
    pub fn eval_pose_with(&self, node: &Node, clips: &dyn ClipLibrary, mode: MeshAdditiveMode) -> Pose { self.eval(node, clips, false, mode) }
    fn eval(&self, node: &Node, clips: &dyn ClipLibrary, additive: bool, mode: MeshAdditiveMode) -> Pose {
        match node {
            Node::Ref => if additive { self.identity_pose() } else { self.bind_pose() },
            Node::Clip { name, time, looping } => match clips.clip(name) { Some(c) => self.clip_pose(c, *time, *looping), None => { note_missing(format!("animation clip {name} (pose tree)")); if additive { self.identity_pose() } else { self.bind_pose() } } },
            Node::Mix(children) => {
                let total: f32 = children.iter().filter(|c| c.1 > 0.).map(|c| c.1).sum();
                if children.is_empty() { return if additive { self.identity_pose() } else { self.bind_pose() }; }
                if total <= 1e-6 || children.len() == 1 { return self.eval(&children[0].0, clips, additive, mode); }
                let n = self.names.len(); let mut pos = vec![Vec3::ZERO; n]; let mut acc = vec![Vec4::ZERO; n]; let mut reference: Option<Vec<Quat>> = None;
                for (c, w) in children {
                    if *w <= 0. { continue; } let w = w / total; let p = self.eval(c, clips, additive, mode);
                    if reference.is_none() { reference = Some(p.iter().map(|x| x.1).collect()); }
                    let refq = reference.as_ref().unwrap();
                    for i in 0..n { pos[i] += p[i].0 * w; let mut q = p[i].1; if refq[i].dot(q) < 0. { q = -q; } acc[i] += Vec4::new(q.x, q.y, q.z, q.w) * w; }
                }
                (0..n).map(|i| (pos[i], if acc[i].length_squared() > 1e-12 { Quat::from_xyzw(acc[i].x, acc[i].y, acc[i].z, acc[i].w).normalize() } else { Quat::IDENTITY })).collect()
            }
            Node::MixProfile { from, to, w, profile } => {
                let a = self.eval(from, clips, additive, mode); let b = self.eval(to, clips, additive, mode);
                (0..a.len()).map(|i| { let s = profile.iter().find(|(n, _)| *n == self.names[i]).map_or(1., |e| e.1); Self::blend_pose(&a[i], &b[i], (w * s).clamp(0., 1.)) }).collect()
            }
            Node::ApplyAdditive { base, add, alpha } => {
                let b = self.eval(base, clips, additive, mode); if *alpha <= 1e-4 { return b; } let a = self.eval(add, clips, true, mode);
                (0..b.len()).map(|i| (b[i].0 + a[i].0 * *alpha, (Quat::IDENTITY.slerp(a[i].1, *alpha) * b[i].1).normalize())).collect()
            }
            Node::ApplyMeshAdditive { base, add, alpha } => { let b = self.eval(base, clips, additive, mode); if *alpha <= 1e-4 { return b; } let a = self.eval(add, clips, true, mode); let tr = self.tracked_bones(add, clips); self.mesh_additive(b, &a, *alpha, &tr, mode) }
            Node::AimOffset { base, samples, alpha } => {
                let b = self.eval(base, clips, additive, mode); if *alpha <= 1e-4 || samples.is_empty() { return b; }
                let mix = Node::Mix(samples.iter().map(|(n, w)| (Node::clip(n, 0., false), *w)).collect());
                let a = self.eval(&mix, clips, true, mode); let tr = self.tracked_bones(&mix, clips); self.mesh_additive(b, &a, *alpha, &tr, mode)
            }
            Node::Layered { base, blend, filters, weight, mesh_space, .. } => {
                let b = self.eval(base, clips, additive, mode); if *weight <= 1e-4 { return b; } let c = self.eval(blend, clips, additive, mode);
                let mask = self.mask_weights(filters);
                if !*mesh_space { return (0..b.len()).map(|i| Self::blend_pose(&b[i], &c[i], mask[i] * weight)).collect(); }
                let (mb, mc) = (self.mesh_rotations(&b), self.mesh_rotations(&c)); let mut new_mesh: Vec<Quat> = Vec::with_capacity(b.len()); let mut out = Vec::with_capacity(b.len());
                for i in 0..b.len() { let w = mask[i] * weight; let nm = mb[i].slerp(mc[i], w).normalize();
                    let q = match self.parents[i] { Some(j) => (new_mesh[j].conjugate() * nm).normalize(), None => nm }; new_mesh.push(nm); out.push((b[i].0.lerp(c[i].0, w), q)); }
                out
            }
            Node::RotateRoot { base, yaw, pitch } => {
                let mut p = self.eval(base, clips, additive, mode); if yaw.abs() < 1e-5 && pitch.abs() < 1e-5 { return p; }
                if let Some(r) = self.bone_index("root") {
                    // UE yaw + = to the right, pitch + = nose up; in this glTF space (up +Y, forward +Z, right -X) that is a rotation of -yaw about Y and -pitch about X.
                    let rot = Quat::from_rotation_y(-yaw.to_radians()) * Quat::from_rotation_x(-pitch.to_radians());
                    // FAnimNode_RotateRootBone::Evaluate: Pose[Root].SetRotation(Pose[Root].GetRotation() * MeshSpaceDeltaQuat).
                    p[r].1 = (p[r].1 * rot).normalize();
                }
                p
            }
            Node::HandRetarget { base, fk_weight, alpha } => {
                let mut p = self.eval(base, clips, additive, mode);
                let (Some(hr), Some(hl), Some(ir), Some(il), Some(gun)) = (self.bone_index("hand_r"), self.bone_index("hand_l"), self.bone_index("ik_hand_r"), self.bone_index("ik_hand_l"), self.bone_index("ik_hand_gun")) else { return p };
                let c = self.component(&p); let w = fk_weight.clamp(0., 1.);
                // FAnimNode_HandIKRetargeting::EvaluateBoneTransforms, exe RVA 0x1ab8db0 (decoded, see parity/gun-fit/GUN_FIT.md): with HandFKWeight w
                // (struct +0xC0; RightHandFK +0x70, LeftHandFK +0x80, RightHandIK +0x90, LeftHandIK +0xA0, IKBonesToMove +0xB0)
                //   offset = Lerp(LeftHandFK, RightHandFK, w) - Lerp(LeftHandIK, RightHandIK, w) = Lerp(leftGap, rightGap, w),
                // added to the translation of every IKBonesToMove bone. w = Hand_IK_to_FK_LR_Weight = 1.0, i.e. the RIGHT hand's FK-IK gap: the gun IK
                // bone (and with it ik_hand_l) follows the right hand that carries the weapon. (This was inverted before: w = 1 took the left gap.)
                let off = (c[hl].0 - c[il].0).lerp(c[hr].0 - c[ir].0, w) * alpha.clamp(0., 1.);
                let world = (c[gun].0 + off, c[gun].1); p[gun] = self.local_of(gun, &c, world); p
            }
            Node::CopyBone { base, from, to, alpha } => {
                let mut p = self.eval(base, clips, additive, mode);
                let (Some(s), Some(d)) = (self.bone_index(from), self.bone_index(to)) else { return p };
                let c = self.component(&p); let a = alpha.clamp(0., 1.);
                let copied = (c[d].0.lerp(c[s].0, a), c[d].1.slerp(c[s].1, a));
                p[d] = self.local_of(d, &c, copied); p
            }
            Node::ModifyBone { base, bone, yaw, pitch } => {
                let mut p = self.eval(base, clips, additive, mode); let Some(i) = self.bone_index(bone) else { return p };
                if yaw.abs() < 1e-5 && pitch.abs() < 1e-5 { return p; }
                let c = self.component(&p); let rot = Quat::from_rotation_y(-yaw.to_radians()) * Quat::from_rotation_x(-pitch.to_radians());
                p[i] = self.local_of(i, &c, (c[i].0, (rot * c[i].1).normalize())); p
            }
            Node::HeadTrack { base, yaw, pitch, limit } => {
                let mut p = self.eval(base, clips, additive, mode); let Some(h) = self.bone_index("head") else { return p };
                // the head's forward axis in its own frame = the bind pose's world +Z (the T-pose character faces +Z)
                let bind = self.component(&self.bind_pose()); let f_local = bind[h].1.conjugate() * Vec3::Z;
                let c = self.component(&p); let f_cur = (c[h].1 * f_local).normalize();
                let target = (Quat::from_rotation_y(-yaw.to_radians()) * Quat::from_rotation_x(-pitch.to_radians())) * Vec3::Z;
                let ang = f_cur.angle_between(target); let max = limit.to_radians(); let t = if ang > max { max / ang } else { 1. };
                let d = Quat::IDENTITY.slerp(Quat::from_rotation_arc(f_cur, target), t);
                p[h] = self.local_of(h, &c, (c[h].0, (d * c[h].1).normalize())); p
            }
            Node::TwoBoneIk { base, ik, alpha } => { let mut p = self.eval(base, clips, additive, mode); if *alpha > 1e-4 { self.two_bone_ik(&mut p, ik, *alpha); } p }
        }
    }
    /// AAT_RotationOffsetMeshSpace accumulation (`AccumulateMeshSpaceRotationAdditiveToLocalPose`): the delta rotates each bone in mesh
    /// space; local rotations are rebuilt against the already-deformed parent so a bone with no delta keeps its mesh orientation.
    /// `tracked[i]`: the additive animation has a track for bone i. A bone WITHOUT a track (face / eye / jaw / lid / brow bones, twist and
    /// dyn bones: the aim-offset clips key only the body chain) has no delta of its own, so it keeps its LOCAL pose and rides on its
    /// (deformed) parent. Treating it as an identity mesh-space delta pinned it to its old mesh orientation instead: the face bones
    /// stayed behind while the head rotated (eyes / mouth / lids detached from the head, 10 cm at +-80 deg pitch). `mode` =
    /// [`MeshAdditiveMode::Legacy`] ignores the mask (the pre-fix behaviour).
    pub fn mesh_additive(&self, base: Pose, add: &[BoneXform], alpha: f32, tracked: &[bool], mode: MeshAdditiveMode) -> Pose {
        let mb = self.mesh_rotations(&base); let mut nm: Vec<Quat> = Vec::with_capacity(base.len()); let mut out = Vec::with_capacity(base.len());
        let legacy = mode == MeshAdditiveMode::Legacy;
        for i in 0..base.len() {
            if !tracked[i] && !legacy { if let Some(j) = self.parents[i] { let q = base[i].1; nm.push((nm[j] * q).normalize()); out.push((base[i].0 + add[i].0 * alpha, q)); continue; } }
            let n = (Quat::IDENTITY.slerp(add[i].1, alpha) * mb[i]).normalize();
            let q = match self.parents[i] { Some(j) => (nm[j].conjugate() * n).normalize(), None => n }; nm.push(n); out.push((base[i].0 + add[i].0 * alpha, q)); }
        out
    }
    /// Bones for which at least one of `node`'s clips has a track (the mask `mesh_additive` needs; the old `Model::tracked_bones`).
    pub fn tracked_bones(&self, node: &Node, clips: &dyn ClipLibrary) -> Vec<bool> {
        let mut names = vec![]; node.clip_names(&mut names);
        self.names.iter().map(|b| names.iter().any(|n| clips.clip(n).is_some_and(|c| c.has_bone(b)))).collect()
    }
    /// UE bone-space vector (cm) to this model's bone-local frame (metres). Calibrated by `calibrate_joint_target_mapping`: (x,y,z) -> (x,-z,y)
    /// (the inverse of the clip loader's (x,z,-y) basis) keeps the authored elbow / knee in place when the effector is the FK end bone
    /// (worst displacement 0 cm for the hands, <1 cm for the left foot); the loader basis itself moves the elbow by up to 27 cm.
    pub fn ue_bone_vector(v: [f32; 3]) -> Vec3 { Vec3::new(v[0], -v[2], v[1]) * 0.01 }
    /// AnimNode_TwoBoneIK (`AnimationCore::SolveTwoBoneIK` + the node's rotation handling) on the chain end -> parent -> grandparent.
    fn two_bone_ik(&self, pose: &mut Pose, ik: &TwoBone, alpha: f32) {
        let (Some(end), Some(eff), Some(jb)) = (self.bone_index(ik.bone), self.bone_index(ik.effector), self.bone_index(ik.joint_bone)) else { return };
        let Some(lower) = self.parents[end] else { return }; let Some(upper) = self.parents[lower] else { return };
        let c = self.component(pose);
        let (root_p, joint_p, end_p) = (c[upper].0, c[lower].0, c[end].0);
        let effector = c[eff].0; let joint_target = c[jb].0 + c[jb].1 * Self::ue_bone_vector(ik.joint_target);
        let (new_joint, new_end) = solve_two_bone_ik(root_p, joint_p, end_p, joint_target, effector, ik.stretch, ik.start_stretch, ik.max_stretch);
        let old_u = (joint_p - root_p).normalize_or_zero(); let new_u = (new_joint - root_p).normalize_or_zero();
        let up_rot = (Quat::from_rotation_arc(old_u, new_u) * c[upper].1).normalize();
        let old_l = (end_p - joint_p).normalize_or_zero(); let new_l = (new_end - new_joint).normalize_or_zero();
        let lo_rot = (Quat::from_rotation_arc(old_l, new_l) * c[lower].1).normalize();
        // bTakeRotationFromEffectorSpace: the end bone takes the effector bone's component rotation
        let end_rot = c[eff].1;
        let mut nc = c.clone(); nc[upper] = (root_p, up_rot);
        let new_upper = self.local_of(upper, &c, nc[upper]); nc[lower] = (new_joint, lo_rot); let new_lower = self.local_of(lower, &nc, nc[lower]); nc[end] = (new_end, end_rot); let new_end_local = self.local_of(end, &nc, nc[end]);
        let a = alpha.clamp(0., 1.);
        pose[upper] = Self::blend_pose(&pose[upper], &new_upper, a); pose[lower] = Self::blend_pose(&pose[lower], &new_lower, a); pose[end] = Self::blend_pose(&pose[end], &new_end_local, a);
        // diagnostics for the pickaxe test: FK-to-IK gap before the solve and the residual after it, per hand
        let k = if ik.bone == "hand_r" { Some(0) } else if ik.bone == "hand_l" { Some(1) } else { None };
        if let Some(k) = k { let after = self.component(pose); HAND_IK_STATS.with(|s| { let mut v = s.get(); v[k] = (end_p - effector).length(); v[2 + k] = (after[end].0 - effector).length(); s.set(v); }); }
    }
    /// Character-local global matrices of a pose (the old `Model::apply_pose` minus the vertex skinning): `globals[i]` = parent chain of
    /// `from_scale_rotation_translation(local scale, rotation, translation)`.
    pub fn globals(&self, pose: &[BoneXform]) -> Vec<Mat4> {
        let n = self.names.len(); let mut globals = vec![Mat4::IDENTITY; n];
        for i in 0..n { let local = Mat4::from_scale_rotation_translation(self.local[i].2, pose[i].1, pose[i].0); globals[i] = self.parents[i].map(|j| globals[j] * local).unwrap_or(local); }
        globals
    }
    /// World (character-local) position of a bone in a `globals` result (the old `Model::bone_position`).
    pub fn bone_position(&self, globals: &[Mat4], name: &str) -> Option<Vec3> { let i = self.names.iter().position(|n| n == name)?; Some(globals.get(i)?.transform_point3(Vec3::ZERO)) }
    /// The pickaxe attach frame: weapon_r with the socket yaw (the old `Model::right_hand_socket`).
    pub fn right_hand_socket(&self, globals: &[Mat4]) -> Option<Mat4> { let i = self.names.iter().position(|n| n == "weapon_r")?; Some(*globals.get(i)? * Mat4::from_rotation_y(PICK_SOCKET_YAW)) }
}

#[cfg(test)]
mod curve_blend_tests {
    //! Exe-verified curve-combine rules (see `CURVE_BLEND.md`).
    use super::*;
    use crate::CurveBlend;
    struct C(f32);
    impl ClipSource for C {
        fn sample_bone(&self, _: &str, _: f32, _: bool, _: BoneXform) -> Option<BoneXform> { None }
        fn is_additive(&self) -> bool { false }
        fn duration(&self) -> f32 { 1. }
        fn has_bone(&self, _: &str) -> bool { false }
        fn sample_float_curve(&self, _: &str, _: f32, _: bool) -> f32 { self.0 }
        /// Fixture convention: a value of 0 means the sequence does not carry the curve.
        fn has_float_curve(&self, _: &str) -> bool { self.0 != 0. }
    }
    fn lib(v: &[(&str, f32)]) -> HashMap<String, C> { v.iter().map(|(n, x)| (n.to_string(), C(*x))).collect() }
    /// A sequence that carries the curve with an authored value of exactly 0.
    struct AuthoredZero;
    impl ClipSource for AuthoredZero {
        fn sample_bone(&self, _: &str, _: f32, _: bool, _: BoneXform) -> Option<BoneXform> { None }
        fn is_additive(&self) -> bool { false }
        fn duration(&self) -> f32 { 1. }
        fn has_bone(&self, _: &str) -> bool { false }
        fn sample_float_curve(&self, _: &str, _: f32, _: bool) -> f32 { 0. }
        fn has_float_curve(&self, _: &str) -> bool { true }
    }
    /// The curve's bValid flag, not its value, decides the MaxWeight override: an authored 0 in the blend pose replaces the base.
    #[test] fn layered_max_weight_authored_zero_still_overrides() {
        struct L(C, AuthoredZero);
        impl ClipLibrary for L { fn clip(&self, n: &str) -> Option<&dyn ClipSource> { match n { "base" => Some(&self.0), "blend" => Some(&self.1), _ => None } } }
        assert_eq!(sample_blended_curve(&layered(CurveBlend::MaxWeight, 1.), &L(C(0.9), AuthoredZero), "c"), 0.);
    }
    fn clip(n: &str) -> Node { Node::clip(n, 0., false) }
    fn layered(opt: CurveBlend, weight: f32) -> Node {
        Node::Layered { base: clip("base").bx_pub(), blend: clip("blend").bx_pub(), filters: vec![], weight, mesh_space: false, curve_blend: opt }
    }

    /// RVA 0x1f87ed0 default branch: MaxWeight is an override by validity, not max(): a smaller valid blend value replaces the base,
    /// and a curve absent from the blend pose keeps the base value.
    #[test] fn layered_max_weight_overrides_with_valid_blend_curve_instead_of_max() {
        let n = layered(CurveBlend::MaxWeight, 1.);
        assert_eq!(sample_blended_curve(&n, &lib(&[("base", 0.9), ("blend", 0.25)]), "c"), 0.25);
        assert_eq!(sample_blended_curve(&n, &lib(&[("base", 0.9), ("blend", 0.)]), "c"), 0.9);
        // weight-independent once above the 1e-5 gate (the exe arithmetic has no weight), and gated below it
        assert_eq!(sample_blended_curve(&layered(CurveBlend::MaxWeight, 0.2), &lib(&[("base", 0.9), ("blend", 0.25)]), "c"), 0.25);
        assert_eq!(sample_blended_curve(&layered(CurveBlend::MaxWeight, 5e-6), &lib(&[("base", 0.9), ("blend", 0.25)]), "c"), 0.9);
    }

    /// RVA 0x1f87ed0 option 2: base + blend * w (Accumulate), applied for any w > 1e-5 (ZERO_ANIMWEIGHT_THRESH), not only > 1e-4.
    #[test] fn layered_blend_by_weight_accumulates_down_to_the_1e_5_threshold() {
        let l = lib(&[("base", 1.), ("blend", 2.)]);
        let v = sample_blended_curve(&layered(CurveBlend::BlendByWeight, 5e-5), &l, "c");
        assert!((v - (1. + 2. * 5e-5)).abs() < 1e-9, "{v}");
        assert_eq!(sample_blended_curve(&layered(CurveBlend::BlendByWeight, 5e-6), &l, "c"), 1.);
    }

    /// RVA 0x1ab5160 -> 0x1f852d0: the rotation-offset blend space never accumulates its curves; the output curve is the base curve.
    #[test] fn rotation_offset_blend_space_keeps_only_the_base_curve() {
        let n = Node::AimOffset { base: clip("base").bx_pub(), samples: vec![("aim".to_string(), 1.)], alpha: 1. };
        assert_eq!(sample_blended_curve(&n, &lib(&[("base", 0.4), ("aim", 7.)]), "c"), 0.4);
    }

    /// RVA 0x1ab2c40 / 0x1fda7a0: ApplyAdditive passes the base through at Alpha <= 1e-5 and otherwise accumulates with the raw Alpha
    /// (the exe does not clamp it to 1).
    #[test] fn apply_additive_uses_unclamped_alpha_and_the_1e_5_gate() {
        let l = lib(&[("base", 1.), ("add", 0.5)]);
        let mk = |alpha| Node::ApplyAdditive { base: clip("base").bx_pub(), add: clip("add").bx_pub(), alpha };
        assert_eq!(sample_blended_curve(&mk(2.), &l, "c"), 2.);
        assert_eq!(sample_blended_curve(&mk(5e-6), &l, "c"), 1.);
    }
}

