//! Shared context + named test scenes (used by `fn-ui-demo`, screenshot mode and the pixel-diff helper).
//!
//! A scene is a pure function `(ctx, w, h) -> RenderList` that builds a screen from fixed test state at a pixel size. Scene names follow the old
//! runtime's capture names so the diff helper can find the old BMP (`old_capture` is a file-stem template with `{w}x{h}` placeholders, searched in the
//! old `verification` and `verification-1.10` folders).
use crate::assets::{AssetSource, DirAssets, UiAssets};
use crate::draw::RenderList;
use crate::text::{Fonts, TextAtlases};

/// Everything a screen needs to turn state into a draw list.
pub struct UiContext {
    pub assets: UiAssets,
    pub atlases: TextAtlases,
    pub fonts: Fonts,
}
impl UiContext {
    pub fn new(source: Box<dyn AssetSource>) -> Result<Self, String> {
        let fonts = Fonts::load(source.as_ref()).ok_or_else(|| "no usable UI font (NotoSans / system font)".to_owned())?;
        Ok(UiContext { assets: UiAssets::new(source), atlases: TextAtlases::new(), fonts })
    }
    /// Index the old runtime's `assets` tree (or `FN_UI_ASSETS`).
    pub fn from_default_assets() -> Result<Self, String> { Self::new(Box::new(DirAssets::new(DirAssets::default_root()))) }
    /// UI assets straight from the Fortnite install (textures decoded on demand from the paks, fonts read from them), `fallback` = an exported
    /// folder consulted for whatever the install does not provide (loading tips, ...). Fails when the install cannot be opened.
    #[cfg(feature = "pak")]
    pub fn from_install(install: &std::path::Path, fallback: Option<&std::path::Path>) -> Result<Self, String> {
        let pak: Box<dyn AssetSource> = Box::new(crate::pak_assets::PakAssets::open(install)?);
        match fallback {
            Some(dir) => Self::new(Box::new(crate::assets::LayeredAssets::new(vec![pak, Box::new(DirAssets::new(dir))]))),
            None => Self::new(pak),
        }
    }
    /// The best available source on this machine: the install (`FN_ROOT` or the default location) with the exported folder as the fallback,
    /// or the exported folder alone when the install is not there.
    #[cfg(feature = "pak")]
    pub fn auto() -> Result<Self, String> {
        let install = crate::pak_assets::default_install();
        let folder = DirAssets::default_root();
        let fallback = folder.is_dir().then_some(folder.as_path());
        if install.exists() { if let Ok(c) = Self::from_install(&install, fallback) { return Ok(c); } }
        Self::from_default_assets()
    }
}

pub struct SceneDef {
    pub name: &'static str,
    /// File-stem template of the old capture, e.g. `"settings18-video-{w}x{h}"` (None = no old capture exists).
    pub old_capture: Option<&'static str>,
    /// True when the old capture shows a 3D scene behind the UI (HUD): only pixels the UI covers opaquely are comparable.
    pub over_world: bool,
    pub build: fn(&mut UiContext, f32, f32) -> RenderList,
}

/// Every registered scene (menu + settings + HUD + loading).
pub fn all_scenes() -> Vec<SceneDef> {
    let mut v = Vec::new();
    #[cfg(feature = "menu")]
    v.extend(crate::menu::scenes());
    v.extend(crate::screens::scenes());
    v
}
pub fn find_scene(name: &str) -> Option<SceneDef> { all_scenes().into_iter().find(|s| s.name == name) }
