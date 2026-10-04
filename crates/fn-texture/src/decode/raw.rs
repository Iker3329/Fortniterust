//! Uncompressed pixel formats.

use crate::decoded::Pixels;
use crate::format::PixelFormat;
use crate::{Error, Result};
use fn_ueio::f32_to_f16;

const HALF_ONE: u16 = 0x3c00;

pub fn decode(format: &PixelFormat, src: &[u8], w: usize, h: usize) -> Result<Pixels> {
    let n = w * h;
    Ok(match format {
        // UEViewer treats PF_A8R8G8B8 exactly like PF_B8G8R8A8 (the UE3 name was kept for the same layout).
        PixelFormat::B8G8R8A8 | PixelFormat::A8R8G8B8 => {
            let mut out = Vec::with_capacity(n * 4);
            for p in src[..n * 4].chunks_exact(4) {
                out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
            Pixels::Rgba8(out)
        }
        PixelFormat::R8G8B8A8 => Pixels::Rgba8(src[..n * 4].to_vec()),
        PixelFormat::G8 => {
            let mut out = Vec::with_capacity(n * 4);
            for &g in &src[..n] {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            Pixels::Rgba8(out)
        }
        PixelFormat::G16 => {
            // UEViewer cannot decode G16; we keep the 8 most significant bits.
            let mut out = Vec::with_capacity(n * 4);
            for p in src[..n * 2].chunks_exact(2) {
                let g = p[1];
                out.extend_from_slice(&[g, g, g, 255]);
            }
            Pixels::Rgba8(out)
        }
        PixelFormat::FloatRGBA => Pixels::RgbaF16(src[..n * 8].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()),
        PixelFormat::R16F => {
            let mut out = Vec::with_capacity(n * 4);
            for c in src[..n * 2].chunks_exact(2) {
                out.extend_from_slice(&[u16::from_le_bytes([c[0], c[1]]), 0, 0, HALF_ONE]);
            }
            Pixels::RgbaF16(out)
        }
        PixelFormat::R32F => {
            let mut out = Vec::with_capacity(n * 4);
            for c in src[..n * 4].chunks_exact(4) {
                let f = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                out.extend_from_slice(&[f32_to_f16(f), 0, 0, HALF_ONE]);
            }
            Pixels::RgbaF16(out)
        }
        PixelFormat::V8U8 => {
            // Same arithmetic as UEViewer's TPF_V8U8 (offset 128), including its quirky blue channel.
            let mut out = Vec::with_capacity(n * 4);
            for c in src[..n * 2].chunks_exact(2) {
                let u = c[0].wrapping_add(128);
                let v = c[1].wrapping_add(128);
                let uf = (u as i32 - 128) as f32 / 255.0 * 2.0 - 1.0;
                let vf = (v as i32 - 128) as f32 / 255.0 * 2.0 - 1.0;
                let t = 1.0 - uf * uf - vf * vf;
                let z = if t >= 0.0 { (255i32 - 255 * (t.sqrt().floor() as i32)) as u8 } else { 255 };
                out.extend_from_slice(&[u, v, z, 255]);
            }
            Pixels::Rgba8(out)
        }
        PixelFormat::A1 => {
            // Layout assumed (never seen in cooked data): rows padded to whole bytes, least significant bit first.
            let stride = w.div_ceil(8);
            let mut out = Vec::with_capacity(n * 4);
            for y in 0..h {
                for x in 0..w {
                    let bit = (src[y * stride + x / 8] >> (x & 7)) & 1;
                    let v = if bit != 0 { 255 } else { 0 };
                    out.extend_from_slice(&[v, v, v, 255]);
                }
            }
            Pixels::Rgba8(out)
        }
        other => return Err(Error::UnsupportedFormat(other.name().to_string())),
    })
}
