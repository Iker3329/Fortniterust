//! CONTAINERS (2026-10-08): the hold-to-use ring of the `KeybindWidget` (`HoldKeybindImage` in `SizeBoxBackplate`), shown by the interaction prompt of a
//! hold interaction (a chest / ammo box search), drawn from the COOKED material `/Game/UI/Materials/HollowCircularProgressBar` (Slate material rastered on
//! the CPU by `athena_lobby::slate_mat`, nothing redrawn by hand).
//!
//! SOURCES:
//! * `KeybindWidget_C` tree (fn-inspect): `Overlay_612` [`OverlaySlot_1` Fill/Fill: `SizeBoxBackplate` (W/H override, `HoldOverlay` > `HoldKeybindImage`,
//!   brush = the material)] [`OverlaySlot_2` Center/Center: `Switcher` (the keycap)]: the ring box is the widget's size and the keycap sits centred in it.
//! * `ShowHoldBackPlate` bytecode: W = H = `[24, 24, 32, 48, 64, 64][Brush Size] + 10 + (Switcher index > 0 ? 0 : 14)` (index 0 = a text key, 1 = a key
//!   brush); `IsHoldKeybind` -> `SizeBoxBackplate` Visible + MID `SetScalarParameterValue(BackAlpha, 0.2)` / `(OutlineAlpha, 0.4)`, else Collapsed. The
//!   material's parameters are `Back Alpha` (with a space), `OutlineAlpha`, `Percentage`, `CountdownStartTime`, `CountDownTime`, `Color`: the `BackAlpha`
//!   call names no parameter of the material and changes nothing (as in the original), `OutlineAlpha` becomes 0.4.
//! * `UFortKeybindWidget::StartHoldProgress` (exe 0x142eac8b0) / tick `FUN_142eb05c0`: `Percentage` = `(now - start) / duration` while `now < start +
//!   duration`; `StopHoldProgress` (0x142ead980) sets it back to 0. The controller broadcasts them while the Use hold of an interaction runs.
//! * The interaction indicator's keycap is `Brush Size` Medium (3) with a text key: the backplate is 48 + 10 + 14 = 72 Slate units square.
use crate::assets::{Sampler, Texture};
use crate::draw::TextureId;
use crate::draw::RenderList;
use crate::layout::Rect;
use crate::scene::UiContext;
use super::athena_lobby::slate_mat::SlateMat;
use super::athena_lobby::store::Store;
use std::path::PathBuf;
use std::rc::Rc;

pub const MATERIAL: &str = "/Game/UI/Materials/HollowCircularProgressBar.HollowCircularProgressBar";
/// `ShowHoldBackPlate` size table indexed by `EFortBrushSize`.
pub const BACKPLATE_SIZES: [f32; 6] = [24., 24., 32., 48., 64., 64.];
/// `EFortBrushSize::Medium` (the interaction indicator's keycap).
pub const MEDIUM: usize = 3;
/// `OutlineAlpha` set by `ShowHoldBackPlate` for a hold keybind.
pub const OUTLINE_ALPHA: f32 = 0.4;
/// Raster cache steps of `Percentage` (CHOSEN: 1/128, below one pixel of arc at 72 units).
pub const STEPS: f32 = 128.;

/// Backplate size (Slate units) of a keybind widget: `brush` = `EFortBrushSize`, `key_brush` = the Switcher shows a key image (index 1).
pub fn backplate_units(brush: usize, key_brush: bool) -> f32 { BACKPLATE_SIZES[brush.min(5)] + 10. + if key_brush { 0. } else { 14. } }

/// The ring material (loaded once from the install's paks; a failure is reported once and nothing is drawn).
#[derive(Default)]
pub struct HoldRing { install: Option<PathBuf>, store: Option<Option<Rc<Store>>>, warned: bool }
impl Clone for HoldRing { fn clone(&self) -> Self { HoldRing { install: self.install.clone(), store: None, warned: false } } }
impl HoldRing {
    pub fn set_install(&mut self, install: PathBuf) { if self.install.as_ref() != Some(&install) { self.install = Some(install); self.store = None; } }
    /// The ring texture at `percentage` (0..1) for a `px` x `px` pixel box; None when the material is unavailable.
    pub fn texture(&mut self, ctx: &mut UiContext, px: u32, percentage: f32) -> Option<TextureId> {
        let p = (percentage.clamp(0., 1.) * STEPS).round() / STEPS;
        let key = format!("holdring:{px}:{}", (p * STEPS) as u32);
        if let Some(t) = ctx.assets.store.named(&key) { return Some(t); }
        if self.store.is_none() {
            self.store = Some(match self.install.as_ref().map(|i| Store::open(i)) { Some(Ok(s)) => Some(Rc::new(s)), Some(Err(e)) => { eprintln!("[holdring] paks unavailable: {e}"); None } None => None });
        }
        let store = self.store.clone().flatten()?;
        let ov = fn_dxbc_cli::uniform::Overrides { vectors: Default::default(), scalars: [("Percentage".to_owned(), p), ("OutlineAlpha".to_owned(), OUTLINE_ALPHA)].into_iter().collect() };
        let m = match SlateMat::load_with(&store, &mut ctx.assets, MATERIAL, &ov, &Default::default()) {
            Ok(m) if m.is_ready() => m,
            Ok(_) => { if !self.warned { eprintln!("[holdring] {MATERIAL}: no Slate pixel shader: ring not drawn"); self.warned = true; } return None }
            Err(e) => { if !self.warned { eprintln!("[holdring] {MATERIAL}: {e}: ring not drawn"); self.warned = true; } return None }
        };
        let s = px as f32;
        let rgba = m.raster(px, px, (s, s), (s, s), [0.; 4], false, [1.; 4], false, 0.);
        Some(ctx.assets.store.insert_named(&key, Texture::new(px, px, rgba, Sampler::LINEAR)))
    }
    /// Draws the ring into `rect` (pixels).
    pub fn draw(&mut self, ctx: &mut UiContext, rect: Rect, percentage: f32, out: &mut RenderList) -> bool {
        let px = rect.w.round().max(1.) as u32;
        let Some(t) = self.texture(ctx, px, percentage) else { return false };
        out.quad(t, rect, [0., 0., 1., 1.], [1., 1., 1., 1.]);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn the_interaction_keycap_backplate_is_72_units() {
        assert_eq!(backplate_units(MEDIUM, false), 72.);
        assert_eq!(backplate_units(MEDIUM, true), 58.);
        assert_eq!(backplate_units(2, false), 56.);
    }
}
