//! Skeleton data needed to sample animations: bone names, hierarchy, reference pose and per-bone
//! translation retargeting modes (`USkeleton::BoneTree`), plus the skeleton's named retarget poses
//! (`USkeleton::AnimRetargetSources`).
//!
//! ## `USkeleton` binary layout (UE 4.16, cooked, as parsed here)
//!
//! After the tagged property list (which carries `BoneTree`, `Sockets`, `SlotGroups`, ...) and the `int32`
//! `bSerializeGuid` (0):
//!
//! ```text
//! FReferenceSkeleton
//!   TArray<FMeshBoneInfo>  RefBoneInfo    i32 n; n * { FName name (i32 index, i32 number); i32 parent }
//!   TArray<FTransform>     RefBonePose    i32 n; n * { quat xyzw f32x4, translation f32x3, scale f32x3 }
//!   TMap<FName,int32>      NameToIndexMap i32 n; n * { FName, i32 }
//! TMap<FName,FReferencePose> AnimRetargetSources
//!                                         i32 n; n * { FName key; FName PoseName; TArray<FTransform> }
//! ```
//!
//! The rest (guid, smart names, ...) is not needed and ignored.

use crate::{err, Error, Result};
use fn_ueio::props::{read_props, Ctx, PropValue};
use fn_ueio::{Package, Reader};

/// UE `FTransform` (local space, centimetres, UE axes: X forward, Y right, Z up, left-handed).
/// Quaternion is stored `[x, y, z, w]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneTransform {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}

impl BoneTransform {
    pub const IDENTITY: BoneTransform = BoneTransform { rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0; 3], scale: [1.0; 3] };
    /// Identity of an additive pose: no rotation, no translation and a scale *delta* of zero (additive scale keys of
    /// 4.16 clips hold `scale - 1`: all additive scale keys in Fortnite 1.8 lie within +-0.16 of zero).
    pub const ADDITIVE_IDENTITY: BoneTransform = BoneTransform { rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0; 3], scale: [0.0; 3] };
}

impl Default for BoneTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// `EBoneTranslationRetargetingMode`. The numeric values are the ones the old md5anim `// retarget` lines carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranslationRetarget {
    /// Use translation from the animation.
    Animation = 0,
    /// Use translation from the (target) skeleton.
    Skeleton = 1,
    /// Translation from the animation, scaled by `|target bone translation| / |source bone translation|`.
    AnimationScaled = 2,
    /// Animation translation plus the additive offset from the source pose (not implemented by UEViewer either).
    AnimationRelative = 3,
    OrientAndScale = 4,
}

impl TranslationRetarget {
    pub fn from_name(s: &str) -> Self {
        let s = s.rsplit("::").next().unwrap_or(s);
        match s {
            "Animation" => Self::Animation,
            "Skeleton" => Self::Skeleton,
            "AnimationScaled" => Self::AnimationScaled,
            "AnimationRelative" => Self::AnimationRelative,
            "OrientAndScale" => Self::OrientAndScale,
            _ => Self::Animation,
        }
    }
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Skeleton,
            2 => Self::AnimationScaled,
            3 => Self::AnimationRelative,
            4 => Self::OrientAndScale,
            _ => Self::Animation,
        }
    }
}

/// A named retarget pose (`FReferencePose`), keyed by `UAnimSequence::RetargetSource`.
#[derive(Debug, Clone)]
pub struct RetargetSource {
    pub name: String,
    pub pose: Vec<BoneTransform>,
}

/// Everything the sampler needs from a `USkeleton` asset.
#[derive(Debug, Clone, Default)]
pub struct SkeletonInfo {
    pub bone_names: Vec<String>,
    /// Parent bone index, `-1` for the root.
    pub parents: Vec<i32>,
    pub ref_pose: Vec<BoneTransform>,
    /// Per-bone translation retargeting mode (`BoneTree[i].TranslationRetargetingMode`).
    pub retarget: Vec<TranslationRetarget>,
    pub retarget_sources: Vec<RetargetSource>,
}

fn read_fname(r: &mut Reader, names: &[String]) -> Result<String> {
    let idx = r.i32()?;
    let num = r.i32()?;
    let base = names.get(idx as usize).cloned().ok_or_else(|| Error::Format(format!("bad FName index {idx}")))?;
    Ok(if num > 0 { format!("{}_{}", base, num - 1) } else { base })
}

fn read_transform(r: &mut Reader) -> Result<BoneTransform> {
    let rotation = r.quat()?;
    let translation = r.vec3()?;
    let scale = r.vec3()?;
    Ok(BoneTransform { rotation, translation, scale })
}

fn read_transforms(r: &mut Reader) -> Result<Vec<BoneTransform>> {
    let n = r.i32()?;
    if n < 0 || n as usize * 40 > r.remaining() {
        return err(format!("bad transform array count {n}"));
    }
    (0..n).map(|_| read_transform(r)).collect()
}

impl SkeletonInfo {
    /// Parse the `Skeleton` export of a skeleton package.
    pub fn from_package(pkg: &Package) -> Result<SkeletonInfo> {
        let idx = pkg.find_export("Skeleton").ok_or_else(|| Error::Format("package has no Skeleton export".into()))?;
        let (start, end) = pkg.export_range(idx);
        let mut r = Reader::at(&pkg.data[..end], start);
        let ctx = Ctx { names: &pkg.names, file_version: pkg.summary.file_version };
        let props = read_props(&mut r, &ctx)?;
        let _serialize_guid = r.i32()?;

        let n = r.i32()?;
        if n < 0 || n as usize * 12 > r.remaining() {
            return err(format!("bad RefBoneInfo count {n}"));
        }
        let mut bone_names = Vec::with_capacity(n as usize);
        let mut parents = Vec::with_capacity(n as usize);
        for _ in 0..n {
            bone_names.push(read_fname(&mut r, &pkg.names)?);
            parents.push(r.i32()?);
        }
        if let Some(p) = parents.first_mut() {
            *p = -1;
        }
        let ref_pose = read_transforms(&mut r)?;
        if ref_pose.len() != bone_names.len() {
            return err(format!("RefBonePose count {} != bone count {}", ref_pose.len(), bone_names.len()));
        }
        let m = r.i32()?;
        if m < 0 || m as usize * 12 > r.remaining() {
            return err(format!("bad NameToIndexMap count {m}"));
        }
        for _ in 0..m {
            r.skip(12)?;
        }
        let k = r.i32()?;
        if k < 0 || k as usize * 20 > r.remaining() {
            return err(format!("bad AnimRetargetSources count {k}"));
        }
        let mut retarget_sources = Vec::new();
        for _ in 0..k {
            let key = read_fname(&mut r, &pkg.names)?;
            let _pose_name = read_fname(&mut r, &pkg.names)?;
            let pose = read_transforms(&mut r)?;
            retarget_sources.push(RetargetSource { name: key, pose });
        }

        let mut retarget = vec![TranslationRetarget::Animation; bone_names.len()];
        if let Some(PropValue::Array(nodes)) = props.get("BoneTree") {
            for (i, node) in nodes.iter().enumerate().take(retarget.len()) {
                if let Some(f) = node.as_fields() {
                    if let Some(m) = f.text("TranslationRetargetingMode") {
                        retarget[i] = TranslationRetarget::from_name(m);
                    }
                }
            }
        }
        Ok(SkeletonInfo { bone_names, parents, ref_pose, retarget, retarget_sources })
    }

    pub fn num_bones(&self) -> usize {
        self.bone_names.len()
    }

    pub fn find_bone(&self, name: &str) -> Option<usize> {
        self.bone_names.iter().position(|n| n == name)
    }

    pub fn find_retarget_source(&self, name: &str) -> Option<&RetargetSource> {
        self.retarget_sources.iter().find(|s| s.name == name)
    }

    /// Product of the `Scale3D` of all ancestors of `bone` (the bone's own scale is excluded) taken from `pose`.
    /// This is `GetBoneScale()` of the UEViewer exporter (`BAKE_BONE_SCALES`).
    pub fn ancestor_scale(&self, pose: &[BoneTransform], bone: usize) -> [f32; 3] {
        let mut s = [1.0f32; 3];
        let mut b = self.parents.get(bone).copied().unwrap_or(-1);
        let mut guard = 0;
        while b >= 0 && guard < 1024 {
            if let Some(t) = pose.get(b as usize) {
                for k in 0..3 {
                    s[k] *= t.scale[k];
                }
            }
            b = self.parents.get(b as usize).copied().unwrap_or(-1);
            guard += 1;
        }
        s
    }

    /// `AdjustBoneScales()` of UEViewer: translations multiplied by the accumulated ancestor scale. Returns `None`
    /// when the pose has a different bone count than this skeleton.
    pub fn bake_pose_scales(&self, pose: &[BoneTransform]) -> Option<Vec<BoneTransform>> {
        if pose.len() != self.num_bones() {
            return None;
        }
        Some(
            (0..pose.len())
                .map(|i| {
                    let s = self.ancestor_scale(pose, i);
                    let mut t = pose[i];
                    for k in 0..3 {
                        t.translation[k] *= s[k];
                    }
                    t
                })
                .collect(),
        )
    }
}

#[cfg(feature = "mesh")]
impl SkeletonInfo {
    /// Convert a skeleton parsed by `fn-mesh` (same data, different container).
    pub fn from_mesh_skeleton(s: &fn_mesh::Skeleton) -> SkeletonInfo {
        use fn_mesh::TranslationRetarget as M;
        let tf = |t: &fn_mesh::Transform| BoneTransform { rotation: t.rotation, translation: t.translation, scale: t.scale };
        SkeletonInfo {
            bone_names: s.bones.iter().map(|b| b.name.clone()).collect(),
            parents: s.bones.iter().map(|b| b.parent).collect(),
            ref_pose: s.bones.iter().map(|b| BoneTransform { rotation: b.ref_rot, translation: b.ref_pos, scale: b.ref_scale }).collect(),
            retarget: s
                .bones
                .iter()
                .map(|b| match b.retarget {
                    M::Animation => TranslationRetarget::Animation,
                    M::Skeleton => TranslationRetarget::Skeleton,
                    M::AnimationScaled => TranslationRetarget::AnimationScaled,
                    M::AnimationRelative => TranslationRetarget::AnimationRelative,
                    M::OrientAndScale => TranslationRetarget::OrientAndScale,
                })
                .collect(),
            retarget_sources: s.retarget_sources.iter().map(|r| RetargetSource { name: r.name.clone(), pose: r.pose.iter().map(tf).collect() }).collect(),
        }
    }
}