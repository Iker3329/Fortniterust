/// Pixel formats (`EPixelFormat`, serialised as the string `PF_*`) that appear in cooked UE 4 textures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PixelFormat {
    /// `PF_DXT1` = BC1, 8 bytes per 4x4 block.
    Dxt1,
    /// `PF_DXT3` = BC2, 16 bytes per block.
    Dxt3,
    /// `PF_DXT5` = BC3, 16 bytes per block.
    Dxt5,
    /// `PF_BC4`, 8 bytes per block (single channel).
    Bc4,
    /// `PF_BC5`, 16 bytes per block (two channels, normal maps).
    Bc5,
    /// `PF_BC6H`, 16 bytes per block (HDR, unsigned half float RGB).
    Bc6h,
    /// `PF_BC7`, 16 bytes per block.
    Bc7,
    /// `PF_B8G8R8A8` (bytes in memory: B, G, R, A).
    B8G8R8A8,
    /// `PF_A8R8G8B8` (UE 3 name; UEViewer decodes it exactly like B8G8R8A8).
    A8R8G8B8,
    /// `PF_R8G8B8A8` (bytes in memory: R, G, B, A).
    R8G8B8A8,
    /// `PF_G8`, 8 bit grey.
    G8,
    /// `PF_G16`, 16 bit grey (little endian).
    G16,
    /// `PF_FloatRGBA`, four IEEE half floats (8 bytes per pixel).
    FloatRGBA,
    /// `PF_R16F` / `PF_R16F_FILTER`, one half float.
    R16F,
    /// `PF_R32_FLOAT`, one f32.
    R32F,
    /// `PF_V8U8`, two signed bytes (u, v).
    V8U8,
    /// `PF_A1`, one bit per pixel (never seen in practice).
    A1,
    /// Any other format; the contained string is the stored name.
    Other(String),
}

impl PixelFormat {
    /// Parse the `PF_*` string stored in `FTexturePlatformData`.
    pub fn from_name(name: &str) -> PixelFormat {
        match name {
            "PF_DXT1" => PixelFormat::Dxt1,
            "PF_DXT3" => PixelFormat::Dxt3,
            "PF_DXT5" => PixelFormat::Dxt5,
            "PF_BC4" => PixelFormat::Bc4,
            "PF_BC5" => PixelFormat::Bc5,
            "PF_BC6H" => PixelFormat::Bc6h,
            "PF_BC7" => PixelFormat::Bc7,
            "PF_B8G8R8A8" => PixelFormat::B8G8R8A8,
            "PF_A8R8G8B8" => PixelFormat::A8R8G8B8,
            "PF_R8G8B8A8" => PixelFormat::R8G8B8A8,
            "PF_G8" => PixelFormat::G8,
            "PF_G16" => PixelFormat::G16,
            "PF_FloatRGBA" => PixelFormat::FloatRGBA,
            "PF_R16F" | "PF_R16F_FILTER" => PixelFormat::R16F,
            "PF_R32_FLOAT" => PixelFormat::R32F,
            "PF_V8U8" => PixelFormat::V8U8,
            "PF_A1" => PixelFormat::A1,
            other => PixelFormat::Other(other.to_string()),
        }
    }

    /// The canonical `PF_*` name.
    pub fn name(&self) -> &str {
        match self {
            PixelFormat::Dxt1 => "PF_DXT1",
            PixelFormat::Dxt3 => "PF_DXT3",
            PixelFormat::Dxt5 => "PF_DXT5",
            PixelFormat::Bc4 => "PF_BC4",
            PixelFormat::Bc5 => "PF_BC5",
            PixelFormat::Bc6h => "PF_BC6H",
            PixelFormat::Bc7 => "PF_BC7",
            PixelFormat::B8G8R8A8 => "PF_B8G8R8A8",
            PixelFormat::A8R8G8B8 => "PF_A8R8G8B8",
            PixelFormat::R8G8B8A8 => "PF_R8G8B8A8",
            PixelFormat::G8 => "PF_G8",
            PixelFormat::G16 => "PF_G16",
            PixelFormat::FloatRGBA => "PF_FloatRGBA",
            PixelFormat::R16F => "PF_R16F",
            PixelFormat::R32F => "PF_R32_FLOAT",
            PixelFormat::V8U8 => "PF_V8U8",
            PixelFormat::A1 => "PF_A1",
            PixelFormat::Other(s) => s,
        }
    }

    /// `true` if the format is a 4x4 block compressed format.
    pub fn is_block_compressed(&self) -> bool {
        matches!(self, PixelFormat::Dxt1 | PixelFormat::Dxt3 | PixelFormat::Dxt5 | PixelFormat::Bc4 | PixelFormat::Bc5 | PixelFormat::Bc6h | PixelFormat::Bc7)
    }

    /// `true` if decoding yields half-float output ([`crate::Pixels::RgbaF16`]).
    pub fn is_float(&self) -> bool {
        matches!(self, PixelFormat::Bc6h | PixelFormat::FloatRGBA | PixelFormat::R16F | PixelFormat::R32F)
    }

    /// Bytes of one 4x4 block (block formats) or one pixel (linear formats); `None` for unsupported formats and
    /// for `A1` (which is bit-packed).
    pub fn bytes_per_unit(&self) -> Option<usize> {
        Some(match self {
            PixelFormat::Dxt1 | PixelFormat::Bc4 => 8,
            PixelFormat::Dxt3 | PixelFormat::Dxt5 | PixelFormat::Bc5 | PixelFormat::Bc6h | PixelFormat::Bc7 => 16,
            PixelFormat::B8G8R8A8 | PixelFormat::A8R8G8B8 | PixelFormat::R8G8B8A8 | PixelFormat::R32F => 4,
            PixelFormat::G8 => 1,
            PixelFormat::G16 | PixelFormat::R16F | PixelFormat::V8U8 => 2,
            PixelFormat::FloatRGBA => 8,
            PixelFormat::A1 | PixelFormat::Other(_) => return None,
        })
    }

    /// Size in bytes of one slice of a `w` x `h` mip.
    pub fn mip_bytes(&self, w: usize, h: usize) -> Option<usize> {
        if let PixelFormat::A1 = self {
            return Some(w.div_ceil(8) * h);
        }
        let unit = self.bytes_per_unit()?;
        Some(if self.is_block_compressed() { w.div_ceil(4) * h.div_ceil(4) * unit } else { w * h * unit })
    }
}
