//! Pose-tree evaluation: the `fn_animgraph::Node` tree on a [`BoneTree`], a faithful port of the old `assets::Model::eval_pose`
//! (`eval`, `mesh_additive` incl. the face fix's tracked-bone mask, `two_bone_ik`, HandIKRetargeting, HeadTrack, ModifyBone, RotateRoot,
//! layered blends with BlendDepth masks).
//!
//! Poses are local `(translation, rotation)` per bone. `RefPose` / a missing clip is the bind pose (identity delta where the node is used
//! additively). Mesh-space additive (`AAT_RotationOffsetMeshSpace`) rotates each bone in mesh space; **bones without a track in the additive
//! clip(s)** (face / eye / jaw / lid / brow bones, twist and dyn bones: the aim-offset clips key only the body chain, `ClipSource::has_bone`
//! is false for them) keep their LOCAL pose and ride on the deformed parent; the pre-fix behaviour (pinned to their old mesh orientation,
//! which detached the face from the head by 20 cm) stays available as [`EvalOptions::legacy_mesh_additive`] for the `--face-test --legacy`
//! comparison.
use crate::skel::{BoneTree, BoneXform, Pose};
use fn_animgraph::{ClipLibrary, ClipSource, Node, TwoBone};
use glam::{Quat, Vec3, Vec4};
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, Default)]
pub struct EvalOptions {
    /// Pre-face-fix mesh-space additive (the old `LEGACY_MESH_ADDITIVE` switch).
    pub legacy_mesh_additive: bool,
}

/// AnimationCore::SolveTwoBoneIK (UE 4.16): positions of the joint and the end for the chain root -> joint -> end reaching `effector`
/// with the joint pulled toward `joint_target` (the plane through root, effector and joint target). With `allow_stretching` the limb
/// lengths scale by `(max_stretch-1) * clamp((reach/maxLen - start_stretch)/(max_stretch-start_stretch), 0, 1)`.
#[allow(clippy::too_many_arguments)]
pub fn solve_two_bone_ik(root: Vec3, joint: Vec3, end: Vec3, joint_target: Vec3, effector: Vec3, allow_stretching: bool, start_stretch: f32, max_stretch: f32) -> (Vec3, Vec3) {
    const SMALL: f32 = 1.0e-4;
    let mut lower = (end - joint).length();
    let mut upper = (joint - root).length();
    let mut max_limb = lower + upper;
    let delta = effector - root;
    let mut desired_length = delta.length();
    let dir = if desired_length < SMALL { desired_length = SMALL; Vec3::X } else { delta / desired_length };
    let jt = joint_target - root;
    let bend = if jt.length_squared() < SMALL { Vec3::Y } else if dir.cross(jt).length_squared() < SMALL { dir.any_orthonormal_vector() } else { (jt - dir * jt.dot(dir)).normalize() };
    if allow_stretching {
        let range = max_stretch - start_stretch;
        if range > SMALL && max_limb > SMALL {
            let scaling = (max_stretch - 1.) * (((desired_length / max_limb) - start_stretch) / range).clamp(0., 1.);
            if scaling > SMALL { lower *= 1. + scaling; upper *= 1. + scaling; max_limb *= 1. + scaling; }
        }
    }
    if desired_length > max_limb { return (root + dir * upper, root + dir * max_limb); }
    let two_ab = 2. * upper * desired_length;
    let mut cos = if two_ab != 0. { (upper * upper + desired_length * desired_length - lower * lower) / two_ab } else { 0. };
    let reverse = cos < 0.;
    cos = cos.clamp(-1., 1.);
    let angle = cos.acos();
    let line = upper * angle.sin();
    let mut proj = (upper * upper - line * line).max(0.).sqrt();
    if reverse { proj = -proj; }
    (root + dir * proj + bend * line, effector)
}

/// UE bone-space vector (cm) to this model's bone-local frame (metres). Calibrated by the old `calibrate_joint_target_mapping`: (x,y,z) -> (x,-z,y)
/// (the inverse of the clip loader's (x,z,-y) basis) keeps the authored elbow / knee in place when the effector is the FK end bone.
pub fn ue_bone_vector(v: [f32; 3]) -> Vec3 { Vec3::new(v[0], -v[2], v[1]) * 0.01 }

fn blend_pose(a: &BoneXform, b: &BoneXform, t: f32) -> BoneXform { (a.0.lerp(b.0, t), a.1.slerp(b.1, t)) }

/// Evaluates pose trees on one skeleton. Cheap to create; keeps the diagnostics of the last evaluation.
pub struct PoseEvaluator<'a> {
    pub tree: &'a BoneTree,
    pub clips: &'a dyn ClipLibrary,
    pub opts: EvalOptions,
    /// Last `TwoBoneIk` diagnostics: [raw FK-IK gap right hand, left hand, residual after the solve right, left] in metres (the old `HAND_IK_STATS`).
    pub ik_stats: Cell<[f32; 4]>,
    /// Clip names the tree asked for that the library lacked.
    pub missing: RefCell<Vec<String>>,
    masks: RefCell<Vec<(Vec<(&'static str, i32)>, Vec<f32>)>>,
}

impl<'a> PoseEvaluator<'a> {
    pub fn new(tree: &'a BoneTree, clips: &'a dyn ClipLibrary, opts: EvalOptions) -> PoseEvaluator<'a> {
        PoseEvaluator { tree, clips, opts, ik_stats: Cell::new([0.; 4]), missing: RefCell::new(vec![]), masks: RefCell::new(vec![]) }
    }
    fn note_missing(&self, name: &str) {
        let mut m = self.missing.borrow_mut();
        if !m.iter().any(|x| x == name) { m.push(name.to_owned()); }
    }
    /// Evaluates a pose tree (`additive = false` at the root).
    pub fn eval_pose(&self, node: &Node) -> Pose { self.eval(node, false) }

    fn rest(&self, additive: bool) -> Pose { if additive { self.tree.identity_pose() } else { self.tree.bind_pose() } }

    /// One clip sampled on every bone (additive clips: bones without a track stay the identity delta, others the bind pose).
    fn clip_pose(&self, clip: &dyn ClipSource, t: f32, looping: bool) -> Pose {
        let tr = self.tree;
        let additive = clip.is_additive();
        (0..tr.len())
            .map(|i| {
                let bind = (tr.local[i].0, tr.local[i].1);
                match clip.sample_bone(&tr.names[i], t, looping, bind) { Some(p) => p, None => if additive { (Vec3::ZERO, Quat::IDENTITY) } else { bind } }
            })
            .collect()
    }

    fn mask(&self, filters: &[(&'static str, i32)]) -> Vec<f32> {
        if let Some((_, w)) = self.masks.borrow().iter().find(|(f, _)| f.as_slice() == filters) { return w.clone(); }
        let w = self.tree.mask_weights(filters);
        self.masks.borrow_mut().push((filters.to_vec(), w.clone()));
        w
    }

    fn eval(&self, node: &Node, additive: bool) -> Pose {
        let tr = self.tree;
        match node {
            Node::Ref => self.rest(additive),
            Node::Clip { name, time, looping } => match self.clips.clip(name) {
                Some(c) => self.clip_pose(c, *time, *looping),
                None => { self.note_missing(name); self.rest(additive) }
            },
            Node::Mix(children) => {
                let total: f32 = children.iter().filter(|c| c.1 > 0.).map(|c| c.1).sum();
                if children.is_empty() { return self.rest(additive); }
                if total <= 1e-6 || children.len() == 1 { return self.eval(&children[0].0, additive); }
                let n = tr.len();
                let mut pos = vec![Vec3::ZERO; n];
                let mut acc = vec![Vec4::ZERO; n];
                let mut reference: Option<Vec<Quat>> = None;
                for (c, w) in children {
                    if *w <= 0. { continue; }
                    let w = w / total;
                    let p = self.eval(c, additive);
                    if reference.is_none() { reference = Some(p.iter().map(|x| x.1).collect()); }
                    let refq = reference.as_ref().unwrap();
                    for i in 0..n {
                        pos[i] += p[i].0 * w;
                        let mut q = p[i].1;
                        if refq[i].dot(q) < 0. { q = -q; }
                        acc[i] += Vec4::new(q.x, q.y, q.z, q.w) * w;
                    }
                }
                (0..n).map(|i| (pos[i], if acc[i].length_squared() > 1e-12 { Quat::from_xyzw(acc[i].x, acc[i].y, acc[i].z, acc[i].w).normalize() } else { Quat::IDENTITY })).collect()
            }
            Node::MixProfile { from, to, w, profile } => {
                let a = self.eval(from, additive);
                let b = self.eval(to, additive);
                (0..a.len()).map(|i| { let s = profile.iter().find(|(n, _)| *n == tr.names[i]).map_or(1., |e| e.1); blend_pose(&a[i], &b[i], (w * s).clamp(0., 1.)) }).collect()
            }
            Node::ApplyAdditive { base, add, alpha } => {
                let b = self.eval(base, additive);
                if *alpha <= 1e-4 { return b; }
                let a = self.eval(add, true);
                (0..b.len()).map(|i| (b[i].0 + a[i].0 * *alpha, (Quat::IDENTITY.slerp(a[i].1, *alpha) * b[i].1).normalize())).collect()
            }
            Node::ApplyMeshAdditive { base, add, alpha } => {
                let b = self.eval(base, additive);
                if *alpha <= 1e-4 { return b; }
                let a = self.eval(add, true);
                let tracked = self.tracked_bones(add);
                self.mesh_additive(b, &a, *alpha, &tracked)
            }
            Node::AimOffset { base, samples, alpha } => {
                let b = self.eval(base, additive);
                if *alpha <= 1e-4 || samples.is_empty() { return b; }
                let mix = Node::Mix(samples.iter().map(|(n, w)| (Node::clip(n, 0., false), *w)).collect());
                let a = self.eval(&mix, true);
                let tracked = self.tracked_bones(&mix);
                self.mesh_additive(b, &a, *alpha, &tracked)
            }
            Node::Layered { base, blend, filters, weight, mesh_space, .. } => {
                let b = self.eval(base, additive);
                if *weight <= 1e-4 { return b; }
                let c = self.eval(blend, additive);
                let mask = self.mask(filters);
                if !*mesh_space { return (0..b.len()).map(|i| blend_pose(&b[i], &c[i], mask[i] * weight)).collect(); }
                let (mb, mc) = (tr.mesh_rotations(&b), tr.mesh_rotations(&c));
                let mut new_mesh: Vec<Quat> = Vec::with_capacity(b.len());
                let mut out = Vec::with_capacity(b.len());
                for i in 0..b.len() {
                    let w = mask[i] * weight;
                    let nm = mb[i].slerp(mc[i], w).normalize();
                    let q = match tr.parents[i] { Some(j) => (new_mesh[j].conjugate() * nm).normalize(), None => nm };
                    new_mesh.push(nm);
                    out.push((b[i].0.lerp(c[i].0, w), q));
                }
                out
            }
            Node::RotateRoot { base, yaw, pitch } => {
                let mut p = self.eval(base, additive);
                if yaw.abs() < 1e-5 && pitch.abs() < 1e-5 { return p; }
                if let Some(r) = tr.bone_index("root") {
                    // UE yaw + = to the right, pitch + = nose up; in this glTF space (up +Y, forward +Z, right -X) that is a rotation of -yaw about Y and -pitch about X.
                    let rot = Quat::from_rotation_y(-yaw.to_radians()) * Quat::from_rotation_x(-pitch.to_radians());
                    // FAnimNode_RotateRootBone::Evaluate: Pose[Root].SetRotation(Pose[Root].GetRotation() * MeshSpaceDeltaQuat).
                    p[r].1 = (p[r].1 * rot).normalize();
                }
                p
            }
            Node::HandRetarget { base, fk_weight, alpha } => {
                let mut p = self.eval(base, additive);
                let (Some(hr), Some(hl), Some(ir), Some(il), Some(gun)) = (tr.bone_index("hand_r"), tr.bone_index("hand_l"), tr.bone_index("ik_hand_r"), tr.bone_index("ik_hand_l"), tr.bone_index("ik_hand_gun")) else { return p };
                let c = tr.component(&p);
                let w = fk_weight.clamp(0., 1.);
                // FAnimNode_HandIKRetargeting::EvaluateBoneTransforms, exe RVA 0x1ab8db0 (decoded, see parity/gun-fit/GUN_FIT.md): with HandFKWeight w
                // (struct +0xC0; RightHandFK +0x70, LeftHandFK +0x80, RightHandIK +0x90, LeftHandIK +0xA0, IKBonesToMove +0xB0)
                //   offset = Lerp(LeftHandFK, RightHandFK, w) - Lerp(LeftHandIK, RightHandIK, w) = Lerp(leftGap, rightGap, w),
                // added to the translation of every IKBonesToMove bone. w = Hand_IK_to_FK_LR_Weight = 1.0, i.e. the RIGHT hand's FK-IK gap: the gun IK
                // bone (and with it ik_hand_l) follows the right hand that carries the weapon. (This was inverted before: w = 1 took the left gap.)
                let off = (c[hl].0 - c[il].0).lerp(c[hr].0 - c[ir].0, w) * alpha.clamp(0., 1.);
                let world = (c[gun].0 + off, c[gun].1);
                p[gun] = tr.local_of(gun, &c, world);
                p
            }
            Node::CopyBone { base, from, to, alpha } => {
                let mut p = self.eval(base, additive);
                let (Some(s), Some(d)) = (tr.bone_index(from), tr.bone_index(to)) else { return p };
                let c = tr.component(&p);
                let a = alpha.clamp(0., 1.);
                let copied = (c[d].0.lerp(c[s].0, a), c[d].1.slerp(c[s].1, a));
                p[d] = tr.local_of(d, &c, copied);
                p
            }
            Node::ModifyBone { base, bone, yaw, pitch } => {
                let mut p = self.eval(base, additive);
                let Some(i) = tr.bone_index(bone) else { return p };
                if yaw.abs() < 1e-5 && pitch.abs() < 1e-5 { return p; }
                let c = tr.component(&p);
                let rot = Quat::from_rotation_y(-yaw.to_radians()) * Quat::from_rotation_x(-pitch.to_radians());
                p[i] = tr.local_of(i, &c, (c[i].0, (rot * c[i].1).normalize()));
                p
            }
            Node::HeadTrack { base, yaw, pitch, limit } => {
                let mut p = self.eval(base, additive);
                let Some(h) = tr.bone_index("head") else { return p };
                // the head's forward axis in its own frame = the bind pose's world +Z (the T-pose character faces +Z)
                let bind = tr.component(&tr.bind_pose());
                let f_local = bind[h].1.conjugate() * Vec3::Z;
                let c = tr.component(&p);
                let f_cur = (c[h].1 * f_local).normalize();
                let target = (Quat::from_rotation_y(-yaw.to_radians()) * Quat::from_rotation_x(-pitch.to_radians())) * Vec3::Z;
                let ang = f_cur.angle_between(target);
                let max = limit.to_radians(); // the 75 degree clamp of the graph is `limit`
                let t = if ang > max { max / ang } else { 1. };
                let d = Quat::IDENTITY.slerp(Quat::from_rotation_arc(f_cur, target), t);
                p[h] = tr.local_of(h, &c, (c[h].0, (d * c[h].1).normalize()));
                p
            }
            Node::TwoBoneIk { base, ik, alpha } => {
                let mut p = self.eval(base, additive);
                if *alpha > 1e-4 { self.two_bone_ik(&mut p, ik, *alpha); }
                p
            }
        }
    }

    /// Bones for which at least one of `node`'s clips has a track (`ClipSource::has_bone`); a clip the library lacks tracks nothing.
    fn tracked_bones(&self, node: &Node) -> Vec<bool> {
        let mut names = vec![];
        node.clip_names(&mut names);
        let clips: Vec<&dyn ClipSource> = names.iter().filter_map(|n| self.clips.clip(n)).collect();
        self.tree.names.iter().map(|b| clips.iter().any(|c| c.has_bone(b))).collect()
    }

    /// AAT_RotationOffsetMeshSpace accumulation (`AccumulateMeshSpaceRotationAdditiveToLocalPose`): the delta rotates each bone in mesh
    /// space; local rotations are rebuilt against the already-deformed parent so a bone with no delta keeps its mesh orientation.
    /// `tracked[i]`: the additive animation has a track for bone i. A bone WITHOUT a track has no delta of its own, so it keeps its LOCAL
    /// pose and rides on its (deformed) parent. Treating it as an identity mesh-space delta pinned it to its old mesh orientation
    /// instead: the face bones stayed behind while the head rotated (eyes / mouth / lids detached from the head, 10 cm at +-80 deg pitch).
    pub fn mesh_additive(&self, base: Pose, add: &[BoneXform], alpha: f32, tracked: &[bool]) -> Pose {
        let tr = self.tree;
        let mb = tr.mesh_rotations(&base);
        let mut nm: Vec<Quat> = Vec::with_capacity(base.len());
        let mut out = Vec::with_capacity(base.len());
        let legacy = self.opts.legacy_mesh_additive;
        for i in 0..base.len() {
            if !tracked[i] && !legacy {
                if let Some(j) = tr.parents[i] {
                    let q = base[i].1;
                    nm.push((nm[j] * q).normalize());
                    out.push((base[i].0 + add[i].0 * alpha, q));
                    continue;
                }
            }
            let n = (Quat::IDENTITY.slerp(add[i].1, alpha) * mb[i]).normalize();
            let q = match tr.parents[i] { Some(j) => (nm[j].conjugate() * n).normalize(), None => n };
            nm.push(n);
            out.push((base[i].0 + add[i].0 * alpha, q));
        }
        out
    }

    /// AnimNode_TwoBoneIK (`AnimationCore::SolveTwoBoneIK` + the node's rotation handling) on the chain end -> parent -> grandparent.
    fn two_bone_ik(&self, pose: &mut Pose, ik: &TwoBone, alpha: f32) {
        let tr = self.tree;
        let (Some(end), Some(eff), Some(jb)) = (tr.bone_index(ik.bone), tr.bone_index(ik.effector), tr.bone_index(ik.joint_bone)) else { return };
        let Some(lower) = tr.parents[end] else { return };
        let Some(upper) = tr.parents[lower] else { return };
        let c = tr.component(pose);
        let (root_p, joint_p, end_p) = (c[upper].0, c[lower].0, c[end].0);
        let effector = c[eff].0;
        let joint_target = c[jb].0 + c[jb].1 * ue_bone_vector(ik.joint_target);
        let (new_joint, new_end) = solve_two_bone_ik(root_p, joint_p, end_p, joint_target, effector, ik.stretch, ik.start_stretch, ik.max_stretch);
        let old_u = (joint_p - root_p).normalize_or_zero();
        let new_u = (new_joint - root_p).normalize_or_zero();
        let up_rot = (Quat::from_rotation_arc(old_u, new_u) * c[upper].1).normalize();
        let old_l = (end_p - joint_p).normalize_or_zero();
        let new_l = (new_end - new_joint).normalize_or_zero();
        let lo_rot = (Quat::from_rotation_arc(old_l, new_l) * c[lower].1).normalize();
        // bTakeRotationFromEffectorSpace: the end bone takes the effector bone's component rotation
        let end_rot = c[eff].1;
        let mut nc = c.clone();
        nc[upper] = (root_p, up_rot);
        let new_upper = tr.local_of(upper, &c, nc[upper]);
        nc[lower] = (new_joint, lo_rot);
        let new_lower = tr.local_of(lower, &nc, nc[lower]);
        nc[end] = (new_end, end_rot);
        let new_end_local = tr.local_of(end, &nc, nc[end]);
        let a = alpha.clamp(0., 1.);
        pose[upper] = blend_pose(&pose[upper], &new_upper, a);
        pose[lower] = blend_pose(&pose[lower], &new_lower, a);
        pose[end] = blend_pose(&pose[end], &new_end_local, a);
        // diagnostics for the pickaxe test: FK-to-IK gap before the solve and the residual after it, per hand
        let k = if ik.bone == "hand_r" { Some(0) } else if ik.bone == "hand_l" { Some(1) } else { None };
        if let Some(k) = k {
            let after = tr.component(pose);
            let mut v = self.ik_stats.get();
            v[k] = (end_p - effector).length();
            v[2 + k] = (after[end].0 - effector).length();
            self.ik_stats.set(v);
        }
    }
}

