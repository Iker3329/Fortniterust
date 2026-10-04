//! UMG/Slate-like layout: DPI scaling, canvas anchors, safe zone, box (9-slice) geometry.
//! Pure port of the layout half of the old `ui_layout.rs` (no GL).
//!
//! DPI rule: UE4 engine default `UserInterfaceSettings` (`UIScaleRule=ShortestSide`, curve keys (480,0.444) (720,0.666) (1080,1.0) (8640,8.0)),
//! read from the unencrypted `BaseEngine.ini`. Fortnite's own override lives in the encrypted `DefaultEngine.ini` and is unknown: APPROXIMATION.

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
impl Rect {
    pub const ZERO: Rect = Rect { x: 0., y: 0., w: 0., h: 0. };
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self { Self { x, y, w, h } }
    pub fn right(&self) -> f32 { self.x + self.w }
    pub fn bottom(&self) -> f32 { self.y + self.h }
    pub fn center(&self) -> (f32, f32) { (self.x + self.w * 0.5, self.y + self.h * 0.5) }
    pub fn contains(&self, x: f32, y: f32) -> bool { x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h }
    pub fn intersect(&self, o: Rect) -> Option<Rect> {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = ((self.x + self.w).min(o.x + o.w), (self.y + self.h).min(o.y + o.h));
        if x1 > x0 && y1 > y0 { Some(Rect::new(x0, y0, x1 - x0, y1 - y0)) } else { None }
    }
    pub fn deflate(&self, l: f32, t: f32, r: f32, b: f32) -> Rect { Rect::new(self.x + l, self.y + t, (self.w - l - r).max(0.), (self.h - t - b).max(0.)) }
    pub fn inflate(&self, d: f32) -> Rect { Rect::new(self.x - d, self.y - d, self.w + 2. * d, self.h + 2. * d) }
}

/// UE4 default UserInterfaceSettings curve (approximation, see module docs).
const UI_SCALE_CURVE: [(f32, f32); 4] = [(480., 0.444), (720., 0.666), (1080., 1.0), (8640., 8.0)];
pub fn ui_scale_curve(shortest_side: f32) -> f32 {
    let c = &UI_SCALE_CURVE;
    if shortest_side <= c[0].0 { return c[0].1; }
    for i in 1..c.len() {
        if shortest_side <= c[i].0 {
            let t = (shortest_side - c[i - 1].0) / (c[i].0 - c[i - 1].0);
            return c[i - 1].1 + (c[i].1 - c[i - 1].1) * t;
        }
    }
    c[c.len() - 1].1
}
/// ShortestSide rule: Slate-units-to-pixels scale for a viewport.
pub fn dpi_scale(w: f32, h: f32) -> f32 { ui_scale_curve(w.min(h)) }

/// Safe-zone margins in pixels from fractional insets (PC default is 0).
pub fn safe_zone(w: f32, h: f32, left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect::new(w * left, h * top, w * (1. - left - right), h * (1. - top - bottom))
}

/// Viewport description: pixel size, DPI scale and safe area. Build once per frame.
#[derive(Clone, Copy, Debug)]
pub struct Viewport { pub w: f32, pub h: f32, pub scale: f32, pub safe: Rect }
impl Viewport {
    pub fn new(w: f32, h: f32) -> Self { let (w, h) = (w.max(1.), h.max(1.)); Self { w, h, scale: dpi_scale(w, h), safe: Rect::new(0., 0., w, h) } }
    pub fn with_safe_zone(mut self, left: f32, top: f32, right: f32, bottom: f32) -> Self { self.safe = safe_zone(self.w, self.h, left, top, right, bottom); self }
    /// Slate-unit size of the viewport.
    pub fn slate_size(&self) -> (f32, f32) { (self.w / self.scale, self.h / self.scale) }
    /// Convert slate units to pixels.
    pub fn px(&self, v: f32) -> f32 { v * self.scale }
}

/// UMG canvas-slot layout. Offsets/size are in Slate units; result is in pixels.
/// If anchors are a point (min==max): offsets = (x, y, width, height) and `alignment` pivots the box.
/// If stretched on an axis: offsets are (left, top, right, bottom) margins.
#[derive(Clone, Copy, Debug)]
pub struct CanvasSlot { pub anchor_min: (f32, f32), pub anchor_max: (f32, f32), pub offsets: (f32, f32, f32, f32), pub alignment: (f32, f32) }
impl CanvasSlot {
    pub fn point(anchor: (f32, f32), x: f32, y: f32, w: f32, h: f32, alignment: (f32, f32)) -> Self { Self { anchor_min: anchor, anchor_max: anchor, offsets: (x, y, w, h), alignment } }
    pub fn stretch(margins: (f32, f32, f32, f32)) -> Self { Self { anchor_min: (0., 0.), anchor_max: (1., 1.), offsets: margins, alignment: (0., 0.) } }
    pub fn resolve(&self, area: Rect, scale: f32) -> Rect {
        let (l, t, r, b) = self.offsets;
        let axis = |amin: f32, amax: f32, o0: f32, o1: f32, align: f32, origin: f32, extent: f32| -> (f32, f32) {
            if (amax - amin).abs() < 1e-6 { let size = o1 * scale; (origin + amin * extent + o0 * scale - align * size, size) }
            else { let a = origin + amin * extent + o0 * scale; let c = origin + amax * extent - o1 * scale; (a, c - a) }
        };
        let (x, w) = axis(self.anchor_min.0, self.anchor_max.0, l, r, self.alignment.0, area.x, area.w);
        let (y, h) = axis(self.anchor_min.1, self.anchor_max.1, t, b, self.alignment.1, area.y, area.h);
        Rect::new(x, y, w, h)
    }
}

/// Box-brush (9-slice) geometry. `margin` is (l,t,r,b) as FRACTIONS of the texture (UE FMargin on a box brush); borders are drawn at
/// texture-pixel size * `scale`; the centre stretches; borders shrink when the destination is smaller than their sum (UE does the same).
/// Returns up to nine `(dest rect, uv [u0,v0,u1,v1])` cells (zero-area cells dropped).
pub fn nine_slice_cells(tex_w: u32, tex_h: u32, dest: Rect, margin: [f32; 4], scale: f32) -> Vec<(Rect, [f32; 4])> {
    let [ml, mt, mr, mb] = margin;
    let (bl, bt, br, bb) = (ml * tex_w as f32 * scale, mt * tex_h as f32 * scale, mr * tex_w as f32 * scale, mb * tex_h as f32 * scale);
    let sx = if bl + br > dest.w { dest.w / (bl + br) } else { 1. };
    let sy = if bt + bb > dest.h { dest.h / (bt + bb) } else { 1. };
    let (bl, br, bt, bb) = (bl * sx, br * sx, bt * sy, bb * sy);
    let xs = [dest.x, dest.x + bl, dest.x + dest.w - br, dest.x + dest.w];
    let ys = [dest.y, dest.y + bt, dest.y + dest.h - bb, dest.y + dest.h];
    let us = [0., ml, 1. - mr, 1.];
    let vs = [0., mt, 1. - mb, 1.];
    let mut out = Vec::with_capacity(9);
    for j in 0..3 { for i in 0..3 {
        let r = Rect::new(xs[i], ys[j], xs[i + 1] - xs[i], ys[j + 1] - ys[j]);
        if r.w > 0. && r.h > 0. { out.push((r, [us[i], vs[j], us[i + 1], vs[j + 1]])); }
    } }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn curve_matches_reference_points() { assert!((dpi_scale(1920., 1080.) - 1.).abs() < 1e-6); assert!((dpi_scale(1280., 720.) - 0.666).abs() < 1e-6); assert!((dpi_scale(3840., 2160.) - (1. + 7. * (2160. - 1080.) / 7560.)).abs() < 1e-5); assert_eq!(dpi_scale(100., 5000.), 0.444); }
    #[test] fn shortest_side_rule() { assert_eq!(dpi_scale(800., 1280.), dpi_scale(1280., 800.)); }
    #[test] fn point_anchor_bottom_center() { let s = CanvasSlot::point((0.5, 1.), 0., -80., 200., 20., (0.5, 1.)); let r = s.resolve(Rect::new(0., 0., 1000., 1000.), 2.); assert_eq!(r, Rect::new(300., 800., 400., 40.)); }
    #[test] fn stretch_anchor_margins() { let s = CanvasSlot { anchor_min: (0., 0.), anchor_max: (1., 1.), offsets: (10., 20., 30., 40.), alignment: (0., 0.) }; let r = s.resolve(Rect::new(0., 0., 1000., 500.), 1.); assert_eq!(r, Rect::new(10., 20., 960., 440.)); }
    #[test] fn safe_zone_insets() { let r = safe_zone(1000., 500., 0.1, 0.2, 0.1, 0.2); assert!((r.x - 100.).abs() < 1e-3 && (r.y - 100.).abs() < 1e-3 && (r.w - 800.).abs() < 1e-3 && (r.h - 300.).abs() < 1e-3); }
    #[test] fn nine_slice_covers_destination() { let c = nine_slice_cells(64, 64, Rect::new(10., 10., 200., 100.), [0.25, 0.25, 0.25, 0.25], 1.); assert_eq!(c.len(), 9); let area: f32 = c.iter().map(|q| q.0.w * q.0.h).sum(); assert!((area - 20000.).abs() < 1e-2); }
    #[test] fn nine_slice_small_dest_shrinks_borders() { let c = nine_slice_cells(64, 64, Rect::new(0., 0., 16., 16.), [0.5, 0.5, 0.5, 0.5], 1.); let area: f32 = c.iter().map(|q| q.0.w * q.0.h).sum(); assert!((area - 256.).abs() < 1e-3); }
}
