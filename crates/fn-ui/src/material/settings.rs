//! CPU bake of `MaterialKind::{Settings, Procedural, Panel}` (owned by the menu port). Returns `None` for kinds this module does not handle.
//!
//! All results are PREMULTIPLIED RGBA8 (`w*h*4`, row 0 = top), the format the backends sample.
//!
//! * `Settings { mat, state, fill, scale }` = the cooked 1.8 Settings-screen material ([`crate::material::uimat`]): the default brush of
//!   (`mat`, `state`) rasterised at `w` x `h` with `size_px = (w, h) / scale` (the material's `PixelSize` is in Slate units), `fillPercent = fill`.
//! * `Panel { kind }` = [`outline::panel_bitmap`] (gradient panels are smooth functions of the 0..1 element UV).
//! * `Procedural { style, vis, unit }` = the Esc-menu Outline / Primary-M button material evaluated PER PIXEL (what `outline::expand` draws as
//!   `Fill` runs). A GPU shader mirrors it as follows, per fragment with element-local pixel centre `p = (x + 0.5, y + 0.5)` and element size
//!   `size = (w, h)` in pixels:
//!   1. box UV (TexCoord0): per axis `slice_uv(p, size, margin * image * unit, margin)` with the style's margins (`PrimaryM`: 0.25 x 0.5,
//!      left-shade outlines: 0.25 x 0.1, other outlines 0.1 x 0.1, `image` = 48): the two border bands cover `margin` of UV over
//!      `margin * image * unit` pixels (clamped to half the element), the middle stretches;
//!   2. TexCoord1 (only `PrimaryM`'s vertical gradient): `v1 = p.y / size.y`;
//!   3. outline family: `outline_pixel(look(style, vis), u, v)`; Primary-M: `primary_pixel(vis, u, v, v1)`; both return STRAIGHT sRGB RGBA;
//!   4. quantise to 8 bits (`floor(c * 255 + 0.5)`) and premultiply with `(c * a + 127) / 255`.
use super::outline::{self, MatLook, ProcStyle};
use super::uimat;
use crate::assets::{premultiply_rgba8, UiAssets};
use crate::draw::MaterialKind;

/// Premultiplied bake of the Settings material family. `None` when the (mat, state) pair has no drawn brush (NoDrawType): the caller draws nothing.
pub fn bake_settings(mat: uimat::Mat, state: uimat::MatState, w: u32, h: u32, fill: f32, scale: f32) -> Option<Vec<u8>> {
    let (w, h) = (w.max(1), h.max(1));
    let spec = uimat::default_brush(mat, state)?;
    let s = scale.max(1e-3);
    let p = uimat::MatParams { fill_percent: fill, size_px: (w as f32 / s, h as f32 / s), ..Default::default() };
    Some(premultiply_rgba8(&uimat::raster_brush(&spec, w, h, &p)))
}

/// Premultiplied bake of a gradient panel on a `w` x `h` grid.
pub fn bake_panel(kind: outline::PanelKind, w: u32, h: u32) -> Vec<u8> { premultiply_rgba8(&outline::panel_bitmap(kind, w.max(1) as usize, h.max(1) as usize)) }

/// Per-pixel margins (UV fraction u/v) and image size of a procedural style (see the module docs).
pub fn proc_margins(style: ProcStyle) -> (f32, f32, f32) {
    match style {
        ProcStyle::PrimaryM => (outline::PM_MARGIN.0, outline::PM_MARGIN.1, outline::PM_IMAGE),
        s if s.left_shade() => (outline::OUTLINE_SHADE_MARGIN.0, outline::OUTLINE_SHADE_MARGIN.1, outline::OUTLINE_IMAGE),
        _ => (outline::OUTLINE_MARGIN.0, outline::OUTLINE_MARGIN.1, outline::OUTLINE_IMAGE),
    }
}

/// Premultiplied per-pixel bake of a procedural button box (the rect is exactly `w` x `h` pixels, origin 0,0).
pub fn bake_procedural(style: ProcStyle, vis: outline::Vis, w: u32, h: u32, unit: f32) -> Vec<u8> {
    let (w, h) = (w.max(1), h.max(1));
    let (mu, mv, image) = proc_margins(style);
    let look: Option<MatLook> = if style == ProcStyle::PrimaryM { None } else { Some(outline::look(style, vis)) };
    let q = |c: [f32; 4]| c.map(|k| (k.clamp(0., 1.) * 255. + 0.5) as u8);
    let mut straight = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let py = y as f32 + 0.5;
        let v = outline::slice_uv(py, h as f32, mv * image * unit, mv);
        let v1 = (py / h as f32).clamp(0., 1.);
        for x in 0..w {
            let u = outline::slice_uv(x as f32 + 0.5, w as f32, mu * image * unit, mu);
            let c = match &look { Some(l) => outline::outline_pixel(l, u, v), None => outline::primary_pixel(vis, u, v, v1) };
            straight.extend_from_slice(&q(c));
        }
    }
    premultiply_rgba8(&straight)
}

/// Premultiplied RGBA8 of the menu material kinds, or `None` for kinds owned elsewhere (HUD materials).
pub fn bake(kind: &MaterialKind, w: u32, h: u32, _assets: &mut UiAssets) -> Option<Vec<u8>> {
    match kind {
        MaterialKind::Settings { mat, state, fill, scale } => Some(bake_settings(*mat, *state, w, h, *fill, *scale).unwrap_or_else(|| vec![0; (w.max(1) * h.max(1) * 4) as usize])),
        MaterialKind::Panel { kind } => Some(bake_panel(*kind, w, h)),
        MaterialKind::Procedural { style, vis, unit } => Some(bake_procedural(*style, *vis, w, h, *unit)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::MemAssets;
    use crate::material::types::{PanelKind, Vis};
    fn assets() -> UiAssets { UiAssets::new(Box::new(MemAssets::default())) }
    fn premultiplied_ok(b: &[u8]) -> bool { b.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]) }

    #[test] fn settings_material_bakes_every_state_with_right_size() {
        let mut a = assets();
        for mat in [uimat::Mat::BorderMainL, uimat::Mat::SolidBgShellBlue, uimat::Mat::TabManage, uimat::Mat::SliderFill, uimat::Mat::SettingsSelect, uimat::Mat::ButtonBar, uimat::Mat::RadioM] {
            for state in uimat::MatState::ALL {
                let k = MaterialKind::Settings { mat, state, fill: 0.5, scale: 1. };
                let b = bake(&k, 37, 21, &mut a).expect("settings kind handled");
                assert_eq!(b.len(), 37 * 21 * 4, "{mat:?} {state:?}");
                assert!(premultiplied_ok(&b), "{mat:?} {state:?}");
            }
        }
        let k = MaterialKind::Settings { mat: uimat::Mat::SolidBgShellBlue, state: uimat::MatState::Base, fill: 0., scale: 1. };
        assert!(bake(&k, 64, 32, &mut a).unwrap().chunks_exact(4).any(|p| p[3] > 0), "a drawn material is not transparent");
    }
    #[test] fn slider_fill_follows_fill_percent_and_scale_changes_pixel_size() {
        let at = |fill: f32| bake_settings(uimat::Mat::SliderFill, uimat::MatState::Base, 200, 12, fill, 1.).unwrap();
        assert_ne!(at(0.1), at(0.9), "fillPercent changes the image");
        assert_eq!(at(0.5), at(0.5), "deterministic");
        let a = bake_settings(uimat::Mat::SolidBgShellBlue, uimat::MatState::Base, 90, 80, 0., 1.).unwrap();
        let b = bake_settings(uimat::Mat::SolidBgShellBlue, uimat::MatState::Base, 90, 80, 0., 2.).unwrap();
        assert_eq!(a.len(), b.len());
        assert!(premultiplied_ok(&a) && premultiplied_ok(&b));
    }    #[test] fn panel_kinds_bake_opaque_enough_and_sized() {
        let mut a = assets();
        for kind in [PanelKind::Backdrop, PanelKind::ModalPanel, PanelKind::ModalHeader] {
            let b = bake(&MaterialKind::Panel { kind }, 33, 17, &mut a).unwrap();
            assert_eq!(b.len(), 33 * 17 * 4); assert!(premultiplied_ok(&b));
            assert!(b.chunks_exact(4).any(|p| p[3] > 0), "{kind:?}");
        }
        // matches the semantic bitmap exactly (premultiplied copy)
        let raw = outline::panel_bitmap(PanelKind::ModalPanel, 8, 8);
        assert_eq!(bake_panel(PanelKind::ModalPanel, 8, 8), premultiply_rgba8(&raw));
    }
    #[cfg(feature = "menu")]
    #[test] fn procedural_bake_matches_expand_runs() {
        // The per-pixel bake and the `expand` Fill runs describe the same image: rasterise the runs and compare (straight, alpha 1).
        use crate::layout::Rect;
        let (w, h, unit) = (120u32, 36u32, 0.666f32);
        for (style, vis) in [(ProcStyle::Outline, Vis::Normal), (ProcStyle::OutlineRed, Vis::Hover), (ProcStyle::OutlineLeftShade, Vis::Pressed), (ProcStyle::PrimaryM, Vis::Normal), (ProcStyle::PrimaryM, Vis::Disabled)] {
            let baked = bake_procedural(style, vis, w, h, unit);
            assert_eq!(baked.len(), (w * h * 4) as usize);
            let mut img = vec![0u8; (w * h * 4) as usize];
            for c in crate::menu::outline::expand(style, vis, Rect::new(0., 0., w as f32, h as f32), unit, 1.) {
                if let crate::menu::Cmd::Fill { rect, color } = c {
                    for x in rect.x as u32..(rect.x + rect.w) as u32 { let o = ((rect.y as u32 * w + x) * 4) as usize; img[o..o + 4].copy_from_slice(&color.map(|k| (k * 255. + 0.5) as u8)); }
                }
            }
            assert_eq!(baked, premultiply_rgba8(&img), "{style:?} {vis:?}");
            assert!(premultiplied_ok(&baked));
        }
    }
    #[test] fn procedural_states_differ_and_outline_has_a_hollow_centre() {
        let n = bake_procedural(ProcStyle::Outline, Vis::Normal, 160, 48, 1.);
        let hv = bake_procedural(ProcStyle::Outline, Vis::Hover, 160, 48, 1.);
        assert_ne!(n, hv);
        let a = |b: &[u8], x: usize, y: usize| b[(y * 160 + x) * 4 + 3];
        assert!(a(&n, 80, 0) > 100, "outline band at the top edge");
        assert!(a(&n, 2, 24) > 100 && a(&n, 80, 24) < a(&n, 80, 0), "centre is translucent, band is not");
    }
    #[test] fn foreign_kinds_are_not_handled() {
        let mut a = assets();
        assert!(bake(&MaterialKind::PanelVGrad, 4, 4, &mut a).is_none());
        assert!(bake(&MaterialKind::HudBar { a: [0.; 3], b: [1.; 3] }, 4, 4, &mut a).is_none());
    }
}
