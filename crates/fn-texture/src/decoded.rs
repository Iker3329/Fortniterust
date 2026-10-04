use crate::{Error, Result};
use fn_ueio::f16_to_f32;
use std::path::Path;

/// Decoded pixel storage.
#[derive(Debug, Clone, PartialEq)]
pub enum Pixels {
    /// 8 bit RGBA, `width * height * 4` bytes, row 0 = top.
    Rgba8(Vec<u8>),
    /// IEEE half float RGBA bit patterns, `width * height * 4` values, row 0 = top.
    RgbaF16(Vec<u16>),
}

/// A decoded mip (or cube face).
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Pixels,
}

impl DecodedImage {
    /// Convert to RGBA8. Half-float data is clamped to `[0, 1]` and scaled by 255 with rounding (no tone mapping, no
    /// gamma); NaN becomes 0.
    pub fn to_rgba8(&self) -> Vec<u8> {
        match &self.pixels {
            Pixels::Rgba8(v) => v.clone(),
            Pixels::RgbaF16(v) => v.iter().map(|&h| unit_to_u8(f16_to_f32(h))).collect(),
        }
    }

    /// `true` if every alpha value is 255 (1.0 for floats).
    pub fn is_opaque(&self) -> bool {
        match &self.pixels {
            Pixels::Rgba8(v) => v.chunks_exact(4).all(|p| p[3] == 255),
            Pixels::RgbaF16(v) => v.chunks_exact(4).all(|p| f16_to_f32(p[3]) >= 1.0),
        }
    }

    /// Write a PNG file.
    ///
    /// * `Rgba8` images become 8 bit RGB (when every alpha is 255, like UEViewer's exporter) or RGBA.
    /// * `RgbaF16` (HDR) images are *not* tone mapped: channels are clamped to `[0, 1]` and written as 16 bit RGBA.
    pub fn write_png<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        use image::ImageEncoder;
        let out = |e: std::io::Error| Error::Output(e.to_string());
        let file = std::fs::File::create(path.as_ref()).map_err(out)?;
        let w = std::io::BufWriter::new(file);
        let enc = image::codecs::png::PngEncoder::new(w);
        let (wd, ht) = (self.width as u32, self.height as u32);
        let map = |e: image::ImageError| Error::Output(e.to_string());
        match &self.pixels {
            Pixels::Rgba8(v) => {
                if self.is_opaque() {
                    let rgb: Vec<u8> = v.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
                    enc.write_image(&rgb, wd, ht, image::ExtendedColorType::Rgb8).map_err(map)
                } else {
                    enc.write_image(v, wd, ht, image::ExtendedColorType::Rgba8).map_err(map)
                }
            }
            Pixels::RgbaF16(v) => {
                let mut bytes = Vec::with_capacity(v.len() * 2);
                for &h in v {
                    let u = (f16_to_f32(h).clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                    bytes.extend_from_slice(&u.to_ne_bytes());
                }
                // `image` expects native-endian samples for 16 bit data
                enc.write_image(&bytes, wd, ht, image::ExtendedColorType::Rgba16).map_err(map)
            }
        }
    }
}

fn unit_to_u8(f: f32) -> u8 {
    if f.is_nan() {
        0
    } else {
        (f.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    }
}
