//! CPU reference evaluation of the procedural UI materials.
//!
//! `bake` produces PREMULTIPLIED RGBA8 (`w*h*4`, row 0 = top) of a [`MaterialKind`] at a pixel size. It is the ground truth the WGSL translations in
//! `fn-ui-wgpu` are tested against, and the fallback path of any backend.
pub mod types;
pub mod outline; // pure per-pixel maths of the Esc-menu button/panel materials (owned by the menu port)
pub mod ui_shaders;
pub mod uimat;
pub mod settings; // Settings/Procedural/Panel kinds (owned by the menu port)
pub mod hud; // ItemCard/Compass/HudBar/PanelLine/PanelVGrad (owned by the screens port)

use crate::assets::UiAssets;
use crate::draw::MaterialKind;

/// Premultiplied RGBA8 of `kind` at `w` x `h` pixels (flip/tint are applied by the caller/backend, not here).
pub fn bake(kind: &MaterialKind, w: u32, h: u32, assets: &mut UiAssets) -> Vec<u8> {
    settings::bake(kind, w, h, assets).or_else(|| hud::bake(kind, w, h, assets)).unwrap_or_else(|| vec![0; (w * h * 4) as usize])
}


