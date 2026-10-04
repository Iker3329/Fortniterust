//! Esc-menu material boxes as draw-list commands: the pure per-pixel maths lives in [`crate::material::outline`] (re-exported here); this file
//! only adds the toolkit side (`expand` into `Fill` runs, gradient strips, `Ui::procedural` / `Ui::panel` / `Ui::gradient_v`).
use super::*;
pub use crate::material::outline::*;

/// Number of strips used to approximate a gradient of `len_px` pixels.
fn strips(len_px: f32) -> usize { ((len_px / 6.).ceil() as usize).clamp(2, 48) }

/// Vertical gradient as exact-edge strips (adjacent strips share rounded edges so alpha never double-blends).
pub fn gradient_cmds(out: &mut Vec<Cmd>, rect: Rect, top: [f32; 4], bottom: [f32; 4], vertical: bool) {
    if rect.w <= 0. || rect.h <= 0. { return; }
    let len = if vertical { rect.h } else { rect.w };
    let n = strips(len);
    let origin = if vertical { rect.y } else { rect.x };
    let edge = |i: usize| (origin + len * i as f32 / n as f32).round();
    let (o0, o1) = (origin.round(), (origin + len).round());
    for i in 0..n {
        let (a, b) = (if i == 0 { o0 } else { edge(i) }, if i + 1 == n { o1 } else { edge(i + 1) });
        if b <= a { continue; }
        let t = (i as f32 + 0.5) / n as f32;
        let color = [0, 1, 2, 3].map(|k| top[k] + (bottom[k] - top[k]) * t);
        let r = if vertical { Rect::new(rect.x, a, rect.w, b - a) } else { Rect::new(a, rect.y, b - a, rect.h) };
        out.push(Cmd::Fill { rect: r, color });
    }
}

/// Expand a procedural box into plain `Fill` commands. `unit` = pixels per Slate unit, `alpha` = menu fade.
pub fn expand(style: ProcStyle, vis: Vis, rect: Rect, unit: f32, alpha: f32) -> Vec<Cmd> {
    let mut out = Vec::new();
    if rect.w <= 0. || rect.h <= 0. { return out; }
    // Per-pixel evaluation of the compiled material (see the module docs), merged into horizontal runs of identical 8-bit colour.
    // Pixel centres: the rect is snapped to whole pixels, UVs are taken at (x + 0.5) relative to the UNSNAPPED rect.
    let (mu, mv, image) = match style {
        ProcStyle::PrimaryM => (PM_MARGIN.0, PM_MARGIN.1, PM_IMAGE),
        s if s.left_shade() => (OUTLINE_SHADE_MARGIN.0, OUTLINE_SHADE_MARGIN.1, OUTLINE_IMAGE),
        _ => (OUTLINE_MARGIN.0, OUTLINE_MARGIN.1, OUTLINE_IMAGE),
    };
    let look_ = if style == ProcStyle::PrimaryM { None } else { Some(look(style, vis)) };
    let (x0, x1, y0, y1) = (rect.x.round() as i32, (rect.x + rect.w).round() as i32, rect.y.round() as i32, (rect.y + rect.h).round() as i32);
    let q = |c: [f32; 4]| c.map(|k| (k.clamp(0., 1.) * 255. + 0.5) as u8);
    for y in y0..y1 {
        let py = y as f32 + 0.5 - rect.y;
        let v = slice_uv(py, rect.h, mv * image * unit, mv);
        let v1 = (py / rect.h).clamp(0., 1.);
        let mut run: Option<(i32, [u8; 4])> = None;
        for x in x0..=x1 {
            let c = if x < x1 {
                let u = slice_uv(x as f32 + 0.5 - rect.x, rect.w, mu * image * unit, mu);
                Some(q(match &look_ { Some(l) => outline_pixel(l, u, v), None => primary_pixel(vis, u, v, v1) }))
            } else { None };
            match (run, c) {
                (Some((s, rc)), Some(c)) if rc == c => { run = Some((s, rc)); }
                (r, c) => {
                    if let Some((s, rc)) = r { if rc[3] > 0 { out.push(Cmd::Fill { rect: Rect::new(s as f32, y as f32, (x - s) as f32, 1.), color: [rc[0] as f32 / 255., rc[1] as f32 / 255., rc[2] as f32 / 255., rc[3] as f32 / 255. * alpha] }); } }
                    run = c.map(|c| (x, c));
                }
            }
        }
    }
    out
}

impl<'a> Ui<'a> {
    /// Emit a procedural material box (state is kept in the command so tests can read it; the renderer expands it).
    pub fn procedural(&mut self, style: ProcStyle, vis: Vis, rect: Rect) {
        if rect.w > 0. && rect.h > 0. { self.out.cmds.push(Cmd::Procedural { style, vis, rect, unit: self.vp.scale, alpha: self.alpha }); }
    }
    /// Gradient-panel material over `rect` (the renderer draws `outline::panel_bitmap(kind, ..)` stretched with linear filtering; the menu fade scales its opacity).
    pub fn panel(&mut self, kind: PanelKind, rect: Rect) {
        if rect.w > 0. && rect.h > 0. { self.out.cmds.push(Cmd::Panel { kind, rect, alpha: self.alpha }); }
    }
    /// Vertical gradient (straight-alpha top/bottom colours, menu fade applied).
    pub fn gradient_v(&mut self, rect: Rect, top: [f32; 4], bottom: [f32; 4]) {
        let a = self.alpha;
        gradient_cmds(&mut self.out.cmds, rect, [top[0], top[1], top[2], top[3] * a], [bottom[0], bottom[1], bottom[2], bottom[3] * a], true);
    }
}

