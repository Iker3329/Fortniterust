//! `fn-texture`: UE 4.16 `Texture2D` / `TextureCube` decoder (pure Rust).
//!
//! ```no_run
//! use fn_texture::{DecodeOptions, Texture2D};
//! # fn main() -> Result<(), fn_texture::Error> {
//! let (uasset, uexp, ubulk) = (vec![], vec![], vec![]);
//! let tex = Texture2D::from_files(&uasset, &uexp, &ubulk)?;
//! let mip = tex.best_mip().expect("no mip data");
//! let img = tex.decode(mip, DecodeOptions::default())?;
//! img.write_png("out.png")?;
//! # Ok(()) }
//! ```
//!
//! # Facts about Fortnite 1.8 (UE 4.16) cooked textures (all verified against real packages)
//!
//! * Packages are unversioned (file version 0); `fn-ueio` treats them as 4.16 = file version 513.
//! * Export layout of `Texture2D` / `TextureCube`: tagged properties, `i32 bSerializeGuid` (0), `FStripDataFlags`
//!   (2 bytes, `UTexture`), `FStripDataFlags` (2 bytes, `UTexture2D`), `i32 bCooked`, then a loop of
//!   `FName PixelFormat` (`None` terminates), `i32 SkipOffset` (4.16 has a single i32, an absolute offset in the
//!   `uasset ++ uexp` virtual file where the next FName starts), `FTexturePlatformData`:
//!   `i32 SizeX, i32 SizeY, i32 NumSlices, FString PixelFormat ("PF_DXT5"), i32 FirstMip,
//!   TArray<FTexture2DMipMap>` where a mip is `{ i32 bCooked, FByteBulkData{u32 flags, i32 elementCount,
//!   i32 sizeOnDisk, i64 offsetInFile}, i32 SizeX, i32 SizeY }`. Only the first platform data block is decoded
//!   (like UEViewer); the parser always resyncs on `SkipOffset`.
//! * Bulk data: `BULKDATA_PayloadAtEndOfFile (0x1)` payloads are at `offsetInFile + Summary.bulk_data_start_offset`
//!   in `uasset ++ uexp`; `BULKDATA_PayloadInSeperateFile (0x100)` payloads are at the same fixed-up offset inside
//!   the `.ubulk`; payloads without those flags follow the header inline. `BULKDATA_Unused (0x20)` / empty blocks are
//!   stripped mips. Streamed (`.ubulk`) mips are the large ones, the small tail of the mip chain is inline.
//! * Mip dimensions are `max(1, Mips[0].SizeX >> level)`, i.e. derived from the *first stored mip*, not from
//!   `FTexturePlatformData::SizeX`: `FirstMip` can be > 0 (the cooker dropped the top mips, e.g. a 2048x2048 texture
//!   whose array starts at 1024x1024; `T_MiniMapMask` has platform size 1024 and `FirstMip = 3`, so mip 0 is 128x128).
//!   UEViewer does the same, which is why the old PNGs of such textures are smaller than the nominal size. The
//!   per-mip stored size is wrong for the 1x1 / 2x2 mips of block formats (stored as 4x4) and is ignored.
//!   The mip exported by UEViewer is the *first mip with a payload*, see [`Texture2D::best_mip`].
//! * Inline mips carry flags `0x48` (`ForceInlinePayload | SingleUse`), streamed mips `0x501`
//!   (`PayloadAtEndOfFile | PayloadInSeperateFile | Force_NOT_InlinePayload`) with the data in the `.ubulk`.
//! * Classes: `Texture2D` and `TextureCube` are decoded. `TextureLightProfile` (IES, 24 in the paks),
//!   `TextureRenderTarget2D` (8) and `VolumeTexture` return [`Error::NotATexture`].
//! * Cube faces use UE's cube orientation (they look rotated by 90 degrees when saved as plain images); no
//!   re-orientation is applied.
//! * `NumSlices == 6` is a cube map: the six faces are concatenated inside each mip's payload (face `n` starts at
//!   `n * bytes_per_face`), order +X, -X, +Y, -Y, +Z, -Z.
//! * Block formats store whole 4x4 blocks even for mips smaller than 4x4 (`ceil(w/4) * ceil(h/4)` blocks).
//! * Properties: `SRGB` defaults to **true** when absent; `CompressionSettings` is an enum string such as
//!   `TC_Normalmap`, `TC_Masks`, `TC_Grayscale`, `TC_Default`; `AddressX/Y` default `TA_Wrap`.
//! * Orientation: row 0 is the top row; no flipping is needed to match UEViewer's PNG exports.
//!
//! # What exists in the Fortnite 1.8 paks (scan of all 112,462 `.uasset` name tables + export classes)
//!
//! 13,234 `Texture2D`, 12 `TextureCube`, 24 `TextureLightProfile`, 8 `TextureRenderTarget2D`. Pixel formats of the
//! `Texture2D`s: DXT1 6110, DXT5 3829, BC5 1878, B8G8R8A8 1085, FloatRGBA 295, G8 36, BC4 1. Cube maps: FloatRGBA 9,
//! DXT1 2, DXT5 1. **There is no BC6H, BC7, G16, R16F, V8U8 or A1 texture anywhere**, so those decoders are covered
//! by hand-built unit tests only (BC7 / BC6H follow detex but could not be compared against real data).
//!
//! # Decoder behaviour (matches UEViewer's NVTT based path, which produced the old PNG exports)
//!
//! * BC1/DXT1: 565 endpoints are bit-expanded to 888 (`(v << 3) | (v >> 2)`, `(g << 2) | (g >> 4)`), then
//!   interpolated with truncating integer division (`(2a+b)/3`, 3-colour blocks `(a+b)/2`). The mode is chosen by the
//!   raw 16-bit comparison `c0 > c1`. 3-colour blocks yield transparent black for index 3. UEViewer additionally
//!   zeroes RGB where alpha is 0 (only DXT1).
//! * BC2: explicit 4-bit alpha expanded as `(a << 4) | a`. BC3 / BC4 / BC5: 8-entry palette, 8-value mode when
//!   `a0 > a1` (`(k*a0 + (7-k)*a1)/7`, truncating), otherwise 6-value mode with explicit 0 and 255.
//! * BC4 decodes to grey RGB with alpha 255. BC5 decodes to (x, y, z, 255) where z is rebuilt like UEViewer's
//!   `buildNormal` (`nx = 2*x/255-1`, `ny = ...`, `nz = sqrt(1-nx²-ny²)` or 0, `z = clamp(int(255*(nz+1)/2))`);
//!   this is applied to *every* BC5 texture, not only those with `TC_Normalmap`. Disable via
//!   [`DecodeOptions::bc5_reconstruct_z`].
//! * BC6H: port of detex (unsigned variant), output is half float RGBA (alpha 1.0). BC7: detex algorithm.
//! * `PF_A8R8G8B8` is decoded exactly like `PF_B8G8R8A8`. `PF_G8` becomes grey with alpha 255. `PF_G16` (not supported
//!   by UEViewer) keeps the high byte. `PF_R16F` / `PF_R32_FLOAT` become half float (r, 0, 0, 1).
//! * Colour space: decoded values are the raw stored values; the `SRGB` flag is metadata only (UEViewer does not
//!   convert), and the exports are compared as stored.
//! * PNG output ([`DecodedImage::write_png`]) writes RGB when fully opaque (UEViewer's exporter also drops alpha
//!   when the image is fully transparent, which this crate does not do because it would lose data). HDR images are
//!   clamped to `[0, 1]` and written as 16 bit RGBA without tone mapping.

mod decode;
mod decoded;
mod error;
mod format;
mod texture;

pub use decode::{bc1235, bc6h, bc7, decode_mip, DecodeOptions, Rounding};
pub use decoded::{DecodedImage, Pixels};
pub use error::{Error, Result};
pub use format::PixelFormat;
pub use texture::{Mip, Texture2D};

#[cfg(test)]
mod tests;
