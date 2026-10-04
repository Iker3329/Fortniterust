//! `fn-ueio`: minimal, dependency-free reader for UE 4.16 cooked packages.
//!
//! This crate exists so `fn-texture`, `fn-mesh` and `fn-anim` can be built and verified against plain byte
//! slices independently of the pak / full uasset crates. It provides:
//!
//! * [`Reader`]: little-endian cursor over `&[u8]` (FString, FName index, f16 ...).
//! * [`Package`]: summary, name table, imports, exports of a `.uasset` (+ `.uexp` concatenated).
//! * [`props`]: generic tagged-property reader (UE 4.16 `FPropertyTag` format, file version 514).
//! * [`BulkData`]: `FByteBulkData` header + resolution of the payload (inline, end-of-file, `.ubulk`).
//!
//! Package layout facts verified on Fortnite 1.8 (UE 4.16, file version 514, legacy -7):
//! the `.uasset` holds the header (`TotalHeaderSize` bytes); the `.uexp` holds the export payloads. Export
//! `SerialOffset` values are expressed in the virtual concatenation `uasset ++ uexp`, so
//! `export bytes = (uasset ++ uexp)[SerialOffset .. SerialOffset+SerialSize]`.
//! Bulk data with `BULKDATA_PayloadAtEndOfFile` lives at `BulkDataStartOffset + OffsetInFile` in the same virtual
//! file; with `BULKDATA_PayloadInSeperateFile` it lives at `OffsetInFile` (unfixed) inside the `.ubulk`.

mod package;
pub mod props;
mod reader;

pub use package::{UE4_16_FILE_VERSION, BulkData, Export, Import, Package, Summary, BULK_FORCE_INLINE, BULK_AT_END, BULK_SEPARATE, BULK_UNUSED, BULK_ZLIB};
pub use props::{Prop, PropList, PropValue};
pub use reader::Reader;

use std::fmt;

#[derive(Debug, Clone)]
pub enum Error {
    Eof { pos: usize, wanted: usize },
    Invalid(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Eof { pos, wanted } => write!(f, "unexpected end of data at {pos} (wanted {wanted} bytes)"),
            Error::Invalid(s) => write!(f, "{s}"),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

pub fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}

/// IEEE half -> f32 (no external crate).
pub fn f16_to_f32(h: u16) -> f32 {
    let s = ((h >> 15) & 1) as u32;
    let e = ((h >> 10) & 0x1f) as u32;
    let m = (h & 0x3ff) as u32;
    let bits = if e == 0 {
        if m == 0 {
            s << 31
        } else {
            // subnormal
            let mut e2 = 127 - 15 + 1;
            let mut m2 = m;
            while m2 & 0x400 == 0 {
                m2 <<= 1;
                e2 -= 1;
            }
            (s << 31) | ((e2 as u32) << 23) | ((m2 & 0x3ff) << 13)
        }
    } else if e == 31 {
        (s << 31) | 0x7f80_0000 | (m << 13)
    } else {
        (s << 31) | ((e + 127 - 15) << 23) | (m << 13)
    };
    f32::from_bits(bits)
}

/// f32 -> IEEE half, round to nearest even.
pub fn f32_to_f16(f: f32) -> u16 {
    let x = f.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let man = x & 0x007f_ffff;
    if exp == 255 {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x0080_0000;
        let shift = (14 - e) as u32;
        let mut h = (m >> shift) as u16;
        let rem = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && (h & 1) == 1) {
            h += 1;
        }
        return sign | h;
    }
    let mut h = sign | ((e as u16) << 10) | ((man >> 13) as u16);
    let rem = man & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && (h & 1) == 1) {
        h += 1;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn half_roundtrip() {
        for h in 0u16..=0xffff {
            let f = f16_to_f32(h);
            if f.is_nan() {
                continue;
            }
            assert_eq!(f32_to_f16(f), h, "h={h:#x} f={f}");
        }
    }
    #[test]
    fn half_known() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x7bff), 65504.0);
    }
}
