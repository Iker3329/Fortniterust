//! BC1 (DXT1), BC2 (DXT3), BC3 (DXT5), BC4, BC5 decoders, bit-exact with the NVTT code UEViewer used
//! (`BlockDXT.cpp` / `DirectDrawSurface.cpp`).

use super::{decode_blocks, DecodeOptions, Rounding};
use crate::format::PixelFormat;

type Px = [u8; 4];

#[inline]
fn expand565(c: u16) -> [i32; 3] {
    let r = ((c >> 11) & 31) as i32;
    let g = ((c >> 5) & 63) as i32;
    let b = (c & 31) as i32;
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

/// Build the 4 entry BC1 palette (RGBA). `c0 > c1` (raw 565 compare) selects 4-colour mode, otherwise the 3-colour
/// mode with transparent black as entry 3.
pub fn bc1_palette(c0: u16, c1: u16, rounding: Rounding) -> [Px; 4] {
    let a = expand565(c0);
    let b = expand565(c1);
    let mut p = [[0u8; 4]; 4];
    p[0] = [a[0] as u8, a[1] as u8, a[2] as u8, 255];
    p[1] = [b[0] as u8, b[1] as u8, b[2] as u8, 255];
    let (r3, r2) = match rounding {
        Rounding::NvttTruncate => (0, 0),
        Rounding::Nearest => (1, 1),
    };
    if c0 > c1 {
        for k in 0..3 {
            p[2][k] = ((2 * a[k] + b[k] + r3) / 3) as u8;
            p[3][k] = ((2 * b[k] + a[k] + r3) / 3) as u8;
        }
        p[2][3] = 255;
        p[3][3] = 255;
    } else {
        for k in 0..3 {
            p[2][k] = ((a[k] + b[k] + r2) / 2) as u8;
        }
        p[2][3] = 255;
        p[3] = [0, 0, 0, 0];
    }
    p
}

/// Decode a BC1 colour block (8 bytes) into 16 RGBA pixels.
#[inline]
pub fn bc1_block(b: &[u8], rounding: Rounding, out: &mut [Px; 16]) {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let pal = bc1_palette(c0, c1, rounding);
    let idx = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    for (i, o) in out.iter_mut().enumerate() {
        *o = pal[((idx >> (2 * i)) & 3) as usize];
    }
}

/// Evaluate the 8 entry BC3 / BC4 alpha palette.
pub fn alpha_palette(a0: u8, a1: u8, rounding: Rounding) -> [u8; 8] {
    let (x0, x1) = (a0 as i32, a1 as i32);
    let mut p = [0u8; 8];
    p[0] = a0;
    p[1] = a1;
    let (r7, r5) = match rounding {
        Rounding::NvttTruncate => (0, 0),
        Rounding::Nearest => (3, 2),
    };
    if a0 > a1 {
        for k in 1..7 {
            p[k + 1] = (((7 - k as i32) * x0 + k as i32 * x1 + r7) / 7) as u8;
        }
    } else {
        for k in 1..5 {
            p[k + 1] = (((5 - k as i32) * x0 + k as i32 * x1 + r5) / 5) as u8;
        }
        p[6] = 0;
        p[7] = 255;
    }
    p
}

/// Decode a BC3-style alpha block (8 bytes) into 16 values.
#[inline]
pub fn alpha_block(b: &[u8], rounding: Rounding) -> [u8; 16] {
    let pal = alpha_palette(b[0], b[1], rounding);
    let mut bits = 0u64;
    for (i, &v) in b[2..8].iter().enumerate() {
        bits |= (v as u64) << (8 * i);
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = pal[((bits >> (3 * i)) & 7) as usize];
    }
    out
}

/// UEViewer's `buildNormal`: reconstruct Z from X and Y (see [`DecodeOptions::bc5_reconstruct_z`]).
#[inline]
pub fn build_normal_z(x: u8, y: u8) -> u8 {
    let nx = 2.0f32 * (x as f32 / 255.0f32) - 1.0f32;
    let ny = 2.0f32 * (y as f32 / 255.0f32) - 1.0f32;
    let mut nz = 0.0f32;
    if 1.0f32 - nx * nx - ny * ny > 0.0f32 {
        nz = (1.0f32 - nx * nx - ny * ny).sqrt();
    }
    ((255.0f32 * (nz + 1.0f32) / 2.0f32) as i32).clamp(0, 255) as u8
}

/// Decode a whole BC1/2/3/4/5 image to RGBA8.
pub fn decode(format: &PixelFormat, src: &[u8], w: usize, h: usize, o: &DecodeOptions) -> Vec<u8> {
    let rounding = o.rounding;
    match format {
        PixelFormat::Dxt1 => {
            let zero = o.dxt1_zero_transparent_rgb;
            decode_blocks(src, w, h, 8, o.threads, move |b, out| {
                bc1_block(b, rounding, out);
                if zero {
                    for p in out.iter_mut() {
                        if p[3] == 0 {
                            *p = [0, 0, 0, 0];
                        }
                    }
                }
            })
        }
        PixelFormat::Dxt3 => decode_blocks(src, w, h, 16, o.threads, move |b, out| {
            bc1_block(&b[8..16], rounding, out);
            for (i, p) in out.iter_mut().enumerate() {
                let n = (b[i / 2] >> (4 * (i & 1))) & 0xf;
                p[3] = (n << 4) | n;
            }
        }),
        PixelFormat::Dxt5 => decode_blocks(src, w, h, 16, o.threads, move |b, out| {
            bc1_block(&b[8..16], rounding, out);
            let a = alpha_block(&b[0..8], rounding);
            for (p, a) in out.iter_mut().zip(a) {
                p[3] = a;
            }
        }),
        PixelFormat::Bc4 => decode_blocks(src, w, h, 8, o.threads, move |b, out| {
            let a = alpha_block(b, rounding);
            for (p, a) in out.iter_mut().zip(a) {
                *p = [a, a, a, 255];
            }
        }),
        PixelFormat::Bc5 => {
            let z = o.bc5_reconstruct_z;
            decode_blocks(src, w, h, 16, o.threads, move |b, out| {
                let x = alpha_block(&b[0..8], rounding);
                let y = alpha_block(&b[8..16], rounding);
                for i in 0..16 {
                    let bl = if z { build_normal_z(x[i], y[i]) } else { 0 };
                    out[i] = [x[i], y[i], bl, 255];
                }
            })
        }
        _ => unreachable!("bc1235::decode called with {format:?}"),
    }
}
