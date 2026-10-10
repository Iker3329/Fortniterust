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
    MinimapStorms { map: TextureId, current: [f32; 3] },
    GamePhaseIcon { shader: u8, bg: TextureId, icon: TextureId },
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
            MaterialKind::MinimapStorms { map, current } => { 10u8.hash(&mut h); map.hash(&mut h); for v in current { f(&mut h, *v); } }
            MaterialKind::GamePhaseIcon { shader, bg, icon } => { 11u8.hash(&mut h); shader.hash(&mut h); bg.hash(&mut h); icon.hash(&mut h); }
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
    /// INVENTORY FIX3: an `SBackgroundBlur` post-process pass over what is already drawn under `rect` (see [`BlurCmd`]).
    Blur(BlurCmd),
}

/// INVENTORY FIX3 (inventory_gaps G5): UE 4.16 `SBackgroundBlur::OnPaint` + `FSlatePostProcessor::BlurRect`. `rect` = the widget's clipping rect (pixels);
/// a separable Gaussian (`ComputeWeights`, BILINEAR_FILTER_METHOD: the taps of [`BlurCmd::taps`], weights `1/sqrt(2 pi s^2) exp(-d^2 / 2 s^2)`, not
/// normalised), sigma `sigma` in (downsampled) texels, at 1/`downsample` resolution when `downsample` > 0
/// (4-tap box downsample, bilinear upsample). The output alpha is 1 (`GaussianBlurMain` returns `float4(rgb, 1)`). A backend without access to the
/// target's pixels skips it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurCmd { pub rect: Rect, pub kernel_size: u32, pub sigma: f32, pub downsample: u32 }
impl BlurCmd {
    /// `SBackgroundBlur::ComputeEffectiveKernelSize` + `OnPaint`: `strength` = `BlurStrength * (bApplyAlphaToBlur ? tint alpha : 1)`; kernel
    /// `RoundToInt(strength * 3)`, halved (quartered from 64) and downsampled when > 9 (`Slate.BackgroundBlurDownsample` 1), made odd, clamped to 3..255
    /// (`Slate.BackgroundBlurMaxKernelSize`); sigma `max(0.5, strength) / downsample`. None when `strength` <= 0 (nothing is drawn).
    pub fn from_strength(rect: Rect, strength: f32) -> Option<BlurCmd> { Self::with_radius(rect, strength, None) }
    /// `radius` = `BlurRadius` when `bOverrideAutoRadiusCalculation` (`SetBlurRadius`), replacing `RoundToInt(strength * 3)`.
    pub fn with_radius(rect: Rect, strength: f32, radius: Option<i32>) -> Option<BlurCmd> {
        if !(strength > 0.) || rect.w <= 0. || rect.h <= 0. { return None; }
        let mut k = radius.unwrap_or((strength * 3.).round() as i32);
        let mut ds = 0;
        if k > 9 { ds = if k >= 64 { 4 } else { 2 }; k /= ds; }
        if k % 2 == 0 { k += 1; }
        let k = k.clamp(3, 255) as u32;
        let mut sigma = strength.max(0.5);
        if ds > 0 { sigma /= ds as f32; }
        Some(BlurCmd { rect, kernel_size: k, sigma, downsample: ds as u32 })
    }
    /// `GetWeight` of SlatePostProcessor.cpp.
    pub fn weight(&self, d: f32) -> f32 { let s2 = self.sigma * self.sigma; (1. / (2. * std::f32::consts::PI * s2).sqrt()) * (-(d * d) / (2. * s2)).exp() }
    /// The discrete taps the bilinear-packed shader samples, (offset, weight); each non-zero offset is sampled on both sides. `ComputeWeights` packs
    /// 0, the pair (1, 2), then the pairs (X, X+1), (X+2, X+3) for X = 3, 7, .. < kernel_size; `GaussianBlurMain` reads all of them (kernel 7: 0..=6).
    pub fn taps(&self) -> Vec<(i32, f32)> {
        let k = self.kernel_size as i32;
        let mut d = vec![0, 1, 2];
        let mut x = 3; while x < k { d.extend([x, x + 1, x + 2, x + 3]); x += 4; }
        d.into_iter().map(|d| (d, self.weight(d as f32))).collect()
    }
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
    /// INVENTORY FIX3: an `SBackgroundBlur` pass (see [`BlurCmd::from_strength`]).
    pub fn blur(&mut self, rect: Rect, strength: f32, radius: Option<i32>) { if let Some(b) = BlurCmd::with_radius(rect, strength, radius) { self.cmds.push(Cmd::Blur(b)); } }
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
    /// INVENTORY FIX3: `QuantitySelector_C.BackgroundBlur_2` (BlurStrength 2): kernel RoundToInt(6) -> odd 7, sigma 2, no downsample, taps 0..=6;
    /// the main menu's 16: kernel 48 -> downsample 2 -> 24 -> 25, sigma 8.
    #[test] fn background_blur_params_follow_sbackgroundblur() {
        let r = Rect::new(0., 0., 100., 100.);
        let b = BlurCmd::from_strength(r, 2.).unwrap();
        assert_eq!((b.kernel_size, b.sigma, b.downsample), (7, 2., 0));
        assert_eq!(b.taps().iter().map(|t| t.0).collect::<Vec<_>>(), (0..=6).collect::<Vec<_>>());
        let m = BlurCmd::from_strength(r, 16.).unwrap();
        assert_eq!((m.kernel_size, m.sigma, m.downsample), (25, 8., 2));
        assert_eq!(m.taps().last().unwrap().0, 26);
        assert!(BlurCmd::from_strength(r, 0.).is_none());
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

