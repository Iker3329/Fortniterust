//! Numeric comparison helpers used to verify the decoder against the old UEViewer md5anim exports.

use crate::md5::{Joint, Md5Anim};
use crate::sequence::{AnimSequence, RotationBlend};
use crate::skeleton::{BoneTransform, SkeletonInfo};

/// Quaternion `(x, y, z)` with `w = sqrt(1 - |xyz|^2) >= 0` (the md5 convention).
pub fn md5_quat(q: [f32; 3]) -> [f64; 4] {
    let (x, y, z) = (q[0] as f64, q[1] as f64, q[2] as f64);
    let w = (1.0 - x * x - y * y - z * z).max(0.0).sqrt();
    [x, y, z, w]
}

/// Angle in degrees of the rotation taking `a` to `b` (both md5 `xyz` quaternions).
pub fn rot_angle_deg(a: [f32; 3], b: [f32; 3]) -> f64 {
    angle_between(md5_quat(a), md5_quat(b))
}

pub fn angle_between(a: [f64; 4], b: [f64; 4]) -> f64 {
    // r = conj(a) * b
    let r = [
        a[3] * b[0] - a[0] * b[3] - a[1] * b[2] + a[2] * b[1],
        a[3] * b[1] + a[0] * b[2] - a[1] * b[3] - a[2] * b[0],
        a[3] * b[2] - a[0] * b[1] + a[1] * b[0] - a[2] * b[3],
        a[3] * b[3] + a[0] * b[0] + a[1] * b[1] + a[2] * b[2],
    ];
    let v = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
    (2.0 * v.atan2(r[3].abs())).to_degrees()
}

/// Position error in cm between two joints.
pub fn pos_err(a: [f32; 3], b: [f32; 3]) -> f64 {
    let d = [(a[0] - b[0]) as f64, (a[1] - b[1]) as f64, (a[2] - b[2]) as f64];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Old-runtime style interpolation of exported frames at fractional frame `f` (clamped, not looping): `lerp` for
/// positions, `slerp` for rotations.
pub fn md5_sample(m: &Md5Anim, f: f32) -> Vec<Joint> {
    let n = m.frames.len();
    let f = f.clamp(0.0, (n - 1) as f32);
    let a = f.floor() as usize;
    let b = (a + 1).min(n - 1);
    let t = f - a as f32;
    m.frames[a]
        .iter()
        .zip(&m.frames[b])
        .map(|((p0, q0), (p1, q1))| {
            let p = [p0[0] + (p1[0] - p0[0]) * t, p0[1] + (p1[1] - p0[1]) * t, p0[2] + (p1[2] - p0[2]) * t];
            let (a4, mut b4) = (md5_quat(*q0), md5_quat(*q1));
            let mut dot = a4[0] * b4[0] + a4[1] * b4[1] + a4[2] * b4[2] + a4[3] * b4[3];
            if dot < 0.0 {
                for c in b4.iter_mut() {
                    *c = -*c;
                }
                dot = -dot;
            }
            let tt = t as f64;
            let r = if dot > 0.9995 {
                [a4[0] + (b4[0] - a4[0]) * tt, a4[1] + (b4[1] - a4[1]) * tt, a4[2] + (b4[2] - a4[2]) * tt, a4[3] + (b4[3] - a4[3]) * tt]
            } else {
                let om = dot.acos();
                let (sa, sb) = (((1.0 - tt) * om).sin() / om.sin(), (tt * om).sin() / om.sin());
                [sa * a4[0] + sb * b4[0], sa * a4[1] + sb * b4[1], sa * a4[2] + sb * b4[2], sa * a4[3] + sb * b4[3]]
            };
            let l = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2] + r[3] * r[3]).sqrt();
            let s = if r[3] < 0.0 { -1.0 } else { 1.0 };
            (p, [(s * r[0] / l) as f32, (s * r[1] / l) as f32, (s * r[2] / l) as f32])
        })
        .collect()
}

/// Convert a native UE local transform to the md5 joint convention (position `(x,-y,z)`, rotation conjugated,
/// mirrored and sign-normalised).
pub fn ue_to_md5_joint(t: &BoneTransform) -> Joint {
    let q = t.rotation;
    let mut bo = [-q[0], -q[1], -q[2], q[3]];
    bo[1] *= -1.0;
    bo[3] *= -1.0;
    if bo[3] < 0.0 {
        bo = [-bo[0], -bo[1], -bo[2], -bo[3]];
    }
    ([t.translation[0], -t.translation[1], t.translation[2]], [bo[0], bo[1], bo[2]])
}

/// Error statistics between two sets of joints.
#[derive(Debug, Clone, Default)]
pub struct Diff {
    pub max_pos_cm: f64,
    pub max_rot_deg: f64,
    /// Largest absolute difference of any quaternion x/y/z component. Unlike the angle this is well conditioned for
    /// rotations near 180 degrees, where `w = sqrt(1 - |xyz|^2)` amplifies rounding of the stored components.
    pub max_xyz: f64,
    /// `(frame/sample index, bone index)` of the worst position error.
    pub worst_pos: (usize, usize),
    pub worst_rot: (usize, usize),
    pub joints_compared: usize,
}

impl Diff {
    pub fn merge(&mut self, other: &Diff) {
        if other.max_pos_cm > self.max_pos_cm {
            self.max_pos_cm = other.max_pos_cm;
            self.worst_pos = other.worst_pos;
        }
        if other.max_rot_deg > self.max_rot_deg {
            self.max_rot_deg = other.max_rot_deg;
            self.worst_rot = other.worst_rot;
        }
        self.max_xyz = self.max_xyz.max(other.max_xyz);
        self.joints_compared += other.joints_compared;
    }
}

pub fn compare_joints(new: &[Joint], old: &[Joint], idx: usize, mask: Option<&[bool]>, diff: &mut Diff) {
    for (b, (n, o)) in new.iter().zip(old).enumerate() {
        if let Some(m) = mask {
            if !m[b] {
                continue;
            }
        }
        let pe = pos_err(n.0, o.0);
        let re = rot_angle_deg(n.1, o.1);
        for k in 0..3 {
            diff.max_xyz = diff.max_xyz.max(((n.1[k] - o.1[k]) as f64).abs());
        }
        if pe > diff.max_pos_cm {
            diff.max_pos_cm = pe;
            diff.worst_pos = (idx, b);
        }
        if re > diff.max_rot_deg {
            diff.max_rot_deg = re;
            diff.worst_rot = (idx, b);
        }
        diff.joints_compared += 1;
    }
}

/// Compare every frame of two exports (same frame / joint count required, checked by the caller).
pub fn compare_all_frames(new: &Md5Anim, old: &Md5Anim) -> Diff {
    let mut d = Diff::default();
    for (f, (n, o)) in new.frames.iter().zip(&old.frames).enumerate() {
        compare_joints(n, o, f, None, &mut d);
    }
    d
}

/// Compare the pose sampled by [`AnimSequence::pose_at_frame`] at fractional frame `f` with the old export
/// interpolated like the old runtime. Only bones whose track has keys for both channels are compared (the old export writes zero for the others, `pose_at` the reference pose).
pub fn compare_sampled(seq: &AnimSequence, skel: &SkeletonInfo, old: &Md5Anim, f: f32, sample_idx: usize, blend: RotationBlend, diff: &mut Diff) {
    let pose = seq.pose_at_frame_with(f, skel, blend);
    let mine: Vec<Joint> = pose.iter().map(ue_to_md5_joint).collect();
    let theirs = md5_sample(old, f);
    let mask: Vec<bool> = (0..old.bone_names.len()).map(|b| seq.find_track(b).map(|t| !t.pos.is_empty() && !t.rot.is_empty()).unwrap_or(false)).collect();
    compare_joints(&mine, &theirs, sample_idx, Some(&mask), diff);
}
