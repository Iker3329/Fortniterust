//! Translation retargeting helpers, equivalent to the old runtime's `Clip::fit` / `retarget_pose`
//! (`FortniteRuntime/src/assets.rs`) but expressed in UE space (centimetres, UE axes) and driven directly by
//! [`AnimSequence`] + [`SkeletonInfo`] instead of md5anim comment lines.
//!
//! Rules (per bone, `bind` = the target skeleton's local reference transform):
//!
//! * `Animation` (0) and everything not listed below: keep the sampled translation.
//! * `Skeleton` (1): translation comes from the skeleton: `0` for additive clips, the *source* (retarget base) bone
//!   translation when the sequence carries a retarget base pose, else `bind.translation`.
//! * `AnimationScaled` (2): `anim * |bind| / |source|` (`bind`/`additive` fallback if `|source|` is ~0).
//! * `AnimationRelative` (3) is, like in UEViewer, treated as `Animation`.
//! * Bone 0 is always `Animation`.
//!
//! Channels without keys fall back to `bind` (non additive) or identity (additive).

use crate::sequence::AnimSequence;
use crate::skeleton::{BoneTransform, SkeletonInfo, TranslationRetarget};

/// Retarget settings of one bone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Retarget {
    pub mode: TranslationRetarget,
    /// Translation of the bone in the source (retarget) pose, centimetres.
    pub source: [f32; 3],
}

fn len(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// `retarget_pose` of the old runtime. `pose` = sampled `(translation, rotation)`, `bind` = target reference
/// transform, `source_override` = the sequence has an explicit retarget base pose.
pub fn retarget_pose(r: Retarget, pose: ([f32; 3], [f32; 4]), bind: ([f32; 3], [f32; 4]), additive: bool, source_override: bool) -> ([f32; 3], [f32; 4]) {
    let (p, q) = pose;
    match r.mode {
        TranslationRetarget::Skeleton => (if additive { [0.0; 3] } else if source_override { r.source } else { bind.0 }, q),
        TranslationRetarget::AnimationScaled => {
            let l = len(r.source);
            if l > 0.00001 {
                let k = len(bind.0) / l;
                ([p[0] * k, p[1] * k, p[2] * k], q)
            } else if additive {
                ([0.0; 3], [0.0, 0.0, 0.0, 1.0])
            } else {
                bind
            }
        }
        _ => (p, q),
    }
}

/// The retarget pose the sequence uses as translation source: its own retarget base (`RetargetSource` found in the
/// skeleton, or `RetargetSourceAssetReferencePose`) with scales baked, else the skeleton reference pose.
/// Returns `(pose, is_explicit_base)`.
pub fn source_pose(seq: &AnimSequence, skel: &SkeletonInfo) -> (Vec<BoneTransform>, bool) {
    let base: Option<&[BoneTransform]> = if seq.retarget_source == "None" {
        if seq.retarget_source_asset_pose.is_empty() {
            None
        } else {
            Some(&seq.retarget_source_asset_pose)
        }
    } else {
        skel.find_retarget_source(&seq.retarget_source).map(|s| s.pose.as_slice())
    };
    match base {
        Some(b) => (skel.bake_pose_scales(b).unwrap_or_else(|| b.to_vec()), true),
        None => (skel.bake_pose_scales(&skel.ref_pose).unwrap_or_else(|| skel.ref_pose.clone()), false),
    }
}

/// Retarget settings of every bone for `seq` on `skel`.
pub fn bone_retargets(seq: &AnimSequence, skel: &SkeletonInfo) -> Vec<Retarget> {
    let (src, _) = source_pose(seq, skel);
    (0..skel.num_bones())
        .map(|i| Retarget {
            mode: if i == 0 { TranslationRetarget::Animation } else { skel.retarget.get(i).copied().unwrap_or(TranslationRetarget::Animation) },
            source: src.get(i).map(|t| t.translation).unwrap_or([0.0; 3]),
        })
        .collect()
}

/// Apply translation retargeting to a pose sampled with [`AnimSequence::pose_at`] (the old `Clip::fit` for every
/// bone). `target_ref` is the reference pose of the skeleton/mesh the pose is going to be applied to (normally
/// `skel.ref_pose`).
pub fn retarget_pose_set(seq: &AnimSequence, skel: &SkeletonInfo, target_ref: &[BoneTransform], pose: &[BoneTransform]) -> Vec<BoneTransform> {
    let additive = seq.is_additive();
    let (_, explicit) = source_pose(seq, skel);
    let rts = bone_retargets(seq, skel);
    let mut out = pose.to_vec();
    for (i, o) in out.iter_mut().enumerate() {
        let bind = target_ref.get(i).copied().unwrap_or_default();
        let fallback = if additive { BoneTransform::ADDITIVE_IDENTITY } else { bind };
        let track = seq.find_track(i);
        let has_p = track.map(|t| !t.pos.is_empty()).unwrap_or(false);
        let has_q = track.map(|t| !t.rot.is_empty()).unwrap_or(false);
        if !has_p && !has_q {
            *o = fallback;
            continue;
        }
        let p = if has_p { o.translation } else { fallback.translation };
        let q = if has_q { o.rotation } else { fallback.rotation };
        let (np, nq) = retarget_pose(rts[i], (p, q), (bind.translation, bind.rotation), additive, explicit);
        o.translation = np;
        o.rotation = nq;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    #[test]
    fn scaled_translation_uses_bone_length_ratio() {
        let r = Retarget { mode: TranslationRetarget::AnimationScaled, source: [0.0, 1.0, 0.0] };
        let p = retarget_pose(r, ([0.0, 0.4, 0.0], ID), ([0.0, 2.0, 0.0], ID), false, false);
        assert!((p.0[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn skeleton_translation_does_not_add_reference_offset_twice() {
        let r = Retarget { mode: TranslationRetarget::Skeleton, source: [0.0, 1.0, 0.0] };
        let bind = ([0.0, 2.0, 0.0], ID);
        assert_eq!(retarget_pose(r, ([1.0, 0.0, 0.0], ID), bind, false, false).0, bind.0);
        assert_eq!(retarget_pose(r, ([1.0, 0.0, 0.0], ID), bind, true, false).0, [0.0; 3]);
        assert_eq!(retarget_pose(r, ([1.0, 0.0, 0.0], ID), bind, false, true).0, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn missing_source_bone_falls_back_without_nan() {
        let r = Retarget { mode: TranslationRetarget::AnimationScaled, source: [0.0; 3] };
        let bind = ([0.0, 1.0, 0.0], ID);
        assert_eq!(retarget_pose(r, ([1.0, 0.0, 0.0], ID), bind, false, false), bind);
        assert_eq!(retarget_pose(r, ([1.0, 0.0, 0.0], ID), bind, true, false), ([0.0; 3], ID));
    }

    #[test]
    fn animation_mode_passes_through() {
        let r = Retarget { mode: TranslationRetarget::Animation, source: [1.0; 3] };
        let p = retarget_pose(r, ([3.0, 4.0, 5.0], ID), ([9.0; 3], ID), true, true);
        assert_eq!(p.0, [3.0, 4.0, 5.0]);
        let r = Retarget { mode: TranslationRetarget::AnimationRelative, source: [1.0; 3] };
        assert_eq!(retarget_pose(r, ([3.0, 4.0, 5.0], ID), ([9.0; 3], ID), false, false).0, [3.0, 4.0, 5.0]);
    }
}
