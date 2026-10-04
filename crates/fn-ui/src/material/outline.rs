//! CPU ports of the 1.8 material-driven buttons and panels used by the Esc menu (MainMenu), its backdrop and the
//! confirmation window.
//!
//! The TopBar `ButtonStyle-Outline*`, `ButtonStyle-Primary-M`, `Border_ShellUberBG` and `Border-MainModal/ModalHeader` styles are
//! NOT textures but MD_UI material instances (`M_MenuButton_Master`, `M-Button-Master`, `M_UI_GradientPanel` and the
//! sphere-gradient permutation `M-Border-ShellUberBG`). The cooked UE4.16 materials keep no editor graph, so the maths comes from
//! the pixel shaders UE compiled into them: `node tools/menu-shaders.cjs` disassembles them (reports/menu-shaders/*.asm, same
//! technique as the sky) and evaluates their serialized uniform expression sets with every instance's effective parameters
//! (reports/menu-shaders/cbuffers.json, `tools/menu-material-eval.cjs`). The functions below are line-by-line ports of
//! `outline.plain.asm`, `primary.plain.asm`, `gradientPanel.plain.asm` and `shellUberBG.plain.asm` for the case the menu uses
//! (`s_UseShineSwipe` / `Shine` = 0, so the shine-swipe terms vanish); the tests compare every constant with cbuffers.json.
//!
//! EXACT (per pixel): the box UVs. Slate draws these material brushes with DrawAs Box (9-slice): TexCoord0 is the 9-slice UV (margin
//! bands cover `margin` of UV over `margin * ImageSize * unit` pixels, the middle stretches), TexCoord1 is 0..1 stretched over the
//! element; the Outline band / switch / vertical gradient and the Primary-M shape use TexCoord0, the gradients of Primary-M and the
//! panels use TexCoord1. UE material vectors are LINEAR colours; Slate converts the final colour to sRGB, so `lin_to_srgb` is applied.
//! APPROXIMATE / NOT REPRODUCED: pixel centres are evaluated at (x+0.5) with the rect rounded to whole pixels (Slate interpolates
//! vertices), the Trig units of the gradient `Rotation` (radians, INFERRED), BackgroundBlur (a native post effect) and the shine swipe
//! (it is off in every state the menu uses).
pub use super::types::{PanelKind, ProcStyle, Vis};

/// Linear -> sRGB transfer (UE FLinearColor::ToFColor(true)).
pub fn srgb(c: f32) -> f32 { let c = c.clamp(0., 1.); if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1. / 2.4) - 0.055 } }
pub fn lin_to_srgb(c: [f32; 3]) -> [f32; 3] { [srgb(c[0]), srgb(c[1]), srgb(c[2])] }
fn rgba(c: [f32; 3], a: f32) -> [f32; 4] { let s = lin_to_srgb(c); [s[0], s[1], s[2], a] }


// ---- M_MenuButton_Master instances (linear colours; values are the `effective` parameters in menu-widgets.json) ----
pub const OUTLINE_BASE_COLOR: [f32; 3] = [0., 0.002125, 0.017642];
pub const OUTLINE_BLUE: [f32; 3] = [0.318547, 0.603828, 1.];
pub const OUTLINE_BLUE_PRESSED: [f32; 3] = [0.445201, 0.693872, 1.];
pub const OUTLINE_YELLOW_HOVER: [f32; 3] = [1., 1., 0.072272];
pub const OUTLINE_RED: [f32; 3] = [1., 0.155926, 0.122139];
/// `Box Size` (cb[10].x of M_MenuButton_Master): half-extent of the unfilled interior in UV; the outline band is `0.5 - 0.433` of UV.
pub const OUTLINE_BOX_SIZE: f32 = 0.433;
/// Brush `ImageSize` of every Outline style brush (ButtonStyle-Outline*: size 48x48) and its box margins as UV fractions (L/R, T/B).
pub const OUTLINE_IMAGE: f32 = 48.;
pub const OUTLINE_MARGIN: (f32, f32) = (0.1, 0.1);
/// ButtonStyle-OutlineLeftShade (the icon column) uses margin 0.25 / 0.1.
pub const OUTLINE_SHADE_MARGIN: (f32, f32) = (0.25, 0.1);

/// Effective parameters of one material instance of the Outline family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatLook {
    pub primary: [f32; 3], pub base: [f32; 3], pub dark_base_alpha: f32,
    pub gradient_alpha: f32, pub gradient_min_alpha: f32, pub outline_alpha: f32,
    pub inner_fill_offset: f32, pub switch_offset: f32, pub box_size: f32,
}

/// Material parameters for a state of an Outline-family style (see the style assets: Red uses the plain Outline
/// hovered and disabled materials, only Base/Pressed have `_Red` variants).
pub fn look(style: ProcStyle, vis: Vis) -> MatLook {
    let (shade, red) = (style.left_shade(), style.red());
    let (primary, outline_alpha, ga, gm) = match vis {
        Vis::Normal => (if red { OUTLINE_RED } else { OUTLINE_BLUE }, 0.8, 0.4, 0.75),
        Vis::Hover => (OUTLINE_YELLOW_HOVER, 1., 0.4, 0.75),
        Vis::Pressed => (if red { OUTLINE_RED } else { OUTLINE_BLUE_PRESSED }, 1., 0.8, 1.),
        Vis::Disabled => (OUTLINE_BLUE, 0.4, 0.2, 0.75),
    };
    MatLook { primary, base: OUTLINE_BASE_COLOR, dark_base_alpha: 0.6, gradient_alpha: ga, gradient_min_alpha: gm, outline_alpha,
        inner_fill_offset: if shade { 0.5 } else { 0. }, switch_offset: if shade { 1. } else { 0. }, box_size: OUTLINE_BOX_SIZE }
}

fn smooth01(x: f32) -> f32 { let x = x.clamp(0., 1.); x * x * (3. - 2. * x) }
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t] }
fn lerp1(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }

/// Slate 9-slice UV along one axis for a pixel centre `p` (pixels from the element's start) in an element of `size` pixels: the two margin
/// bands cover `margin_uv` of UV over `margin_px` pixels each (never more than half the element), the middle stretches over the rest.
pub fn slice_uv(p: f32, size: f32, margin_px: f32, margin_uv: f32) -> f32 {
    let m = margin_px.min(size * 0.5).max(1e-4);
    if p < m { p / m * margin_uv } else if p > size - m { 1. - (size - p) / m * margin_uv } else {
        let mid = (size - 2. * m).max(1e-4); margin_uv + (p - m) / mid * (1. - 2. * margin_uv)
    }
}

/// `outline.plain.asm` (shine off): colour/opacity of one pixel of an Outline-family material for TexCoord0 `(u, v)` (the 9-slice box UV).
/// Returns STRAIGHT-alpha sRGB RGBA (the opacity is the material's Opacity output).
///   band  = min(1, |ceil(|uv - 0.5| - BoxSize)|)          (1 inside the outline band, 0 inside the box)
///   fill  = u > InnerFillOffset ? 1 - SwitchOffset : SwitchOffset
///   s     = smoothstep(v)                                   (vertical gradient)
///   rgb   = lerp(Primary, Base, (1 - band) * s * DarkBaseAlpha)
///   alpha = sat(band + fill) * ((1 - band) * (1 - s * (1 - MinAlpha)) * GlobalAlpha + band * OutlineAlpha)
pub fn outline_pixel(l: &MatLook, u: f32, v: f32) -> [f32; 4] {
    let (cx, cy) = ((u - 0.5).abs() - l.box_size, (v - 0.5).abs() - l.box_size);
    let ceil01 = |x: f32| if x > 0. { 1.0f32 } else { 0.0 };
    let band = (ceil01(cx).hypot(ceil01(cy))).min(1.);
    let fill = if u > l.inner_fill_offset { 1. - l.switch_offset } else { l.switch_offset };
    let s = smooth01(v);
    let g = (1. - s) + l.gradient_min_alpha * s;
    let a0 = (1. - band) * g * l.gradient_alpha + band * l.outline_alpha;
    let rgb = lin_to_srgb(lerp3(l.primary, l.base, (1. - band) * s * l.dark_base_alpha));
    [rgb[0], rgb[1], rgb[2], ((band + fill).clamp(0., 1.) * a0).clamp(0., 1.)]
}

// ---- M-Button-Master permutation of the Primary-M style (primary.plain.asm; cbuffers.json slots in brackets) -------------------
/// Size (cb[16].w) and Drop Shadow Distance (cb[17].z): half extents of the face / shadow boxes in UV.
pub const PM_SIZE: f32 = 0.3;
pub const PM_SHADOW_DIST: f32 = 0.22;
/// SS Min / SS Max (cb[17].x/y; note Max < Min: the smoothstep is inverted), DropShadow Alpha (cb[17].w), LG SS Min_1/Max_1 (cb[16].y/z).
pub const PM_SS_MIN: f32 = 0.18;
pub const PM_SS_MAX: f32 = 0.15;
pub const PM_SHADOW_ALPHA: f32 = 1.;
pub const PM_LG: (f32, f32) = (0., 1.);
/// Primary-M brush: ImageSize 48x48, box margins 0.25 (L/R) / 0.5 (T/B).
pub const PM_IMAGE: f32 = 48.;
pub const PM_MARGIN: (f32, f32) = (0.25, 0.5);
/// Opacity of the Alpha parameter per state is `primary_m_look(vis).3`.
/// One pixel of M-Button-Primary-M-*: `(u, v)` = TexCoord0 (9-slice box UV), `v1` = TexCoord1.y (0..1 over the element height).
/// Face = rounded box (inverted smoothstep of the SDF), colour = lerp(RGBA0, RGBA1, smoothstep(v1)) with a Drop Shadow coloured lip along
/// the lower edge (`floor(v + 0.7)` gates it to v >= 0.3). Returns STRAIGHT-alpha sRGB RGBA.
pub fn primary_pixel(vis: Vis, u: f32, v: f32, v1: f32) -> [f32; 4] {
    let (rgba0, rgba1, shadow, alpha) = primary_m_look(vis);
    let (px, py) = (u - 0.5, v - 0.5);
    let sdf = |ex: f32, ey: f32| ((px.abs() - ex).max(0.)).hypot((py.abs() - ey).max(0.));
    let inv = |d: f32| smooth01((d - PM_SS_MIN) / (PM_SS_MAX - PM_SS_MIN));
    let shadow_mix = (1. - inv(sdf(PM_SIZE, PM_SHADOW_DIST))) * if v + 0.7 >= 1. { 1. } else { 0. } * PM_SHADOW_ALPHA;
    let face = inv(sdf(PM_SIZE, PM_SIZE));
    let g = smooth01((v1 - PM_LG.0) / (PM_LG.1 - PM_LG.0));
    let base = lerp3(rgba0, rgba1, g);
    let c = lerp3(lerp3(shadow, base, face), shadow, shadow_mix);
    let rgb = lin_to_srgb([c[0].max(0.), c[1].max(0.), c[2].max(0.)]);
    [rgb[0], rgb[1], rgb[2], (face * alpha).clamp(0., 1.)]
}

// ---- gradient panels: M_UI_GradientPanel (Border-MainModal / ModalHeader) and the ShellUberBG sphere permutation -----------------
/// ShellUberBG (sphere gradient): RGBA0 / RGBA1 (linear, alpha = opacity), Sphere Gradient Pos U/V, Radius, Density, Gradient Contrast / Mult, Global Alpha.
pub const UBER_RGBA0: [f32; 4] = [0., 0.002125, 0.017642, 1.];
pub const UBER_RGBA1: [f32; 4] = [0., 0.004391, 0.064803, 0.6];
pub const UBER_POS: (f32, f32) = (1., 2.2);
pub const UBER_RADIUS: f32 = 2.;
pub const UBER_DENSITY: f32 = 1.;
pub const UBER_CONTRAST: f32 = 0.54317;
pub const UBER_MULT: f32 = 1.886526;
pub const UBER_GLOBAL_ALPHA: f32 = 1.;
/// Gradient Rotation (M-Border-MainModal / ModalHeader): the shader reads cb[3].xy = (cos x, -sin x) with x = Rotation * 0.25 (INFERRED radians).
pub const PANEL_ROTATION: f32 = 1.;
pub const PANEL_ROTATION_SCALE: f32 = 0.25;
/// Scale 1, Position 0, LG SS Min 0, LG SS Max 1, Multiply Base 1, Global Alpha 1 (identical for both modal panels).
pub const PANEL_LG: (f32, f32) = (0., 1.);
pub const PANEL_GLOBAL_ALPHA: f32 = 1.;

/// Linear gradient direction (cb[3].xy of M_UI_GradientPanel) for a Rotation parameter.
pub fn panel_direction(rotation: f32) -> [f32; 2] { let a = rotation * PANEL_ROTATION_SCALE; [a.cos(), -a.sin()] }

/// One pixel of a gradient panel at TexCoord1 `(u, v)` (0..1 stretched over the element). STRAIGHT-alpha sRGB RGBA.
pub fn panel_pixel(kind: PanelKind, u: f32, v: f32) -> [f32; 4] {
    match kind {
        PanelKind::Backdrop => {
            // gradientPanel sphere permutation (shellUberBG.plain.asm): radial falloff centred at (Pos U, Pos V) outside the element.
            let d = (u - UBER_POS.0).hypot(v - UBER_POS.1);
            let n = 1. - d / UBER_RADIUS;
            let y = (n * UBER_DENSITY).powi(2) * 1.442_695;
            let w = (1. - 1. / y.exp()).max(1e-6);
            let lg = if n >= 0. && n.abs() > 1e-5 { w.ln() } else { -19.931_568 };
            let t = ((lg * UBER_CONTRAST).exp() * UBER_MULT).clamp(0., 1.);
            let rgb = lin_to_srgb(lerp3([UBER_RGBA0[0], UBER_RGBA0[1], UBER_RGBA0[2]], [UBER_RGBA1[0], UBER_RGBA1[1], UBER_RGBA1[2]], t));
            [rgb[0], rgb[1], rgb[2], (lerp1(UBER_RGBA0[3], UBER_RGBA1[3], t).clamp(0., 1.) * UBER_GLOBAL_ALPHA).clamp(0., 1.)]
        }
        PanelKind::ModalPanel | PanelKind::ModalHeader => {
            let (c0, c1) = if kind == PanelKind::ModalPanel { (MODAL_RGBA0, MODAL_RGBA1) } else { (HEADER_RGBA0, HEADER_RGBA1) };
            let dir = panel_direction(PANEL_ROTATION);
            let x = dir[0] * (u - 0.5) + dir[1] * (v - 0.5) + 0.5;
            let t = smooth01((x - PANEL_LG.0) / (PANEL_LG.1 - PANEL_LG.0));
            let rgb = lin_to_srgb(lerp3(c0, c1, t));
            [rgb[0], rgb[1], rgb[2], PANEL_GLOBAL_ALPHA]
        }
    }
}

/// RGBA8 (STRAIGHT alpha) bitmap of a panel kind evaluated on a `w` x `h` grid of pixel centres (the renderer stretches it with linear filtering).
pub fn panel_bitmap(kind: PanelKind, w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h { for x in 0..w {
        let c = panel_pixel(kind, (x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
        out.extend(c.map(|k| (k.clamp(0., 1.) * 255. + 0.5) as u8));
    } }
    out
}

// ---- M-Button-Master instances (Primary-M; linear) ------------------------------------------------------------------
/// (RGBA0, RGBA1, drop shadow, alpha) per state, from M-Button-Primary-M-{Base,Hovered,Pressed,Disabled}.
pub fn primary_m_look(vis: Vis) -> ([f32; 3], [f32; 3], [f32; 3], f32) {
    match vis {
        Vis::Normal => ([0.723055, 0.799103, 0.913099], [0.417885, 0.571125, 0.799103], [0.097587, 0.162029, 0.332452], 1.),
        Vis::Hover => ([1., 0.955973, 0.445201], [1., 0.879622, 0.076185], [0.323143, 0.238398, 0.012983], 1.),
        Vis::Pressed => ([1., 1., 1.], [0.973445, 1., 0.651406], [1., 1., 0.], 1.),
        Vis::Disabled => ([0.254152, 0.423268, 0.715694], [0.093059, 0.158961, 0.332452], [0., 0., 0.], 0.6),
    }
}
/// TextStyle-Button-Primary-M colour (linear 0.008023, 0.01096, 0.026241) and its Disabled alpha 0.498.
pub const PRIMARY_M_TEXT: [f32; 3] = [0.008023, 0.01096, 0.026241];

// ---- panels ------------------------------------------------------------------------------------------------------------
/// M-Border-MainModal (M_UI_GradientPanel): RGBA0 / RGBA1 (linear).
pub const MODAL_RGBA0: [f32; 3] = [0.965, 0.965, 0.965];
pub const MODAL_RGBA1: [f32; 3] = [0.5616, 0.633331, 0.72];
/// M-Border-ModalHeader-procedural: RGBA0 / RGBA1 (linear).
pub const HEADER_RGBA0: [f32; 3] = [0.0045, 0.006909, 0.025];
pub const HEADER_RGBA1: [f32; 3] = [0.017642, 0.022174, 0.049707];
/// TextStyle-Base-XS-Black / ContextualHelpText BlueGray text colours (linear).
pub const TEXT_BLACK: [f32; 3] = [0., 0., 0.];
pub const TEXT_BLUE_GRAY: [f32; 3] = [0.520996, 0.552012, 0.745404];

pub fn srgba(c: [f32; 3], a: f32) -> [f32; 4] { rgba(c, a) }
