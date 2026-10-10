//! `*.mat.json` of the exported props and the PNG textures they reference.
use crate::util::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    /// `BLEND_Masked`: pixels with opacity below `alpha_cutoff` are discarded (foliage cards).
    Masked,
    /// `BLEND_Translucent` / `BLEND_Additive` / `BLEND_Modulate`.
    Blend,
}

/// One texture parameter of the material after resolving the instance chain.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureRef {
    /// Material parameter name (`Diffuse`, `NormalMap`, ...).
    pub param: String,
    /// Texture asset name (`T_Tree_Pine01_D`).
    pub name: String,
    /// Absolute path of the exported PNG (RGBA8).
    pub png: PathBuf,
    /// `Texture2D.SRGB` (colour textures true; normal / mask textures false).
    pub srgb: bool,
    /// `TC_Default`, `TC_Normalmap`, `TC_Masks`, ...
    pub compression: String,
    /// Dimensions of the exported PNG and of the cooked top mip it was reduced from.
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub address_x: String,
    pub address_y: String,
    /// `false` when every pixel of the PNG is opaque.
    pub has_alpha: bool,
}

/// Which texture parameters play which role. **Heuristic** (the cooked packages keep only parameter nodes of the
/// material graph, not the arithmetic between them): chosen from parameter names and texture compression settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Slots {
    pub base_color: Option<String>,
    pub normal: Option<String>,
    pub mask: Option<String>,
    /// `base_color` (alpha channel of the colour texture), `mask`, or `None` for opaque materials.
    pub opacity_from: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    pub ue_path: String,
    /// The base `UMaterial` at the end of the instance chain (`root_material`, e.g. `/Game/.../Fortress_BaseMaterial`).
    pub root_material: String,
    pub blend_mode: String,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub two_sided: bool,
    pub shading_model: String,
    pub textures: BTreeMap<String, TextureRef>,
    pub slots: Slots,
    pub scalars: BTreeMap<String, f32>,
    pub vectors: BTreeMap<String, [f32; 4]>,
    pub static_switches: BTreeMap<String, bool>,
    /// Material graph drives `WorldPositionOffset` (wind / foliage animation in the vertex shader).
    pub uses_world_position_offset: bool,
    /// Scalar / vector parameters whose names look like wind / leaf animation controls, e.g. `LeafAnimationAmount`.
    pub wind_params: BTreeMap<String, f32>,
    /// Linear RGB multiplier for canopy textures authored in grey (normalised gradient / subsurface colour; a GUESS), None for coloured textures.
    pub tint_guess: Option<[f32; 3]>,
}

impl Material {
    pub fn load(path: &Path) -> Result<Material> {
        let j = read_json(path)?;
        Material::from_json(&j, path.parent().unwrap_or(Path::new(".")))
    }

    /// `base` is the directory the `png` entries are relative to (the folder of the `.mat.json`).
    pub fn from_json(j: &Value, base: &Path) -> Result<Material> {
        let name = req_str(j, "name", "material")?;
        let blend_mode = str_of(j, "blend_mode").unwrap_or_else(|| "BLEND_Opaque".into());
        let alpha_mode = match str_of(j, "alpha_mode").as_deref() {
            Some("MASK") => AlphaMode::Masked,
            Some("BLEND") => AlphaMode::Blend,
            _ => AlphaMode::Opaque,
        };
        let mut textures = BTreeMap::new();
        if let Some(t) = j.get("textures").and_then(|x| x.as_object()) {
            for (param, e) in t {
                let png = req_str(e, "png", &format!("{name}.textures.{param}"))?;
                textures.insert(
                    param.clone(),
                    TextureRef {
                        param: param.clone(),
                        name: str_of(e, "texture").unwrap_or_default(),
                        png: base.join(png),
                        srgb: bool_of(e, "srgb").unwrap_or(true),
                        compression: str_of(e, "compression").unwrap_or_default(),
                        width: u32_of(e, "width").unwrap_or(0),
                        height: u32_of(e, "height").unwrap_or(0),
                        source_width: u32_of(e, "source_width").unwrap_or(0),
                        source_height: u32_of(e, "source_height").unwrap_or(0),
                        address_x: str_of(e, "address_x").unwrap_or_else(|| "TA_Wrap".into()),
                        address_y: str_of(e, "address_y").unwrap_or_else(|| "TA_Wrap".into()),
                        has_alpha: bool_of(e, "has_alpha").unwrap_or(true),
                    },
                );
            }
        }
        let s = j.get("slots");
        let slot = |k: &str| s.and_then(|s| str_of(s, k));
        let map_f = |k: &str| -> BTreeMap<String, f32> { j.get(k).and_then(|x| x.as_object()).map(|o| o.iter().filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f as f32))).collect()).unwrap_or_default() };
        Ok(Material {
            ue_path: str_of(j, "ue_path").unwrap_or_default(),
            root_material: str_of(j, "root_material").unwrap_or_default(),
            alpha_cutoff: f32_of(j, "opacity_mask_clip_value").unwrap_or(0.3333),
            two_sided: bool_of(j, "two_sided").unwrap_or(false),
            shading_model: str_of(j, "shading_model").unwrap_or_default(),
            textures,
            slots: Slots { base_color: slot("base_color"), normal: slot("normal"), mask: slot("mask"), opacity_from: slot("opacity_from") },
            scalars: map_f("scalars"),
            vectors: j.get("vectors").and_then(|x| x.as_object()).map(|o| o.iter().filter_map(|(k, v)| arr_f32::<4>(Some(v)).map(|a| (k.clone(), a))).collect()).unwrap_or_default(),
            static_switches: j.get("static_switches").and_then(|x| x.as_object()).map(|o| o.iter().filter_map(|(k, v)| v.as_bool().map(|b| (k.clone(), b))).collect()).unwrap_or_default(),
            uses_world_position_offset: j.get("wind").and_then(|w| bool_of(w, "uses_world_position_offset")).unwrap_or(false),
            wind_params: j.get("wind").and_then(|w| w.get("parameters")).and_then(|x| x.as_object()).map(|o| o.iter().filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f as f32))).collect()).unwrap_or_default(),
            tint_guess: arr_f32::<3>(j.get("tint_guess")),
            name,
            blend_mode,
            alpha_mode,
        })
    }

    pub fn base_color_texture(&self) -> Option<&TextureRef> {
        self.slots.base_color.as_ref().and_then(|p| self.textures.get(p))
    }
    pub fn normal_texture(&self) -> Option<&TextureRef> {
        self.slots.normal.as_ref().and_then(|p| self.textures.get(p))
    }
    pub fn mask_texture(&self) -> Option<&TextureRef> {
        self.slots.mask.as_ref().and_then(|p| self.textures.get(p))
    }
}

/// A decoded RGBA8 image (`image` crate PNG decoder).
#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row 0 = top.
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn load(path: &Path) -> Result<Image> {
        let im = std::fs::read(path).map_err(image::ImageError::IoError).and_then(|b| image::load_from_memory(&b)).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?.to_rgba8();
        Ok(Image { width: im.width(), height: im.height(), rgba: im.into_raw() })
    }
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y.min(self.height - 1) * self.width + x.min(self.width - 1)) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
}

impl TextureRef {
    pub fn load_image(&self) -> Result<Image> {
        Image::load(&self.png)
    }
}
