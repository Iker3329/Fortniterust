//! Fortnite 1.8 Settings widgets rebuilt from the decoded widget trees (reports/menu-widgets.json): exact Slate-unit geometry
//! and the original text styles. Look of the material-driven pieces: see [`mat`] (ported shaders once `uimat` provides them,
//! otherwise flat interim fills labelled APPROXIMATE).
//!
//! RECOVERED geometry (Slate units, DPI scale applied by the caller):
//! * rotator row (`RotatorSelector`): label SizeBox 300 + slot padding right 20 (text right aligned, TextStyle-Base-XS-Blue = NotoSans
//!   Regular 16, colour 0.008/0.011/0.026 linear) | control SizeBox min 400, HAlign_Left, containing the `TextRotator` (a CommonButton,
//!   style ButtonStyle-RotatorBorder: MinWidth/MinHeight 32): `<` arrow button 18x20 (slot padding left 8) | value text (TextStyle-Base-XS-B,
//!   NotoSans Bold 16 white, centred, fill) | `>` arrow (padding right 8). Row slot padding 2 on every side.
//! * slider row (`OptionsMenuSlider`): SliderText SizeBox 300 + padding right 20 | SizeBox min 400 -> Button_0 (NormalPadding 4) -> overlay of the
//!   AnalogSlider, `SliderBackground` image 32x40 (material OptionsSliderFill, scalar fillPercent) and `SliderValue` (padding left 15, TextStyle-Base-XS-B,
//!   text = value with exactly two decimals). The hover highlight is the `M_Button_Settings_Select` material in yellow (1, 0.916667, 0).
//! * segmented row (`OptionsMenuRowSelector`): label SizeBox 300 + padding 20 | `SelectorButtonsBox` (fill) holding one `ButtonStyle-Primary-Radio-M`
//!   button per option (MinWidth 160, MinHeight 48, each slot Fill 1).
use crate::menu::outline::srgba;
use crate::menu::*;
use crate::menu::theme;

// ---- recovered constants (Slate units) -------------------------------------------------------------------------------
pub const LABEL_W: f32 = 300.;
pub const LABEL_GAP: f32 = 20.;
pub const CONTROL_MIN_W: f32 = 400.;
pub const ROW_SLOT_PAD: f32 = 2.;
pub const ROTATOR_H: f32 = 32.;
pub const ARROW_W: f32 = 18.;
pub const ARROW_H: f32 = 20.;
pub const ARROW_PAD: f32 = 8.;
pub const SLIDER_BAR_H: f32 = 40.;
pub const SLIDER_BUTTON_PAD: f32 = 4.;
pub const SLIDER_ROW_H: f32 = SLIDER_BAR_H + 2. * SLIDER_BUTTON_PAD;
pub const SEGMENT_MIN_W: f32 = 160.;
pub const SEGMENT_H: f32 = 48.;
pub const FONT_LABEL: f32 = 16.;

/// TextStyle-Base-XS-Blue colour (linear 0.008023, 0.01096, 0.026241) as drawn (sRGB).
pub fn label_color() -> [f32; 4] { srgba([0.008023, 0.01096, 0.026241], 1.) }
pub fn label_disabled() -> [f32; 4] { srgba([0.008023, 0.01096, 0.026241], 0.45) }
pub const WHITE: [f32; 4] = [1., 1., 1., 1.];
pub const YELLOW: [f32; 4] = [1., 0.916667, 0., 1.];

/// The cooked UI materials of the settings widgets (pages/uimat.rs = their ported pixel shaders), drawn through `Cmd::Material`.
pub mod mat {
    use super::*;
    use super::super::uimat::{Mat, MatState};
    const W: [f32; 4] = [1., 1., 1., 1.];
    /// TextRotator background: the `MainBorder` style Border-ItemInfo-Blank-DkBlue (M-ItemInfo-Blank-DkBlue, Box, margin 0.4); the widget's own
    /// Background override (M-ItemInfo-Blank) is replaced by the style's brush when a CommonBorder style is set.
    pub fn rotator_bg(ui: &mut Ui, r: Rect) { ui.material(Mat::ItemInfoBlankDkBlue, MatState::Base, r, 0., W); }
    /// ButtonStyle-RotatorBorder hovered/focused: M_Button_Settings_Select, tint (1, 0.921582, 0).
    pub fn rotator_hover(ui: &mut Ui, r: Rect) { ui.material(Mat::RotatorBorder, MatState::Hovered, r, 0., W); }
    /// `OptionsMenuSlider` Button_0 hovered brush (M_Button_Settings_Select, tint (1, 0.916667, 0)).
    pub fn slider_hover(ui: &mut Ui, r: Rect) { ui.material(Mat::SettingsSelect, MatState::Hovered, r, 0., W); }
    pub fn slider_fill(ui: &mut Ui, bar: Rect, fill: f32, light: bool) { ui.material(if light { Mat::SliderFillLight } else { Mat::SliderFill }, MatState::Base, bar, fill, W); }
    /// ButtonStyle-Primary-Radio-M.
    pub fn radio(ui: &mut Ui, r: Rect, selected: bool, hover: bool, down: bool, enabled: bool) {
        let st = if !enabled { MatState::Disabled } else { match (selected, hover, down) { (true, _, true) => MatState::SelectedPressed, (true, true, _) => MatState::SelectedHovered, (true, ..) => MatState::Selected, (false, _, true) => MatState::Pressed, (false, true, _) => MatState::Hovered, _ => MatState::Base } };
        ui.material(Mat::RadioM, st, r, 0., W);
    }
    /// ButtonStyle-Left / ButtonStyle-Right arrow (M-Button-Outline-Base-Left; the right one is the same material mirrored by ButtonRight's RenderTransform).
    pub fn arrow(ui: &mut Ui, r: Rect, right: bool, hover: bool, down: bool, enabled: bool) {
        let st = if !enabled { MatState::Disabled } else if down { MatState::Pressed } else if hover { MatState::Hovered } else { MatState::Base };
        ui.material_flip(Mat::ArrowLeft, st, r, 0., W, right);
    }
    pub fn tab_manage(ui: &mut Ui, r: Rect, selected: bool, hover: bool) {
        let st = match (selected, hover) { (true, true) => MatState::SelectedHovered, (true, false) => MatState::Selected, (false, true) => MatState::Hovered, _ => return };
        ui.material(Mat::TabManage, st, r, 0., W);
    }
    pub fn tab_secondary(ui: &mut Ui, r: Rect, selected: bool, hover: bool) {
        let st = match (selected, hover) { (true, true) => MatState::SelectedHovered, (true, false) => MatState::Selected, (false, true) => MatState::Hovered, _ => return };
        ui.material(Mat::TabSecondary, st, r, 0., W);
    }
    pub fn bottom_button(ui: &mut Ui, r: Rect, hover: bool, down: bool, enabled: bool) {
        let st = if !enabled { MatState::Disabled } else if down { MatState::Pressed } else if hover { MatState::Hovered } else { MatState::Base };
        ui.material(Mat::ButtonBar, st, r, 0., W);
    }
}

/// Vertical column of rows in pixels (rows are laid out in Slate units, slot padding included by the caller).
pub struct Col { pub x: f32, pub y: f32, pub w: f32, pub scale: f32 }
impl Col {
    pub fn new(x: f32, y: f32, w: f32, scale: f32) -> Self { Self { x, y, w, scale } }
    /// Next slot of `h` units (outer height incl. slot padding `pad`: [l,t,r,b]); returns the inner rect (padding removed).
    pub fn slot(&mut self, h: f32, pad: [f32; 4]) -> Rect {
        let s = self.scale;
        let r = Rect::new(self.x + pad[0] * s, self.y + pad[1] * s, (self.w - (pad[0] + pad[2]) * s).max(0.), (h - pad[1] - pad[3]).max(0.) * s);
        self.y += h * s;
        r
    }
    pub fn height_px(&self, top: f32) -> f32 { self.y - top }
}

/// (label rect, control rect) of a labelled row inside the inner row rect (units are scaled by `s`).
pub fn row_rects(r: Rect, s: f32) -> (Rect, Rect) {
    let label = Rect::new(r.x, r.y, LABEL_W * s, r.h);
    let cx = r.x + (LABEL_W + LABEL_GAP) * s;
    (label, Rect::new(cx, r.y, (CONTROL_MIN_W * s).min((r.x + r.w - cx).max(0.)), r.h))
}

pub struct RotResp { pub delta: i32, pub hot: bool, pub focused: bool }

/// `RotatorSelector` row. `value` is the selected option text. Returns the requested change (-1 / +1) and hover/focus state.
pub fn rotator_row(ui: &mut Ui, key: &str, r: Rect, label: &str, value: &str, enabled: bool) -> RotResp {
    let s = ui.vp.scale;
    let (lr, cr) = row_rects(r, s);
    let id = ui.id(key);
    let it = ui.interact(id, cr, enabled, true);
    let (lid, rid) = (id_with(id, 1), id_with(id, 2));
    let left = Rect::new(cr.x + ARROW_PAD * s, cr.y + (cr.h - ARROW_H * s) * 0.5, ARROW_W * s, ARROW_H * s);
    let right = Rect::new(cr.x + cr.w - (ARROW_PAD + ARROW_W) * s, left.y, ARROW_W * s, ARROW_H * s);
    let il = ui.interact(lid, left, enabled, false);
    let ir = ui.interact(rid, right, enabled, false);
    let mut delta = 0;
    if il.clicked { delta -= 1; }
    if ir.clicked { delta += 1; }
    if enabled && ui.st.focus == Some(id) && !ui.is_inert() { delta += ui.count_nav(NavKey::Right) as i32 - ui.count_nav(NavKey::Left) as i32; }
    let hot = it.hot || il.hot || ir.hot;
    ui.text(lr, label, TextStyle::new(FontId::Regular, FONT_LABEL, if enabled { label_color() } else { label_disabled() }), HAlign::Right);
    mat::rotator_bg(ui, cr);
    if enabled && (hot || it.focused) { mat::rotator_hover(ui, cr); }
    mat::arrow(ui, left, false, il.hot, il.down, enabled);
    mat::arrow(ui, right, true, ir.hot, ir.down, enabled);
    let vr = Rect::new(left.x + left.w, cr.y, (right.x - left.x - left.w).max(0.), cr.h);
    ui.text(vr, value, TextStyle::new(FontId::Bold, FONT_LABEL, if enabled { WHITE } else { [1., 1., 1., 0.5] }), HAlign::Center);
    RotResp { delta, hot, focused: it.focused }
}

pub struct SliderResp { pub value: Option<f32>, pub hot: bool, pub focused: bool }

/// `OptionsMenuSlider` row; `v` in `min..=max`.
pub fn slider_row(ui: &mut Ui, key: &str, r: Rect, label: &str, v: f32, min: f32, max: f32, step: f32, decimals: usize, enabled: bool) -> SliderResp {
    let s = ui.vp.scale;
    slider_row_ex(ui, key, r, label, v, min, max, step, decimals, enabled, label_color(), LABEL_W * s, (LABEL_W + LABEL_GAP) * s)
}
/// Slider row with explicit label colour, label box width and control x offset (pixels): the Light variant (Brightness) has two Fill(1) boxes.
pub fn slider_row_ex(ui: &mut Ui, key: &str, r: Rect, label: &str, v: f32, min: f32, max: f32, step: f32, decimals: usize, enabled: bool, label_col: [f32; 4], label_w: f32, ctl_dx: f32) -> SliderResp {
    let s = ui.vp.scale;
    let lr = Rect::new(r.x, r.y, label_w, r.h);
    let cr = Rect::new(r.x + ctl_dx, r.y, CONTROL_MIN_W * s, r.h);
    let track = Rect::new(cr.x + SLIDER_BUTTON_PAD * s, cr.y + SLIDER_BUTTON_PAD * s, cr.w - 2. * SLIDER_BUTTON_PAD * s, SLIDER_BAR_H * s);
    let id = ui.id(key);
    let it = ui.interact(id, cr, enabled, true);
    let mut value = None;
    if enabled && it.grabbed {
        if let Some((mx, _)) = ui.mouse() { let t = ((mx - track.x) / track.w.max(1.)).clamp(0., 1.); value = Some(crate::menu::widgets::snap_value(min + t * (max - min), min, max, step)); }
    }
    if enabled && ui.st.focus == Some(id) && !ui.is_inert() {
        let d = ui.count_nav(NavKey::Right) as f32 - ui.count_nav(NavKey::Left) as f32;
        if d != 0. { value = Some(crate::menu::widgets::snap_value(v + d * step, min, max, step)); }
    }
    let shown = value.unwrap_or(v);
    ui.text(lr, label, TextStyle::new(FontId::Regular, FONT_LABEL, if enabled { label_col } else { label_disabled() }), HAlign::Right);
    if enabled && (it.hot || it.focused) { mat::slider_hover(ui, cr); }
    mat::slider_fill(ui, track, if max > min { (shown - min) / (max - min) } else { 0. }, false);
    let txt = crate::menu::widgets::format_value(shown, decimals, "");
    ui.text(Rect::new(track.x + 15. * s, track.y, track.w - 15. * s, track.h), &txt, TextStyle::new(FontId::Bold, FONT_LABEL, if enabled { WHITE } else { [1., 1., 1., 0.5] }), HAlign::Left);
    SliderResp { value: value.filter(|x| (*x - v).abs() > f32::EPSILON), hot: it.hot, focused: it.focused }
}

/// `OptionsMenuRowSelector` row: returns (clicked option, hovered option, focused).
pub fn segment_row(ui: &mut Ui, key: &str, r: Rect, label: &str, names: &[&str], selected: Option<usize>, enabled: bool) -> (Option<usize>, Option<usize>, bool) {
    let s = ui.vp.scale;
    let (lr, _) = row_rects(r, s);
    ui.text(lr, label, TextStyle::new(FontId::Regular, FONT_LABEL, if enabled { label_color() } else { label_disabled() }), HAlign::Right);
    let bx = r.x + (LABEL_W + LABEL_GAP) * s;
    let n = names.len().max(1) as f32;
    let bw = ((r.x + r.w - bx) / n).max(0.);
    let (mut clicked, mut hovered, mut focused) = (None, None, false);
    for (i, name) in names.iter().enumerate() {
        let br = Rect::new(bx + i as f32 * bw, r.y, bw, r.h);
        let id = ui.id(&format!("{key}.seg{i}"));
        let it = ui.interact(id, br, enabled, true);
        let sel = selected == Some(i);
        mat::radio(ui, br, sel, it.hot || it.focused, it.down, enabled);
        ui.text(br, name, TextStyle::new(FontId::Bold, 18., srgba([0.008023, 0.01096, 0.026241], if enabled { 1. } else { 0.5 })), HAlign::Center);
        if it.clicked && enabled { clicked = Some(i); }
        if it.hot { hovered = Some(i); }
        if it.focused { focused = true; hovered = hovered.or(Some(i)); }
    }
    (clicked, hovered, focused)
}

/// `HorizontalTabList` of `ButtonStyle-Tab-Secondary` buttons (MinWidth 180, MinHeight 80, tab strip 88 high) centred in `strip`.
/// Returns true when the selection changed.
pub fn secondary_rects(fonts: &Fonts, s: f32, strip: Rect, labels: &[&str]) -> Vec<Rect> {
    let px = (18. * s).round().max(theme::MIN_TEXT_PX);
    // ButtonStyle-Tab-Secondary: MinWidth 180, no ButtonPadding; IconTabButton text slot padding 4+4 (LeftSideImage collapsed); TextStyle-Tab-Secondary = NotoSans Bold 18
    let widths: Vec<f32> = labels.iter().map(|l| (fonts.measure(FontId::Bold, l, px) + 8. * s).max(180. * s)).collect();
    let total: f32 = widths.iter().sum::<f32>() + 12. * s; // TabButtonBox padding 6 + 6
    let mut x = strip.x + (strip.w - total) * 0.5 + 6. * s;
    let y = strip.y + (strip.h - 80. * s) * 0.5;
    widths.iter().map(|w| { let r = Rect::new(x, y, *w, 80. * s); x += w; r }).collect()
}
pub fn secondary_tabs(ui: &mut Ui, key: &str, strip: Rect, labels: &[&str], sel: &mut usize) -> bool {
    let s = ui.vp.scale;
    let rects = secondary_rects(ui.fonts, s, strip, labels);
    let mut changed = false;
    for (i, l) in labels.iter().enumerate() {
        let r = rects[i];
        let id = ui.id(&format!("{key}{i}"));
        let it = ui.interact(id, r, true, true);
        if it.clicked && *sel != i { *sel = i; changed = true; }
        mat::tab_secondary(ui, r, *sel == i, it.hot || it.focused);
        ui.text(r, l, TextStyle::new(FontId::Bold, 18., WHITE), HAlign::Center);
    }
    changed
}

/// Height in pixels the tooltip text takes when wrapped into `rect` (and draws it).
pub fn tooltip_h(ui: &mut Ui, rect: Rect, text: &str) -> f32 { ui.paragraph(rect, text, TextStyle::new(FontId::Regular, FONT_LABEL, label_color()), HAlign::Left) }
/// Same with an explicit colour: the Game tab's TooltipDisplay is TextStyle-Base-XS-Black (colour 0,0,0), the other tabs use XS-Blue.
pub fn tooltip_c(ui: &mut Ui, rect: Rect, text: &str, color: [f32; 4]) -> f32 { ui.paragraph(rect, text, TextStyle::new(FontId::Regular, FONT_LABEL, color), HAlign::Left) }

/// Tooltip text block (TextStyle-Base-XS-Blue, auto wrap) in `rect`.
pub fn tooltip(ui: &mut Ui, rect: Rect, text: &str) {
    if text.is_empty() { return; }
    ui.paragraph(rect, text, TextStyle::new(FontId::Regular, FONT_LABEL, label_color()), HAlign::Left);
}
