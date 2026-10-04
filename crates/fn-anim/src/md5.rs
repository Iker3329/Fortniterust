//! Reproduction of the old UEViewer md5anim export (`ExportMd5.cpp`, fork modified for Fortnite 1.8), so the old
//! consumer (`FortniteRuntime/src/assets.rs: Clip::load`) can read what this crate produces and so the decoder can be
//! verified numerically against the 388 existing exports.
//!
//! ## What an md5anim "frame" is (derived from `ExportMd5.cpp`, `UnAnim4.cpp`, `SkeletalMesh.cpp`)
//!
//! * One joint per **skeleton bone** (266 for `Fortnite_M_Avg_Player_Skeleton`), in skeleton order, *not* per track.
//!   Bones without a track (or channels without keys) are exported as position `(0,0,0)`, rotation identity -
//!   *not* as the reference pose.
//! * One frame per animation key frame (`NumFrames`), at integer frame indices `0..NumFrames-1`
//!   (`GetBonePosition(Frame, NumFrames, Loop=false)`: UEViewer's own key lookup, `Frame == 0` returns the first key).
//!   `frameRate = NumFrames / SequenceLength * RateScale` (**not** `(NumFrames-1)/SequenceLength` as UE samples).
//! * Position (cm): UE `(x, y, z)` is exported as `(x, -y, z)` (`MIRROR_MESH`) after multiplying by the accumulated
//!   ancestor `Scale3D` of the retarget pose / reference pose (`BAKE_BONE_SCALES`, only when the bone count matches).
//! * Rotation: non-root bones' keys are conjugated at load (`FixRotationKeys`), the root is conjugated at export;
//!   then `y` and `w` are negated (mirror), the quaternion is negated if `w < 0`, and only `x y z` are written
//!   (`%.10f`). For `w > 0` this amounts to `(x, -y, z)` of the stored UE quaternion.
//! * `baseframe` = frame 0. `bounds` are dummy `(-100..100)` boxes.
//! * Extra comment lines (consumed by the old runtime): `// additive 0|1`, `// retarget-base 0|1`, and for every bone
//!   `// track "bone" <hasPos> <hasRot>` and `// retarget "bone" <mode> x y z` where `(x, y, z)` is the translation
//!   of the bone in the retarget base pose (or the skeleton reference pose when the sequence has none) in metres
//!   with axes swapped to `(x, z, y)`; the mode is `EBoneTranslationRetargetingMode` (bone 0 is always 0).
//! * Text is written with CRLF line endings and C `printf` number formatting (`%f`, `%.10f`, `%g`).

use crate::retarget;
use crate::sequence::{slerp, AnimSequence, Track};
use crate::skeleton::SkeletonInfo;
use crate::{err, Error, Result};
use std::io::Write;

/// One exported joint transform: position (cm, md5 axes) and quaternion xyz (w dropped).
pub type Joint = ([f32; 3], [f32; 3]);

/// In-memory form of an md5anim file.
#[derive(Debug, Clone, Default)]
pub struct Md5Anim {
    pub num_frames: usize,
    pub frame_rate: f32,
    pub bone_names: Vec<String>,
    pub additive: bool,
    pub retarget_base: bool,
    /// `(has position keys, has rotation keys)` per bone.
    pub track_flags: Vec<(bool, bool)>,
    /// `(mode, x, y, z)` per bone (metres, `(x, z, y)` order as written), `None` when the line is absent.
    pub retarget: Vec<Option<(u8, [f32; 3])>>,
    pub base_frame: Vec<Joint>,
    /// `[frame][bone]`
    pub frames: Vec<Vec<Joint>>,
}

// ---- UEViewer-compatible sampling ---------------------------------------------------------------------------------

/// UEViewer `FindTimeKey` over frame-index keys.
fn find_time_key(times: &[u32], frame: f32) -> usize {
    let n = times.len();
    let (mut low, mut high) = (0i64, n as i64 - 1);
    while low + 4 < high {
        let mid = (low + high) / 2;
        if frame < times[mid as usize] as f32 {
            high = mid - 1;
        } else {
            low = mid;
        }
    }
    let mut i = low;
    while i <= high {
        let c = times[i as usize] as f32;
        if frame == c {
            return i as usize;
        }
        if frame < c {
            return if i > 0 { (i - 1) as usize } else { 0 };
        }
        i += 1;
    }
    if i > high {
        i = high;
    }
    i.max(0) as usize
}

/// UEViewer `GetKeyParams` (non-looping): `(x, y, f)`.
fn key_params(times: &[u32], frame: f32) -> (usize, usize, f32) {
    let x = find_time_key(times, frame);
    let mut y = x + 1;
    if y >= times.len() {
        y = times.len() - 1;
        (x, y, 0.0)
    } else {
        (x, y, (frame - times[x] as f32) / (times[y] as f32 - times[x] as f32))
    }
}

/// Channel keys of a UEViewer `CAnimTrack` (already converted).
struct UevTrack {
    pos: Vec<[f32; 3]>,
    pos_times: Vec<u32>,
    quat: Vec<[f32; 4]>,
    quat_times: Vec<u32>,
}

/// UEViewer `CAnimTrack::GetBonePosition(Frame, NumFrames, Loop = false)`; leaves `dst_*` untouched for empty channels.
fn uev_get_bone_position(t: &UevTrack, frame: f32, num_frames: f32, dst_pos: &mut [f32; 3], dst_quat: &mut [f32; 4]) {
    if num_frames == 1.0 || frame == 0.0 {
        if let Some(p) = t.pos.first() {
            *dst_pos = *p;
        }
        if let Some(q) = t.quat.first() {
            *dst_quat = *q;
        }
        return;
    }
    let n_pos = t.pos.len();
    let n_rot = t.quat.len();
    let (px, py, pf) = if !t.pos_times.is_empty() {
        key_params(&t.pos_times, frame)
    } else if n_pos > 1 {
        let position = frame / num_frames * n_pos as f32;
        let px = position.floor() as usize;
        let mut py = px + 1;
        let mut pf = position - px as f32;
        if py >= n_pos {
            py = n_pos - 1;
            pf = 0.0;
        }
        (px, py, pf)
    } else {
        (0, 0, 0.0)
    };
    let (rx, ry, rf) = if !t.quat_times.is_empty() {
        key_params(&t.quat_times, frame)
    } else if n_rot > 1 {
        let position = frame / num_frames * n_rot as f32;
        let rx = position.floor() as usize;
        let mut ry = rx + 1;
        let mut rf = position - rx as f32;
        if ry >= n_rot {
            ry = n_rot - 1;
            rf = 0.0;
        }
        (rx, ry, rf)
    } else {
        (0, 0, 0.0)
    };
    if pf > 0.0 {
        let (a, b) = (t.pos[px], t.pos[py]);
        *dst_pos = [a[0] + pf * (b[0] - a[0]), a[1] + pf * (b[1] - a[1]), a[2] + pf * (b[2] - a[2])];
    } else if n_pos > 0 {
        *dst_pos = t.pos[px];
    }
    if rf > 0.0 {
        *dst_quat = slerp(t.quat[rx], t.quat[ry], rf);
    } else if n_rot > 0 {
        *dst_quat = t.quat[rx];
    }
}

fn to_uev_track(t: &Track, bone: usize, bake_scale: Option<[f32; 3]>, per_track: bool) -> UevTrack {
    let mut pos = t.pos.clone();
    if per_track && pos.is_empty() {
        pos.push([0.0; 3]); // PerTrackCompression offset -1: UEViewer adds a null key
    }
    if let Some(s) = bake_scale {
        if (s[0] - 1.0).abs() > 0.001 || (s[1] - 1.0).abs() > 0.001 || (s[2] - 1.0).abs() > 0.001 {
            for p in pos.iter_mut() {
                p[0] *= s[0];
                p[1] *= s[1];
                p[2] *= s[2];
            }
        }
    }
    // FixRotationKeys: conjugate every track except bone/track 0
    let mut rot = t.rot.clone();
    if per_track && rot.is_empty() {
        rot.push([0.0, 0.0, 0.0, 1.0]);
    }
    let quat: Vec<[f32; 4]> = rot.iter().map(|q| if bone == 0 { *q } else { [-q[0], -q[1], -q[2], q[3]] }).collect();
    UevTrack { pos, pos_times: t.pos_times.clone(), quat, quat_times: t.rot_times.clone() }
}

fn joint_of(t: Option<&UevTrack>, frame: f32, num_frames: f32, bone: usize) -> Joint {
    let mut bp = [0.0f32; 3];
    let mut bo = [0.0f32, 0.0, 0.0, 1.0];
    if let Some(t) = t {
        uev_get_bone_position(t, frame, num_frames, &mut bp, &mut bo);
    }
    if bone == 0 {
        bo = [-bo[0], -bo[1], -bo[2], bo[3]];
    }
    bo[1] *= -1.0;
    bo[3] *= -1.0;
    bp[1] *= -1.0;
    if bo[3] < 0.0 {
        bo = [-bo[0], -bo[1], -bo[2], -bo[3]];
    }
    (bp, [bo[0], bo[1], bo[2]])
}

/// Build what the UEViewer exporter would have written for `seq` on `skel`.
pub fn export(seq: &AnimSequence, skel: &SkeletonInfo) -> Md5Anim {
    let n = skel.num_bones();
    let (retarget_pose, explicit_base) = retarget::source_pose(seq, skel);
    // bone-scale baking uses the retarget transforms if present else the reference pose; both must match the bone count
    let scale_src: Option<&[crate::skeleton::BoneTransform]> = {
        let raw: &[crate::skeleton::BoneTransform] = if explicit_base {
            if seq.retarget_source == "None" {
                &seq.retarget_source_asset_pose
            } else {
                skel.find_retarget_source(&seq.retarget_source).map(|s| s.pose.as_slice()).unwrap_or(&[])
            }
        } else {
            &skel.ref_pose
        };
        if raw.len() == n {
            Some(raw)
        } else {
            None
        }
    };
    let per_track = seq.key_format == crate::codec::KeyFormat::PerTrackCompression;
    let mut uev: Vec<Option<UevTrack>> = Vec::with_capacity(n);
    let mut flags = Vec::with_capacity(n);
    for b in 0..n {
        match seq.find_track(b) {
            Some(t) => {
                let scale = scale_src.map(|p| skel.ancestor_scale(p, b));
                let u = to_uev_track(t, b, scale, per_track);
                flags.push((!u.pos.is_empty(), !u.quat.is_empty()));
                uev.push(Some(u));
            }
            None => {
                flags.push((false, false));
                uev.push(None);
            }
        }
    }
    let rts = retarget::bone_retargets(seq, skel);
    let retarget_lines: Vec<Option<(u8, [f32; 3])>> = (0..n)
        .map(|i| {
            retarget_pose.get(i).map(|p| {
                let t = p.translation;
                (rts[i].mode as u8, [t[0] * 0.01f32, t[2] * 0.01f32, t[1] * 0.01f32])
            })
        })
        .collect();
    let nf = seq.num_frames;
    let frames: Vec<Vec<Joint>> = (0..nf).map(|f| (0..n).map(|b| joint_of(uev[b].as_ref(), f as f32, nf as f32, b)).collect()).collect();
    let base_frame = if nf > 0 { frames[0].clone() } else { (0..n).map(|b| joint_of(uev[b].as_ref(), 0.0, nf as f32, b)).collect() };
    Md5Anim {
        num_frames: nf,
        frame_rate: if seq.length > 0.0 { nf as f32 / seq.length * seq.rate_scale } else { 0.0 },
        bone_names: skel.bone_names.clone(),
        additive: seq.is_additive(),
        retarget_base: explicit_base,
        track_flags: flags,
        retarget: retarget_lines,
        base_frame,
        frames,
    }
}

// ---- C printf compatible number formatting -------------------------------------------------------------------------

/// `printf("%g", v)` (6 significant digits).
pub fn fmt_g(v: f64) -> String {
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    if !v.is_finite() {
        return if v.is_nan() { "nan".into() } else if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let e = format!("{:.5e}", v);
    let (mant, exp) = e.split_once('e').unwrap();
    let x: i32 = exp.parse().unwrap();
    if x < -4 || x >= 6 {
        let mant = trim_zeros(mant);
        format!("{}e{}{:02}", mant, if x < 0 { '-' } else { '+' }, x.abs())
    } else {
        let prec = (5 - x).max(0) as usize;
        trim_zeros(&format!("{:.*}", prec, v))
    }
}

fn trim_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// Serialise like `ExportMd5Anim`.
pub fn write(m: &Md5Anim, w: &mut impl Write) -> std::io::Result<()> {
    let n = m.bone_names.len();
    let mut s = String::with_capacity(m.num_frames * n * 60 + 4096);
    let nl = "\r\n";
    s.push_str(&format!(
        "MD5Version 10{nl}commandline \"Created with UE Viewer\"{nl}{nl}numFrames {}{nl}numJoints {}{nl}frameRate {}{nl}numAnimatedComponents {}{nl}{nl}",
        m.num_frames,
        n,
        fmt_g(m.frame_rate as f64),
        n * 6
    ));
    s.push_str(&format!("// additive {}{nl}", m.additive as i32));
    s.push_str(&format!("// retarget-base {}{nl}", m.retarget_base as i32));
    for b in 0..n {
        let (hp, hr) = m.track_flags[b];
        s.push_str(&format!("// track \"{}\" {} {}{nl}", m.bone_names[b], hp as i32, hr as i32));
        if let Some(Some((mode, p))) = m.retarget.get(b) {
            s.push_str(&format!("// retarget \"{}\" {} {} {} {}{nl}", m.bone_names[b], mode, fmt_g(p[0] as f64), fmt_g(p[1] as f64), fmt_g(p[2] as f64)));
        }
    }
    s.push_str(&format!("hierarchy {{{nl}"));
    for (i, name) in m.bone_names.iter().enumerate() {
        s.push_str(&format!("\t\"{}\" {} {} {}{nl}", name, if i == 0 { -1 } else { 0 }, 63, i * 6));
    }
    s.push_str(&format!("}}{nl}{nl}bounds {{{nl}"));
    for _ in 0..m.num_frames {
        s.push_str(&format!("\t( -100 -100 -100 ) ( 100 100 100 ){nl}"));
    }
    s.push_str(&format!("}}{nl}{nl}"));
    s.push_str(&format!("baseframe {{{nl}"));
    for (p, q) in &m.base_frame {
        s.push_str(&format!("\t( {:.6} {:.6} {:.6} ) ( {:.10} {:.10} {:.10} ){nl}", p[0] as f64, p[1] as f64, p[2] as f64, q[0] as f64, q[1] as f64, q[2] as f64));
    }
    s.push_str(&format!("}}{nl}{nl}"));
    for (fi, f) in m.frames.iter().enumerate() {
        s.push_str(&format!("frame {fi} {{{nl}"));
        for (p, q) in f {
            s.push_str(&format!("\t{:.6} {:.6} {:.6} {:.10} {:.10} {:.10}{nl}", p[0] as f64, p[1] as f64, p[2] as f64, q[0] as f64, q[1] as f64, q[2] as f64));
        }
        s.push_str(&format!("}}{nl}{nl}"));
    }
    w.write_all(s.as_bytes())
}

/// `fn_anim::md5::write_md5anim`: export `seq` on `skel` as md5anim text.
pub fn write_md5anim(seq: &AnimSequence, skel: &SkeletonInfo, w: &mut impl Write) -> std::io::Result<()> {
    write(&export(seq, skel), w)
}

/// Parse an md5anim text (old or new). Numbers are kept exactly as written (no conversion).
pub fn parse_md5anim(text: &str) -> Result<Md5Anim> {
    let mut m = Md5Anim::default();
    let mut section = "";
    let mut cur: Vec<Joint> = Vec::new();
    let mut in_base = false;
    let mut in_frame = false;
    let mut retarget: std::collections::HashMap<String, (u8, [f32; 3])> = Default::default();
    let mut flags: std::collections::HashMap<String, (bool, bool)> = Default::default();
    let bad = |l: &str| Error::Format(format!("malformed md5anim line: {l}"));
    for l in text.lines().map(str::trim) {
        if l == "// additive 1" {
            m.additive = true;
        }
        if l == "// retarget-base 1" {
            m.retarget_base = true;
        }
        if let Some(rest) = l.strip_prefix("// track \"") {
            let (name, rest) = rest.split_once('"').ok_or_else(|| bad(l))?;
            let v: Vec<&str> = rest.split_whitespace().collect();
            if v.len() < 2 {
                return Err(bad(l));
            }
            flags.insert(name.to_string(), (v[0] == "1", v[1] == "1"));
        }
        if let Some(rest) = l.strip_prefix("// retarget \"") {
            let (name, rest) = rest.split_once('"').ok_or_else(|| bad(l))?;
            let v: Vec<&str> = rest.split_whitespace().collect();
            if v.len() < 4 {
                return Err(bad(l));
            }
            let f = |i: usize| v[i].parse::<f32>().map_err(|_| bad(l));
            retarget.insert(name.to_string(), (v[0].parse::<u8>().map_err(|_| bad(l))?, [f(1)?, f(2)?, f(3)?]));
        }
        if let Some(r) = l.strip_prefix("numFrames ") {
            m.num_frames = r.trim().parse().map_err(|_| bad(l))?;
        }
        if let Some(r) = l.strip_prefix("frameRate ") {
            m.frame_rate = r.trim().parse().map_err(|_| bad(l))?;
        }
        if l == "hierarchy {" {
            section = "hierarchy";
            continue;
        }
        if l == "bounds {" {
            section = "bounds";
            continue;
        }
        if l == "baseframe {" {
            in_base = true;
            cur.clear();
            continue;
        }
        if l.starts_with("frame ") && l.ends_with('{') {
            in_frame = true;
            cur.clear();
            continue;
        }
        if l == "}" {
            section = "";
            if in_base {
                m.base_frame = std::mem::take(&mut cur);
                in_base = false;
            } else if in_frame {
                m.frames.push(std::mem::take(&mut cur));
                in_frame = false;
            }
            continue;
        }
        if section == "hierarchy" && l.starts_with('"') {
            m.bone_names.push(l.split('"').nth(1).ok_or_else(|| bad(l))?.to_string());
        }
        if in_base || in_frame {
            let cleaned: String = l.chars().map(|c| if c == '(' || c == ')' { ' ' } else { c }).collect();
            let v: Vec<f32> = cleaned.split_whitespace().map(|x| x.parse::<f32>().map_err(|_| bad(l))).collect::<Result<_>>()?;
            if v.len() != 6 {
                return Err(bad(l));
            }
            cur.push(([v[0], v[1], v[2]], [v[3], v[4], v[5]]));
        }
    }
    if m.frames.is_empty() {
        return err("md5anim has no frames");
    }
    m.track_flags = m.bone_names.iter().map(|n| flags.get(n).copied().unwrap_or((true, true))).collect();
    m.retarget = m.bone_names.iter().map(|n| retarget.get(n).copied()).collect();
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printf_g() {
        assert_eq!(fmt_g(31.25), "31.25");
        assert_eq!(fmt_g(0.0), "0");
        assert_eq!(fmt_g(1.24006e-16), "1.24006e-16");
        assert_eq!(fmt_g(0.931191), "0.931191");
        assert_eq!(fmt_g(-5.30405e-15), "-5.30405e-15");
        assert_eq!(fmt_g(0.0674937), "0.0674937");
        assert_eq!(fmt_g(100000.0), "100000");
        assert_eq!(fmt_g(1234567.0), "1.23457e+06");
        assert_eq!(fmt_g(0.0001), "0.0001");
        assert_eq!(fmt_g(0.00001), "1e-05");
        assert_eq!(fmt_g(30.0), "30");
    }

    #[test]
    fn slerp_matches_endpoints_and_midpoint() {
        let a = [0.0, 0.0, 0.0, 1.0];
        let s = (0.5f32).sqrt();
        let b = [0.0, 0.0, s, s]; // 90 degrees about z
        let m = slerp(a, b, 0.5);
        let half = (std::f32::consts::FRAC_PI_4 / 2.0).sin();
        assert!((m[2] - half).abs() < 1e-5);
        assert_eq!(slerp(a, b, 0.0), a);
        assert_eq!(slerp(a, b, 1.0), b);
    }

    #[test]
    fn time_key_lookup_matches_uev() {
        let t = [0u32, 3, 7, 12, 20, 30, 31, 40];
        assert_eq!(find_time_key(&t, 0.0), 0);
        assert_eq!(find_time_key(&t, 5.0), 1);
        assert_eq!(find_time_key(&t, 12.0), 3);
        assert_eq!(find_time_key(&t, 39.9), 6);
        assert_eq!(find_time_key(&t, 100.0), 7);
        let (x, y, f) = key_params(&t, 5.0);
        assert_eq!((x, y), (1, 2));
        assert!((f - 0.5).abs() < 1e-6);
    }

    #[test]
    fn joint_conversion_flips_y_and_conjugates() {
        // stored UE quat (x,y,z,w); non-root export = (x, -y, z)
        let w = (1.0f32 - 0.14).sqrt();
        let tr = Track { bone: 5, pos: vec![[1.0, 2.0, 3.0]], rot: vec![[0.1, -0.2, 0.3, w]], ..Default::default() };
        for bone in [0usize, 5] {
            let t = to_uev_track(&tr, bone, None, false);
            let (p, q) = joint_of(Some(&t), 0.0, 1.0, bone);
            assert_eq!(p, [1.0, -2.0, 3.0]);
            assert!((q[0] - 0.1).abs() < 1e-6 && (q[1] - 0.2).abs() < 1e-6 && (q[2] - 0.3).abs() < 1e-6, "{q:?}");
        }
        // no track: identity at export (-0.0 artefacts aside)
        let (p, q) = joint_of(None, 0.0, 1.0, 3);
        assert_eq!(p, [0.0; 3]);
        assert_eq!(q, [0.0; 3]);
    }

    #[test]
    fn write_parse_roundtrip() {
        let m = Md5Anim {
            num_frames: 2,
            frame_rate: 30.0,
            bone_names: vec!["Root".into(), "pelvis".into()],
            additive: true,
            retarget_base: false,
            track_flags: vec![(true, true), (false, true)],
            retarget: vec![Some((0, [0.0; 3])), Some((2, [0.5, 0.25, -1.0]))],
            base_frame: vec![([0.0; 3], [0.0; 3]), ([1.0, 2.0, 3.0], [0.1, 0.2, 0.3])],
            frames: vec![vec![([0.0; 3], [0.0; 3]), ([1.0, 2.0, 3.0], [0.1, 0.2, 0.3])], vec![([0.0; 3], [0.0; 3]), ([1.5, 2.0, 3.0], [0.1, 0.2, 0.3])]],
        };
        let mut buf = Vec::new();
        write(&m, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("\r\n"));
        let p = parse_md5anim(&text).unwrap();
        assert_eq!(p.num_frames, 2);
        assert_eq!(p.bone_names, m.bone_names);
        assert!(p.additive && !p.retarget_base);
        assert_eq!(p.track_flags, m.track_flags);
        assert_eq!(p.retarget[1], Some((2, [0.5, 0.25, -1.0])));
        assert_eq!(p.frames[1][1].0, [1.5, 2.0, 3.0]);
    }
}
