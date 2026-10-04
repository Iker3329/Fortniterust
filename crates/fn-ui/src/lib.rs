//! fn-ui: renderer-agnostic UI core for the Fortnite 1.8 runtime rewrite.
//!
//! Layers (bottom up):
//! * [`layout`]   DPI scaling, canvas anchors, safe zone, 9-slice geometry.
//! * [`input`]    abstract mouse/key/gamepad events (semantic `Event`s).
//! * [`router`]   windowing-system-agnostic raw input (`InputEvent`: pointer, wheel, key down/up with L/R modifiers, pad buttons and sticks, focus)
//!                -> `Event`s, with auto-repeat, stuck-key and focus-loss safety.
//! * [`assets`]   CPU texture store + asset sources (directory, in-memory, layered; `pak_assets` = straight from the install, feature `pak`).
//! * [`text`]     fontdue glyph atlases -> quads.
//! * [`draw`]     the low-level [`draw::RenderList`] a GPU backend consumes (quads, materials, clips).
//! * [`material`] CPU reference evaluation of the procedural UI materials (ports of the cooked pixel shaders).
//! * [`menu`]     immediate-mode toolkit + Esc menu + Settings screens (semantic `menu::DrawList`, lowered by `menu::lower`).
//! * [`settings`] re-export of `fn-settings` (settings data model / INI / apply layer / `PlatformHost`; no OS code).
//! * [`screens`]  HUD, quickbar, vitals, compass, minimap, counters, loading screen as pure state -> RenderList.
pub mod assets;
pub mod draw;
pub mod input;
pub mod layout;
pub mod material;
#[cfg(feature = "pak")]
pub mod pak_assets;
#[cfg(feature = "menu")]
pub mod inputdev;
#[cfg(feature = "menu")]
pub mod menu;
#[cfg(all(test, feature = "menu"))]
pub(crate) mod testutil;
pub mod raster;
pub mod router;
pub mod scene;
pub mod screens;
#[cfg(feature = "menu")]
/// The settings model is the real one, `fn-settings` (typed `Settings`, descriptor table, ini persistence, apply layer, `PlatformHost`).
pub use fn_settings as settings;
pub mod text;



