//! Renderer-agnostic LOW-LEVEL draw list: what a GPU backend (fn-ui-wgpu) consumes.
//!
//! All coordinates are PIXELS, origin top-left, y down. Colours are STRAIGHT (non-premultiplied) RGBA in 0..1 in the same (gamma/sRGB-encoded)
//! space as the textures: Slate composites in gamma space on a non-sRGB back buffer, so a backend must NOT sRGB-convert on write and must blend
//! with `ONE, ONE_MINUS_SRC_ALPHA` after premultiplying (`tint.rgb * tint.a`, texel already premultiplied by `Texture::premultiplied`).
//!
//! Commands are painted in order. `Quad` = textured (or `TextureId::WHITE`) tinted quad with arbitrary corner positions (rotation allowed);
//! `Material` = a procedural UI material over a rect (see [`MaterialKind`]); `PushClip`/`PopClip` = axis-aligned scissor stack.
use crate::assets::TexInfo;
use crate::layout::{nine_slice_cells, Rect};
use crate::material::types::{PanelKind, ProcStyle, Vis};
use crate::material::uimat::{Mat, MatState};
use std::hash::{Hash, Hasher};

/// Handle into the [`crate::assets::TextureStore`]. `NONE` (0) = missing optional texture: quads using it are skipped, never drawn white.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TextureId(pub u32);
impl TextureId {
    pub const NONE: TextureId = TextureId(0);
    /// 1x1 opaque white texel (solid fills).
    pub const WHITE: TextureId = TextureId(1);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub tex: TextureId,
    /// Corner positions TL, TR, BR, BL (pixels).
    pub pos: [[f32; 2]; 4],
    /// u0, v0, u1, v1 over the texture (TL corner = (u0, v0), BR = (u1, v1); swap u0/u1 to mirror).
    pub uv: [f32; 4],
    /// Straight RGBA multiplier.
    pub tint: [f32; 4],
}
impl Quad {
    pub fn rect(tex: TextureId, r: Rect, uv: [f32; 4], tint: [f32; 4]) -> Self {
        Quad { tex, pos: [[r.x, r.y], [r.x + r.w, r.y], [r.x + r.w, r.y + r.h], [r.x, r.y + r.h]], uv, tint }
    }
    /// Axis-aligned bounds.
    pub fn bounds(&self) -> Rect {
        let xs = self.pos.map(|p| p[0]); let ys = self.pos.map(|p| p[1]);
        let (x0, x1) = (xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max));
        let (y0, y1) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

/// A procedural UI material. Every variant has a CPU reference evaluation (`crate::material::bake`) that produces premultiplied RGBA8 for a given
/// pixel size; GPU backends may instead evaluate the WGSL translation of the same cooked pixel shader and must match it to rounding.
#[derive(Clone, Debug, PartialEq)]
pub enum MaterialKind {
    /// 1.8 Settings-screen material family (`menu::pages::uimat`): `fill` = slider fillPercent, `scale` = DPI scale (`size_px / scale` = material PixelSize).
    Settings { mat: Mat, state: MatState, fill: f32, scale: f32 },
    /// Esc-menu outline / primary-M buttons (`menu::outline`), `unit` = pixels per Slate unit.
    Procedural { style: ProcStyle, vis: Vis, unit: f32 },
    /// Gradient panels: ShellUberBG backdrop, modal panel, modal header.
    Panel { kind: PanelKind },
    /// M-Item-Inst-S item card; `colors` = RarityData Color1..4 (linear); `icon`/`hero_tri` are textures sampled by the shader.
    ItemCard { colors: [[f32; 3]; 4], icon: TextureId, hero_tri: TextureId },
    /// M_Compass: `pan` = yaw/360 + 0.25, strip wraps, feather clamps.
    Compass { pan: f32, strip: TextureId, feather: TextureId },
    /// HUD-Bar_Material: colour = lerp(a, b, u) (linear inputs, sRGB-encoded by the shader).
    HudBar { a: [f32; 3], b: [f32; 3] },
    /// M_Border_HUD_Line_Vert (M_UI_GradientPanel instance) over the 4x8 mask texture.
    PanelLine { mask: TextureId, global_alpha: f32 },
    /// M_Border_HUD_VGradient (Border_HUD_ResourceBg).
    PanelVGrad,
    /// M_Border_LightNavy_VGrad (Border_HUD_ResourceBg_Highlighted): the same cooked shader as `PanelVGrad` with its own parameters
    /// ([`crate::material::hud::VGRAD_HIGHLIGHTED`]); the pack of the current resource.
    PanelVGradHighlighted,
    /// M_HealProgress (`CircularProgressBar` instance, `AthenaEquipProgress`): radial wipe of `texture` (`Results_TimerFill`) by `percentage` (0..=1).
    RadialProgress { percentage: f32, texture: TextureId },
}
impl MaterialKind {
    /// Stable cache key (floats hashed by bit pattern). Together with the destination pixel size this identifies a baked texture.
    pub fn cache_key(&self) -> u64 {
        struct Fnv(u64);
        impl Hasher for Fnv { fn finish(&self) -> u64 { self.0 } fn write(&mut self, b: &[u8]) { for x in b { self.0 ^= *x as u64; self.0 = self.0.wrapping_mul(0x100000001b3); } } }
        let mut h = Fnv(0xcbf29ce484222325);
        let f = |h: &mut Fnv, v: f32| v.to_bits().hash(h);
        match self {
            MaterialKind::Settings { mat, state, fill, scale } => { 0u8.hash(&mut h); mat.hash(&mut h); state.hash(&mut h); f(&mut h, (fill.clamp(0., 1.) * 100.).round()); f(&mut h, *scale); }
            MaterialKind::Procedural { style, vis, unit } => { 1u8.hash(&mut h); style.hash(&mut h); vis.hash(&mut h); f(&mut h, *unit); }
            MaterialKind::Panel { kind } => { 2u8.hash(&mut h); kind.hash(&mut h); }
            MaterialKind::ItemCard { colors, icon, hero_tri } => { 3u8.hash(&mut h); for c in colors { for v in c { f(&mut h, *v); } } icon.hash(&mut h); hero_tri.hash(&mut h); }
            MaterialKind::Compass { pan, strip, feather } => { 4u8.hash(&mut h); f(&mut h, (pan * 1536.).round()); strip.hash(&mut h); feather.hash(&mut h); }
            MaterialKind::HudBar { a, b } => { 5u8.hash(&mut h); for v in a.iter().chain(b) { f(&mut h, *v); } }
            MaterialKind::PanelLine { mask, global_alpha } => { 6u8.hash(&mut h); mask.hash(&mut h); f(&mut h, *global_alpha); }
            MaterialKind::PanelVGrad => { 7u8.hash(&mut h); }
            MaterialKind::PanelVGradHighlighted => { 8u8.hash(&mut h); }
            MaterialKind::RadialProgress { percentage, texture } => { 9u8.hash(&mut h); f(&mut h, (percentage.clamp(0., 1.) * 1000.).round()); texture.hash(&mut h); }
        }
        h.finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialCmd {
    pub kind: MaterialKind,
    pub rect: Rect,
    /// Straight RGBA widget colour/opacity multiplier (includes menu fade).
    pub tint: [f32; 4],
    pub flip_x: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Quad(Quad),
    Material(MaterialCmd),
    PushClip(Rect),
    PopClip,
}

/// Ordered low-level draw list. Build one per frame, hand it to a backend.
#[derive(Clone, Debug, Default)]
pub struct RenderList { pub cmds: Vec<Cmd> }
impl RenderList {
    pub fn new() -> Self { Self::default() }
    pub fn clear(&mut self) { self.cmds.clear(); }
    pub fn is_empty(&self) -> bool { self.cmds.is_empty() }
    pub fn push(&mut self, c: Cmd) { self.cmds.push(c); }
    pub fn extend(&mut self, o: RenderList) { self.cmds.extend(o.cmds); }
    pub fn quad_count(&self) -> usize { self.cmds.iter().filter(|c| matches!(c, Cmd::Quad(_))).count() }

    /// Textured quad with explicit uv. `tex == NONE` is dropped here.
    pub fn quad(&mut self, tex: TextureId, rect: Rect, uv: [f32; 4], tint: [f32; 4]) {
        if tex != TextureId::NONE && rect.w > 0. && rect.h > 0. { self.cmds.push(Cmd::Quad(Quad::rect(tex, rect, uv, tint))); }
    }
    pub fn sprite(&mut self, tex: TextureId, rect: Rect, tint: [f32; 4]) { self.quad(tex, rect, [0., 0., 1., 1.], tint); }
    pub fn sprite_flip(&mut self, tex: TextureId, rect: Rect, flip_x: bool, tint: [f32; 4]) {
        self.quad(tex, rect, if flip_x { [1., 0., 0., 1.] } else { [0., 0., 1., 1.] }, tint);
    }
    pub fn fill(&mut self, rect: Rect, color: [f32; 4]) { self.quad(TextureId::WHITE, rect, [0., 0., 1., 1.], color); }
    /// Box brush: see [`nine_slice_cells`].
    pub fn nine_slice(&mut self, tex: TexInfo, dest: Rect, margin: [f32; 4], scale: f32, tint: [f32; 4]) {
        if tex.id == TextureId::NONE { return; }
        for (r, uv) in nine_slice_cells(tex.w, tex.h, dest, margin, scale) { self.quad(tex.id, r, uv, tint); }
    }
    /// Image of `size` (w,h) centred on `centre`, rotated clockwise (y down) by `angle_rad`.
    pub fn rotated(&mut self, tex: TextureId, centre: (f32, f32), size: (f32, f32), angle_rad: f32, tint: [f32; 4]) {
        if tex == TextureId::NONE { return; }
        let (s, c) = angle_rad.sin_cos();
        let p = |u: f32, v: f32| { let (x, y) = ((u - 0.5) * size.0, (v - 0.5) * size.1); [centre.0 + x * c - y * s, centre.1 + x * s + y * c] };
        self.cmds.push(Cmd::Quad(Quad { tex, pos: [p(0., 0.), p(1., 0.), p(1., 1.), p(0., 1.)], uv: [0., 0., 1., 1.], tint }));
    }
    pub fn material(&mut self, kind: MaterialKind, rect: Rect, tint: [f32; 4], flip_x: bool) {
        if rect.w > 0. && rect.h > 0. { self.cmds.push(Cmd::Material(MaterialCmd { kind, rect, tint, flip_x })); }
    }
    pub fn push_clip(&mut self, r: Rect) { self.cmds.push(Cmd::PushClip(r)); }
    pub fn pop_clip(&mut self) { self.cmds.push(Cmd::PopClip); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn nine_slice_emits_nine_quads() {
        let mut l = RenderList::new();
        l.nine_slice(TexInfo { id: TextureId(7), w: 64, h: 64 }, Rect::new(10., 10., 200., 100.), [0.25; 4], 1., [1.; 4]);
        assert_eq!(l.quad_count(), 9);
        let area: f32 = l.cmds.iter().map(|c| if let Cmd::Quad(q) = c { let b = q.bounds(); b.w * b.h } else { 0. }).sum();
        assert!((area - 20000.).abs() < 1e-2);
    }
    #[test] fn missing_texture_is_skipped() { let mut l = RenderList::new(); l.sprite(TextureId::NONE, Rect::new(0., 0., 5., 5.), [1.; 4]); assert!(l.is_empty()); }
    #[test] fn rotated_quarter_turn_swaps_extents() {
        let mut l = RenderList::new(); l.rotated(TextureId(3), (50., 50.), (20., 10.), std::f32::consts::FRAC_PI_2, [1.; 4]);
        let Cmd::Quad(q) = &l.cmds[0] else { panic!() }; let b = q.bounds();
        assert!((b.w - 10.).abs() < 1e-3 && (b.h - 20.).abs() < 1e-3);
    }
    #[test] fn cache_key_distinguishes_params() {
        let a = MaterialKind::HudBar { a: [0.1; 3], b: [0.2; 3] }; let b = MaterialKind::HudBar { a: [0.1; 3], b: [0.3; 3] };
        assert_ne!(a.cache_key(), b.cache_key()); assert_eq!(a.cache_key(), a.clone().cache_key());
    }
}

