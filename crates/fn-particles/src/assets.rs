//! The exported particle asset tree (`<assets>/particles`, written by `fn-particles-export`) and the material "look" derived from it.
//!
//! ```text
//! particles/index.json            { systems: { "<package path>": "<Name>" }, meshes: {...}, pickaxe_impact: { surface_effects: { "4": "<package>.<object>" } } }
//! particles/systems/<Name>.json   ParticleSystem
//! particles/materials/<Name>.json MaterialDef
//! particles/meshes/<Name>.json    MeshDef (game space)
//! particles/textures/<Name>.png   RGBA8
//! ```
use crate::model::{MaterialDef, MeshDef, ParticleSystem};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Catalog {
    pub root: PathBuf,
    systems: HashMap<String, Arc<ParticleSystem>>,
    by_name: HashMap<String, Arc<ParticleSystem>>,
    materials: HashMap<String, Arc<MaterialDef>>,
    meshes: HashMap<String, Arc<MeshDef>>,
    mesh_by_path: HashMap<String, String>,
    /// `ImpactPhysicalSurfaceEffects[surface]` of the pickaxe impact blueprint -> package path.
    pub impact_effects: HashMap<u32, String>,
}

fn pkg(path: &str) -> &str { path.split('.').next().unwrap_or(path) }
fn short(path: &str) -> &str { path.rsplit(|c| c == '.' || c == '/').next().unwrap_or(path) }
fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T, String> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))
}

impl Catalog {
    /// Loads `<dir>` (the `particles` folder). Errors when there is no `index.json`; missing individual files are skipped.
    pub fn load(dir: &Path) -> Result<Catalog, String> {
        let index: serde_json::Value = read_json(&dir.join("index.json"))?;
        let mut c = Catalog { root: dir.to_path_buf(), systems: HashMap::new(), by_name: HashMap::new(), materials: HashMap::new(), meshes: HashMap::new(), mesh_by_path: HashMap::new(), impact_effects: HashMap::new() };
        for (package, name) in index["systems"].as_object().into_iter().flatten() {
            let Some(name) = name.as_str() else { continue };
            if let Ok(s) = read_json::<ParticleSystem>(&dir.join("systems").join(format!("{name}.json"))) { let a = Arc::new(s); c.systems.insert(package.clone(), a.clone()); c.by_name.insert(name.to_string(), a); }
        }
        for (path, name) in index["meshes"].as_object().into_iter().flatten() { if let Some(n) = name.as_str() { c.mesh_by_path.insert(pkg(path).to_string(), n.to_string()); } }
        for e in std::fs::read_dir(dir.join("materials")).into_iter().flatten().flatten() {
            if let Ok(m) = read_json::<MaterialDef>(&e.path()) { c.materials.insert(pkg(&m.ue_path).to_string(), Arc::new(m)); }
        }
        for e in std::fs::read_dir(dir.join("meshes")).into_iter().flatten().flatten() {
            if let Ok(m) = read_json::<MeshDef>(&e.path()) { c.meshes.insert(m.name.clone(), Arc::new(m)); }
        }
        for (k, v) in index["pickaxe_impact"]["surface_effects"].as_object().into_iter().flatten() {
            if let (Ok(k), Some(v)) = (k.parse::<u32>(), v.as_str()) { c.impact_effects.insert(k, pkg(v).to_string()); }
        }
        Ok(c)
    }
    /// A system by package path (`/Game/.../P_X`), object path (`/Game/.../P_X.P_X`) or short name.
    pub fn system(&self, path: &str) -> Option<Arc<ParticleSystem>> {
        self.systems.get(pkg(path)).or_else(|| self.by_name.get(short(path))).cloned()
    }
    pub fn system_count(&self) -> usize { self.systems.len() }
    pub fn material(&self, ue_path: &str) -> Option<Arc<MaterialDef>> { self.materials.get(pkg(ue_path)).cloned() }
    pub fn mesh(&self, ue_path: &str) -> Option<Arc<MeshDef>> { self.mesh_by_path.get(pkg(ue_path)).and_then(|n| self.meshes.get(n)).cloned().or_else(|| self.meshes.get(short(ue_path)).cloned()) }
    pub fn texture_path(&self, png: &str) -> PathBuf { self.root.join(png) }
    pub fn systems(&self) -> impl Iterator<Item = &Arc<ParticleSystem>> { self.by_name.values() }
}

// ---- material look -----------------------------------------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend { Opaque, Masked, Translucent, Additive, Modulate }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel { R, G, B, A }
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ramp {
    None,
    /// `lerp(dark, light, value)`.
    Two { dark: [f32; 3], light: [f32; 3] },
    /// darks -> mids (0..0.5) -> highlights (0.5..1).
    Three { darks: [f32; 3], mids: [f32; 3], highlights: [f32; 3] },
}

/// What the renderer needs to draw a particle with this material. Derived from [`MaterialDef`] with rules driven by the exported data; where the cooked graph does not say
/// (the arithmetic nodes are stripped) the rule is named in `notes`.
#[derive(Clone, Debug)]
pub struct Look {
    pub name: String,
    pub blend: Blend,
    pub unlit: bool,
    pub two_sided: bool,
    pub clip: f32,
    /// PNG path relative to the particle root, if the material has a colour texture.
    pub texture: Option<String>,
    pub texture_srgb: bool,
    /// Where the opacity comes from: `None` = only the particle alpha, else a texture channel.
    pub opacity: Option<Channel>,
    /// Which channel drives the ramp (when `ramp` is not `None`) or is the grey value of the colour.
    pub value_channel: Option<Channel>,
    pub ramp: Ramp,
    pub opacity_exponent: f32,
    /// Self-lit share of the base colour (scalar parameter `Emissive Boost`, the material has an `EmissiveColor` input; how the graph combines it is not recoverable: added as `base * boost`).
    pub emissive: f32,
    pub notes: Vec<String>,
}

fn pick_channel(v: &[f32; 4]) -> Channel { let m = v[0].max(v[1]).max(v[2]).max(v[3]); if m <= 0.0 { Channel::R } else if v[0] == m { Channel::R } else if v[1] == m { Channel::G } else if v[2] == m { Channel::B } else { Channel::A } }
fn rgb(v: &[f32; 4]) -> [f32; 3] { [v[0], v[1], v[2]] }

impl Look {
    pub fn from_material(m: &MaterialDef) -> Look {
        let blend = if m.is_additive() { Blend::Additive } else if m.is_modulate() { Blend::Modulate } else if m.is_translucent() { Blend::Translucent } else if m.is_masked() { Blend::Masked } else { Blend::Opaque };
        let tex = m.color_texture.as_ref().and_then(|p| m.textures.get(p));
        let mut look = Look { name: m.name.clone(), blend, unlit: m.is_unlit(), two_sided: m.two_sided, clip: m.clip, texture: tex.map(|t| t.png.clone()), texture_srgb: tex.map_or(true, |t| t.srgb), opacity: None, value_channel: None, ramp: Ramp::None, opacity_exponent: 1.0, emissive: m.scalars.get("Emissive Boost").copied().unwrap_or(0.0), notes: vec![] };
        if let Some(t) = tex {
            look.opacity = if t.has_alpha && !matches!(blend, Blend::Opaque) { Some(Channel::A) } else { None };
        }
        // smoke family: `OpacityMaskPicker` / `DiffuseMaskPicker` (channel masks) + `<n> - Daytime Darks / Mids / Highlights Smoke` ramps
        if let (Some(op), Some(df)) = (m.vectors.get("OpacityMaskPicker"), m.vectors.get("DiffuseMaskPicker")) {
            look.opacity = Some(pick_channel(op)); look.value_channel = Some(pick_channel(df));
            look.opacity_exponent = m.scalars.get("Opacity Exponent").copied().unwrap_or(1.0).max(0.01);
            let get = |k: &str| m.vectors.get(k).map(rgb);
            if let (Some(d), Some(mi), Some(h)) = (get("2 - Daytime Darks Smoke"), get("2 - Daytime Mids Smoke"), get("2 - Daytime Highlights Smoke")) {
                look.ramp = Ramp::Three { darks: d, mids: mi, highlights: h };
                look.notes.push("smoke: opacity / diffuse channels from OpacityMaskPicker / DiffuseMaskPicker, colour ramp = the daytime vectors (name-driven; the graph arithmetic is stripped)".into());
            }
        }
        // impact flash family: DarkCol / LightCol lerped by the texture
        if let (Some(d), Some(l)) = (m.vectors.get("DarkCol"), m.vectors.get("LightCol")) {
            look.ramp = Ramp::Two { dark: rgb(d), light: rgb(l) }; look.value_channel = Some(if look.opacity == Some(Channel::A) { Channel::A } else { Channel::R });
            if look.opacity.is_none() { look.opacity = look.value_channel; }
            look.notes.push("flash: colour = lerp(DarkCol, LightCol, texture), opacity from the same channel (name-driven)".into());
        }
        if look.texture.is_some() && look.opacity.is_none() && matches!(blend, Blend::Translucent | Blend::Additive | Blend::Masked) && look.value_channel.is_none() {
            look.opacity = Some(Channel::R); look.notes.push("opacity taken from the red channel of the colour texture (the texture has no alpha)".into());
        }
        look
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TextureRef;
    fn tex(name: &str, alpha: bool) -> TextureRef { TextureRef { name: name.into(), ue_path: String::new(), png: format!("textures/{name}.png"), srgb: true, compression: "TC_Default".into(), width: 8, height: 8, has_alpha: alpha } }
    #[test] fn smoke_pickers_and_daytime_ramp() {
        let mut m = MaterialDef { blend_mode: "BLEND_Translucent".into(), shading_model: "MSM_Unlit".into(), ..Default::default() };
        m.textures.insert("Smoke Map SubUV".into(), tex("T_SmokePoof_8x8_01", false)); m.color_texture = Some("Smoke Map SubUV".into());
        m.vectors.insert("OpacityMaskPicker".into(), [1., 0., 0., 0.]); m.vectors.insert("DiffuseMaskPicker".into(), [0., 1., 0., 0.]);
        for (k, v) in [("2 - Daytime Darks Smoke", 0.1), ("2 - Daytime Mids Smoke", 0.4), ("2 - Daytime Highlights Smoke", 0.8)] { m.vectors.insert(k.into(), [v, v, v, 1.0]); }
        let l = Look::from_material(&m);
        assert_eq!(l.blend, Blend::Translucent); assert!(l.unlit); assert_eq!(l.opacity, Some(Channel::R)); assert_eq!(l.value_channel, Some(Channel::G));
        assert!(matches!(l.ramp, Ramp::Three { .. }));
    }
    #[test] fn masked_foliage_uses_alpha_and_plain_material_has_no_ramp() {
        let mut m = MaterialDef { blend_mode: "BLEND_Masked".into(), shading_model: "MSM_DefaultLit".into(), two_sided: true, clip: 0.33, ..Default::default() };
        m.textures.insert("@a".into(), tex("T_Tree_Leaf", true)); m.color_texture = Some("@a".into());
        let l = Look::from_material(&m); assert_eq!(l.blend, Blend::Masked); assert_eq!(l.opacity, Some(Channel::A)); assert!(matches!(l.ramp, Ramp::None)); assert!(l.two_sided && !l.unlit);
    }
}
