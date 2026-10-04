//! Lowering of the semantic menu [`DrawList`] into the renderer-agnostic [`RenderList`] (pure port of the replay loop of the old `menu/render.rs`).
//!
//! | menu command | lowered to |
//! |---|---|
//! | `Fill` | white-texel quad |
//! | `Brush` | box brushes (`BrushId::box_margin`) as 9-slice with the DPI scale, plain brushes as a sprite; missing textures are skipped |
//! | `Sprite` | sprite, optionally mirrored |
//! | `Text` | glyph quads from the shared atlases; with `shadow`: a first pass offset by `SHADOW_OFFSET * scale` in `0.75 * alpha` black |
//! | `PushClip` / `PopClip` | same |
//! | `Procedural` | DEFAULT: `outline::expand` Fill runs (what the old GL runtime drew and what its captures show); with
//! |  | [`LowerOptions::procedural_materials`]: `MaterialKind::Procedural` over the rect |
//! | `Panel` | `MaterialKind::Panel` with the menu fade as alpha |
//! | `Material` | `MaterialKind::Settings` |
use super::{BrushId, Cmd, DrawList};
use crate::draw::{MaterialKind, RenderList};
use crate::layout::dpi_scale;
use crate::scene::UiContext;
use crate::text::{SHADOW_COLOR, SHADOW_OFFSET};

#[derive(Clone, Copy, Debug, Default)]
pub struct LowerOptions {
    /// Emit `MaterialKind::Procedural` for the Esc-menu outline / Primary-M buttons instead of expanding them to `Fill` runs.
    pub procedural_materials: bool,
}

/// Lower `list` for a `w` x `h` client (old behaviour: Procedural boxes are `Fill` runs).
pub fn lower(list: &DrawList, ctx: &mut UiContext, w: f32, h: f32, out: &mut RenderList) { lower_with(list, ctx, w, h, out, &LowerOptions::default()); }

pub fn lower_with(list: &DrawList, ctx: &mut UiContext, w: f32, h: f32, out: &mut RenderList, opts: &LowerOptions) {
    let scale = dpi_scale(w, h);
    for c in &list.cmds {
        match c {
            Cmd::Fill { rect, color } => out.fill(*rect, *color),
            Cmd::Brush { brush, rect, tint } => {
                let t = ctx.assets.texture(&brush.file_name());
                match brush.box_margin() { Some(m) => out.nine_slice(t, *rect, m, scale, *tint), None => out.sprite(t.id, *rect, *tint) }
            }
            Cmd::Sprite { brush, rect, flip_x, tint } => {
                let t = ctx.assets.texture(&brush.file_name());
                out.sprite_flip(t.id, *rect, *flip_x, *tint);
            }
            Cmd::Text { x, top, box_h, text, font, px, color, shadow } => {
                let so = SHADOW_OFFSET * scale;
                let sh = if *shadow { Some(([so, so], [SHADOW_COLOR[0], SHADOW_COLOR[1], SHADOW_COLOR[2], SHADOW_COLOR[3] * color[3]])) } else { None };
                ctx.atlases.draw_line(&mut ctx.assets.store, &ctx.fonts, out, *font, *px, *x, *top, *box_h, text, *color, sh);
            }
            Cmd::Procedural { style, vis, rect, unit, alpha } => {
                if opts.procedural_materials { out.material(MaterialKind::Procedural { style: *style, vis: *vis, unit: *unit }, *rect, [1., 1., 1., *alpha], false); }
                else { for e in super::outline::expand(*style, *vis, *rect, *unit, *alpha) { if let Cmd::Fill { rect, color } = e { out.fill(rect, color); } } }
            }
            Cmd::Panel { kind, rect, alpha } => out.material(MaterialKind::Panel { kind: *kind }, *rect, [1., 1., 1., *alpha], false),
            Cmd::Material { mat, state, rect, fill, tint, scale, flip_x } => out.material(MaterialKind::Settings { mat: *mat, state: *state, fill: *fill, scale: *scale }, *rect, *tint, *flip_x),
            Cmd::PushClip(r) => out.push_clip(*r),
            Cmd::PopClip => out.pop_clip(),
        }
    }
}

/// Every brush texture file the menu may ask for (preload / missing-asset report).
pub fn brush_files() -> Vec<String> { BrushId::all().into_iter().map(|b| b.file_name()).collect() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::Cmd as L;
    use crate::layout::Rect;
    use crate::material::types::{PanelKind, ProcStyle, Vis};

    fn ctx() -> Option<UiContext> { crate::testutil::ui_context() }

    #[test] fn fills_clips_panels_and_materials_map_one_to_one() {
        let Some(mut c) = ctx() else { return };
        let mut l = DrawList::default();
        l.cmds.push(Cmd::PushClip(Rect::new(0., 0., 50., 50.)));
        l.cmds.push(Cmd::Fill { rect: Rect::new(1., 2., 10., 10.), color: [1., 0., 0., 0.5] });
        l.cmds.push(Cmd::PopClip);
        l.cmds.push(Cmd::Panel { kind: PanelKind::Backdrop, rect: Rect::new(0., 0., 100., 50.), alpha: 0.4 });
        l.cmds.push(Cmd::Material { mat: super::super::pages::uimat::Mat::SliderFill, state: super::super::pages::uimat::MatState::Base, rect: Rect::new(0., 0., 80., 8.), fill: 0.3, tint: [1.; 4], scale: 0.666, flip_x: true });
        let mut out = RenderList::new(); lower(&l, &mut c, 1280., 720., &mut out);
        assert!(matches!(out.cmds[0], L::PushClip(_)) && matches!(out.cmds[2], L::PopClip));
        assert!(matches!(&out.cmds[1], L::Quad(q) if q.tex == crate::draw::TextureId::WHITE && q.tint == [1., 0., 0., 0.5]));
        assert!(matches!(&out.cmds[3], L::Material(m) if matches!(m.kind, MaterialKind::Panel { kind: PanelKind::Backdrop }) && m.tint[3] == 0.4));
        assert!(matches!(&out.cmds[4], L::Material(m) if m.flip_x && matches!(m.kind, MaterialKind::Settings { fill, scale, .. } if fill == 0.3 && scale == 0.666)));
    }
    #[test] fn procedural_default_expands_to_fills_and_option_keeps_the_material() {
        let Some(mut c) = ctx() else { return };
        let mut l = DrawList::default();
        l.cmds.push(Cmd::Procedural { style: ProcStyle::Outline, vis: Vis::Hover, rect: Rect::new(10., 10., 120., 40.), unit: 0.666, alpha: 0.8 });
        let mut a = RenderList::new(); lower(&l, &mut c, 1280., 720., &mut a);
        assert!(a.quad_count() > 20 && !a.cmds.iter().any(|x| matches!(x, L::Material(_))), "old behaviour = Fill runs");
        let mut b = RenderList::new(); lower_with(&l, &mut c, 1280., 720., &mut b, &LowerOptions { procedural_materials: true });
        assert_eq!(b.cmds.len(), 1);
        assert!(matches!(&b.cmds[0], L::Material(m) if matches!(m.kind, MaterialKind::Procedural { style: ProcStyle::Outline, vis: Vis::Hover, .. }) && m.tint[3] == 0.8));
    }
    #[test] fn text_shadow_pass_uses_scaled_offset_and_faded_black() {
        let Some(mut c) = ctx() else { return };
        let mk = |shadow| { let mut l = DrawList::default(); l.cmds.push(Cmd::Text { x: 20., top: 20., box_h: 30., text: "Hi".into(), font: super::super::FontId::Bold, px: 24, color: [1., 1., 1., 0.5], shadow }); l };
        let (mut p, mut s) = (RenderList::new(), RenderList::new());
        lower(&mk(false), &mut c, 1920., 1080., &mut p); lower(&mk(true), &mut c, 1920., 1080., &mut s);
        assert_eq!(s.quad_count(), p.quad_count() * 2);
        let (L::Quad(sh), L::Quad(fg)) = (&s.cmds[0], &s.cmds[p.quad_count()]) else { panic!() };
        assert_eq!(sh.tint, [0., 0., 0., 0.375]); assert_eq!(fg.tint, [1., 1., 1., 0.5]);
        let (dx, dy) = (sh.pos[0][0] - fg.pos[0][0], sh.pos[0][1] - fg.pos[0][1]);
        assert!((dx - 1.6).abs() <= 1.0 && (dy - 1.6).abs() <= 1.0, "scale 1.0 at 1080p, pixel snapped: {dx} {dy}");
    }
    #[test] fn box_brushes_become_nine_slices_and_plain_brushes_sprites() {
        let Some(mut c) = ctx() else { return };
        let mut l = DrawList::default();
        l.cmds.push(Cmd::Brush { brush: BrushId::Button(super::super::BtnKind::Default, super::super::BtnSize::M, Vis::Normal), rect: Rect::new(0., 0., 200., 40.), tint: [1.; 4] });
        l.cmds.push(Cmd::Brush { brush: BrushId::Arrow, rect: Rect::new(0., 0., 20., 20.), tint: [1.; 4] });
        l.cmds.push(Cmd::Sprite { brush: BrushId::Arrow, rect: Rect::new(0., 0., 20., 20.), flip_x: true, tint: [1.; 4] });
        let mut out = RenderList::new(); lower(&l, &mut c, 1280., 720., &mut out);
        if c.assets.missing.is_empty() {
            assert_eq!(out.quad_count(), 9 + 1 + 1);
            let L::Quad(q) = out.cmds.last().unwrap() else { panic!() }; assert_eq!(q.uv, [1., 0., 0., 1.], "mirrored");
        }
    }
    #[test] fn missing_textures_are_skipped_not_drawn_white() {
        let mut c = UiContext::new(Box::new(crate::assets::MemAssets::default())).ok();
        let Some(c) = c.as_mut() else { return };
        let mut l = DrawList::default();
        l.cmds.push(Cmd::Brush { brush: BrushId::Arrow, rect: Rect::new(0., 0., 20., 20.), tint: [1.; 4] });
        let mut out = RenderList::new(); lower(&l, c, 1280., 720., &mut out);
        assert!(out.is_empty()); assert_eq!(c.assets.missing.len(), 1);
    }
}
