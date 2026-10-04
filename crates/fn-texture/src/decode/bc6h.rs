//! BC6H (unsigned half-float HDR) decoder, a port of detex's `decompress-bptc-float.cpp`
//! (`DecompressBlockBPTCFloatShared` with `signed_flag == false`, the variant UEViewer uses for `PF_BC6H`).
//!
//! The 14 bitstream modes are listed in the same order as detex numbers them (0..=13); the bit positions are given
//! as `(data0|data1, first bit, last bit)` ranges exactly as in the specification tables.

use super::decode_blocks;
use super::tables::{ANCHOR2, P2, WEIGHT3, WEIGHT4};

const HALF_ONE: u16 = 0x3c00;

/// Endpoint precision (bits) per detex mode.
const EPB: [u32; 14] = [10, 7, 11, 11, 11, 9, 8, 8, 8, 6, 10, 11, 12, 16];
const MAP_MODE: [i8; 32] = [0, 1, 2, 10, -1, -1, 3, 11, -1, -1, 4, 12, -1, -1, 5, 13, -1, -1, 6, -1, -1, -1, 7, -1, -1, -1, 8, -1, -1, -1, 9, -1];

/// Bits `lo..=hi` of a 64 bit word.
#[inline]
fn g(d: u64, lo: u32, hi: u32) -> i32 {
    ((d >> lo) & ((1u64 << (hi - lo + 1)) - 1)) as i32
}

/// Bits `bit0` down to `bit1` of a word, in reversed order (detex's `detexGetBits64Reversed`).
#[inline]
fn rev(d: u64, bit0: u32, bit1: u32) -> i32 {
    let mut v = 0i32;
    for i in 0..=(bit0 - bit1) {
        v |= (((d >> (bit0 - i)) & 1) as i32) << i;
    }
    v
}

fn sign_extend(v: i32, bits: u32) -> i32 {
    if v & (1 << (bits - 1)) != 0 {
        v | !((1i32 << bits) - 1)
    } else {
        v
    }
}

fn unquantize(x: i32, mode: usize) -> i32 {
    let epb = EPB[mode];
    if mode == 13 {
        x
    } else if x == 0 {
        0
    } else if x == (1 << epb) - 1 {
        0xffff
    } else {
        ((x << 15) + 0x4000) >> (epb - 1)
    }
}

fn interpolate(e0: i32, e1: i32, idx: usize, bits: u32) -> i32 {
    let w = if bits == 3 { WEIGHT3[idx] } else { WEIGHT4[idx] } as i32;
    ((64 - w) * e0 + w * e1 + 32) >> 6
}

/// Decode one BC6H block into 16 pixels of half-float RGBA bit patterns (alpha = 1.0). Invalid mode -> zeros
/// (alpha 1.0).
pub fn decode_block(b: &[u8], out: &mut [[u16; 4]; 16]) {
    let d0 = u64::from_le_bytes(b[0..8].try_into().unwrap());
    let d1 = u64::from_le_bytes(b[8..16].try_into().unwrap());

    let m2 = (d0 & 3) as usize;
    let mode: i32 = if m2 < 2 { m2 as i32 } else { MAP_MODE[m2 | (((d0 >> 2) & 7) as usize) << 2] as i32 };
    if mode < 0 {
        *out = [[0, 0, 0, HALF_ONE]; 16];
        return;
    }
    let mode = mode as usize;

    let mut r = [0i32; 4];
    let mut gg = [0i32; 4];
    let mut bb = [0i32; 4];
    let mut part = 0usize;
    let (mut dr, mut dg, mut db) = (0u32, 0u32, 0u32);
    let idx_start: u32; // bit index (in the 128 bit block) where the pixel indices start

    macro_rules! common_tail {
        () => {
            part = g(d1, 13, 17) as usize;
            idx_start = 64 + 18;
        };
    }

    match mode {
        0 => {
            gg[2] = g(d0, 2, 2) << 4;
            bb[2] = g(d0, 3, 3) << 4;
            bb[3] = g(d0, 4, 4) << 4;
            r[0] = g(d0, 5, 14);
            gg[0] = g(d0, 15, 24);
            bb[0] = g(d0, 25, 34);
            r[1] = g(d0, 35, 39);
            gg[3] = g(d0, 40, 40) << 4;
            gg[2] |= g(d0, 41, 44);
            gg[1] = g(d0, 45, 49);
            bb[3] |= g(d0, 50, 50);
            gg[3] |= g(d0, 51, 54);
            bb[1] = g(d0, 55, 59);
            bb[3] |= g(d0, 60, 60) << 1;
            bb[2] |= g(d0, 61, 63);
            bb[2] |= g(d1, 0, 0) << 3;
            r[2] = g(d1, 1, 5);
            bb[3] |= g(d1, 6, 6) << 2;
            r[3] = g(d1, 7, 11);
            bb[3] |= g(d1, 12, 12) << 3;
            common_tail!();
            dr = 5;
            dg = 5;
            db = 5;
        }
        1 => {
            gg[2] = g(d0, 2, 2) << 5;
            gg[3] = g(d0, 3, 3) << 4;
            gg[3] |= g(d0, 4, 4) << 5;
            r[0] = g(d0, 5, 11);
            bb[3] = g(d0, 12, 12);
            bb[3] |= g(d0, 13, 13) << 1;
            bb[2] = g(d0, 14, 14) << 4;
            gg[0] = g(d0, 15, 21);
            bb[2] |= g(d0, 22, 22) << 5;
            bb[3] |= g(d0, 23, 23) << 2;
            gg[2] |= g(d0, 24, 24) << 4;
            bb[0] = g(d0, 25, 31);
            bb[3] |= g(d0, 32, 32) << 3;
            bb[3] |= g(d0, 33, 33) << 5;
            bb[3] |= g(d0, 34, 34) << 4;
            r[1] = g(d0, 35, 40);
            gg[2] |= g(d0, 41, 44);
            gg[1] = g(d0, 45, 50);
            gg[3] |= g(d0, 51, 54);
            bb[1] = g(d0, 55, 60);
            bb[2] |= g(d0, 61, 63);
            bb[2] |= g(d1, 0, 0) << 3;
            r[2] = g(d1, 1, 6);
            r[3] = g(d1, 7, 12);
            common_tail!();
            dr = 6;
            dg = 6;
            db = 6;
        }
        2 => {
            r[0] = g(d0, 5, 14);
            gg[0] = g(d0, 15, 24);
            bb[0] = g(d0, 25, 34);
            r[1] = g(d0, 35, 39);
            r[0] |= g(d0, 40, 40) << 10;
            gg[2] = g(d0, 41, 44);
            gg[1] = g(d0, 45, 48);
            gg[0] |= g(d0, 49, 49) << 10;
            bb[3] = g(d0, 50, 50);
            gg[3] = g(d0, 51, 54);
            bb[1] = g(d0, 55, 58);
            bb[0] |= g(d0, 59, 59) << 10;
            bb[3] |= g(d0, 60, 60) << 1;
            bb[2] = g(d0, 61, 63);
            bb[2] |= g(d1, 0, 0) << 3;
            r[2] = g(d1, 1, 5);
            bb[3] |= g(d1, 6, 6) << 2;
            r[3] = g(d1, 7, 11);
            bb[3] |= g(d1, 12, 12) << 3;
            common_tail!();
            dr = 5;
            dg = 4;
            db = 4;
        }
        3 => {
            r[0] = g(d0, 5, 14);
            gg[0] = g(d0, 15, 24);
            bb[0] = g(d0, 25, 34);
            r[1] = g(d0, 35, 38);
            r[0] |= g(d0, 39, 39) << 10;
            gg[3] = g(d0, 40, 40) << 4;
            gg[2] = g(d0, 41, 44);
            gg[1] = g(d0, 45, 49);
            gg[0] |= g(d0, 50, 50) << 10;
            gg[3] |= g(d0, 51, 54);
            bb[1] = g(d0, 55, 58);
            bb[0] |= g(d0, 59, 59) << 10;
            bb[3] = g(d0, 60, 60) << 1;
            bb[2] = g(d0, 61, 63);
            bb[2] |= g(d1, 0, 0) << 3;
            r[2] = g(d1, 1, 4);
            bb[3] |= g(d1, 5, 5);
            bb[3] |= g(d1, 6, 6) << 2;
            r[3] = g(d1, 7, 10);
            gg[2] |= g(d1, 11, 11) << 4;
            bb[3] |= g(d1, 12, 12) << 3;
            common_tail!();
            dr = 4;
            db = 4;
            dg = 5;
        }
        4 => {
            r[0] = g(d0, 5, 14);
            gg[0] = g(d0, 15, 24);
            bb[0] = g(d0, 25, 34);
            r[1] = g(d0, 35, 38);
            r[0] |= g(d0, 39, 39) << 10;
            bb[2] = g(d0, 40, 40) << 4;
            gg[2] = g(d0, 41, 44);
            gg[1] = g(d0, 45, 48);
            gg[0] |= g(d0, 49, 49) << 10;
            bb[3] = g(d0, 50, 50);
            gg[3] = g(d0, 51, 54);
            bb[1] = g(d0, 55, 59);
            bb[0] |= g(d0, 60, 60) << 10;
            bb[2] |= g(d0, 61, 63);
            bb[2] |= g(d1, 0, 0) << 3;
            r[2] = g(d1, 1, 4);
            bb[3] |= g(d1, 5, 5) << 1;
            bb[3] |= g(d1, 6, 6) << 2;
            r[3] = g(d1, 7, 10);
            bb[3] |= g(d1, 11, 11) << 4;
            bb[3] |= g(d1, 12, 12) << 3;
            common_tail!();
            dr = 4;
            dg = 4;
            db = 5;
        }
        5 => {
            r[0] = g(d0, 5, 13);
            bb[2] = g(d0, 14, 14) << 4;
            gg[0] = g(d0, 15, 23);
            gg[2] = g(d0, 24, 24) << 4;
            bb[0] = g(d0, 25, 33);
            bb[3] = g(d0, 34, 34) << 4;
            r[1] = g(d0, 35, 39);
            gg[3] = g(d0, 40, 40) << 4;
            gg[2] |= g(d0, 41, 44);
            gg[1] = g(d0, 45, 49);
            bb[3] |= g(d0, 50, 50);
            gg[3] |= g(d0, 51, 54);
            bb[1] = g(d0, 55, 59);
            bb[3] |= g(d0, 60, 60) << 1;
            bb[2] |= g(d0, 61, 63);
            bb[2] |= g(d1, 0, 0) << 3;
            r[2] = g(d1, 1, 5);
            bb[3] |= g(d1, 6, 6) << 2;
            r[3] = g(d1, 7, 11);
            bb[3] |= g(d1, 12, 12) << 3;
            common_tail!();
            dr = 5;
            dg = 5;
            db = 5;
        }
        6 => {
            r[0] = g(d0, 5, 12);
            gg[3] = g(d0, 13, 13) << 4;
            bb[2] = g(d0, 14, 14) << 4;
            gg[0] = g(d0, 15, 22);
            bb[3] = g(d0, 23, 23) << 2;
            gg[2] = g(d0, 24, 24) << 4;
            bb[0] = g(d0, 25, 32);
            bb[3] |= g(d0, 33, 33) << 3;
            bb[3] |= g(d0, 34, 34) << 4;
            r[1] = g(d0, 35, 40);
            gg[2] |= g(d0, 41, 44);
            gg[1] = g(d0, 45, 49);
            bb[3] |= g(d0, 50, 50);
            gg[3] |= g(d0, 51, 54);
            bb[1] = g(d0, 55, 59);
            bb[3] |= g(d0, 60, 60) << 1;
            bb[2] |= g(d0, 61, 63);
            bb[2] |= g(d1, 0, 0) << 3;
            r[2] = g(d1, 1, 6);
            r[3] = g(d1, 7, 12);
            common_tail!();
            dr = 6;
            dg = 5;
            db = 5;
        }
        7 => {
            r[0] = g(d0, 5, 12);
            bb[3] = g(d0, 13, 13);
            bb[2] = g(d0, 14, 14) << 4;
            gg[0] = g(d0, 15, 22);
            gg[2] = g(d0, 23, 23) << 5;
            gg[2] |= g(d0, 24, 24) << 4;
            bb[0] = g(d0, 25, 32);
            gg[3] = g(d0, 33, 33) << 5;
            bb[3] |= g(d0, 34, 34) << 4;
            r[1] = g(d0, 35, 39);
            gg[3] |= g(d0, 40, 40) << 4;
            gg[2] |= g(d0, 41, 44);
            gg[1] = g(d0, 45, 50);
            gg[3] |= g(d0, 51, 54);
            bb[1] = g(d0, 55, 59);
            bb[3] |= g(d0, 60, 60) << 1;
            bb[2] |= g(d0, 61, 63);
            bb[2] |= g(d1, 0, 0) << 3;
            r[2] = g(d1, 1, 5);
            bb[3] |= g(d1, 6, 6) << 2;
            r[3] = g(d1, 7, 11);
            bb[3] |= g(d1, 12, 12) << 3;
            common_tail!();
            dr = 5;
            db = 5;
            dg = 6;
        }
        8 => {
            r[0] = g(d0, 5, 12);
            bb[3] = g(d0, 13, 13) << 1;
            bb[2] = g(d0, 14, 14) << 4;
            gg[0] = g(d0, 15, 22);
            bb[2] |= g(d0, 23, 23) << 5;
            gg[2] = g(d0, 24, 24) << 4;
            bb[0] = g(d0, 25, 32);
            bb[3] |= g(d0, 33, 33) << 5;
            bb[3] |= g(d0, 34, 34) << 4;
            r[1] = g(d0, 35, 39);
            gg[3] = g(d0, 40, 40) << 4;
            gg[2] |= g(d0, 41, 44);
            gg[1] = g(d0, 45, 49);
            bb[3] |= g(d0, 50, 50);
            gg[3] |= g(d0, 51, 54);
            bb[1] = g(d0, 55, 60);
            bb[2] |= g(d0, 61, 63);
            bb[2] |= g(d1, 0, 0) << 3;
            r[2] = g(d1, 1, 5);
            bb[3] |= g(d1, 6, 6) << 2;
            r[3] = g(d1, 7, 11);
            bb[3] |= g(d1, 12, 12) << 3;
            common_tail!();
            dr = 5;
            dg = 5;
            db = 6;
        }
        9 => {
            r[0] = g(d0, 5, 10);
            gg[3] = g(d0, 11, 11) << 4;
            bb[3] = g(d0, 12, 13);
            bb[2] = g(d0, 14, 14) << 4;
            gg[0] = g(d0, 15, 20);
            gg[2] = g(d0, 21, 21) << 5;
            bb[2] |= g(d0, 22, 22) << 5;
            bb[3] |= g(d0, 23, 23) << 2;
            gg[2] |= g(d0, 24, 24) << 4;
            bb[0] = g(d0, 25, 30);
            gg[3] |= g(d0, 31, 31) << 5;
            bb[3] |= g(d0, 32, 32) << 3;
            bb[3] |= g(d0, 33, 33) << 5;
            bb[3] |= g(d0, 34, 34) << 4;
            r[1] = g(d0, 35, 40);
            gg[2] |= g(d0, 41, 44);
            gg[1] = g(d0, 45, 50);
            gg[3] |= g(d0, 51, 54);
            bb[1] = g(d0, 55, 60);
            bb[2] |= g(d0, 61, 63);
            bb[2] |= g(d1, 0, 0) << 3;
            r[2] = g(d1, 1, 6);
            r[3] = g(d1, 7, 12);
            common_tail!();
        }
        10 => {
            r[0] = g(d0, 5, 14);
            gg[0] = g(d0, 15, 24);
            bb[0] = g(d0, 25, 34);
            r[1] = g(d0, 35, 44);
            gg[1] = g(d0, 45, 54);
            bb[1] = g(d0, 55, 63);
            bb[1] |= g(d1, 0, 0) << 9;
            idx_start = 65;
        }
        11 => {
            r[0] = g(d0, 5, 14);
            gg[0] = g(d0, 15, 24);
            bb[0] = g(d0, 25, 34);
            r[1] = g(d0, 35, 43);
            r[0] |= g(d0, 44, 44) << 10;
            gg[1] = g(d0, 45, 53);
            gg[0] |= g(d0, 54, 54) << 10;
            bb[1] = g(d0, 55, 63);
            bb[0] |= g(d1, 0, 0) << 10;
            idx_start = 65;
            dr = 9;
            dg = 9;
            db = 9;
        }
        12 => {
            r[0] = g(d0, 5, 14);
            gg[0] = g(d0, 15, 24);
            bb[0] = g(d0, 25, 34);
            r[1] = g(d0, 35, 42);
            r[0] |= rev(d0, 44, 43) << 10;
            gg[1] = g(d0, 45, 52);
            gg[0] |= rev(d0, 54, 53) << 10;
            bb[1] = g(d0, 55, 62);
            bb[0] |= g(d0, 63, 63) << 11;
            bb[0] |= g(d1, 0, 0) << 10;
            idx_start = 65;
            dr = 8;
            dg = 8;
            db = 8;
        }
        _ => {
            r[0] = g(d0, 5, 14);
            gg[0] = g(d0, 15, 24);
            bb[0] = g(d0, 25, 34);
            r[1] = g(d0, 35, 38);
            r[0] |= rev(d0, 44, 39) << 10;
            gg[1] = g(d0, 45, 48);
            gg[0] |= rev(d0, 54, 49) << 10;
            bb[1] = g(d0, 55, 58);
            bb[0] |= rev(d0, 63, 59) << 11;
            bb[0] |= g(d1, 0, 0) << 10;
            idx_start = 65;
            dr = 4;
            dg = 4;
            db = 4;
        }
    }

    let two_subsets = mode < 10;
    let nep = if two_subsets { 4 } else { 2 };
    let mask = (1i32 << EPB[mode]) - 1;
    if mode != 9 && mode != 10 {
        // endpoints 1..n are stored as signed deltas from endpoint 0
        for i in 1..nep {
            r[i] = (r[0] + sign_extend(r[i], dr)) & mask;
            gg[i] = (gg[0] + sign_extend(gg[i], dg)) & mask;
            bb[i] = (bb[0] + sign_extend(bb[i], db)) & mask;
        }
    }
    for i in 0..nep {
        r[i] = unquantize(r[i], mode);
        gg[i] = unquantize(gg[i], mode);
        bb[i] = unquantize(bb[i], mode);
    }

    let subset_of = |i: usize| -> usize {
        if two_subsets {
            P2[part * 16 + i] as usize
        } else {
            0
        }
    };
    let anchor = |s: usize| -> usize {
        if s == 0 {
            0
        } else {
            ANCHOR2[part] as usize
        }
    };
    // modes 10..=13 (one subset) use 4 bit indices
    let ibits: u32 = if d0 & 3 == 3 { 4 } else { 3 };
    let mut bits = idx_start;
    let mut take = |n: u32| -> usize {
        let v = if bits >= 64 { (d1 >> (bits - 64)) & ((1u64 << n) - 1) } else { (d0 >> bits) & ((1u64 << n) - 1) };
        bits += n;
        v as usize
    };
    for i in 0..16 {
        let s = subset_of(i);
        let n = if i == anchor(s) { ibits - 1 } else { ibits };
        let idx = take(n);
        let ch = |a: &[i32; 4]| -> u16 { (interpolate(a[2 * s], a[2 * s + 1], idx, ibits) * 31 / 64) as u16 };
        out[i] = [ch(&r), ch(&gg), ch(&bb), HALF_ONE];
    }
}

/// Decode a whole BC6H image to half-float RGBA bit patterns.
pub fn decode(src: &[u8], w: usize, h: usize, threads: usize) -> Vec<u16> {
    decode_blocks(src, w, h, 16, threads, |b, out| decode_block(b, out))
}
