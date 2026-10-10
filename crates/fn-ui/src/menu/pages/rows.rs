//! Descriptor-driven settings rows and the generic tab page.
//!
//! A [`RowDef`] is one row of the original 1.8 OptionsMenu (label, hover text, visibility gate: all RECOVERED, see
//! `layout_table.rs`) bound to a settings descriptor (`settings::Entry`: kind/range/step/default/live-vs-restart/wired-vs-stored)
//! or to the HUD visibility list. [`draw_row18`] renders it with the toolkit widgets. Only original text and widgets are drawn: no badges,
//! markers or notes (settings that are stored without a backend yet behave like any other row).
//! Gated rows (the original hides them on PC/Battle Royale/consoles) are drawn disabled. The tooltip shows only the original hover text.
use super::layout_table;
use crate::menu::*;
use crate::settings::{self, Entry, KindDef, Settings, Value};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
pub enum Bind {
    /// One settings descriptor id.
    Setting(&'static str),
    /// The single original "Mouse Sensitivity" slider: sets X, and Y too while Y still equals X (Y is an assumed extension).
    MouseSens,
    /// HUD element visibility, stored as `[HUD] <tag>=True/False` through `Settings::set_extra` (STORED ONLY).
    Hud { tag: &'static str, default_on: bool },
    /// A rotator over fixed choices stored as `[section] key=<option text>` through `Settings::set_extra` (e.g. Matchmaking Region: the `MMRegion`
    /// strings Europe / North America / Oceania). `default` indexes `opts`.
    Choice { section: &'static str, key: &'static str, opts: &'static [&'static str], default: usize },
    /// Original row without any counterpart in this runtime's settings model.
    None,
}
/// Control the original row uses: `Entry` = whatever the descriptor kind needs (slider / arrow rotator), `OnOff` = Off|On rotator,
/// `Segmented` = the original `OptionsMenuRowSelector` (a row of buttons: low | medium | high | epic ...), `Static` = no counterpart here.
#[derive(Clone, Copy, Debug)]
pub enum Wid { Entry, OnOff, Segmented, Static(&'static str) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate { None, WindowedFullscreen, XboxOnly, ConsoleOnly, HiddenInBr, NoBackend, Native }
#[derive(Debug)]
pub struct RowDef {
    pub key: &'static str, pub label: &'static str, pub tips: &'static [&'static str],
    pub bind: Bind, pub wid: Wid, pub gate: Gate, pub extension: bool,
}
#[derive(Debug)]
pub struct SubTab { pub id: &'static str, pub label: &'static str, pub rows: &'static [RowDef] }
#[derive(Debug)]
pub struct TabDef { pub id: &'static str, pub label: &'static str, pub order: i32, pub texts: &'static [&'static str], pub subs: &'static [SubTab] }

/// HUD tags in original order (the `[HUD]` extra keys the bridge syncs).
pub fn hud_tags() -> &'static [&'static str] {
    static T: OnceLock<Vec<&'static str>> = OnceLock::new();
    T.get_or_init(|| layout_table::TABS.iter().flat_map(|t| t.subs.iter()).flat_map(|s| s.rows.iter()).filter_map(|r| if let Bind::Hud { tag, .. } = r.bind { Some(tag) } else { None }).collect())
}

/// `(section, key)` of every extra (non-descriptor) value the rows store: the HUD tags and the `Choice` rows. The bridge syncs these.
pub fn extra_keys() -> &'static [(&'static str, &'static str)] {
    static T: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
    T.get_or_init(|| layout_table::TABS.iter().flat_map(|t| t.subs.iter()).flat_map(|s| s.rows.iter()).filter_map(|r| match r.bind {
        Bind::Hud { tag, .. } => Some(("HUD", tag)),
        Bind::Choice { section, key, .. } => Some((section, key)),
        _ => None,
    }).chain(settings::extras::CONTROLLER_KEYS.iter().copied()).collect())
}
/// Index of the stored choice of a [`Bind::Choice`] row (the default when nothing valid is stored).
pub fn choice_index(snap: &Settings, section: &str, key: &str, opts: &[&str], default: usize) -> usize {
    snap.extra(section, key).and_then(|v| opts.iter().position(|o| o.eq_ignore_ascii_case(v.trim()))).unwrap_or(default.min(opts.len().saturating_sub(1)))
}

// ---- row state ------------------------------------------------------------------------------------------------------
pub struct RowInfo { pub enabled: bool }

pub fn entry_of(row: &RowDef) -> Option<&'static Entry> {
    match row.bind { Bind::Setting(id) => settings::entry(id), Bind::MouseSens => settings::entry("input.mouse_sensitivity"), _ => None }
}
/// Enabled state of `row` given the current working settings (gated rows are drawn disabled, without any extra text).
pub fn row_info(row: &RowDef, snap: &Settings) -> RowInfo {
    let mut enabled = !matches!(row.wid, Wid::Static(_));
    match row.gate {
        Gate::WindowedFullscreen => if snap.video.window_mode == settings::WindowMode::WindowedFullscreen { enabled = false; },
        Gate::NoBackend => enabled = false,
        // Xbox-only / console-only / hidden-in-BR rows are not drawn at all in a context where the original collapses them
        // (`visibility::visible_rows`); native platform checks of the original are not recoverable: the row is simply shown.
        Gate::None | Gate::XboxOnly | Gate::ConsoleOnly | Gate::HiddenInBr | Gate::Native => {}
    }
    RowInfo { enabled }
}

// ---- value access ---------------------------------------------------------------------------------------------------
fn hud_on(snap: &Settings, tag: &str, default_on: bool) -> bool {
    match snap.extra("HUD", tag) { Some(v) => !(v.eq_ignore_ascii_case("false") || v == "0" || v.eq_ignore_ascii_case("off")), None => default_on }
}
pub fn hud_set(s: &mut Settings, tag: &str, on: bool) { s.set_extra("HUD", tag, if on { "True" } else { "False" }); }

/// Default value of a row, applied to the working settings (used by Reset To Default).
pub fn reset_row(s: &mut Settings, row: &RowDef) {
    match row.bind {
        Bind::Setting(id) => { let _ = s.reset(id); }
        Bind::MouseSens => { let _ = s.reset("input.mouse_sensitivity"); let _ = s.reset("input.mouse_sensitivity_y"); }
        Bind::Hud { tag, default_on } => hud_set(s, tag, default_on),
        Bind::Choice { section, key, opts, default } => s.set_extra(section, key, opts[default.min(opts.len() - 1)]),
        Bind::None => {}
    }
}

// ---- page state -----------------------------------------------------------------------------------------------------
#[derive(Default)]
pub struct PageState {
    pub sub: usize,
    pub desc_row: Option<usize>,
    modes: Vec<(u32, u32)>,
    modes_loaded: bool,
    desc_need: Option<((i32, i32, usize), f32)>,
}

impl PageState { pub fn modes_len(&self) -> usize { self.modes.len() } }
fn display_modes(snap: &Settings, cache: &mut PageState, display: &[(u32, u32)]) -> Vec<(u32, u32)> {
    if !cache.modes_loaded {
        let mut m: Vec<(u32, u32)> = display.iter().copied().filter(|&(w, h)| w >= 640 && h >= 360).collect();
        m.sort(); m.dedup();
        cache.modes = m; cache.modes_loaded = true;
    }
    let mut m = cache.modes.clone();
    let cur = (snap.video.width, snap.video.height);
    if cur.0 > 0 && !m.contains(&cur) { m.push(cur); m.sort(); }
    m
}

/// Slot padding [l,t,r,b] of a row in the original tab trees (VerticalBoxSlot.Padding): Video 2 (Quality bottom 20, the other segmented rows 0 top/bottom),
/// Game sliders 0, Game rotators 2, Audio 5.
pub fn slot_pad(tab: &str, row: &RowDef) -> [f32; 4] {
    match (tab, row.wid, row.key) {
        ("video", Wid::Segmented, "Quality") => [2., 2., 2., 20.],
        ("video", Wid::Segmented, _) => [2., 0., 2., 0.],
        ("audio", _, _) => [5., 5., 5., 5.],
        (_, _, _) if matches!(row.bind, Bind::Hud { .. }) => [0., 0., 0., 0.], // the 2+2 is inside OptionsMenuHudRotator (TextRotator slot padding [0,2,0,2])
        ("game", _, _) if matches!(row.bind, Bind::Setting(id) if settings::entry(id).map_or(false, |e| matches!(e.kind, KindDef::Slider { .. }))) || matches!(row.bind, Bind::MouseSens) => [0., 0., 0., 0.],
        ("game", _, "SafeZone") => [0., 0., 0., 0.],
        _ => [2., 2., 2., 2.],
    }
}

/// UserWidget.Padding of the row instance in the tab tree (TabVideoOptions: [2,2,2,2] on 3DResolution, ViewDistance, Anti-Aliasing, Textures, Effects, PostProcessing).
pub fn user_pad(tab: &str, row: &RowDef) -> f32 {
    if tab == "video" && matches!(row.key, "3DResolution" | "ViewDistance" | "Anti-Aliasing" | "Textures" | "Effects" | "PostProcessing") { 2. } else { 0. }
}
/// Inner height (Slate units) of the control the row uses.
pub fn row_inner_h(row: &RowDef) -> f32 {
    use super::slate;
    match (row.bind, row.wid) {
        (_, Wid::Static(_)) => slate::ROTATOR_H,
        (Bind::Hud { .. }, _) => slate::ROTATOR_H + 4.,
        (Bind::Choice { .. }, _) => slate::ROTATOR_H,
        (Bind::Setting(_), _) | (Bind::MouseSens, _) => match entry_of(row).map(|e| e.kind) {
            Some(KindDef::Slider { .. }) => slate::SLIDER_ROW_H,
            Some(KindDef::Enum(_)) if matches!(row.wid, Wid::Segmented) => slate::SEGMENT_H,
            _ => slate::ROTATOR_H,
        },
        _ => slate::ROTATOR_H,
    }
}
/// Pixel geometry of one row: the user-widget rect (slot padding removed, includes its own Padding) and the inner rect (that Padding removed too).
pub struct RowGeo { pub row: Rect, pub inner: Rect }
pub fn row_geometry(col: &mut super::slate::Col, tab: &str, row: &RowDef) -> RowGeo {
    let pad = slot_pad(tab, row);
    let up = user_pad(tab, row);
    let h = row_inner_h(row) + 2. * up;
    let r = col.slot(h + pad[1] + pad[3], pad);
    let s = col.scale;
    RowGeo { row: r, inner: Rect::new(r.x + up * s, r.y + up * s, (r.w - 2. * up * s).max(0.), (r.h - 2. * up * s).max(0.)) }
}

/// Exact 1.8 row (slate.rs widgets). Returns (hot, focused) for the tooltip panel.
pub fn draw_row18(ui: &mut Ui, col: &mut super::slate::Col, tab: &str, row: &RowDef, snap: &Settings, area: Rect, st: &mut PageState) -> (bool, bool) {
    use super::slate;
    let info = row_info(row, snap);
    let enabled = info.enabled;
    let geo = row_geometry(col, tab, row);
    let rect = geo.inner;
    let (mx, my) = ui.mouse().unwrap_or((-1., -1.));
    let hot_row = rect_contains(rect, mx, my) && rect_contains(area, mx, my);
    let key = row.key;
    let (mut hot, mut focused) = (hot_row, false);
    match (row.bind, row.wid) {
        (_, Wid::Static(_)) => { let r = slate::rotator_row(ui, key, rect, row.label, "", false); hot |= r.hot; }
        (Bind::Choice { section, key: ckey, opts, default }, _) => {
            let cur = choice_index(snap, section, ckey, opts, default) as i32;
            let n = opts.len().max(1) as i32;
            let r = slate::rotator_row(ui, key, rect, row.label, opts.get(cur as usize).copied().unwrap_or(""), enabled);
            hot |= r.hot; focused = r.focused;
            if r.delta != 0 { let nv = (cur + r.delta).rem_euclid(n) as usize; ui.session.with_settings(|s| s.set_extra(section, ckey, opts[nv])); }
        }
        (Bind::Hud { tag, default_on }, _) => {
            let cur = hud_on(snap, tag, default_on) as i32;
            let rr = Rect::new(rect.x, rect.y + ui.u(2.), rect.w, (rect.h - ui.u(4.)).max(0.));
            let r = slate::rotator_row(ui, key, rr, row.label, ["Off", "On"][cur as usize], enabled);
            hot |= r.hot; focused = r.focused;
            if r.delta != 0 { let nv = (cur + r.delta).rem_euclid(2) == 1; ui.session.with_settings(|s| hud_set(s, tag, nv)); }
        }
        (Bind::Setting(_), _) | (Bind::MouseSens, _) => {
            let Some(e) = entry_of(row) else { return (hot, false) };
            let id = e.id;
            let Some(cur) = snap.get(id) else { return (hot, false) };
            match (e.kind, cur) {
                (KindDef::Toggle, Value::Bool(b)) => {
                    let r = slate::rotator_row(ui, key, rect, row.label, if b { "On" } else { "Off" }, enabled);
                    hot |= r.hot; focused = r.focused;
                    if r.delta != 0 { ui.session.with_settings(|s| { let _ = s.set(id, Value::Bool(!b)); }); }
                }
                (KindDef::Enum(opts), Value::Index(i)) => {
                    if matches!(row.wid, Wid::Segmented) {
                        let names: Vec<&str> = opts.iter().filter(|o| o.0 != "custom").map(|o| o.1).collect();
                        let cur = if i < names.len() { Some(i) } else { None };
                        let (clicked, hov, foc) = slate::segment_row(ui, key, rect, row.label, &names, cur, enabled);
                        focused = foc; hot |= hov.is_some();
                        if let Some(h) = hov { ui.session.hover_seg = Some((row.key, h)); }
                        if let Some(c) = clicked { if Some(c) != cur { ui.session.with_settings(|s| { let _ = s.set(id, Value::Index(c)); }); } }
                    } else {
                        let n = opts.len().max(1) as i32;
                        let r = slate::rotator_row(ui, key, rect, row.label, opts.get(i).map_or("", |o| o.1), enabled);
                        hot |= r.hot; focused = r.focused;
                        if r.delta != 0 { let nv = (i as i32 + r.delta).rem_euclid(n) as usize; ui.session.with_settings(|s| { let _ = s.set(id, Value::Index(nv)); }); }
                    }
                }
                (KindDef::Slider { min, max, step, decimals }, Value::Float(v)) => {
                    let r = slate::slider_row(ui, key, rect, row.label, v, min, max, step, decimals as usize, enabled);
                    hot |= r.hot; focused = r.focused;
                    if let Some(nv) = r.value {
                        let linked = matches!(row.bind, Bind::MouseSens) && snap.get("input.mouse_sensitivity_y") == Some(Value::Float(v));
                        ui.session.with_settings(|s| { let _ = s.set(id, Value::Float(nv)); if linked { let _ = s.set("input.mouse_sensitivity_y", Value::Float(nv)); } });
                    }
                }
                (KindDef::Resolution, Value::Size(w, h)) => {
                    let modes = display_modes(snap, st, &ui.session.sizes);
                    // recovered row format `{X}x{Y} {AspectRatio}` (Fortnite.VideoOptions), e.g. "1280x720 16:9"
                    let names: Vec<String> = modes.iter().map(|m| super::video::resolution_label(m.0, m.1)).collect();
                    let cur = modes.iter().position(|m| *m == (w, h)).unwrap_or(0);
                    let r = slate::rotator_row(ui, key, rect, row.label, names.get(cur).map_or("", |s| s.as_str()), enabled && !modes.is_empty());
                    hot |= r.hot; focused = r.focused;
                    if r.delta != 0 && !modes.is_empty() { let nv = (cur as i32 + r.delta).rem_euclid(modes.len() as i32) as usize; let m = modes[nv]; ui.session.with_settings(|s| { let _ = s.set(id, Value::Size(m.0, m.1)); }); }
                }
                _ => {}
            }
        }
        _ => {}
    }
    (hot, focused)
}

fn tip_for(row: &RowDef, snap: &Settings, hover_seg: Option<(&'static str, usize)>) -> String {
    if row.tips.is_empty() { return String::new(); }
    let mut idx = match (row.bind, entry_of(row)) { (Bind::Setting(id), Some(_)) => if let Some(Value::Index(i)) = snap.get(id) { i } else { 0 }, _ => 0 };
    // a segmented row shows the hover text of the button under the mouse (the original sets the tooltip on button hover)
    if let (Wid::Segmented, Some((k, seg))) = (row.wid, hover_seg) { if k == row.key { idx = seg; } }
    row.tips[idx.min(row.tips.len() - 1)].to_owned()
}

/// One paragraph of the description panel (label, tip, marker lines). `pad_r`: pixels kept free on the right.
#[derive(Clone, Debug)]
pub struct DescBlock { pub text: String, pub font: FontId, pub size: f32, pub color: [f32; 4], pub gap: f32, pub pad_r: f32 }
impl DescBlock { pub fn new(text: &str, font: FontId, size: f32, color: [f32; 4], gap: f32) -> Self { Self { text: text.to_owned(), font, size, color, gap, pad_r: 0. } } }
/// One laid-out line: `y` is relative to the panel's inner top, `h` the line box height (pixels).
#[derive(Clone, Debug)]
pub struct DescLine { pub text: String, pub font: FontId, pub px: u32, pub color: [f32; 4], pub y: f32, pub h: f32, pub pad_r: f32 }
/// Lay `blocks` out in a `w` x `h` pixel box as whole lines. Tries the nominal sizes, shrinks every block together down to
/// `MIN_TEXT_PX`, and only then drops trailing lines (the last kept line ends in "..."). Pure; tested with the longest hover strings.
pub fn fit_desc_blocks(fonts: &Fonts, blocks: &[DescBlock], scale: f32, w: f32, h: f32) -> Vec<DescLine> {
    let layout = |f: f32| -> (Vec<DescLine>, f32, bool) {
        let (mut out, mut y, mut at_min) = (Vec::new(), 0f32, true);
        for b in blocks {
            let raw = b.size * f * scale;
            if raw.round() > theme::MIN_TEXT_PX { at_min = false; }
            let px = raw.round().max(theme::MIN_TEXT_PX) as u32;
            let lh = (fonts.line_height(b.font, px as f32) * 1.15).ceil();
            for l in wrap_text(fonts, b.font, &b.text, px as f32, (w - b.pad_r).max(1.)) { out.push(DescLine { text: l, font: b.font, px, color: b.color, y, h: lh, pad_r: b.pad_r }); y += lh; }
            y += b.gap * f * scale;
        }
        (out, y - blocks.last().map_or(0., |b| b.gap * f * scale), at_min)
    };
    let mut f = 1.0f32;
    loop {
        let (lines, total, at_min) = layout(f);
        if total <= h + 0.01 { return lines; }
        if at_min || f <= 0.3 {
            let mut keep: Vec<DescLine> = lines.into_iter().filter(|l| l.y + l.h <= h + 0.01).collect();
            if keep.is_empty() { keep = layout(f).0.into_iter().take(1).collect(); }
            if let Some(last) = keep.last_mut() { let t = format!("{} ...", last.text.trim_end()); last.text = fit_text(fonts, last.font, &t, last.px as f32, (w - last.pad_r).max(1.)); }
            return keep;
        }
        f -= 0.04;
    }
}

/// Page geometry (pixels) of a rows-with-tooltip tab, from the widget trees: TabVideoOptions = HBox[SizeBox(left padding 100, MaxDesiredWidth 1250) {rows},
/// TooltipDisplay (Fill, padding [15,15,115,15], VAlign top)]; TabGameOptions / TabAudioOptions = HBox[rows column (padding left 200, Fill 1), TooltipDisplay
/// (Fill 1, padding [30,30,230,30])] (two equal halves); the Game tab puts the 88 high GameTabs strip above the rows.
pub struct PageGeo { pub col_x: f32, pub col_w: f32, pub top: f32, pub strip: Option<Rect>, pub tooltip: Rect }
pub fn page_geo(tab: &str, area: Rect, s: f32, has_sub: bool, desired_units: f32) -> PageGeo {
    let half = area.w * 0.5;
    let (col_x, col_w, tip_pad, tip_x0) = if tab == "video" {
        let cw = (desired_units * s).min(1250. * s);
        (area.x + 100. * s, cw, [15., 15., 115., 15.], area.x + 100. * s + cw)
    } else { (area.x + 200. * s, half - 200. * s, [30., 30., 230., 30.], area.x + half) };
    let (strip, top) = if has_sub { (Some(Rect::new(col_x, area.y, col_w, 88. * s)), area.y + 88. * s) } else { (None, area.y) };
    let x = tip_x0 + tip_pad[0] * s;
    let tooltip = Rect::new(x, area.y + tip_pad[1] * s, (area.x + area.w - x - tip_pad[2] * s).max(0.), (area.h - (tip_pad[1] + tip_pad[3]) * s).max(0.));
    PageGeo { col_x, col_w, top, strip, tooltip }
}
/// Desired width (units) of the rows column: label 300 + gap 20 + control (segmented rows: 160 per option) + slot padding, widest row.
pub fn desired_col_units(tab: &str, rows: &[&RowDef]) -> f32 {
    use super::slate;
    rows.iter().map(|r| {
        let p = slot_pad(tab, r);
        let ctl = match (r.wid, entry_of(r).map(|e| e.kind)) { (Wid::Segmented, Some(KindDef::Enum(o))) => slate::SEGMENT_MIN_W * o.iter().filter(|x| x.0 != "custom").count() as f32, _ => slate::CONTROL_MIN_W };
        slate::LABEL_W + slate::LABEL_GAP + ctl + p[0] + p[2]
    }).fold(0f32, f32::max)
}

/// Tooltip panel of the exact screen: only the original hover text (TextStyle-Base-XS-Blue) of the hovered row. Nothing is drawn when no row is hovered.
pub fn draw_tooltip(ui: &mut Ui, rect: Rect, row: Option<&RowDef>, snap: &Settings, black: bool) {
    let Some(r) = row else { return };
    let tip = tip_for(r, snap, ui.session.hover_seg);
    if !tip.is_empty() { let r = Rect::new(rect.x, rect.y, rect.w, rect.h); if black { super::slate::tooltip_c(ui, r, &tip, [0., 0., 0., 1.]); } else { super::slate::tooltip_h(ui, r, &tip); } }
}

// ---- generic page ---------------------------------------------------------------------------------------------------
pub struct GenericPage { pub tab: &'static TabDef, st: PageState, ctx: super::visibility::MenuContext }
impl GenericPage {
    pub fn new(tab: &'static TabDef) -> Self { Self { tab, st: PageState::default(), ctx: Default::default() } }
    /// The sub tab shown now (the sub tab bar is collapsed in a match, see `visibility::visible_subs`).
    fn sub(&self) -> Option<&'static SubTab> {
        let subs = super::visibility::visible_subs(self.tab, &self.ctx);
        subs.get(self.st.sub.min(subs.len().saturating_sub(1))).copied()
    }
}

impl SettingsPage for GenericPage {
    fn id(&self) -> &str { self.tab.id }
    fn title(&self) -> &str { self.tab.label }
    fn order(&self) -> i32 { self.tab.order }
    fn is_fallback(&self) -> bool { true }
    fn on_show(&mut self) { self.st.desc_row = None; self.st.modes_loaded = false; self.st.desc_need = None; }
    /// Re-opening Settings starts at the first sub tab again (Game, not HUD).
    fn on_close(&mut self) { self.st.sub = 0; self.st.desc_row = None; }
    fn reset_prompt(&self) -> Option<Option<String>> {
        let sub = self.sub()?;
        if sub.rows.is_empty() { return None; }
        match (self.tab.id, sub.id) { ("game", "game") => Some(Some("Reset game options to default?".into())), ("game", "hud") => Some(Some("Reset HUD options to default?".into())), _ => Some(None) }
    }
    fn reset_to_default(&mut self, sx: &mut super::Session) {
        if let Some(sub) = self.sub() { sx.with_settings(|s| for r in sub.rows { reset_row(s, r); }); }
    }
    fn ui(&mut self, ui: &mut Ui, area: Rect) {
        use super::slate;
        let snap = ui.session.settings_clone();
        let ctx = super::visibility::MenuContext::from_settings(&snap);
        self.ctx = ctx;
        let subs = super::visibility::visible_subs(self.tab, &ctx);
        if !subs.is_empty() { self.st.sub = self.st.sub.min(subs.len() - 1); }
        ui.session.hover_seg = None;
        let s = ui.vp.scale;
        let tab = self.tab.id;
        if !self.tab.texts.is_empty() && self.tab.subs.is_empty() {
            // Live Streaming (TabAccountLinkage, WebsiteConfigurationBox): VerticalBox padding top 100: header CommonTextBlock (TextStyle-Header-L-NavyBlue,
            // Burbank 48, centred) | EpicLogo Image 214x244 (padding 20 top / bottom, centred) | AccountLinkText (TextStyle-Base-S-Blue, NotoSans 18,
            // padding left / right 200, wrapped). The URL row ("URL:" + button) shows GetAccountLinkingUIURL(), a server-provided value that does not
            // exist offline, so it is not drawn. The other configuration (FortTwitchLogin) needs the Twitch login state: not available either.
            let navy = crate::menu::outline::srgba([0.017642, 0.022174, 0.049707], 1.);
            let mut y = area.y + ui.u(25.) + ui.u(100.);
            let px = ui.text_px(48.) as f32;
            let hh = ui.fonts.line_height(FontId::Title, px);
            ui.text(Rect::new(area.x, y, area.w, hh), self.tab.texts[0], TextStyle::new(FontId::Title, 48., navy), HAlign::Center);
            y += hh + ui.u(20.);
            let (lw, lh) = (ui.u(214.), ui.u(244.));
            ui.sprite(BrushId::Tex(TexId::EpicLogo), Rect::new(area.x + (area.w - lw) * 0.5, y, lw, lh), false, [1., 1., 1., 1.]);
            y += lh + ui.u(20.);
            let tr = Rect::new(area.x + ui.u(200.), y, (area.w - ui.u(400.)).max(0.), 0.);
            ui.paragraph(tr, self.tab.texts[1], TextStyle::new(FontId::Regular, 18., slate::label_color()), HAlign::Left);
            return;

        }
        let Some(&sub0) = subs.get(self.st.sub) else { return };
        let vis_rows = super::visibility::visible_rows(self.tab.id, sub0, &ctx);
        let pg = page_geo(tab, area, s, subs.len() > 1, desired_col_units(tab, &vis_rows));
        if let Some(strip) = pg.strip {
            // GameTabs: HorizontalTabList (88 high) centred above the rows, ButtonStyle-Tab-Secondary (MinWidth 180, MinHeight 80)
            let labels: Vec<&str> = subs.iter().map(|x| x.label).collect();
            if slate::secondary_tabs(ui, "sub", strip, &labels, &mut self.st.sub) { self.st.desc_row = None; }
        }
        let Some(&sub) = subs.get(self.st.sub) else { return };
        let vis_rows = super::visibility::visible_rows(self.tab.id, sub, &ctx);
        let list = Rect::new(pg.col_x, pg.top, pg.col_w, (area.y + area.h - pg.top).max(0.));
        let mut hot_row: Option<usize> = None;
        let mut focus_row: Option<usize> = None;
        let name = format!("rows.{}", sub.id);
        let st = &mut self.st;
        ui.push_id(&name);
        ui.scroll_area("scroll", list, |ui, view| {
            let mut col = slate::Col::new(view.x, view.y, view.w, s);
            for (i, row) in vis_rows.iter().enumerate() {
                let (hot, focused) = draw_row18(ui, &mut col, tab, row, &snap, list, st);
                if hot { hot_row = Some(i); }
                if focused { focus_row = Some(i); }
            }
            col.height_px(view.y)
        });
        ui.pop_id();
        if let Some(i) = hot_row.or(focus_row) { self.st.desc_row = Some(i); }
        let shown = self.st.desc_row.and_then(|i| vis_rows.get(i).copied());
        let tip_rect = pg.tooltip;
        draw_tooltip(ui, tip_rect, shown, &snap, tab == "game");

    }
}

/// Fallback tabs built from the settings catalog for tabs the original layout JSON does not enumerate (Input key list, Controller).
fn catalog_tab(id: &'static str, label: &'static str, order: i32, tab: &'static str, only_bind: Option<bool>) -> &'static TabDef {
    let rows: Vec<RowDef> = settings::entries().iter().filter(|e| e.tab == tab && only_bind.map_or(true, |b| e.id.starts_with("bind.") == b) ).map(|e| RowDef {
        key: e.id, label: e.label,
        tips: Box::leak(vec![e.tooltip].into_boxed_slice()),
        bind: Bind::Setting(e.id), wid: Wid::Entry, gate: Gate::None, extension: !e.recovered,
    }).collect();
    let subs: &'static [SubTab] = Box::leak(vec![SubTab { id: "", label: "", rows: Box::leak(rows.into_boxed_slice()) }].into_boxed_slice());
    Box::leak(Box::new(TabDef { id, label, order, texts: &[], subs }))
}

/// Every tab of the original screen in original order, as generic pages.
pub fn generic_tabs() -> Vec<&'static TabDef> {
    static TABS: OnceLock<Vec<&'static TabDef>> = OnceLock::new();
    TABS.get_or_init(|| {
        let mut v = Vec::new();
        for t in layout_table::TABS {
            match t.id {
                "input" => v.push(catalog_tab("input", t.label, t.order, "Input", None)),
                "controller" => v.push(catalog_tab("controller", t.label, t.order, "Controller", None)),
                _ => v.push(t),
            }
        }
        v
    }).clone()
}
