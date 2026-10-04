//! Key decoders of the UE 4.16 animation compression codecs (`AnimEncoding*.cpp`), written after the
//! UEViewer implementation (`UnAnim4.cpp`, `UnMeshTypes.h`) that produced the reference md5anim files.
//!
//! All formats decode to plain `[f32; 3]` vectors / `[f32; 4]` (`x y z w`) quaternions. Rotation formats
//! store only `x y z` (the stored quaternion is "NoW"); `w` is restored as `sqrt(1 - x^2 - y^2 - z^2)` clamped at 0.
//!
//! | `AnimationCompressionFormat` | id | bits/key | rotation decode                                   | translation decode |
//! |------------------------------|----|----------|----------------------------------------------------|--------------------|
//! | `ACF_None`                   | 0  | 96       | like Float96NoW (UEViewer reads 3 floats)           | 3 x f32            |
//! | `ACF_Float96NoW`             | 1  | 96       | 3 x f32                                            | 3 x f32            |
//! | `ACF_Fixed48NoW`             | 2  | 48       | `(u16 - 32767) / 32767`                            | const/var: `(u16 - 32767) * 128/32767`; per-track: `u16 - 255` |
//! | `ACF_IntervalFixed32NoW`     | 3  | 32       | 11/11/10 bit `(v/1023 - 1) * range + min` (z: /511) | 10/11/11 bit (x: /511) `* range + min` |
//! | `ACF_Fixed32NoW`             | 4  | 32       | 11/11/10 bit `v/1023 - 1` (z: /511)                | n/a                |
//! | `ACF_Float32NoW`             | 5  | 32       | 11/11/10 bit mini-float (3 exponent bits, bias 123) | n/a                |
//! | `ACF_Identity`               | 6  | 0        | `(0,0,0,1)`                                        | `(0,0,0)`          |
//!
//! Bit-fields are packed LSB first: rotations `z:10 | y:11 | x:11` (x in the top bits), translations
//! `x:10 | y:11 | z:11` (x in the low bits).

use crate::{err, Error, Result};
use fn_ueio::Reader;

/// `AnimationCompressionFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CompressionFormat {
    None = 0,
    Float96NoW = 1,
    Fixed48NoW = 2,
    IntervalFixed32NoW = 3,
    Fixed32NoW = 4,
    Float32NoW = 5,
    Identity = 6,
}

impl CompressionFormat {
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Self::None,
            1 => Self::Float96NoW,
            2 => Self::Fixed48NoW,
            3 => Self::IntervalFixed32NoW,
            4 => Self::Fixed32NoW,
            5 => Self::Float32NoW,
            6 => Self::Identity,
            _ => return err(format!("unknown animation compression format {v}")),
        })
    }
    pub fn from_name(s: &str) -> Option<Self> {
        let s = s.rsplit("::").next().unwrap_or(s);
        Some(match s {
            "ACF_None" => Self::None,
            "ACF_Float96NoW" => Self::Float96NoW,
            "ACF_Fixed48NoW" => Self::Fixed48NoW,
            "ACF_IntervalFixed32NoW" => Self::IntervalFixed32NoW,
            "ACF_Fixed32NoW" => Self::Fixed32NoW,
            "ACF_Float32NoW" => Self::Float32NoW,
            "ACF_Identity" => Self::Identity,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "ACF_None",
            Self::Float96NoW => "ACF_Float96NoW",
            Self::Fixed48NoW => "ACF_Fixed48NoW",
            Self::IntervalFixed32NoW => "ACF_IntervalFixed32NoW",
            Self::Fixed32NoW => "ACF_Fixed32NoW",
            Self::Float32NoW => "ACF_Float32NoW",
            Self::Identity => "ACF_Identity",
        }
    }
}

/// `AnimationKeyFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyFormat {
    /// Every track has 1 or `NumFrames` keys, evenly spaced.
    ConstantKeyLerp = 0,
    /// Per-track key count, with a table of frame indices per track.
    VariableKeyLerp = 1,
    /// Per-track header (format, component mask, key count, optional time table).
    PerTrackCompression = 2,
}

impl KeyFormat {
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Self::ConstantKeyLerp,
            1 => Self::VariableKeyLerp,
            2 => Self::PerTrackCompression,
            _ => return err(format!("unknown animation key format {v}")),
        })
    }
    pub fn from_name(s: &str) -> Option<Self> {
        let s = s.rsplit("::").next().unwrap_or(s);
        Some(match s {
            "AKF_ConstantKeyLerp" => Self::ConstantKeyLerp,
            "AKF_VariableKeyLerp" => Self::VariableKeyLerp,
            "AKF_PerTrackCompression" => Self::PerTrackCompression,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::ConstantKeyLerp => "AKF_ConstantKeyLerp",
            Self::VariableKeyLerp => "AKF_VariableKeyLerp",
            Self::PerTrackCompression => "AKF_PerTrackCompression",
        }
    }
}

/// `w = sqrt(1 - |xyz|^2)`, 0 when the squared length exceeds 1 (UEViewer `RESTORE_QUAT_W`).
pub fn restore_w(x: f32, y: f32, z: f32) -> f32 {
    let w_sq = 1.0 - (x * x + y * y + z * z);
    if w_sq > 0.0 {
        w_sq.sqrt()
    } else {
        0.0
    }
}

/// `FQuatFixed48NoW`: three u16, `(v - 32767) / 32767`.
pub fn quat_fixed48(x: u16, y: u16, z: u16) -> [f32; 4] {
    let (x, y, z) = ((x as f32 - 32767.0) / 32767.0, (y as f32 - 32767.0) / 32767.0, (z as f32 - 32767.0) / 32767.0);
    [x, y, z, restore_w(x, y, z)]
}

/// `FQuatFixed32NoW`: `z:10 | y:11 | x:11` (LSB first).
pub fn quat_fixed32(v: u32) -> [f32; 4] {
    let (x, y, z) = unpack_rot32(v);
    let (x, y, z) = (x as f32 / 1023.0 - 1.0, y as f32 / 1023.0 - 1.0, z as f32 / 511.0 - 1.0);
    [x, y, z, restore_w(x, y, z)]
}

/// `FQuatIntervalFixed32NoW`: like [`quat_fixed32`] but scaled into `[mins, mins + 2*ranges]` per component.
pub fn quat_interval32(v: u32, mins: [f32; 3], ranges: [f32; 3]) -> [f32; 4] {
    let (x, y, z) = unpack_rot32(v);
    let x = (x as f32 / 1023.0 - 1.0) * ranges[0] + mins[0];
    let y = (y as f32 / 1023.0 - 1.0) * ranges[1] + mins[1];
    let z = (z as f32 / 511.0 - 1.0) * ranges[2] + mins[2];
    [x, y, z, restore_w(x, y, z)]
}

fn unpack_rot32(v: u32) -> (u32, u32, u32) {
    (v >> 21, (v >> 10) & 0x7ff, v & 0x3ff)
}

/// `FQuatFloat32NoW`: 11/11/10 bit mini-floats (3 exponent bits biased by 123, 7/7/6 mantissa bits + sign).
pub fn quat_float32(data: u32) -> [f32; 4] {
    let x = data >> 21;
    let y = (data >> 10) & 0x7ff;
    let z = data & 0x3ff;
    let fx = f32::from_bits((((x >> 7) & 7) + 123) << 23 | (((x & 0x7f) | 32 * (x & 0xffff_fc00)) << 16));
    let fy = f32::from_bits((((y >> 7) & 7) + 123) << 23 | (((y & 0x7f) | 32 * (y & 0xffff_fc00)) << 16));
    let fz = f32::from_bits((((z >> 6) & 7) + 123) << 23 | (((z & 0x3f) | 32 * (z & 0xffff_fe00)) << 17));
    [fx, fy, fz, restore_w(fx, fy, fz)]
}

/// `FVectorIntervalFixed32`: `x:10 | y:11 | z:11` (x in the low bits).
pub fn vec_interval32(v: u32, mins: [f32; 3], ranges: [f32; 3]) -> [f32; 3] {
    let x = v & 0x3ff;
    let y = (v >> 10) & 0x7ff;
    let z = v >> 21;
    [
        (x as f32 / 511.0 - 1.0) * ranges[0] + mins[0],
        (y as f32 / 1023.0 - 1.0) * ranges[1] + mins[1],
        (z as f32 / 1023.0 - 1.0) * ranges[2] + mins[2],
    ]
}

/// `FVectorFixed48` (constant / variable key formats): `(v - 32767) * 128 / 32767`.
pub fn vec_fixed48(x: u16, y: u16, z: u16) -> [f32; 3] {
    let scale = 128.0f32 / 32767.0;
    [(x as f32 - 32767.0) * scale, (y as f32 - 32767.0) * scale, (z as f32 - 32767.0) * scale]
}

/// `DecodeFixed48_PerTrackComponent<7>` (per-track translation / scale): `(v - 255) * 1.0`.
pub fn per_track_fixed48_component(v: u16) -> f32 {
    const LOG2: u32 = 7;
    let offset = (1i32 << (15 - LOG2)) - 1;
    let inv_factor = 1.0f32 / (offset >> LOG2) as f32;
    (v as i32 - offset) as f32 * inv_factor
}

/// Number of stored components for a per-track component mask (`NumComponentsPerMask`).
pub fn components_for_mask(mask: u8) -> usize {
    const N: [usize; 8] = [3, 1, 1, 2, 1, 2, 2, 3];
    N[(mask & 7) as usize]
}

/// Read `Mins`/`Ranges` for an interval format. Full 6 floats when `mask` is `None` (constant / variable key
/// formats), only the masked components for per-track data.
pub fn read_interval(r: &mut Reader, mask: Option<u8>) -> Result<([f32; 3], [f32; 3])> {
    let mut mins = [0.0f32; 3];
    let mut ranges = [0.0f32; 3];
    match mask {
        None => {
            for m in mins.iter_mut() {
                *m = r.f32()?;
            }
            for g in ranges.iter_mut() {
                *g = r.f32()?;
            }
            // `Mins`/`Ranges` are two FVectors: all mins first, then all ranges.
        }
        Some(mask) => {
            for k in 0..3 {
                if mask & (1 << k) != 0 {
                    mins[k] = r.f32()?;
                    ranges[k] = r.f32()?;
                }
            }
        }
    }
    Ok((mins, ranges))
}

/// Decode one rotation key. `mask`: per-track component mask (`None` for constant/variable formats, where all
/// components are stored).
pub fn read_quat_key(r: &mut Reader, fmt: CompressionFormat, mask: Option<u8>, mins: [f32; 3], ranges: [f32; 3]) -> Result<[f32; 4]> {
    Ok(match fmt {
        CompressionFormat::None | CompressionFormat::Float96NoW => {
            let m = mask.map(|m| if m & 7 == 0 { 7 } else { m & 7 }).unwrap_or(7);
            let mut v = [0.0f32; 3];
            for (k, c) in v.iter_mut().enumerate() {
                if m & (1 << k) != 0 {
                    *c = r.f32()?;
                }
            }
            [v[0], v[1], v[2], restore_w(v[0], v[1], v[2])]
        }
        CompressionFormat::Fixed48NoW => {
            let m = mask.map(|m| m & 7).unwrap_or(7);
            let mut v = [32767u16; 3];
            for (k, c) in v.iter_mut().enumerate() {
                if m & (1 << k) != 0 {
                    *c = r.u16()?;
                }
            }
            quat_fixed48(v[0], v[1], v[2])
        }
        CompressionFormat::Fixed32NoW => quat_fixed32(r.u32()?),
        CompressionFormat::IntervalFixed32NoW => quat_interval32(r.u32()?, mins, ranges),
        CompressionFormat::Float32NoW => quat_float32(r.u32()?),
        CompressionFormat::Identity => [0.0, 0.0, 0.0, 1.0],
    })
}

/// Decode one translation / scale key.
pub fn read_vec_key(r: &mut Reader, fmt: CompressionFormat, mask: Option<u8>, mins: [f32; 3], ranges: [f32; 3]) -> Result<[f32; 3]> {
    Ok(match fmt {
        CompressionFormat::None | CompressionFormat::Float96NoW => {
            let m = mask.map(|m| if m & 7 == 0 { 7 } else { m & 7 }).unwrap_or(7);
            let mut v = [0.0f32; 3];
            for (k, c) in v.iter_mut().enumerate() {
                if m & (1 << k) != 0 {
                    *c = r.f32()?;
                }
            }
            v
        }
        CompressionFormat::IntervalFixed32NoW => vec_interval32(r.u32()?, mins, ranges),
        CompressionFormat::Fixed48NoW => match mask {
            None => {
                let (x, y, z) = (r.u16()?, r.u16()?, r.u16()?);
                vec_fixed48(x, y, z)
            }
            Some(m) => {
                let m = m & 7;
                let mut v = [0.0f32; 3];
                for (k, c) in v.iter_mut().enumerate() {
                    if m & (1 << k) != 0 {
                        *c = per_track_fixed48_component(r.u16()?);
                    }
                }
                v
            }
        },
        CompressionFormat::Identity => [0.0; 3],
        CompressionFormat::Fixed32NoW | CompressionFormat::Float32NoW => {
            return Err(Error::Format(format!("{} is not a valid translation/scale format", fmt.name())));
        }
    })
}

/// Bytes occupied by one key of a per-track block (used to relocate the byte stream, see
/// [`crate::sequence`]): `(per-key bytes)`.
pub fn per_track_key_bytes(fmt: CompressionFormat, mask: u8) -> usize {
    let nc = components_for_mask(mask);
    match fmt {
        CompressionFormat::None | CompressionFormat::Float96NoW => 4 * nc,
        CompressionFormat::Fixed48NoW => 2 * nc,
        CompressionFormat::IntervalFixed32NoW | CompressionFormat::Fixed32NoW | CompressionFormat::Float32NoW => 4,
        CompressionFormat::Identity => 0,
    }
}

/// Read the key time table: `u8` entries when `num_frames < 256`, else `u16`; the caller aligns afterwards.
/// A track with `num_keys <= 1` has no table.
pub fn read_time_table(r: &mut Reader, num_keys: usize, num_frames: usize) -> Result<Vec<u32>> {
    if num_keys <= 1 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(num_keys);
    if num_frames < 256 {
        for _ in 0..num_keys {
            out.push(r.u8()? as u32);
        }
    } else {
        for _ in 0..num_keys {
            out.push(r.u16()? as u32);
        }
    }
    Ok(out)
}

pub fn align4(pos: usize) -> usize {
    (pos + 3) & !3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn fixed48_known_values() {
        // all components 32767 -> identity
        assert_eq!(quat_fixed48(32767, 32767, 32767), [0.0, 0.0, 0.0, 1.0]);
        // x = +0.5 -> raw = 32767 + 16383.5; use 49150 -> (49150-32767)/32767 = 0.49998
        let q = quat_fixed48(49150, 32767, 32767);
        assert!(close(q[0], 16383.0 / 32767.0));
        assert!(close(q[3], (1.0 - q[0] * q[0]).sqrt()));
        // extremes: 0 -> -1.0, 65534 -> +1.0, w clamps to 0
        let q = quat_fixed48(0, 32767, 32767);
        assert_eq!(q[0], -1.0);
        assert_eq!(q[3], 0.0);
        let q = quat_fixed48(65534, 32767, 32767);
        assert_eq!(q[0], 1.0);
    }

    #[test]
    fn fixed32_known_values() {
        // x = 1023 (-> 0), y = 1023 (-> 0), z = 511 (-> 0)  => identity
        let v = (1023u32 << 21) | (1023 << 10) | 511;
        assert_eq!(quat_fixed32(v), [0.0, 0.0, 0.0, 1.0]);
        // x max (2046 -> +1.0), others zero
        let v = (2046u32 << 21) | (1023 << 10) | 511;
        let q = quat_fixed32(v);
        assert!(close(q[0], 2046.0 / 1023.0 - 1.0));
        assert_eq!(q[3], 0.0);
        // z only: z = 0 -> -1
        let v = (1023u32 << 21) | (1023 << 10);
        let q = quat_fixed32(v);
        assert_eq!(q[2], -1.0);
        // y = 1534 -> 1534/1023 - 1 = 0.49951
        let v = (1023u32 << 21) | (1534 << 10) | 511;
        assert!(close(quat_fixed32(v)[1], 1534.0 / 1023.0 - 1.0));
    }

    #[test]
    fn interval32_known_values() {
        let mins = [0.1, -0.2, 0.3];
        let ranges = [0.5, 0.25, 0.125];
        let v = (1023u32 << 21) | (1023 << 10) | 511; // all "zero"
        let q = quat_interval32(v, mins, ranges);
        assert!(close(q[0], 0.1) && close(q[1], -0.2) && close(q[2], 0.3));
        assert!(close(q[3], (1.0f32 - 0.01 - 0.04 - 0.09).sqrt()));
        // top value of x: 2046 -> +1 * range + min
        let v = (2046u32 << 21) | (0 << 10) | 1022;
        let q = quat_interval32(v, mins, ranges);
        assert!(close(q[0], 0.6));
        assert!(close(q[1], -0.2 - 0.25));
        assert!(close(q[2], 0.3 + 0.125 * (1022.0 / 511.0 - 1.0)));
    }

    #[test]
    fn float32_known_values() {
        // x = 0: exponent field 0 + 123 -> 2^-4 * (1.0) = 0.0625 with mantissa 0: value is 2^(123-127)=0.0625
        let q = quat_float32(0);
        assert!(close(q[0], 0.0625) && close(q[1], 0.0625) && close(q[2], 0.0625));
        // exponent 3 (bits 7..9 = 3) -> 2^(126-127)=0.5; mantissa 0x40 (top bit) -> *1.5 = 0.75
        let x = (3u32 << 7) | 0x40;
        let data = x << 21;
        let q = quat_float32(data);
        assert!(close(q[0], 0.75), "{}", q[0]);
        // negative: sign bit 10
        let x = 0x400u32 | (3 << 7);
        let q = quat_float32(x << 21);
        assert!(close(q[0], -0.5), "{}", q[0]);
        // z: 10 bits, exponent bits 6..8, mantissa 6 bits, sign bit 9
        let z = 0x200u32 | (3 << 6) | 0x20;
        let q = quat_float32(z);
        assert!(close(q[2], -0.75), "{}", q[2]);
    }

    #[test]
    fn vec_interval_known_values() {
        let mins = [10.0, 20.0, 30.0];
        let ranges = [1.0, 2.0, 4.0];
        let v = 511u32 | (1023 << 10) | (1023 << 21);
        assert_eq!(vec_interval32(v, mins, ranges), mins);
        let v = 1022u32 | (0 << 10) | (2046 << 21);
        let t = vec_interval32(v, mins, ranges);
        assert!(close(t[0], 11.0) && close(t[1], 18.0) && close(t[2], 34.0));
    }

    #[test]
    fn fixed48_vec_known_values() {
        assert_eq!(vec_fixed48(32767, 32767, 32767), [0.0; 3]);
        let v = vec_fixed48(65534, 0, 32767);
        assert!(close(v[0], 128.0) && close(v[1], -128.0));
        // per-track variant: (v - 255)
        assert_eq!(per_track_fixed48_component(255), 0.0);
        assert_eq!(per_track_fixed48_component(265), 10.0);
        assert_eq!(per_track_fixed48_component(250), -5.0);
    }

    #[test]
    fn float96_and_masks() {
        let mut data = Vec::new();
        for f in [0.1f32, 0.2, 0.3] {
            data.extend_from_slice(&f.to_le_bytes());
        }
        let mut r = Reader::new(&data);
        let q = read_quat_key(&mut r, CompressionFormat::Float96NoW, Some(0), [0.0; 3], [0.0; 3]).unwrap();
        assert!(close(q[0], 0.1) && close(q[1], 0.2) && close(q[2], 0.3));
        assert!(close(q[3], (1.0f32 - 0.01 - 0.04 - 0.09).sqrt()));
        // translation with mask X|Z reads two floats
        let mut r = Reader::new(&data);
        let t = read_vec_key(&mut r, CompressionFormat::Float96NoW, Some(5), [0.0; 3], [0.0; 3]).unwrap();
        assert!(close(t[0], 0.1) && t[1] == 0.0 && close(t[2], 0.2));
        assert_eq!(r.tell(), 8);
        // fixed48 rotation with only Y stored
        let mut d = Vec::new();
        d.extend_from_slice(&49150u16.to_le_bytes());
        let mut r = Reader::new(&d);
        let q = read_quat_key(&mut r, CompressionFormat::Fixed48NoW, Some(2), [0.0; 3], [0.0; 3]).unwrap();
        assert_eq!(q[0], 0.0);
        assert!(close(q[1], 16383.0 / 32767.0));
        assert_eq!(q[2], 0.0);
    }

    #[test]
    fn mask_component_counts() {
        assert_eq!(components_for_mask(0), 3);
        assert_eq!(components_for_mask(1), 1);
        assert_eq!(components_for_mask(3), 2);
        assert_eq!(components_for_mask(7), 3);
        assert_eq!(components_for_mask(8 | 5), 2);
    }

    #[test]
    fn time_table_widths() {
        let data = [1u8, 5, 9, 0];
        let mut r = Reader::new(&data);
        assert_eq!(read_time_table(&mut r, 3, 100).unwrap(), vec![1, 5, 9]);
        let data = [1u8, 0, 5, 1];
        let mut r = Reader::new(&data);
        assert_eq!(read_time_table(&mut r, 2, 300).unwrap(), vec![1, 261]);
        assert!(read_time_table(&mut r, 1, 300).unwrap().is_empty());
    }
}
