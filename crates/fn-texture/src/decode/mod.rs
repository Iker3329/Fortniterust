//! Pixel decoders. Everything here is pure Rust and operates on one mip slice.

pub mod bc1235;
pub mod bc6h;
pub mod bc7;
pub mod raw;
mod tables;

use crate::decoded::{DecodedImage, Pixels};
use crate::format::PixelFormat;
use crate::{Error, Result};

/// How the 1/3 and 2/3 (and 1/7 ... 6/7) interpolants of BC1-BC5 are rounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// NVTT / UEViewer behaviour: integer division truncates (`(2*a+b)/3`). Default; matches the old PNG exports.
    NvttTruncate,
    /// Round to nearest (`(2*a+b+1)/3`), closer to what GPUs produce.
    Nearest,
}

/// Decoder options.
#[derive(Debug, Clone, Copy)]
pub struct DecodeOptions {
    /// BC5 only: rebuild the blue channel as the Z of a unit normal like UEViewer does (default `true`):
    /// `nx = 2*x/255-1`, `ny = 2*y/255-1`, `nz = sqrt(1-nx²-ny²)` (0 when the radicand is not positive),
    /// `z = clamp(int(255*(nz+1)/2), 0, 255)`. When `false` the blue channel is 0.
    pub bc5_reconstruct_z: bool,
    /// Interpolation rounding for BC1/BC2/BC3/BC4/BC5. Default [`Rounding::NvttTruncate`].
    pub rounding: Rounding,
    /// BC1 only: set RGB to 0 where alpha is 0 (UEViewer's `PostProcessAlpha`, default `true`). The NVTT decoder
    /// already yields (0,0,0,0) for index 3 of 3-colour blocks, so this only matters for other alpha sources.
    pub dxt1_zero_transparent_rgb: bool,
    /// Worker threads for large images: 0 = automatic, 1 = single threaded.
    pub threads: usize,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        DecodeOptions { bc5_reconstruct_z: true, rounding: Rounding::NvttTruncate, dxt1_zero_transparent_rgb: true, threads: 0 }
    }
}

/// Decode one slice of `w` x `h` pixels from raw mip bytes.
pub fn decode_mip(format: &PixelFormat, src: &[u8], w: usize, h: usize, o: &DecodeOptions) -> Result<DecodedImage> {
    let need = format.mip_bytes(w, h).ok_or_else(|| Error::UnsupportedFormat(format.name().to_string()))?;
    if src.len() < need {
        return Err(Error::Invalid(format!("{} {w}x{h} needs {need} bytes, got {}", format.name(), src.len())));
    }
    let px = match format {
        PixelFormat::Dxt1 | PixelFormat::Dxt3 | PixelFormat::Dxt5 | PixelFormat::Bc4 | PixelFormat::Bc5 => Pixels::Rgba8(bc1235::decode(format, src, w, h, o)),
        PixelFormat::Bc7 => Pixels::Rgba8(bc7::decode(src, w, h, o.threads)),
        PixelFormat::Bc6h => Pixels::RgbaF16(bc6h::decode(src, w, h, o.threads)),
        other => raw::decode(other, src, w, h)?,
    };
    Ok(DecodedImage { width: w, height: h, pixels: px })
}

fn thread_count(requested: usize, pixels: usize) -> usize {
    if pixels < 128 * 128 {
        return 1;
    }
    if requested > 0 {
        return requested;
    }
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// Decode a block compressed image. `f` decodes one block (`block_bytes` bytes) into 16 RGBA pixels (row-major 4x4).
/// Edge blocks are clipped. Block rows are distributed over threads.
pub(crate) fn decode_blocks<T, F>(src: &[u8], w: usize, h: usize, block_bytes: usize, threads: usize, f: F) -> Vec<T>
where
    T: Copy + Default + Send + Sync,
    F: Fn(&[u8], &mut [[T; 4]; 16]) + Sync,
{
    let mut out = vec![T::default(); w * h * 4];
    if w == 0 || h == 0 {
        return out;
    }
    let bw = w.div_ceil(4);
    let stride = w * 4;
    let mut rows: Vec<(usize, &mut [T])> = out.chunks_mut(stride * 4).enumerate().collect();
    let run = |rows: &mut [(usize, &mut [T])]| {
        let mut px = [[T::default(); 4]; 16];
        for (by, chunk) in rows.iter_mut() {
            let rows_here = chunk.len() / stride;
            for bx in 0..bw {
                let off = (*by * bw + bx) * block_bytes;
                f(&src[off..off + block_bytes], &mut px);
                let cols = (w - bx * 4).min(4);
                for y in 0..rows_here {
                    for x in 0..cols {
                        let d = y * stride + (bx * 4 + x) * 4;
                        chunk[d..d + 4].copy_from_slice(&px[y * 4 + x]);
                    }
                }
            }
        }
    };
    let n = thread_count(threads, w * h).min(rows.len()).max(1);
    if n == 1 {
        run(&mut rows);
    } else {
        let per = rows.len().div_ceil(n);
        std::thread::scope(|s| {
            for part in rows.chunks_mut(per) {
                s.spawn(|| run(part));
            }
        });
    }
    out
}
