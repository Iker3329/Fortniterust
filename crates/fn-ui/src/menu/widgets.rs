//! Widgets of the menu toolkit (immediate mode: call every frame, state lives in `UiState`).
//!
//! button, tab bar, toggle/checkbox, slider (numeric value), arrow-cycle selector, dropdown (popup list),
//! keybind button, scroll area, section header, plus the `Form` row-layout helper that settings pages use.
//! Every widget takes a PIXEL rect; use `ui.u(units)` / `Form` to derive them from Slate units.
//! See mod.rs for the interaction model and provenance of the textures.
use super::theme::*;
use super::*;

const POPUP_BLOCKER: Id = 0xB10C_0001;
const POPUP_LIST: Id = 0xB10C_0002;

fn vis_of(enabled: bool, it: &Interact) -> Vis { if !enabled { Vis::Disabled } else if it.down { Vis::Pressed } else if it.hot || it.focused { Vis::Hover } else { Vis::Normal } }
fn text_color(enabled: bool) -> [f32; 4] { if enabled { TEXT } else { TEXT_DISABLED } }

/// Snap `v` to `min + k*step` and clamp; NaN/inf and degenerate ranges collapse to `min`.
pub fn snap_value(v: f32, min: f32, max: f32, step: f32) -> f32 {
    if !(max > min) || !min.is_finite() || !max.is_finite() { return if min.is_finite() { min } else { 0. }; }
    if !v.is_finite() { return min; }
    let v = v.clamp(min, max);
    if step > 0. && step.is_finite() { (((v - min) / step).round() * step + min).clamp(min, max) } else { v }
}
/// Format a slider value: fixed decimals + suffix (e.g. "50", "0.50", "75%").
pub fn format_value(v: f32, decimals: usize, suffix: &str) -> String { format!("{:.*}{}", decimals, if v == 0. { 0. } else { v }, suffix) }

/// Tab widths in pixels: natural widths, shrunk proportionally when they would overflow `avail`.
pub fn tab_widths(natural: &[f32], avail: f32) -> Vec<f32> {
    let sum: f32 = natural.iter().sum();
    if sum <= avail || sum <= 0. { natural.to_vec() } else { natural.iter().map(|w| w * avail / sum).collect() }
}

/// How a tab bar is laid out in a given width (see [`tab_layout`]).
#[derive(Clone, Debug, PartialEq)]
pub enum TabLayout {
    /// Every tab visible (widths in order, summing to at most the available width).
    Fit(Vec<f32>),
    /// Too narrow for all tabs: only tabs `first..last` (always containing the selected one) are shown between two arrow margins.
    Window { first: usize, last: usize, widths: Vec<f32> },
}
/// Tab bar layout: natural widths if they fit; otherwise the compact widths (text-sized, no truncation) stretched towards the
/// natural ones; otherwise a window of whole tabs around `selected` with `arrow_w` margins on both sides (the selected tab is
/// shrunk, text ellipsised, only when it alone is wider than the bar). Pure; unit-tested.
pub fn tab_layout(natural: &[f32], compact: &[f32], selected: usize, avail: f32, arrow_w: f32) -> TabLayout {
    let (sn, sc): (f32, f32) = (natural.iter().sum(), compact.iter().sum());
    if sn <= avail || natural.is_empty() { return TabLayout::Fit(natural.to_vec()); }
    if sc <= avail {
        let t = if sn > sc { ((avail - sc) / (sn - sc)).clamp(0., 1.) } else { 0. };
        return TabLayout::Fit(natural.iter().zip(compact).map(|(n, c)| c + t * (n - c)).collect());
    }
    let room = (avail - 2. * arrow_w).max(0.);
    let sel = selected.min(compact.len() - 1);
    let (mut a, mut b, mut sum) = (sel, sel + 1, compact[sel]);
    loop {
        let mut grew = false;
        if b < compact.len() && sum + compact[b] <= room { sum += compact[b]; b += 1; grew = true; }
        if a > 0 && sum + compact[a - 1] <= room { a -= 1; sum += compact[a]; grew = true; }
        if !grew { break; }
    }
    let mut widths: Vec<f32> = compact[a..b].to_vec();
    if sum > room && widths.len() == 1 { widths[0] = room; }
    TabLayout::Window { first: a, last: b, widths }
}

impl<'a> Ui<'a> {
    fn culled(&self, rect: Rect) -> bool { self.clip().map_or(false, |c| rect_intersect(rect, c).is_none()) }

    /// Row split used by every labelled widget: (label area, control area) in pixels.
    pub fn row_split(&self, rect: Rect) -> (Rect, Rect) {
        let pad = self.u(12.);
        let frac = if rect.w / self.vp.scale < 520. { 0.5 } else { LABEL_FRAC };
        let lw = ((rect.w - pad * 3.) * frac).max(0.);
        let label = Rect::new(rect.x + pad, rect.y, lw, rect.h);
        let cx = label.x + lw + pad;
        let ch = self.u(32.).min(rect.h);
        (label, Rect::new(cx, rect.y + (rect.h - ch) * 0.5, (rect.x + rect.w - pad - cx).max(0.), ch))
    }
    fn row_bg(&mut self, rect: Rect, it: &Interact, enabled: bool) {
        if !enabled || self.culled(rect) { return; }
        if it.focused { self.fill(rect, [0., 0.55, 0.5, 0.26]); let bar = Rect::new(rect.x, rect.y, self.u(4.), rect.h); self.fill(bar, TEAL_BRIGHT); }
        else if it.hot { self.fill(rect, [0., 0.5, 0.45, 0.15]); }
    }
    fn row_label(&mut self, label_rect: Rect, label: &str, enabled: bool) {
        self.text(label_rect, label, TextStyle::new(FontId::Bold, FONT_BODY, text_color(enabled)), HAlign::Left);
    }

    pub fn label(&mut self, rect: Rect, text: &str, size: f32, color: [f32; 4], align: HAlign) { self.text(rect, text, TextStyle::new(FontId::Regular, size, color), align); }

    /// Section header: bold text with an accent underline.
    pub fn section_header(&mut self, rect: Rect, text: &str) {
        if self.culled(rect) { return; }
        let pad = self.u(12.);
        self.text(Rect::new(rect.x + pad, rect.y, rect.w - 2. * pad, rect.h - self.u(4.)), text, TextStyle::new(FontId::Bold, FONT_HEADER, TEAL_BRIGHT), HAlign::Left);
        self.fill(Rect::new(rect.x + pad, rect.y + rect.h - self.u(3.), rect.w - 2. * pad, self.u(2.).max(1.)), [TEAL_BRIGHT[0], TEAL_BRIGHT[1], TEAL_BRIGHT[2], 0.55]);
    }
    pub fn separator(&mut self, rect: Rect) { self.fill(Rect::new(rect.x, rect.y + rect.h * 0.5, rect.w, 1.), [1., 1., 1., 0.12]); }

    /// Push button using the original 1.10 button textures. `name` is the id key (must be unique per screen).
    pub fn button(&mut self, name: &str, rect: Rect, label: &str, kind: BtnKind, size: BtnSize, enabled: bool) -> Response {
        let id = self.id(name);
        let it = self.interact(id, rect, enabled, true);
        if !self.culled(rect) {
            self.brush(BrushId::Button(kind, size, vis_of(enabled, &it)), rect, WHITE);
            // Content padding from the style assets: M = 12, L = 20 Slate units on each side.
            let pad = self.u(match size { BtnSize::M => 12., BtnSize::L => 20. });
            let fs = match size { BtnSize::M => FONT_BUTTON_M, BtnSize::L => FONT_BUTTON_L };
            self.text(Rect::new(rect.x + pad, rect.y, rect.w - 2. * pad, rect.h), label, TextStyle::new(FontId::Bold, fs, text_color(enabled)).shadowed(), HAlign::Center);
        }
        Response { clicked: it.clicked && enabled, changed: false, hot: it.hot, focused: it.focused }
    }

    /// Tab bar (Tab1-M textures). One focus stop; Left/Right (focused) and Ctrl+Tab/LB/RB (anywhere) change the
    /// selection. Tabs shrink (text ellipsised) when the row is too narrow. Returns `changed` when `selected` moved.
    pub fn tab_bar(&mut self, name: &str, rect: Rect, tabs: &[&str], selected: &mut usize) -> Response {
        let bar = self.id(name);
        let n = tabs.len();
        if n == 0 { return Response::default(); }
        if *selected >= n { *selected = n - 1; }
        let mut changed = false;
        let enabled = true;
        let mut it_bar = Interact { enabled, ..Default::default() };
        if !self.inert {
            self.register_focusable(bar);
            it_bar.focused = self.st.focus == Some(bar) && self.st.focus_visible;
            if self.take_nav(NavKey::TabNext) { *selected = (*selected + 1) % n; changed = true; }
            if self.take_nav(NavKey::TabPrev) { *selected = (*selected + n - 1) % n; changed = true; }
            if self.st.focus == Some(bar) {
                let (l, r) = (self.count_nav(NavKey::Left), self.count_nav(NavKey::Right));
                let new = (*selected as i64 + r as i64 - l as i64).clamp(0, n as i64 - 1) as usize;
                if new != *selected { *selected = new; changed = true; }
            }
        }
        let px = self.text_px(FONT_BUTTON_M) as f32;
        let measured: Vec<f32> = tabs.iter().map(|t| self.fonts.measure(FontId::Bold, &sanitize(t), px)).collect();
        let natural: Vec<f32> = measured.iter().map(|m| (m + self.u(48.)).max(self.u(96.))).collect();
        let compact: Vec<f32> = measured.iter().map(|m| (m + self.u(30.)).max(self.u(56.))).collect();
        let arrow_w = self.u(18.);
        let (widths, (first, last), arrows) = match tab_layout(&natural, &compact, *selected, rect.w, arrow_w) { TabLayout::Fit(w) => { let n = w.len(); (w, (0, n), false) } TabLayout::Window { first, last, widths } => (widths, (first, last), true) };
        let mut x = rect.x + if arrows { arrow_w } else { 0. };
        if arrows {
            // hidden tabs on a side: a "<" / ">" marker (clickable: previous / next tab) in the reserved margin
            for (left, hidden) in [(true, first > 0), (false, last < n)] {
                let ar = Rect::new(if left { rect.x } else { rect.x + rect.w - arrow_w }, rect.y, arrow_w, rect.h);
                let aid = id_with(bar, id_of(if left { "tab-prev" } else { "tab-next" }));
                let it = self.interact(aid, ar, hidden, false);
                if hidden {
                    if it.clicked { *selected = if left { *selected - 1 } else { *selected + 1 }; changed = true; self.st.focus = Some(bar); }
                    self.text(ar, if left { "<" } else { ">" }, TextStyle::new(FontId::Bold, FONT_BUTTON_M, if it.hot { WHITE } else { TEAL_BRIGHT }).shadowed(), HAlign::Center);
                }
            }
        }
        for (i, t) in tabs.iter().enumerate() {
            if i < first || i >= last { continue; }
            let wi = widths[i - first];
            let tr = Rect::new(x, rect.y, wi, rect.h);
            x += wi;
            let id = id_with(bar, id_of(t) ^ i as u64);
            let it = self.interact(id, tr, true, false);
            if it.clicked && *selected != i { *selected = i; changed = true; self.st.focus = Some(bar); }
            if self.culled(tr) { continue; }
            let active = *selected == i;
            let v = if active { TabVis::Active } else if it.hot { TabVis::PassiveHover } else { TabVis::Passive };
            self.brush(BrushId::Tab(v), tr, WHITE);
            let pad = self.u(14.);
            self.text(Rect::new(tr.x + pad, tr.y, tr.w - 2. * pad, tr.h), t, TextStyle::new(FontId::Bold, FONT_BUTTON_M, if active { WHITE } else { TEXT_DIM }).shadowed(), HAlign::Center);
            if active && it_bar.focused { self.fill(Rect::new(tr.x + self.u(6.), tr.y + tr.h - self.u(4.), tr.w - self.u(12.), self.u(3.)), TEAL_BRIGHT); }
        }
        Response { clicked: false, changed, hot: false, focused: it_bar.focused }
    }

    /// Check box row (Checkbox_Glass textures). Whole row is clickable; Enter/Space/A toggles when focused.
    pub fn toggle(&mut self, name: &str, rect: Rect, label: &str, value: &mut bool, enabled: bool) -> Response {
        let id = self.id(name);
        let it = self.interact(id, rect, enabled, true);
        let mut changed = false;
        if it.clicked && enabled { *value = !*value; changed = true; }
        if !self.culled(rect) {
            self.row_bg(rect, &it, enabled);
            let (lr, cr) = self.row_split(rect);
            self.row_label(lr, label, enabled);
            let sz = self.u(28.).min(cr.h.max(self.u(20.)));
            let box_r = Rect::new(cr.x, cr.y + (cr.h - sz) * 0.5, sz, sz);
            let v = if !enabled { Vis::Normal } else if it.down { Vis::Pressed } else if it.hot || it.focused { Vis::Hover } else { Vis::Normal };
            self.sprite(BrushId::Check(*value, v), box_r, false, if enabled { WHITE } else { [1., 1., 1., 0.45] });
        }
        Response { clicked: it.clicked && enabled, changed, hot: it.hot, focused: it.focused }
    }

    /// Slider with a numeric value readout. Drag the track, Left/Right (focused) change by `step`, Home/End jump.
    /// The mouse wheel is deliberately NOT bound (it scrolls the page). Value is snapped to `step` and clamped.
    pub fn slider(&mut self, name: &str, rect: Rect, label: &str, value: &mut f32, min: f32, max: f32, step: f32, decimals: usize, suffix: &str, enabled: bool) -> Response {
        let id = self.id(name);
        let (lr, cr) = self.row_split(rect);
        let val_w = self.u(70.).min(cr.w * 0.4);
        let track_area = Rect::new(cr.x, rect.y, (cr.w - val_w - self.u(8.)).max(0.), rect.h);
        let row_it = self.interact(id, track_area, enabled, true);
        let valid = max > min && min.is_finite() && max.is_finite();
        let mut changed = false;
        let before = *value;
        let thumb_w = self.u(16.);
        if enabled && valid {
            let usable = (track_area.w - thumb_w).max(1.);
            if row_it.grabbed {
                if let Some((mx, _)) = self.st.mouse { let t = ((mx - track_area.x - thumb_w * 0.5) / usable).clamp(0., 1.); *value = snap_value(min + t * (max - min), min, max, step); }
            }
            if self.st.focus == Some(id) && !self.inert {
                let (l, r) = (self.count_nav(NavKey::Left) as f32, self.count_nav(NavKey::Right) as f32);
                if l + r > 0. { let st = if step > 0. { step } else { (max - min) / 100. }; *value = snap_value(*value + (r - l) * st, min, max, step); }
                if self.take_nav(NavKey::Home) { *value = min; }
                if self.take_nav(NavKey::End) { *value = max; }
            }
        }
        if !before.is_finite() || (valid && (*value < min || *value > max)) { *value = snap_value(*value, min, max, step); }
        if (*value - before).abs() > f32::EPSILON || (before.is_nan() && !value.is_nan()) { changed = true; }
        if !self.culled(rect) {
            self.row_bg(rect, &row_it, enabled);
            self.row_label(lr, label, enabled);
            let th = self.u(8.);
            let track = Rect::new(track_area.x, rect.y + (rect.h - th) * 0.5, track_area.w, th);
            self.fill(track, TRACK);
            let t = if valid { ((*value - min) / (max - min)).clamp(0., 1.) } else { 0. };
            let usable = (track.w - thumb_w).max(0.);
            let tx = track.x + t * usable;
            let fill_c = if enabled { [0., 0.72, 0.62, 1.] } else { [0.4, 0.42, 0.43, 1.] };
            self.fill(Rect::new(track.x, track.y, (tx + thumb_w * 0.5 - track.x).max(0.), th), fill_c);
            let thumb_h = self.u(28.).min(rect.h);
            let thumb = Rect::new(tx, rect.y + (rect.h - thumb_h) * 0.5, thumb_w, thumb_h);
            let v = vis_of(enabled, &Interact { down: row_it.grabbed && self.st.mouse_down, ..row_it });
            self.brush(BrushId::Button(BtnKind::Default, BtnSize::M, v), thumb, WHITE);
            let txt = format_value(*value, decimals, suffix);
            self.text(Rect::new(cr.x + cr.w - val_w, rect.y, val_w, rect.h), &txt, TextStyle::new(FontId::Bold, FONT_BODY, text_color(enabled)), HAlign::Right);
        }
        Response { clicked: false, changed, hot: row_it.hot, focused: row_it.focused }
    }

    /// Arrow-cycle selector: `<  value  >`. Arrows/Left/Right/Enter step through `options` (wraps around).
    pub fn selector(&mut self, name: &str, rect: Rect, label: &str, options: &[&str], selected: &mut usize, enabled: bool) -> Response {
        let id = self.id(name);
        let n = options.len();
        let it = self.interact(id, rect, enabled && n > 0, true);
        let (lr, cr) = self.row_split(rect);
        let aw = self.u(28.).min(cr.w * 0.3);
        let (left, right) = (Rect::new(cr.x, cr.y, aw, cr.h), Rect::new(cr.x + cr.w - aw, cr.y, aw, cr.h));
        let live = enabled && n > 0;
        let il = self.interact(id_with(id, 1), left, live, false);
        let ir = self.interact(id_with(id, 2), right, live, false);
        let mut changed = false;
        if n > 0 && *selected >= n { *selected = n - 1; }
        if live {
            let mut delta: i64 = 0;
            if il.clicked { delta -= 1; } if ir.clicked { delta += 1; }
            if it.clicked && !il.clicked && !ir.clicked { delta += 1; } // centre click / Enter cycles forward
            if self.st.focus == Some(id) && !self.inert { delta += self.count_nav(NavKey::Right) as i64 - self.count_nav(NavKey::Left) as i64; }
            if delta != 0 { *selected = ((*selected as i64 + delta).rem_euclid(n as i64)) as usize; changed = true; }
        }
        if !self.culled(rect) {
            self.row_bg(rect, &it, live);
            self.row_label(lr, label, enabled);
            let col = |h: bool| if !live { [1., 1., 1., 0.35] } else if h { TEAL_BRIGHT } else { WHITE };
            let asz = self.u(20.);
            let ay = cr.y + (cr.h - asz) * 0.5;
            self.sprite(BrushId::Arrow, Rect::new(left.x + (aw - asz) * 0.5, ay, asz, asz), false, col(il.hot));
            self.sprite(BrushId::Arrow, Rect::new(right.x + (aw - asz) * 0.5, ay, asz, asz), true, col(ir.hot));
            let txt = options.get(*selected).copied().unwrap_or("");
            self.text(Rect::new(left.x + aw + self.u(4.), cr.y, cr.w - 2. * aw - self.u(8.), cr.h), txt, TextStyle::new(FontId::Bold, FONT_BODY, text_color(live)), HAlign::Center);
        }
        Response { clicked: it.clicked, changed, hot: it.hot, focused: it.focused }
    }

    /// Dropdown: a box that opens a popup list (drawn last, above everything; click outside or Esc closes).
    /// The chosen index is written one frame after the click (the popup is built at the end of the screen pass).
    pub fn dropdown(&mut self, name: &str, rect: Rect, label: &str, options: &[&str], selected: &mut usize, enabled: bool) -> Response {
        let id = self.id(name);
        let n = options.len();
        let live = enabled && n > 0;
        let is_open = self.st.open_dropdown == Some(id);
        let it = self.interact_ex(id, rect, live, true, !is_open);
        let (lr, cr) = self.row_split(rect);
        let mut changed = false;
        if n > 0 && *selected >= n { *selected = n - 1; }
        if let Some((pid, idx, _)) = self.st.popup_result { if pid == id && !self.inert { *selected = idx.min(n.saturating_sub(1)); changed = true; self.st.popup_result = None; } }
        if live && it.clicked && !is_open && !self.inert { self.st.open_dropdown = Some(id); self.st.dd_hi = *selected; self.st.dd_scroll = 0.; self.st.popup_press = None; }
        let open = self.st.open_dropdown == Some(id) && !self.inert;
        if open { self.st.popup = Some(PopupReq { id, anchor: cr, items: options.iter().map(|s| (*s).to_owned()).collect(), selected: *selected }); }
        if !self.culled(rect) {
            self.row_bg(rect, &it, live);
            self.row_label(lr, label, enabled);
            let v = if !live { Vis::Disabled } else if open || it.down { Vis::Pressed } else if it.hot || it.focused { Vis::Hover } else { Vis::Normal };
            self.brush(BrushId::Button(BtnKind::Default, BtnSize::M, v), cr, WHITE);
            let pad = self.u(12.);
            let tri_w = self.u(14.);
            self.text(Rect::new(cr.x + pad, cr.y, cr.w - 2. * pad - tri_w, cr.h), options.get(*selected).copied().unwrap_or(""), TextStyle::new(FontId::Bold, FONT_BODY, text_color(live)).shadowed(), HAlign::Left);
            // down-pointing triangle from scanlines (the only arrow texture points left).
            let rows = (self.u(7.)).round().max(3.) as i32;
            let (cx, cy) = (cr.x + cr.w - pad - tri_w * 0.5, cr.y + cr.h * 0.5 - rows as f32 * 0.5);
            for r in 0..rows { let w = tri_w * (1. - r as f32 / rows as f32); self.fill(Rect::new(cx - w * 0.5, cy + r as f32, w, 1.), if live { WHITE } else { [1., 1., 1., 0.4] }); }
        }
        Response { clicked: it.clicked, changed, hot: it.hot, focused: it.focused }
    }

    /// Keybind button: click to listen, then press a key or mouse button. Esc cancels, Backspace/Delete unbinds,
    /// a click elsewhere cancels. `binding` holds a virtual-key code (mouse buttons use VK_LBUTTON.. codes).
    pub fn keybind(&mut self, name: &str, rect: Rect, label: &str, binding: &mut Option<u16>, enabled: bool) -> Response {
        let id = self.id(name);
        let it = self.interact(id, rect, enabled, true);
        let (lr, cr) = self.row_split(rect);
        let mut changed = false;
        let mut listening = self.st.listening == Some(id) && !self.inert;
        if listening {
            if let Some(vk) = self.st.raw_key.take() {
                match vk {
                    0x1B => {} // Escape: cancel
                    0x08 | 0x2E => { if binding.is_some() { *binding = None; changed = true; } }
                    k => { if *binding != Some(k) { *binding = Some(k); changed = true; } }
                }
                self.st.listening = None; listening = false;
            } else if self.st.pressed_any && self.st.press_target != Some(id) { self.st.listening = None; listening = false; }
            else if it.clicked { self.st.listening = None; listening = false; }
        } else if it.clicked && enabled && !self.inert { self.st.listening = Some(id); listening = true; }
        if !self.culled(rect) {
            self.row_bg(rect, &it, enabled);
            self.row_label(lr, label, enabled);
            let (kind, v) = if !enabled { (BtnKind::Default, Vis::Disabled) } else if listening { (BtnKind::Emphasis, Vis::Normal) } else { (BtnKind::Default, if it.down { Vis::Pressed } else if it.hot || it.focused { Vis::Hover } else { Vis::Normal }) };
            self.brush(BrushId::Button(kind, BtnSize::M, v), cr, WHITE);
            let txt = if listening { "Press a key...".to_owned() } else { binding.map(key_name).unwrap_or_else(|| "Unbound".to_owned()) };
            let pad = self.u(10.);
            self.text(Rect::new(cr.x + pad, cr.y, cr.w - 2. * pad, cr.h), &txt, TextStyle::new(FontId::Bold, FONT_BODY, if !enabled { TEXT_DISABLED } else if binding.is_none() && !listening { TEXT_DIM } else { WHITE }).shadowed(), HAlign::Center);
        }
        Response { clicked: it.clicked, changed, hot: it.hot, focused: it.focused }
    }

    /// Scroll container. `body(ui, view)` lays widgets out starting at `view.y` (already shifted by the scroll
    /// offset) within `view.w` and returns the content height in pixels. Content is clipped to `rect` (clipped
    /// widgets cannot be hovered or clicked); the wheel scrolls the innermost area under the cursor; the track
    /// at the right edge drags; keyboard focus scrolls its widget into view; PageUp/PageDown page.
    pub fn scroll_area(&mut self, name: &str, rect: Rect, body: impl FnOnce(&mut Ui, ScrollView) -> f32) {
        let id = self.id(name);
        let s = self.vp.scale;
        let mut state = self.st.scroll.get(&id).copied().unwrap_or_default();
        let view_u = rect.h / s;
        let mut max = (state.content_h - view_u).max(0.);
        self.register_region(id, rect, true);
        if !self.inert {
            if self.st.wheel != 0. && self.st.hot_owner == Some(id) { state.offset -= self.st.wheel * WHEEL_UNITS; self.st.wheel = 0.; }
            if self.st.hot_owner == Some(id) || self.st.hot.is_none() {
                if self.take_nav(NavKey::PageDown) { state.offset += view_u * 0.9; }
                if self.take_nav(NavKey::PageUp) { state.offset -= view_u * 0.9; }
            }
        }
        let bar_w = self.u(SCROLLBAR_W);
        let track = Rect::new(rect.x + rect.w - bar_w, rect.y, bar_w, rect.h);
        let (thumb_h, scroll_range) = { let th = (view_u / state.content_h.max(1.) * track.h).clamp(self.u(28.).min(track.h), track.h); (th, (track.h - th).max(1.)) };
        if max > 0. {
            let it = self.interact(id_with(id, 0x7ACC), track, true, false);
            if it.grabbed { if let Some((_, my)) = self.st.mouse { state.offset = ((my - track.y - thumb_h * 0.5) / scroll_range).clamp(0., 1.) * max; } }
        }
        state.offset = state.offset.clamp(0., max);
        let content_w = if max > 0. { rect.w - bar_w - self.u(6.) } else { rect.w };
        self.push_clip(rect);
        self.scrolls.push(ScrollCtx { id, clip: rect, want: None });
        let h_px = body(self, ScrollView { x: rect.x, y: rect.y - state.offset * s, w: content_w, clip: rect, scale: s });
        let ctx = self.scrolls.pop().unwrap();
        self.pop_clip();
        state.content_h = (h_px / s).max(0.); state.view_h = view_u;
        max = (state.content_h - view_u).max(0.);
        if let Some((top, bottom)) = ctx.want {
            if top < rect.y { state.offset -= (rect.y - top) / s + 6.; } else if bottom > rect.y + rect.h { state.offset += (bottom - rect.y - rect.h) / s + 6.; }
        }
        state.offset = state.offset.clamp(0., max);
        self.st.scroll.insert(id, state);
        if max > 0. && !self.culled(rect) {
            self.fill(track, [0., 0., 0., 0.35]);
            let ty = track.y + state.offset / max * (track.h - thumb_h).max(0.);
            let active = self.st.active == Some(id_with(id, 0x7ACC));
            self.fill(Rect::new(track.x + self.u(2.), ty, track.w - self.u(4.), thumb_h), if active { TEAL_BRIGHT } else { [0.0, 0.62, 0.56, 0.95] });
        }
    }

    /// Scrolling settings form: `body(ui, form)` adds rows via `Form` methods. Row ids are scoped by `name`.
    pub fn form(&mut self, name: &str, area: Rect, body: impl FnOnce(&mut Ui, &mut Form)) {
        self.push_id(name);
        self.scroll_area("scroll", area, |ui, view| { let mut f = Form::new(ui, view); body(ui, &mut f); f.height() });
        self.pop_id();
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollView { pub x: f32, pub y: f32, pub w: f32, pub clip: Rect, pub scale: f32 }

/// Vertical row layout for settings pages (Slate-unit heights, pixel output).
pub struct Form { view: ScrollView, y: f32, pad: f32, gap: f32 }
impl Form {
    pub fn new(ui: &Ui, view: ScrollView) -> Self { let pad = ui.u(8.); Self { view, y: view.y + pad, pad, gap: ui.u(ROW_GAP) } }
    fn take(&mut self, h_units: f32) -> Rect {
        let h = h_units * self.view.scale;
        let r = Rect::new(self.view.x + self.pad, self.y, (self.view.w - 2. * self.pad).max(0.), h);
        self.y += h + self.gap; r
    }
    /// Vertical gap in Slate units.
    pub fn space(&mut self, units: f32) { self.y += units * self.view.scale; }
    /// Content height in pixels (for the scroll area).
    pub fn height(&self) -> f32 { self.y + self.pad - self.view.y }
    /// Next row rect (custom widgets).
    pub fn row(&mut self, h_units: f32) -> Rect { self.take(h_units) }
    pub fn header(&mut self, ui: &mut Ui, text: &str) { self.space(10.); let r = self.take(34.); ui.section_header(r, text); }
    pub fn toggle(&mut self, ui: &mut Ui, label: &str, v: &mut bool, enabled: bool) -> Response { let r = self.take(ROW_H); ui.toggle(label, r, label, v, enabled) }
    pub fn slider(&mut self, ui: &mut Ui, label: &str, v: &mut f32, min: f32, max: f32, step: f32, decimals: usize, suffix: &str, enabled: bool) -> Response { let r = self.take(ROW_H); ui.slider(label, r, label, v, min, max, step, decimals, suffix, enabled) }
    pub fn selector(&mut self, ui: &mut Ui, label: &str, options: &[&str], sel: &mut usize, enabled: bool) -> Response { let r = self.take(ROW_H); ui.selector(label, r, label, options, sel, enabled) }
    pub fn dropdown(&mut self, ui: &mut Ui, label: &str, options: &[&str], sel: &mut usize, enabled: bool) -> Response { let r = self.take(ROW_H); ui.dropdown(label, r, label, options, sel, enabled) }
    pub fn keybind(&mut self, ui: &mut Ui, label: &str, binding: &mut Option<u16>, enabled: bool) -> Response { let r = self.take(ROW_H); ui.keybind(label, r, label, binding, enabled) }
    pub fn button(&mut self, ui: &mut Ui, label: &str, kind: BtnKind, enabled: bool) -> Response {
        let r = self.take(BTN_M_H + 8.);
        let w = (ui.u(280.)).min(r.w);
        ui.button(label, Rect::new(r.x + ui.u(12.), r.y + ui.u(4.), w, ui.u(BTN_M_H)), label, kind, BtnSize::M, enabled)
    }
    pub fn note(&mut self, ui: &mut Ui, text: &str) {
        let pad = ui.u(12.);
        let r = Rect::new(self.view.x + self.pad + pad, self.y, (self.view.w - 2. * self.pad - 2. * pad).max(0.), 0.);
        let h = ui.paragraph(r, text, TextStyle::new(FontId::Regular, 15., TEXT_DIM), HAlign::Left);
        self.y += h + self.gap;
    }
}

/// Dropdown popup list, built after the screen's widgets so it sits on top of everything.
pub(crate) fn finish_overlays(ui: &mut Ui) {
    if ui.inert { ui.st.popup = None; return; }
    let Some(req) = ui.st.popup.take() else { return };
    ui.st.popup_handled = true;
    let n = req.items.len();
    if n == 0 { ui.st.open_dropdown = None; return; }
    let item_h = ui.u(36.);
    let gap = ui.u(2.);
    let below = ui.vp.h - (req.anchor.y + req.anchor.h) - gap;
    let above = req.anchor.y - gap;
    let max_fit = |space: f32| ((space / item_h).floor() as usize).max(1);
    let want = n.min(8);
    let (place_below, visible) = if below >= want as f32 * item_h || below >= above { (true, want.min(max_fit(below))) } else { (false, want.min(max_fit(above))) };
    let list_h = visible as f32 * item_h;
    let w = req.anchor.w.max(ui.u(180.)).min(ui.vp.w);
    let x = req.anchor.x.min(ui.vp.w - w).max(0.);
    let y = if place_below { req.anchor.y + req.anchor.h + gap } else { req.anchor.y - gap - list_h };
    let list = Rect::new(x, y.max(0.), w, list_h);
    let id = req.id;
    let item_id = |i: usize| id_with(id, 0x1000 + i as u64);
    // regions: full-screen blocker first, list above it, items above the list
    let screen = ui.screen();
    ui.st.next_regions.push(Region { id: POPUP_BLOCKER, rect: screen, scroll_owner: None });
    ui.st.next_regions.push(Region { id: POPUP_LIST, rect: list, scroll_owner: None });
    let max_scroll = (n - visible) as f32;
    let mut scroll = ui.st.dd_scroll.clamp(0., max_scroll);
    let mut hi = ui.st.dd_hi.min(n - 1);
    // wheel over the list scrolls it (rows)
    let hot = ui.st.hot;
    let over_list = hot == Some(POPUP_LIST) || (0..n).any(|i| hot == Some(item_id(i)));
    if over_list && ui.st.wheel != 0. { scroll = (scroll - ui.st.wheel * 2.).clamp(0., max_scroll); ui.st.wheel = 0.; }
    let mut close = false; let mut chosen: Option<usize> = None;
    // keyboard
    let mut kb_moved = false;
    for k in std::mem::take(&mut ui.st.nav) {
        match k {
            NavKey::Up => { hi = hi.saturating_sub(1); kb_moved = true; }
            NavKey::Down => { hi = (hi + 1).min(n - 1); kb_moved = true; }
            NavKey::Home => { hi = 0; kb_moved = true; }
            NavKey::End => { hi = n - 1; kb_moved = true; }
            NavKey::PageUp => { hi = hi.saturating_sub(visible); kb_moved = true; }
            NavKey::PageDown => { hi = (hi + visible).min(n - 1); kb_moved = true; }
            NavKey::Activate => chosen = Some(hi),
            NavKey::Back => close = true,
            _ => {}
        }
    }
    // mouse: hover highlights (only when the mouse actually moved, so keyboard stays in control)
    if ui.st.mouse_moved { for i in 0..visible { if hot == Some(item_id(scroll.round() as usize + i)) { hi = scroll.round() as usize + i; } } }
    // visible window follows the highlight
    if kb_moved { if (hi as f32) < scroll { scroll = hi as f32; } else if hi as f32 > scroll + visible as f32 - 1. { scroll = hi as f32 - visible as f32 + 1.; } }
    scroll = scroll.clamp(0., max_scroll);
    let first = scroll.round() as usize;
    // register item regions for the rows currently visible
    for i in 0..visible { let r = Rect::new(list.x, list.y + i as f32 * item_h, list.w, item_h); ui.st.next_regions.push(Region { id: item_id(first + i), rect: r, scroll_owner: None }); }
    // clicks: press outside the list closes; press+release on the same item selects
    if let Some(p) = ui.st.press_target {
        if p == POPUP_BLOCKER { close = true; }
        else if let Some(i) = (0..n).find(|i| item_id(*i) == p) { ui.st.popup_press = Some(i); }
    }
    if ui.st.released { if let (Some(rt), Some(pp)) = (ui.st.release_target, ui.st.popup_press) { if rt == item_id(pp) { chosen = Some(pp); } } ui.st.popup_press = None; }
    ui.st.dd_scroll = scroll; ui.st.dd_hi = hi;
    // draw
    let pad = ui.u(12.);
    ui.fill(rect_inflate(list, ui.u(2.).max(1.)), PANEL_EDGE);
    ui.fill(list, [0.01, 0.055, 0.085, 1.]);
    ui.push_clip(list);
    for i in 0..visible {
        let idx = first + i;
        let r = Rect::new(list.x, list.y + i as f32 * item_h, list.w, item_h);
        if idx == hi { ui.fill(r, [0., 0.5, 0.45, 0.65]); }
        if idx == req.selected { ui.fill(Rect::new(r.x, r.y, ui.u(4.), r.h), TEAL_BRIGHT); }
        ui.text(Rect::new(r.x + pad, r.y, r.w - 2. * pad, r.h), &req.items[idx], TextStyle::new(FontId::Bold, FONT_BODY, if idx == hi { WHITE } else { TEXT_DIM }), HAlign::Left);
    }
    ui.pop_clip();
    if max_scroll > 0. {
        let th = (visible as f32 / n as f32 * list.h).max(ui.u(16.));
        let ty = list.y + scroll / max_scroll * (list.h - th);
        ui.fill(Rect::new(list.x + list.w - ui.u(5.), ty, ui.u(3.), th), [0.0, 0.7, 0.62, 0.9]);
    }
    if let Some(i) = chosen { ui.st.popup_result = Some((id, i, 0)); ui.st.open_dropdown = None; ui.st.popup_press = None; }
    else if close { ui.st.open_dropdown = None; ui.st.popup_press = None; }
}
