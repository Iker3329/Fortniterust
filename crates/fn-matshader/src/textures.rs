//! Textures of the original packages uploaded to the gun pass with their stored mip chain, and the sampler states of the material's sampler members.
//!
//! The pixel program binds, per `Texture2D_k` member of the Material uniform buffer, a texture register and, per sampler member, a sampler register. In UE 4.16 the
//! member layout is `Texture2D_k` = member `2k`, its asset sampler `Texture2D_kSampler` = member `2k + 1`, then `Wrap_WorldGroupSettings` = member `2 * N`,
//! `Clamp_WorldGroupSettings` = `2 * N + 1` (N = number of 2D texture expressions). The sampler state of an asset sampler is the texture's own addressing / `Filter`;
//! the world-group samplers are wrap / clamp with the default (anisotropic trilinear) world texture group filtering.
use fn_pak::PakSet;
use fn_render::gun_material::*;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct TexInfo { pub filter: String, pub address_x: String, pub address_y: String, pub srgb: bool, pub width: usize, pub height: usize, pub levels: usize, pub compression: String }

#[derive(Default)]
pub struct TextureCache { map: HashMap<String, (GunTexId, TexInfo)> }

fn strip_enum(s: &str) -> String { s.rsplit("::").next().unwrap_or(s).to_owned() }
pub fn long_package(object_path: &str) -> &str { object_path.split('.').next().unwrap_or(object_path) }

impl TextureCache {
    pub fn load(&mut self, set: &PakSet, gun: &mut GunRenderer, object_path: &str) -> Result<(GunTexId, TexInfo), String> {
        if let Some(t) = self.map.get(object_path) { return Ok(t.clone()); }
        let long = long_package(object_path);
        let a = set.read_game(long, "uasset").map_err(|e| format!("{long}: {e}"))?.into_owned();
        let x = set.read_game(long, "uexp").map(|c| c.into_owned()).unwrap_or_default();
        let b = set.read_game(long, "ubulk").map(|c| c.into_owned()).unwrap_or_default();
        let p = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| format!("{long}: {e}"))?;
        let t = fn_texture::Texture2D::parse(&p).map_err(|e| format!("{long}: {e}"))?;
        if t.is_cube() { return Err(format!("{long}: cube map where a 2D texture is bound")); }
        let mut levels = vec![];
        for i in t.available_mips() {
            let img = t.decode(i, fn_texture::DecodeOptions::default()).map_err(|e| format!("{long}: mip {i}: {e}"))?;
            levels.push((img.width as u32, img.height as u32, img.to_rgba8()));
        }
        if levels.is_empty() { return Err(format!("{long}: no mip available")); }
        let info = TexInfo { filter: t.props.text("Filter").map(strip_enum).unwrap_or_else(|| "TF_Default".into()), address_x: t.address_x.clone(), address_y: t.address_y.clone(), srgb: t.srgb, width: levels[0].0 as usize, height: levels[0].1 as usize, levels: levels.len(), compression: t.compression_settings.clone() };
        let id = gun.create_texture(&GunTextureDesc { label: t.name.clone(), srgb: t.srgb, levels });
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
