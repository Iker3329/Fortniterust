//! The Fortnite 1.8 Settings screen (`/Game/UI/Frontend/Settings/OptionsMenu` hosted by the HUD state layout's MainContentStack):
//! background, top bar (icon, "Settings" title, tab strip), content switcher, bottom bar with the action buttons.
//!
//! RECOVERED (reports/menu-widgets.json, OptionsMenu bytecode):
//! * layout in Slate units: the screen is `Overlay { MainVB { OptionsMenu (fill), BottomBarSizedSpacer 60 }, BottomBarWidget (bottom, fill width) }`;
//!   OptionsMenu = Border-MainL background, top bar `CommonBorder_0` (Border-SolidBG-ShellBlue; icon 52x52 at left padding 16, title
//!   "Settings" TextStyle-PageTitle = NotoSans Regular 30 with left padding 10, tab strip centred in the remaining width, strip height 88),
//!   content `SafeZone_1` with padding [0,25,0,25];
//! * tab buttons: `ButtonStyle-Tab-Manage` (MinWidth 56, MinHeight 80, ButtonPadding 20/0/20/0), label text without a text style = engine default font
//!   (ASSUMED Roboto Bold 24, white), tab order Video, Game, Brightness, Audio, Input, Controller, Live Streaming;
//! * footer actions (SetActionHandlerState per tab): Video [Restore]; Game [Restore] (+Reset HUD To Default only in a match); Brightness/Audio keep the
//!   previous tab's set; Input (keyboard) [Reset To Default]; Controller [Toggle Combat/Building Controls, Restore]; Live Streaming []; Apply only after a
//!   change (and never on Input/Controller/Live Streaming); Back always. Order: ResetHUDToDefault, ToggleMode, Apply, ResetToDefault, Restore, Back.
//!   Back = `HandleBack`: revert the unapplied edits (Handle Reset) and pop; there is no unsaved-changes prompt in 1.8.
//! Materials are flat approximations until `uimat` is wired (see slate.rs).
use super::slate;
use crate::menu::outline::srgba;
use crate::menu::*;

pub const TOPBAR_H: f32 = 88.;
pub const BOTTOM_H: f32 = 60.;
pub const CONTENT_PAD_V: f32 = 25.;
pub const ICON_SIZE: f32 = 52.;
pub const TITLE_PAD_L: f32 = 10.;
pub const HBOX5_PAD_L: f32 = 16.;
pub const TAB_MIN_W: f32 = 56.;
pub const TAB_H: f32 = 80.;
pub const TAB_PAD_LR: f32 = 20.;
pub const TAB_TEXT_SIZE: f32 = 24.;
pub const TAB_STRIP_PAD: f32 = 6.;
pub const TITLE_SIZE: f32 = 30.;
pub const BTN_MIN_W: f32 = 160.;
pub const BTN_MIN_H: f32 = 32.;
pub const BTN_MARGIN: f32 = 5.;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action { ResetHud, ToggleMode, Apply, ResetToDefault, Restore, Back }
impl Action {
    pub fn label(self) -> &'static str { match self { Action::ResetHud => "Reset HUD To Default", Action::ToggleMode => "Toggle Combat/Building Controls", Action::Apply => "Apply", Action::ResetToDefault => "Reset To Default", Action::Restore => "Restore", Action::Back => "Back" } }
    pub fn key(self) -> &'static str { match self { Action::ResetHud => "D", Action::ToggleMode => "", Action::Apply => "A", Action::ResetToDefault => "R", Action::Restore => "R", Action::Back => "Esc" } }
}

// [restore, reset_hud] kept by Brightness/Audio live in Session::inherited (reset with Session::reset_transients).

/// Footer actions of `tab` in registration order (bytecode `SetActionHandlerState`; 0 = shown). `in_zone` = IsInZone, `dirty` = Apply Visible.
pub fn footer_actions(tab: &str, in_zone: bool, dirty: bool, gamepad: bool, inherited: &mut [bool; 2]) -> Vec<Action> {
    let (restore, hud, toggle, reset, apply_ok) = match tab {
        "video" => (true, false, false, false, true),
        "game" => (true, in_zone, false, false, true),
        // The 1.8 Input tab registers no Apply handler: its bindings take effect immediately (InputPage requests the apply itself after every edit).
        "input" => (false, false, false, !gamepad, false),
        "controller" => (true, false, true, false, false),
        "livestream" => (false, false, false, false, false),
        _ => { let [r, h] = *inherited; (r, h, false, false, true) } // brightness / audio: unchanged by the tab switch
    };
    if matches!(tab, "video" | "game") { *inherited = [restore, hud]; }
    let mut v = Vec::new();
    if hud { v.push(Action::ResetHud); }
    if toggle { v.push(Action::ToggleMode); }
    if apply_ok && dirty { v.push(Action::Apply); }
    if reset { v.push(Action::ResetToDefault); }
    if restore { v.push(Action::Restore); }
    v.push(Action::Back);
    v
}

/// Measured geometry of the whole screen (pure; used by the screen and by the layout test). `labels` = tab titles.
pub fn chrome_geo(fonts: &Fonts, s: f32, vw: f32, vh: f32, labels: &[&str]) -> Geo {
    let tpx = (TAB_TEXT_SIZE * s).round().max(theme::MIN_TEXT_PX);
    let tab_w: Vec<f32> = labels.iter().map(|l| fonts.measure(FontId::RobotoBold, l, tpx) / s).collect();
    let title_px = (TITLE_SIZE * s).round().max(theme::MIN_TEXT_PX);
    geometry(vw, vh, &tab_w, fonts.measure(FontId::Regular, "Settings", title_px) / s, fonts.line_height(FontId::Regular, title_px) / s)
}
/// Footer button rects in Slate units (right aligned in the 60 high bar, slot padding 5, text line height + 12 high, at least 32).
pub fn footer_rects(fonts: &Fonts, s: f32, vw: f32, vh: f32, actions: &[Action]) -> Vec<(Rect, Rect)> {
    let bpx = (18. * s).round().max(theme::MIN_TEXT_PX);
    let lh = fonts.line_height(FontId::Regular, bpx) / s;
    let h = (lh + 12.).max(BTN_MIN_H);
    // IconTextButton (ButtonStyle-BottomBar): ButtonPadding 4+4, InputActionWidget slot pad 2 (key icon width unknown = 0), text slot padding 5+5, empty RightExtraContentSlot 4+4
    let widths: Vec<f32> = actions.iter().map(|a| (fonts.measure(FontId::Regular, a.label(), bpx) / s + 28.).max(BTN_MIN_W)).collect();
    let mut right = vw - BTN_MARGIN;
    let mut out = vec![(Rect::new(0., 0., 0., 0.), Rect::new(0., 0., 0., 0.)); actions.len()];
    for i in (0..actions.len()).rev() {
        let r = Rect::new(right - widths[i], vh - BOTTOM_H + (BOTTOM_H - h) * 0.5, widths[i], h);
        let tw = fonts.measure(FontId::Regular, actions[i].label(), bpx) / s;
        out[i] = (r, Rect::new(r.x + 11., r.y + 4., tw, lh));
        right -= widths[i] + 2. * BTN_MARGIN;
    }
    out
}
pub const KEY_W: f32 = 30.;

/// Pure geometry of the screen in Slate units.
pub struct Geo {
    pub vw: f32, pub vh: f32,
    pub background: Rect, pub topbar: Rect, pub icon: Rect, pub title: Rect, pub tabs: Rect, pub tab_rects: Vec<Rect>, pub content: Rect, pub bottom: Rect,
}
/// `tab_text_w`: measured label widths in units; `title_w`: measured title width in units.
pub fn geometry(vw: f32, vh: f32, tab_text_w: &[f32], title_w: f32, title_h: f32) -> Geo {
    let main_h = (vh - BOTTOM_H).max(0.);
    let background = Rect::new(0., 0., vw, main_h);
    let topbar = Rect::new(0., 0., vw, TOPBAR_H);
    let icon = Rect::new(HBOX5_PAD_L, (TOPBAR_H - ICON_SIZE) * 0.5, ICON_SIZE, ICON_SIZE);
    let title = Rect::new(icon.x + ICON_SIZE + TITLE_PAD_L, (TOPBAR_H - title_h) * 0.5, title_w, title_h);
    let hbox5_w = HBOX5_PAD_L + ICON_SIZE + TITLE_PAD_L + title_w;
    // tab button width = max(MinWidth, ButtonPadding 20+20 + text + its slot padding 4+4); LeftSideImage is Collapsed (NoDrawType brush, bytecode)
    let widths: Vec<f32> = tab_text_w.iter().map(|t| (2. * TAB_PAD_LR + t + 8.).max(TAB_MIN_W)).collect();
    let total: f32 = widths.iter().sum::<f32>() + 2. * TAB_STRIP_PAD;
    let space = (vw - hbox5_w).max(0.);
    let sx = hbox5_w + (space - total) * 0.5;
    let tabs = Rect::new(sx, 0., total, TOPBAR_H);
    let mut x = sx + TAB_STRIP_PAD;
    let mut tab_rects = Vec::new();
    for w in &widths { tab_rects.push(Rect::new(x, (TOPBAR_H - TAB_H) * 0.5, *w, TAB_H)); x += w; }
    let content = Rect::new(0., TOPBAR_H + CONTENT_PAD_V, vw, (main_h - TOPBAR_H - 2. * CONTENT_PAD_V).max(0.));
    Geo { vw, vh, background, topbar, icon, title, tabs, tab_rects, content, bottom: Rect::new(0., main_h, vw, BOTTOM_H) }
}

fn scaled(r: Rect, s: f32) -> Rect { Rect::new(r.x * s, r.y * s, r.w * s, r.h * s) }

/// Draw the screen. `shown` = the tab whose `on_show` has run (SettingsScreen state).
pub fn ui(ui: &mut Ui, cx: &mut Ctx, shown: &mut Option<usize>) {
    let s = ui.vp.scale;
    let (vw, vh) = ui.vp.slate_size();
    let n = cx.pages.len();
    let labels: Vec<String> = cx.pages.iter().map(|p| p.title().to_owned()).collect();
    let lrefs: Vec<&str> = labels.iter().map(|l| l.as_str()).collect();
    let g = chrome_geo(ui.fonts, s, vw, vh, &lrefs);
    // ---- background + top bar (interim fills; materials: M-Border-MainL over the blurred game, M_UI_Border-SolidBG-ShellBlue)
    let bg = scaled(g.background, s);
    ui.material(super::uimat::Mat::BorderMainL, super::uimat::MatState::Base, bg, 0., WHITE_T);
    ui.material(super::uimat::Mat::SolidBgShellBlue, super::uimat::MatState::Base, scaled(g.topbar, s), 0., WHITE_T);
    ui.brush(BrushId::Icon(IconKind::Config), scaled(g.icon, s), WHITE_T);
    ui.text(scaled(g.title, s), "Settings", TextStyle::new(FontId::Regular, TITLE_SIZE, WHITE_T), HAlign::Left);
    // ---- tabs
    let mut leave = false;
    if n > 0 {
        if *cx.settings_tab >= n { *cx.settings_tab = n - 1; }
        let before = *cx.settings_tab;
        let mut sel = before;
        if !ui.is_inert() {
            if ui.take_nav(NavKey::TabNext) { sel = (sel + 1) % n; }
            if ui.take_nav(NavKey::TabPrev) { sel = (sel + n - 1) % n; }
        }
        for (i, r) in g.tab_rects.iter().enumerate() {
            let pr = scaled(*r, s);
            let id = ui.id(&format!("tab{i}"));
            let it = ui.interact(id, pr, true, true);
            if it.clicked { sel = i; }
            let active = sel == i;
            slate::mat::tab_manage(ui, pr, active, it.hot || it.focused);
            ui.text(pr, &labels[i], TextStyle::new(FontId::RobotoBold, TAB_TEXT_SIZE, WHITE_T), HAlign::Center);
        }
        if shown.is_none() { *shown = Some(before); if !ui.is_inert() { cx.pages[before].on_show(); } }
        if sel != before { cx.pages[before].on_hide(); *cx.settings_tab = sel; cx.pages[sel].on_show(); *shown = Some(sel); }
    }
    // ---- content
    let content = scaled(g.content, s);
    let tab_id = if n > 0 { cx.pages[*cx.settings_tab].id().to_owned() } else { String::new() };
    if n == 0 { ui.text(content, "No settings pages are registered yet.", TextStyle::new(FontId::Regular, 17., slate::label_color()), HAlign::Center); }
    else {
        let sel = *cx.settings_tab;
        ui.push_id(&tab_id);
        ui.push_clip(content);
        cx.pages[sel].ui(ui, content);
        ui.pop_clip();
        ui.pop_id();
    }
    // ---- bottom bar + actions
    let bottom = scaled(g.bottom, s);
    ui.fill(bottom, srgba([0.007499, 0.009134, 0.024158], 1.));
    let snap = ui.session.settings_clone();
    let ctx = super::visibility::MenuContext::from_settings(&snap);
    let (dirty, busy) = (ui.session.is_dirty(), ui.session.busy());
    let actions = footer_actions(&tab_id, ctx.in_zone, dirty, false, &mut ui.session.inherited);
    // buttons right aligned (see footer_rects)
    let fr = footer_rects(ui.fonts, s, vw, vh, &actions);
    let rects: Vec<Rect> = fr.iter().map(|x| x.0).collect();
    let rects_t: Vec<Rect> = fr.iter().map(|x| scaled(x.1, s)).collect();
    for (i, a) in actions.iter().enumerate() {
        let r = scaled(rects[i], s);
        let id = ui.id(&format!("act{i}"));
        let enabled = !(matches!(a, Action::Apply) && busy);
        let it = ui.interact(id, r, enabled, true);
        let hot = it.hot || it.focused;
        slate::mat::bottom_button(ui, r, hot, it.down, enabled);
        let tc = if hot { srgba([0.01, 0.01, 0.01], 1.) } else { [1., 1., 1., 0.5] };
        // text: left aligned at x+11 (InputActionWidget pad 2 + text pad 5 + ButtonPadding 4); the key icon width is unknown (UNRESOLVED), so this runtime
        // draws a small key prompt only when it fits between the button's right edge and the text (APPROXIMATE, not original art)
        let tw = rects_t[i].w;
        let tx = r.x + 11. * s;
        if !a.key().is_empty() && (r.w - 11. * s - tw) > (KEY_W + 12.) * s {
            let kr = Rect::new(r.x + r.w - (KEY_W + 8.) * s, r.y + 6. * s, KEY_W * s, r.h - 12. * s);
            ui.fill(kr, [0., 0., 0., 0.45]);
            ui.text(kr, a.key(), TextStyle::new(FontId::Bold, 14., [1., 1., 1., 0.9]), HAlign::Center);
        }
        ui.text(Rect::new(tx, r.y, r.w - 11. * s, r.h), a.label(), TextStyle::new(FontId::Regular, 18., tc), HAlign::Left);
        if it.clicked {
            match a {
                Action::Apply => ui.session.apply_button(),
                Action::Restore => ui.session.discard_changes(),
                Action::Back => leave = true,
                Action::ResetToDefault | Action::ResetHud => { if n > 0 { let sel = *cx.settings_tab; cx.pages[sel].reset_to_default(ui.session); } }
                Action::ToggleMode => { if n > 0 { let sel = *cx.settings_tab; cx.pages[sel].toggle_mode(); } }
            }
        }
    }
    if ui.back_pressed() { leave = true; }
    if leave { ui.session.discard_changes(); cx.pop(); } // pop runs SettingsScreen::on_leave, which tells every page the screen closed
}

const WHITE_T: [f32; 4] = [1., 1., 1., 1.];
