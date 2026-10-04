//! `.fnanim`: compact binary animation container that the new runtime can load without any UE code.
//!
//! All integers little endian. `str` = `u16 byte_length` + UTF-8 bytes.
//!
//! ```text
//! offset  type      field
//! 0       [u8;4]    magic "FNAN"
//! 4       u16       version (1)
//! 6       u16       flags: bit0 additive, bit1 retarget-base (RetargetSource pose present),
//!                          bit2 root motion enabled, bit3 step interpolation
//! 8       u32       num_frames
//! 12      u32       num_bones   (skeleton bones, in skeleton order)
//! 16      f32       length            SequenceLength (seconds)
//! 20      f32       rate_scale
//! 24      f32       frame_rate        NumFrames / length * rate_scale (what the md5anim files carry)
//! 28      u8        additive_kind     0 none, 1 local space base, 2 rotation offset mesh space
//! 29      u8        ref_pose_type     0 none, 1 ref pose, 2 anim scaled, 3 anim frame
//! 30      u8        precision         0 = F32 (lossless), 1 = Q16 (compact)
//! 31      u8        reserved (0)
//! 32      i32       ref_frame_index
//! 36      str       name
//!         str       skeleton          skeleton object name (may be empty)
//!         str       retarget_source   RetargetSource name ("None" when unset)
//!         str       ref_pose_seq      additive base sequence name (may be empty)
//!         per bone (num_bones):
//!           str     bone name
//!           u8      channels          bit0 position keys, bit1 rotation keys, bit2 scale keys
//!           u8      retarget_mode     EBoneTranslationRetargetingMode (0..4; bone 0 is always 0)
//!           f32x3   retarget_source   translation (cm, UE axes) of the bone in the retarget base pose
//!                                     (or the skeleton reference pose) - the `// retarget` line of the md5anim
//!         per bone, per present channel in order position, rotation, scale:
//!           u32     key_count         1 (constant) or num_frames (one key per frame, integer frames)
//!           F32 / key_count == 1:     key_count * C f32       (C = 3 for position/scale, 4 for rotation xyzw)
//!           Q16 and key_count > 1:    C * { f32 min, f32 range } then key_count * C u16,
//!                                     value = min + range * (q / 65535)
//!         u32       notify_count, each: str name, str class, f32 trigger_time, f32 duration,
//!                   f32 trigger_chance, i32 track_index
//!         u32       sync_marker_count, each: str name, f32 time
//!         u32       curve_count, each: str name, i32 flags, f32 default, u32 key_count,
//!                   key_count * { f32 time, f32 value, f32 arrive_tangent, f32 leave_tangent, u8 interp_mode }
//!         [u8;4]    trailer "ENDF"
//! ```
//!
//! Channel values are the *decoded* local transforms in UE space (centimetres, UE axes, quaternion `x y z w`), as
//! returned by [`AnimSequence::pose_at_frame`] at integer frames. Channels the sequence has no data for are absent
//! (the consumer substitutes the reference pose, or identity for additive clips). Q16 quantisation error is at most
//! `range / 131070` per component (rotations: range <= 2 -> <= 1.6e-5).

use crate::retarget;
use crate::sequence::{nlerp, AdditiveAnimType, AnimSequence, CurveKey, FloatCurve, Interpolation, Notify, RefPoseType, SyncMarker};
use crate::skeleton::{BoneTransform, SkeletonInfo, TranslationRetarget};
use crate::{err, Result};

pub const MAGIC: &[u8; 4] = b"FNAN";
pub const VERSION: u16 = 1;

/// Storage precision of multi-key channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// 32-bit floats, lossless.
    F32,
    /// 16-bit normalised values with per-channel ranges.
    Q16,
}

/// One skeleton bone of a [`FnAnim`].
#[derive(Debug, Clone, PartialEq)]
pub struct FnBone {
    pub name: String,
    pub retarget: TranslationRetarget,
    /// Translation of the bone in the retarget source pose (cm).
    pub retarget_source: [f32; 3],
    /// `None` = no data for the channel; otherwise 1 (constant) or `num_frames` keys.
    pub pos: Option<Vec<[f32; 3]>>,
    pub rot: Option<Vec<[f32; 4]>>,
    pub scale: Option<Vec<[f32; 3]>>,
}

/// In-memory form of a `.fnanim` file.
#[derive(Debug, Clone, PartialEq)]
pub struct FnAnim {
    pub name: String,
    pub skeleton: String,
    pub retarget_source: String,
    pub ref_pose_seq: String,
    pub num_frames: usize,
    pub length: f32,
    pub rate_scale: f32,
    pub frame_rate: f32,
    pub additive: bool,
    pub additive_kind: u8,
    pub ref_pose_type: u8,
    pub ref_frame_index: i32,
    pub retarget_base: bool,
    pub root_motion: bool,
    pub step: bool,
    pub bones: Vec<FnBone>,
    pub notifies: Vec<Notify>,
    pub sync_markers: Vec<SyncMarker>,
    pub curves: Vec<FloatCurve>,
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(u16::MAX as usize);
    out.extend_from_slice(&(n as u16).to_le_bytes());
    out.extend_from_slice(&b[..n]);
}
fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

struct Cur<'a> {
    d: &'a [u8],
    p: usize,
}
impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.p + n > self.d.len() {
            return err(format!("fnanim: truncated at {} (+{n})", self.p));
        }
        let s = &self.d[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn str(&mut self) -> Result<String> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

fn write_channel<const C: usize>(out: &mut Vec<u8>, keys: &[[f32; C]], precision: Precision) {
    put_u32(out, keys.len() as u32);
    if precision == Precision::F32 || keys.len() == 1 {
        for k in keys {
            for c in k {
                put_f32(out, *c);
            }
        }
        return;
    }
    let mut mins = [f32::MAX; C];
    let mut maxs = [f32::MIN; C];
    for k in keys {
        for c in 0..C {
            mins[c] = mins[c].min(k[c]);
            maxs[c] = maxs[c].max(k[c]);
        }
    }
    for c in 0..C {
        put_f32(out, mins[c]);
        put_f32(out, maxs[c] - mins[c]);
    }
    for k in keys {
        for c in 0..C {
            let range = maxs[c] - mins[c];
            let q = if range > 0.0 { ((k[c] - mins[c]) / range * 65535.0).round().clamp(0.0, 65535.0) as u16 } else { 0 };
            out.extend_from_slice(&q.to_le_bytes());
        }
    }
}

fn read_channel<const C: usize>(c: &mut Cur, precision: Precision, num_frames: usize) -> Result<Vec<[f32; C]>> {
    let n = c.u32()? as usize;
    if n != 1 && n != num_frames {
        return err(format!("fnanim: channel has {n} keys, expected 1 or {num_frames}"));
    }
    let mut out = Vec::with_capacity(n);
    if precision == Precision::F32 || n == 1 {
        for _ in 0..n {
            let mut k = [0.0f32; C];
            for v in k.iter_mut() {
                *v = c.f32()?;
            }
            out.push(k);
        }
        return Ok(out);
    }
    let mut mins = [0.0f32; C];
    let mut ranges = [0.0f32; C];
    for i in 0..C {
        mins[i] = c.f32()?;
        ranges[i] = c.f32()?;
    }
    for _ in 0..n {
        let mut k = [0.0f32; C];
        for i in 0..C {
            k[i] = mins[i] + ranges[i] * (c.u16()? as f32 / 65535.0);
        }
        out.push(k);
    }
    Ok(out)
}

/// Collapse `keys` to a single key when all are exactly equal.
fn collapse<const C: usize>(keys: Vec<[f32; C]>) -> Vec<[f32; C]> {
    if keys.len() > 1 && keys.iter().all(|k| k.iter().zip(&keys[0]).all(|(a, b)| a.to_bits() == b.to_bits())) {
        vec![keys[0]]
    } else {
        keys
    }
}

impl FnAnim {
    /// Bake a sequence for `skel` (decoded channels resampled at every integer frame).
    pub fn from_sequence(seq: &AnimSequence, skel: &SkeletonInfo) -> FnAnim {
        let rts = retarget::bone_retargets(seq, skel);
        let (_, explicit) = retarget::source_pose(seq, skel);
        let step = seq.interpolation == Interpolation::Step;
        let nf = seq.num_frames.max(1);
        let bones = (0..skel.num_bones())
            .map(|b| {
                let mut fb = FnBone { name: skel.bone_names[b].clone(), retarget: rts[b].mode, retarget_source: rts[b].source, pos: None, rot: None, scale: None };
                if let Some(t) = seq.find_track(b) {
                    let samples: Vec<_> = (0..nf).map(|f| t.sample(seq.num_frames, f as f32, step)).collect();
                    if !t.pos.is_empty() {
                        fb.pos = Some(collapse(samples.iter().map(|s| s.pos.unwrap()).collect()));
                    }
                    if !t.rot.is_empty() {
                        fb.rot = Some(collapse(samples.iter().map(|s| s.rot.unwrap()).collect()));
                    }
                    if !t.scale.is_empty() {
                        fb.scale = Some(collapse(samples.iter().map(|s| s.scale.unwrap()).collect()));
                    }
                }
                fb
            })
            .collect();
        FnAnim {
            name: seq.name.clone(),
            skeleton: seq.skeleton_name.clone().unwrap_or_default(),
            retarget_source: seq.retarget_source.clone(),
            ref_pose_seq: seq.additive.ref_pose_seq.clone().unwrap_or_default(),
            num_frames: seq.num_frames,
            length: seq.length,
            rate_scale: seq.rate_scale,
            frame_rate: seq.frame_rate(),
            additive: seq.is_additive(),
            additive_kind: match seq.additive.kind {
                AdditiveAnimType::None => 0,
                AdditiveAnimType::LocalSpaceBase => 1,
                AdditiveAnimType::RotationOffsetMeshSpace => 2,
            },
            ref_pose_type: match seq.additive.base {
                RefPoseType::None => 0,
                RefPoseType::RefPose => 1,
                RefPoseType::AnimScaled => 2,
                RefPoseType::AnimFrame => 3,
            },
            ref_frame_index: seq.additive.ref_frame_index,
            retarget_base: explicit,
            root_motion: seq.enable_root_motion,
            step,
            bones,
            notifies: seq.notifies.clone(),
            sync_markers: seq.sync_markers.clone(),
            curves: seq.curves.clone(),
        }
    }

    pub fn write(&self, out: &mut Vec<u8>, precision: Precision) {
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        let flags = (self.additive as u16) | ((self.retarget_base as u16) << 1) | ((self.root_motion as u16) << 2) | ((self.step as u16) << 3);
        out.extend_from_slice(&flags.to_le_bytes());
        put_u32(out, self.num_frames as u32);
        put_u32(out, self.bones.len() as u32);
        put_f32(out, self.length);
        put_f32(out, self.rate_scale);
        put_f32(out, self.frame_rate);
        out.push(self.additive_kind);
        out.push(self.ref_pose_type);
        out.push(if precision == Precision::F32 { 0 } else { 1 });
        out.push(0);
        put_u32(out, self.ref_frame_index as u32);
        put_str(out, &self.name);
        put_str(out, &self.skeleton);
        put_str(out, &self.retarget_source);
        put_str(out, &self.ref_pose_seq);
        for b in &self.bones {
            put_str(out, &b.name);
            out.push(b.pos.is_some() as u8 | (b.rot.is_some() as u8) << 1 | (b.scale.is_some() as u8) << 2);
            out.push(b.retarget as u8);
            for v in b.retarget_source {
                put_f32(out, v);
            }
        }
        for b in &self.bones {
            if let Some(k) = &b.pos {
                write_channel(out, k, precision);
            }
            if let Some(k) = &b.rot {
                write_channel(out, k, precision);
            }
            if let Some(k) = &b.scale {
                write_channel(out, k, precision);
            }
        }
        put_u32(out, self.notifies.len() as u32);
        for n in &self.notifies {
            put_str(out, &n.notify_name);
            put_str(out, n.notify_class.as_deref().unwrap_or(""));
            put_f32(out, n.trigger_time());
            put_f32(out, n.duration);
            put_f32(out, n.notify_trigger_chance);
            put_u32(out, n.track_index as u32);
        }
        put_u32(out, self.sync_markers.len() as u32);
        for m in &self.sync_markers {
            put_str(out, &m.name);
            put_f32(out, m.time);
        }
        put_u32(out, self.curves.len() as u32);
        for c in &self.curves {
            put_str(out, &c.name);
            put_u32(out, c.flags as u32);
            put_f32(out, c.default_value);
            put_u32(out, c.keys.len() as u32);
            for k in &c.keys {
                put_f32(out, k.time);
                put_f32(out, k.value);
                put_f32(out, k.arrive_tangent);
                put_f32(out, k.leave_tangent);
                out.push(k.interp_mode);
            }
        }
        out.extend_from_slice(b"ENDF");
    }

    pub fn read(data: &[u8]) -> Result<FnAnim> {
        let mut c = Cur { d: data, p: 0 };
        if c.take(4)? != MAGIC {
            return err("fnanim: bad magic");
        }
        let version = c.u16()?;
        if version != VERSION {
            return err(format!("fnanim: unsupported version {version}"));
        }
        let flags = c.u16()?;
        let num_frames = c.u32()? as usize;
        let num_bones = c.u32()? as usize;
        let length = c.f32()?;
        let rate_scale = c.f32()?;
        let frame_rate = c.f32()?;
        let additive_kind = c.u8()?;
        let ref_pose_type = c.u8()?;
        let precision = match c.u8()? {
            0 => Precision::F32,
            1 => Precision::Q16,
            p => return err(format!("fnanim: bad precision {p}")),
        };
        c.u8()?;
        let ref_frame_index = c.i32()?;
        let name = c.str()?;
        let skeleton = c.str()?;
        let retarget_source = c.str()?;
        let ref_pose_seq = c.str()?;
        if num_bones > 100_000 {
            return err("fnanim: implausible bone count");
        }
        let mut bones = Vec::with_capacity(num_bones);
        let mut chan = Vec::with_capacity(num_bones);
        for _ in 0..num_bones {
            let name = c.str()?;
            let ch = c.u8()?;
            let mode = TranslationRetarget::from_u8(c.u8()?);
            let src = [c.f32()?, c.f32()?, c.f32()?];
            chan.push(ch);
            bones.push(FnBone { name, retarget: mode, retarget_source: src, pos: None, rot: None, scale: None });
        }
        for (b, ch) in bones.iter_mut().zip(chan) {
            if ch & 1 != 0 {
                b.pos = Some(read_channel::<3>(&mut c, precision, num_frames)?);
            }
            if ch & 2 != 0 {
                b.rot = Some(read_channel::<4>(&mut c, precision, num_frames)?);
            }
            if ch & 4 != 0 {
                b.scale = Some(read_channel::<3>(&mut c, precision, num_frames)?);
            }
        }
        let n = c.u32()? as usize;
        let mut notifies = Vec::new();
        for _ in 0..n {
            let nm = c.str()?;
            let cls = c.str()?;
            let t = c.f32()?;
            let dur = c.f32()?;
            let chance = c.f32()?;
            let track = c.i32()?;
            notifies.push(Notify { notify_name: nm, notify_class: if cls.is_empty() { None } else { Some(cls) }, link_value: t, duration: dur, notify_trigger_chance: chance, track_index: track, ..Default::default() });
        }
        let n = c.u32()? as usize;
        let mut sync_markers = Vec::new();
        for _ in 0..n {
            sync_markers.push(SyncMarker { name: c.str()?, time: c.f32()? });
        }
        let n = c.u32()? as usize;
        let mut curves = Vec::new();
        for _ in 0..n {
            let name = c.str()?;
            let flags = c.i32()?;
            let default_value = c.f32()?;
            let nk = c.u32()? as usize;
            let mut keys = Vec::new();
            for _ in 0..nk {
                keys.push(CurveKey { time: c.f32()?, value: c.f32()?, arrive_tangent: c.f32()?, leave_tangent: c.f32()?, interp_mode: c.u8()? });
            }
            curves.push(FloatCurve { name, flags, default_value, keys });
        }
        if c.take(4)? != b"ENDF" {
            return err("fnanim: bad trailer");
        }
        Ok(FnAnim {
            name,
            skeleton,
            retarget_source,
            ref_pose_seq,
            num_frames,
            length,
            rate_scale,
            frame_rate,
            additive: flags & 1 != 0,
            additive_kind,
            ref_pose_type,
            ref_frame_index,
            retarget_base: flags & 2 != 0,
            root_motion: flags & 4 != 0,
            step: flags & 8 != 0,
            bones,
            notifies,
            sync_markers,
            curves,
        })
    }

    /// Local pose at a (fractional) frame. Bones/channels without data take `defaults` (the skeleton reference pose;
    /// identity for additive clips - pass an identity slice then).
    pub fn pose_at_frame(&self, frame: f32, defaults: &[BoneTransform]) -> Vec<BoneTransform> {
        let nf = self.num_frames.max(1);
        let f = frame.clamp(0.0, (nf - 1) as f32);
        let (i0, i1, a) = if nf <= 1 {
            (0, 0, 0.0)
        } else {
            let i0 = (f.floor() as usize).min(nf - 1);
            if i0 >= nf - 1 {
                (nf - 1, nf - 1, 0.0)
            } else {
                (i0, i0 + 1, if self.step { 0.0 } else { f - i0 as f32 })
            }
        };
        self.bones
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let mut t = defaults.get(i).copied().unwrap_or_default();
                let pick3 = |k: &Vec<[f32; 3]>| {
                    let (x, y) = (k[i0.min(k.len() - 1)], k[i1.min(k.len() - 1)]);
                    if a > 0.0 && k.len() > 1 {
                        [x[0] + (y[0] - x[0]) * a, x[1] + (y[1] - x[1]) * a, x[2] + (y[2] - x[2]) * a]
                    } else {
                        x
                    }
                };
                if let Some(k) = &b.pos {
                    t.translation = pick3(k);
                }
                if let Some(k) = &b.scale {
                    t.scale = pick3(k);
                }
                if let Some(k) = &b.rot {
                    let (x, y) = (k[i0.min(k.len() - 1)], k[i1.min(k.len() - 1)]);
                    t.rotation = if a > 0.0 && k.len() > 1 { nlerp(x, y, a) } else { x };
                }
                t
            })
            .collect()
    }

    /// Pose at `time` seconds (clamped), using UE's `(NumFrames - 1) / length` key spacing.
    pub fn pose_at(&self, time: f32, defaults: &[BoneTransform]) -> Vec<BoneTransform> {
        let f = if self.num_frames > 1 && self.length > 0.0 { time / self.length * (self.num_frames - 1) as f32 } else { 0.0 };
        self.pose_at_frame(f, defaults)
    }

    /// All frames: `[frame][bone]`.
    pub fn bake_frames(&self, defaults: &[BoneTransform]) -> Vec<Vec<BoneTransform>> {
        (0..self.num_frames).map(|f| self.pose_at_frame(f as f32, defaults)).collect()
    }
}

impl AnimSequence {
    /// Serialise this sequence as `.fnanim` (see the [module docs](crate::fnanim)) for `skel`.
    pub fn write_fnanim(&self, skel: &SkeletonInfo, precision: Precision, out: &mut Vec<u8>) {
        FnAnim::from_sequence(self, skel).write(out, precision);
    }

    /// Parse a `.fnanim` file.
    pub fn read_fnanim(data: &[u8]) -> Result<FnAnim> {
        FnAnim::read(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FnAnim {
        let n = 5;
        FnAnim {
            name: "Clip".into(),
            skeleton: "Skel".into(),
            retarget_source: "None".into(),
            ref_pose_seq: String::new(),
            num_frames: n,
            length: 1.0,
            rate_scale: 1.0,
            frame_rate: 5.0,
            additive: false,
            additive_kind: 0,
            ref_pose_type: 0,
            ref_frame_index: 0,
            retarget_base: true,
            root_motion: false,
            step: false,
            bones: vec![
                FnBone {
                    name: "root".into(),
                    retarget: TranslationRetarget::Animation,
                    retarget_source: [0.0; 3],
                    pos: Some((0..n).map(|i| [i as f32 * 10.0, 5.0, -(i as f32)]).collect()),
                    rot: Some(vec![[0.0, 0.0, 0.0, 1.0]]),
                    scale: None,
                },
                FnBone { name: "arm".into(), retarget: TranslationRetarget::AnimationScaled, retarget_source: [1.0, 2.0, 3.0], pos: None, rot: Some((0..n).map(|i| { let a = i as f32 * 0.1; [0.0, a.sin(), 0.0, a.cos()] }).collect()), scale: Some(vec![[1.0, 1.0, 2.0]]) },
            ],
            notifies: vec![Notify { notify_name: "Step".into(), notify_class: Some("AnimNotify_FootStep_Left_C".into()), link_value: 0.25, duration: 0.0, notify_trigger_chance: 1.0, track_index: 2, ..Default::default() }],
            sync_markers: vec![SyncMarker { name: "LeftPlant".into(), time: 0.5 }],
            curves: vec![FloatCurve { name: "Alpha".into(), flags: 4, default_value: 0.0, keys: vec![CurveKey { time: 0.0, value: 1.0, arrive_tangent: 0.0, leave_tangent: 0.5, interp_mode: 2 }] }],
        }
    }

    #[test]
    fn lossless_roundtrip() {
        let a = sample();
        let mut buf = Vec::new();
        a.write(&mut buf, Precision::F32);
        assert_eq!(&buf[..4], b"FNAN");
        let b = FnAnim::read(&buf).unwrap();
        assert_eq!(a, b);
        // poses
        let d = vec![BoneTransform::IDENTITY; 2];
        assert_eq!(a.bake_frames(&d), b.bake_frames(&d));
    }

    #[test]
    fn q16_roundtrip_within_tolerance() {
        let a = sample();
        let mut buf = Vec::new();
        a.write(&mut buf, Precision::Q16);
        let b = FnAnim::read(&buf).unwrap();
        let d = vec![BoneTransform::IDENTITY; 2];
        let (fa, fb) = (a.bake_frames(&d), b.bake_frames(&d));
        for (x, y) in fa.iter().zip(&fb) {
            for (p, q) in x.iter().zip(y) {
                for k in 0..3 {
                    assert!((p.translation[k] - q.translation[k]).abs() <= 40.0 / 131070.0 + 1e-5);
                }
                for k in 0..4 {
                    assert!((p.rotation[k] - q.rotation[k]).abs() < 2e-5, "{:?} {:?}", p.rotation, q.rotation);
                }
            }
        }
        // constant channels stay exact
        assert_eq!(b.bones[0].rot, a.bones[0].rot);
    }

    #[test]
    fn truncated_and_bad_input_rejected() {
        let a = sample();
        let mut buf = Vec::new();
        a.write(&mut buf, Precision::F32);
        assert!(FnAnim::read(&buf[..buf.len() - 1]).is_err());
        assert!(FnAnim::read(&buf[..20]).is_err());
        let mut bad = buf.clone();
        bad[0] = b'X';
        assert!(FnAnim::read(&bad).is_err());
        let mut bad = buf.clone();
        bad[4] = 9;
        assert!(FnAnim::read(&bad).is_err());
    }

    #[test]
    fn fractional_pose_and_defaults() {
        let a = sample();
        let mut d = vec![BoneTransform::IDENTITY; 2];
        d[1].translation = [7.0, 8.0, 9.0];
        let p = a.pose_at_frame(1.5, &d);
        assert_eq!(p[0].translation, [15.0, 5.0, -1.5]);
        assert_eq!(p[1].translation, [7.0, 8.0, 9.0]); // no position channel -> default
        assert_eq!(p[1].scale, [1.0, 1.0, 2.0]);
        let q = a.pose_at(0.5, &d); // length 1, 5 frames -> frame 2
        assert_eq!(q[0].translation, [20.0, 5.0, -2.0]);
    }
}
