//! Textures of the original packages uploaded to the gun pass with their stored mip chain, and the sampler states of the material's sampler members.
//!
//! The pixel program binds, per `Texture2D_k` member of the Material uniform buffer, a texture register and, per sampler member, a sampler register. In UE 4.16 the
//! member layout is `Texture2D_k` = member `2k`, its asset sampler `Texture2D_kSampler` = member `2k + 1`, then `Wrap_WorldGroupSettings` = member `2 * N`,
//! `Clamp_WorldGroupSettings` = `2 * N + 1` (N = number of 2D texture expressions). The sampler state of an asset sampler is the texture's own addressing / `Filter`;
//! the world-group samplers are wrap / clamp with the default (anisotropic trilinear) world texture group filtering.
use fn_pak::PakSet;
use fn_render::gun_material::*;
use fn_render::fx::{FxLevel, FxTexFormat};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct TexInfo { pub filter: String, pub address_x: String, pub address_y: String, pub srgb: bool, pub width: usize, pub height: usize, pub levels: usize, pub compression: String }

#[derive(Default)]
pub struct TextureCache { map: HashMap<String, (GunTexId, TexInfo)> }

fn strip_enum(s: &str) -> String { s.rsplit("::").next().unwrap_or(s).to_owned() }
pub fn long_package(object_path: &str) -> &str { object_path.split('.').next().unwrap_or(object_path) }

/// The parsed `Texture2D` of `object_path` (2D only).
fn parse_2d(set: &PakSet, object_path: &str) -> Result<fn_texture::Texture2D, String> {
    let long = long_package(object_path);
    let a = set.read_game(long, "uasset").map_err(|e| format!("{long}: {e}"))?.into_owned();
    let x = set.read_game(long, "uexp").map(|c| c.into_owned()).unwrap_or_default();
    let b = set.read_game(long, "ubulk").map(|c| c.into_owned()).unwrap_or_default();
    let p = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| format!("{long}: {e}"))?;
    let t = fn_texture::Texture2D::parse(&p).map_err(|e| format!("{long}: {e}"))?;
    if t.is_cube() { return Err(format!("{long}: cube map where a 2D texture is bound")); }
    Ok(t)
}
fn info_of(t: &fn_texture::Texture2D, levels: &[(u32, u32, Vec<u8>)]) -> TexInfo {
    TexInfo { filter: t.props.text("Filter").map(strip_enum).unwrap_or_else(|| "TF_Default".into()), address_x: t.address_x.clone(), address_y: t.address_y.clone(), srgb: t.srgb, width: levels[0].0 as usize, height: levels[0].1 as usize, levels: levels.len(), compression: t.compression_settings.clone() }
}
/// Every stored mip of `t` decoded to RGBA8.
fn rgba_levels(t: &fn_texture::Texture2D, long: &str) -> Result<Vec<(u32, u32, Vec<u8>)>, String> {
    let mut levels = vec![];
    for i in t.available_mips() {
        let img = t.decode(i, fn_texture::DecodeOptions::default()).map_err(|e| format!("{long}: mip {i}: {e}"))?;
        levels.push((img.width as u32, img.height as u32, img.to_rgba8()));
    }
    if levels.is_empty() { return Err(format!("{long}: no mip available")); }
    Ok(levels)
}

/// Decodes the package of a 2D texture with every mip the cooked package stores (nothing is generated).
pub fn decode_2d(set: &PakSet, object_path: &str) -> Result<(GunTextureDesc, TexInfo), String> {
    let t = parse_2d(set, object_path)?;
    let levels = rgba_levels(&t, long_package(object_path))?;
    let info = info_of(&t, &levels);
    Ok((GunTextureDesc { label: t.name.clone(), srgb: t.srgb, levels }, info))
}

/// TEXTURE MEMORY (2026-10-06, BC passthrough): what the gun pass uploads for a 2D texture. The cooked BC blocks as they are when the device takes them
/// (`fn_fx::gpu::raw_bc_levels`: DXT1 / DXT3 / DXT5 / BC4 / BC5 / BC7 with a level-0 size that is a multiple of 4, `fn_render::fx::bc_native_hint`), else
/// the RGBA8 decode of [`decode_2d`]. The format goes alongside (`GunRenderer::create_texture_fmt`).
pub fn read_2d(set: &PakSet, object_path: &str) -> Result<(GunTextureDesc, TexInfo, FxTexFormat), String> {
    let long = long_package(object_path);
    let t = parse_2d(set, object_path)?;
    let (format, levels) = match fn_fx::gpu::raw_bc_levels(&t, &t.available_mips()) {
        Some((format, raw)) if !raw.is_empty() => (format, raw.into_iter().map(|l| (l.width, l.height, l.rgba8)).collect()),
        _ => (FxTexFormat::Rgba8, rgba_levels(&t, long)?),
    };
    let info = info_of(&t, &levels);
    Ok((GunTextureDesc { label: t.name.clone(), srgb: t.srgb, levels }, info, format))
}
/// TEXTURE MEMORY: BC levels of [`read_2d`] decoded to RGBA8 (the decode [`decode_2d`] does: `fn_fx::gpu::decode_bc_levels`), for a renderer that does not
/// take BC.
fn bc_to_rgba(d: GunTextureDesc, format: FxTexFormat) -> Result<GunTextureDesc, String> {
    let levels: Vec<FxLevel> = d.levels.into_iter().map(|(width, height, rgba8)| FxLevel { width, height, depth: 1, rgba8 }).collect();
    let rgba = fn_fx::gpu::decode_bc_levels(format, &levels).map_err(|e| format!("{}: {e}", d.label))?;
    Ok(GunTextureDesc { label: d.label, srgb: d.srgb, levels: rgba.into_iter().map(|l| (l.width, l.height, l.rgba8)).collect() })
}

/// FIRST USE (2026-10-06): mip chains read on a worker ahead of [`TextureCache::load`] (`crate::gun::prepare_*`), object path -> result, consumed once.
/// TEXTURE MEMORY: the [`read_2d`] result (BC blocks or RGBA8, with the format), i.e. what `load` uploads.
static PREDECODED: std::sync::Mutex<Option<HashMap<String, Result<(GunTextureDesc, TexInfo, FxTexFormat), String>>>> = std::sync::Mutex::new(None);
static PREDECODED_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Byte budget of the held chains (decoded RGBA8, or the raw blocks of a BC texture: 4-8x smaller); past it a texture decodes on demand as before. Smaller in
/// the browser build, whose heap is shared with everything else (4 GB wasm32 limit).
const PREDECODE_BUDGET: usize = if cfg!(target_arch = "wasm32") { 96 << 20 } else { 384 << 20 };
fn held_bytes(r: &Result<(GunTextureDesc, TexInfo, FxTexFormat), String>) -> usize { r.as_ref().map_or(0, |(d, _, _)| d.levels.iter().map(|l| l.2.len()).sum()) }

/// Reads (decodes, or copies the BC blocks of) `object_path` on the calling (worker) thread so that a later [`TextureCache::load`] only uploads it. Skipped
/// past [`PREDECODE_BUDGET`].
pub fn predecode_2d(set: &PakSet, object_path: &str) {
    use std::sync::atomic::Ordering::Relaxed;
    if PREDECODED_BYTES.load(Relaxed) >= PREDECODE_BUDGET { return; }
    if UPLOADED.lock().ok().and_then(|g| g.as_ref().map(|s| s.contains(object_path))).unwrap_or(false) { return; } // already on the GPU: nothing would take it
    if PREDECODED.lock().ok().and_then(|g| g.as_ref().map(|m| m.contains_key(object_path))).unwrap_or(false) { return; }
    let r = read_2d(set, object_path);
    if let Ok(mut g) = PREDECODED.lock() {
        let m = g.get_or_insert_with(HashMap::new);
        if !m.contains_key(object_path) { PREDECODED_BYTES.fetch_add(held_bytes(&r), Relaxed); m.insert(object_path.to_owned(), r); }
    }
}
/// Paths some [`TextureCache`] uploaded (a predecode of them would never be taken).
static UPLOADED: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);
fn take_predecoded(object_path: &str) -> Option<Result<(GunTextureDesc, TexInfo, FxTexFormat), String>> {
    let r = PREDECODED.lock().ok()?.as_mut()?.remove(object_path)?;
    PREDECODED_BYTES.fetch_sub(held_bytes(&r), std::sync::atomic::Ordering::Relaxed);
    Some(r)
}

impl TextureCache {
    /// True when `object_path` is already uploaded (a prepare step can skip decoding it).
    pub fn has(&self, object_path: &str) -> bool { self.map.contains_key(object_path) }
    pub fn load(&mut self, set: &PakSet, gun: &mut GunRenderer, object_path: &str) -> Result<(GunTexId, TexInfo), String> {
        if let Some(t) = self.map.get(object_path) { let _ = take_predecoded(object_path); return Ok(t.clone()); } // a late predecode of an uploaded texture is dropped
        let (desc, info, format) = match take_predecoded(object_path) { Some(r) => r?, None => read_2d(set, object_path)? };
        // TEXTURE MEMORY: blocks read while the hint said BC, for a renderer that does not take them: the RGBA8 decode, here
        let (desc, format) = if format.is_bc() && !gun.accepts_bc(info.width as u32, info.height as u32) { (bc_to_rgba(desc, format)?, FxTexFormat::Rgba8) } else { (desc, format) };
        if let Ok(mut g) = UPLOADED.lock() { g.get_or_insert_with(Default::default).insert(object_path.to_owned()); }
        let id = gun.create_texture_fmt(&desc, format);
        self.map.insert(object_path.to_owned(), (id, info.clone()));
        Ok((id, info))
    }
}

fn address(s: &str) -> GunAddress { match s { "TA_Clamp" => GunAddress::Clamp, "TA_Mirror" => GunAddress::Mirror, _ => GunAddress::Wrap } }
fn filtered(address: GunAddress, filter: &str) -> GunSamplerDesc {
    use GunFilter::*;
    match filter {
        "TF_Nearest" => GunSamplerDesc { mag: Nearest, min: Nearest, mip: Nearest, address, aniso: 1 },
        "TF_Bilinear" => GunSamplerDesc { mag: Linear, min: Linear, mip: Nearest, address, aniso: 1 },
        "TF_Trilinear" => GunSamplerDesc { mag: Linear, min: Linear, mip: Linear, address, aniso: 1 },
        _ => GunSamplerDesc { mag: Linear, min: Linear, mip: Linear, address, aniso: 8 },
    }
}
/// `Texture2D_kSampler`: the texture asset's own state.
pub fn asset_sampler(info: &TexInfo) -> GunSamplerDesc { filtered(address(&info.address_x), &info.filter) }
/// `Wrap_WorldGroupSettings` / `Clamp_WorldGroupSettings`: the world texture group's default filtering.
pub fn world_group_sampler(clamp: bool) -> GunSamplerDesc { filtered(if clamp { GunAddress::Clamp } else { GunAddress::Wrap }, "TF_Default") }

/// What a sampler member of the Material uniform buffer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplerMember { Asset(usize), WrapWorld, ClampWorld }
pub fn sampler_member(member: usize, texture_count: usize) -> Result<SamplerMember, String> {
    if member < 2 * texture_count { if member % 2 == 1 { Ok(SamplerMember::Asset(member / 2)) } else { Err(format!("member {member} is a texture, not a sampler")) } }
    else if member == 2 * texture_count { Ok(SamplerMember::WrapWorld) }
    else if member == 2 * texture_count + 1 { Ok(SamplerMember::ClampWorld) }
    else { Err(format!("sampler member {member} beyond the material's {texture_count} textures + 2 world-group samplers")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn sampler_member_layout() {
        assert_eq!(sampler_member(1, 5), Ok(SamplerMember::Asset(0)));
        assert_eq!(sampler_member(9, 5), Ok(SamplerMember::Asset(4)));
        assert_eq!(sampler_member(10, 5), Ok(SamplerMember::WrapWorld));
        assert_eq!(sampler_member(11, 5), Ok(SamplerMember::ClampWorld));
        assert!(sampler_member(12, 5).is_err() && sampler_member(2, 5).is_err());
    }
}
