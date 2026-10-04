//! In-game Esc menu entry point, generic confirm dialog, Settings container and demo pages.
//!
//! The Esc menu itself is `mainmenu.rs` (a rebuild of the original `/Game/UI/Frontend/MainMenu/MainMenu`, the same
//! overlay as the lobby hamburger menu; there is no separate pause widget and no "Resume" button in 1.10). The first
//! version of this file built a Resume / Settings / Exit Game menu from the task brief before that evidence existed;
//! it was replaced. `PauseScreen` stays as the name the menu stack and the tests use for the root screen.
//! Flow: Esc (menu closed) -> `MenuSystem::open()` -> PauseScreen (= MainMenuScreen); Esc / pad B/Start at the root
//! closes it (`HostRequest::Closed`, host re-captures the mouse); the Settings icon pushes SettingsScreen; Leave Match
//! pushes the original "Leave Match?" confirmation (`mainmenu::ConfirmationWindow`) and, confirmed, emits
//! `HostRequest::QuitToDesktop`.
//!
//! `ConfirmDialog` below is the toolkit's generic (non-original, approximated) confirm dialog.
use super::theme::*;
use super::*;

/// Slate-unit size of the viewport.
fn slate(ui: &Ui) -> (f32, f32) { ui.vp.slate_size() }

/// Root screen of the menu stack: the original MainMenu overlay in its in-match context.
pub type PauseScreen = super::mainmenu::MainMenuScreen;

// ---- confirm dialog ----------------------------------------------------------------------------------------------
type Callback = Box<dyn FnMut(&mut Ctx)>;
/// Modal confirm dialog. Esc / pad B cancels; the safe (cancel) button has the initial focus.
pub struct ConfirmDialog { pub title: String, pub body: String, pub confirm: String, pub cancel: String, pub danger: bool, on_confirm: Option<Callback> }
impl ConfirmDialog {
    pub fn new(title: &str, body: &str, confirm: &str, cancel: &str, danger: bool, on_confirm: impl FnMut(&mut Ctx) + 'static) -> Self {
        Self { title: title.into(), body: body.into(), confirm: confirm.into(), cancel: cancel.into(), danger, on_confirm: Some(Box::new(on_confirm)) }
    }
}
/// Dialog width in Slate units (520 max, never wider than the viewport).
pub fn dialog_width(vw_units: f32) -> f32 { 520f32.min((vw_units - 32.).max(vw_units * 0.8)) }
/// Dialog geometry in pixels: (panel, body rect, confirm button, cancel button). Pure for tests.
pub fn dialog_layout(vw_units: f32, vh_units: f32, scale: f32, body_h_units: f32) -> (Rect, Rect, Rect, Rect) {
    let w = dialog_width(vw_units);
    let pad = 24.;
    let title_h = 44.;
    let btn_h = BTN_M_H + 4.;
    let h = (pad + title_h + body_h_units + 20. + btn_h + pad).min(vh_units - 16.);
    let (x, y) = ((vw_units - w) * 0.5, (vh_units - h) * 0.5);
    let panel = Rect::new(x * scale, y * scale, w * scale, h * scale);
    let body = Rect::new((x + pad) * scale, (y + pad + title_h) * scale, (w - 2. * pad) * scale, body_h_units * scale);
    let bw = ((w - 2. * pad - 12.) * 0.5).min(220.);
    let by = (y + h - pad - btn_h) * scale;
    let total = 2. * bw + 12.;
    let bx = x + (w - total) * 0.5;
    let confirm = Rect::new(bx * scale, by, bw * scale, btn_h * scale);
    let cancel = Rect::new((bx + bw + 12.) * scale, by, bw * scale, btn_h * scale);
    (panel, body, confirm, cancel)
}
impl Screen for ConfirmDialog {
    fn name(&self) -> &str { "confirm" }
    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let (vw, vh) = slate(ui);
        ui.fill(ui.screen(), [0., 0., 0., 0.35]);
        let px = ui.text_px(FONT_BODY) as f32;
        let wrap_w = ((dialog_width(vw) - 48.) * ui.vp.scale).max(1.);
        let lines = wrap_text(ui.fonts, FontId::Regular, &self.body, px, wrap_w).len() as f32;
        let lh = (ui.fonts.line_height(FontId::Regular, px) * 1.15).ceil() / ui.vp.scale;
        let (panel, body, confirm, cancel) = dialog_layout(vw, vh, ui.vp.scale, (lines * lh).max(lh));
        ui.fill(rect_inflate(panel, ui.u(2.).max(1.)), PANEL_EDGE);
        ui.fill(panel, [0.01, 0.055, 0.085, 1.]);
        ui.text(Rect::new(panel.x + ui.u(24.), panel.y + ui.u(20.), panel.w - ui.u(48.), ui.u(44.)), &self.title, TextStyle::new(FontId::Bold, 26., WHITE).shadowed(), HAlign::Left);
        ui.paragraph(body, &self.body, TextStyle::new(FontId::Regular, FONT_BODY, TEXT_DIM), HAlign::Left);
        let (cid, xid) = (ui.id("confirm_yes"), ui.id("confirm_no"));
        ui.default_focus(xid);
        if ui.button("confirm_yes", confirm, &self.confirm, if self.danger { BtnKind::Critical } else { BtnKind::Emphasis }, BtnSize::M, true).clicked {
            if let Some(mut f) = self.on_confirm.take() { f(cx); }
        }
        let cancelled = ui.button("confirm_no", cancel, &self.cancel, BtnKind::Default, BtnSize::M, true).clicked;
        ui.horizontal_focus(&[cid, xid]);
        if cancelled || ui.back_pressed() { cx.pop(); }
    }
}

// ---- settings container -------------------------------------------------------------------------------------------
/// true = the pre-1.8 toolkit layout (kept compiled for reference); the 1.8 screen is `pages::screen18`.
const LEGACY_LAYOUT: bool = false;
pub struct SettingsScreen { shown: Option<usize> }
impl SettingsScreen { pub fn new() -> Self { Self { shown: None } } }

/// Settings layout in pixels: (title, close button, tab bar, page area, back button). Pure for tests.
pub fn settings_layout(vw_units: f32, vh_units: f32, scale: f32) -> (Rect, Rect, Rect, Rect, Rect) {
    let w = 1200f32.min((vw_units - 32.).max(vw_units * 0.8));
    let x = (vw_units - w) * 0.5;
    let (top, title_h, tab_h, back_h) = (20f32, 52f32, BTN_M_H, BTN_M_H);
    let title = Rect::new(x, top, w - 52., title_h);
    let close = Rect::new(x + w - 40., top + 8., 36., 36.);
    let tabs = Rect::new(x, top + title_h + 4., w, tab_h);
    let page_y = tabs.y + tab_h + 2.;
    let back = Rect::new(x, vh_units - 20. - back_h, 160f32.min(w), back_h);
    let page = Rect::new(x, page_y, w, (back.y - 12. - page_y).max(40.));
    let s = |r: Rect| Rect::new(r.x * scale, r.y * scale, r.w * scale, r.h * scale);
    (s(title), s(close), s(tabs), s(page), s(back))
}
impl Screen for SettingsScreen {
    fn name(&self) -> &str { "settings" }
    fn opaque(&self) -> bool { true }
    fn on_leave(&mut self, cx: &mut Ctx) { for p in cx.pages.iter_mut() { p.on_close(); } }
    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        // Fortnite 1.8 Settings screen (OptionsMenu): see pages/screen18.rs. The toolkit layout below is kept only for the legacy tests/pages.
        if !LEGACY_LAYOUT { pages::screen18::ui(ui, cx, &mut self.shown); return; }
        let (vw, vh) = slate(ui);
        let (title, close, tabs, page, back) = settings_layout(vw, vh, ui.vp.scale);
        ui.fill(ui.screen(), [0.004, 0.03, 0.05, 0.985]);
        ui.text(title, "Settings", TextStyle::new(FontId::Bold, FONT_TITLE, WHITE).shadowed(), HAlign::Left);
        // close (X) button
        let cid = ui.id("close");
        let it = ui.interact(cid, close, true, true);
        ui.brush(BrushId::Close(it.hot || it.focused || it.down), close, WHITE);
        let mut leave = it.clicked;
        let n = cx.pages.len();
        if n > 0 {
            if *cx.settings_tab >= n { *cx.settings_tab = n - 1; }
            let titles: Vec<String> = cx.pages.iter().map(|p| p.title().to_owned()).collect();
            let refs: Vec<&str> = titles.iter().map(|s| s.as_str()).collect();
            let before = *cx.settings_tab;
            ui.tab_bar("tabs", tabs, &refs, cx.settings_tab);
            if self.shown.is_none() { self.shown = Some(*cx.settings_tab); if !ui.is_inert() { cx.pages[*cx.settings_tab].on_show(); } }
            if *cx.settings_tab != before { cx.pages[before].on_hide(); cx.pages[*cx.settings_tab].on_show(); self.shown = Some(*cx.settings_tab); }
        }
        // page frame
        ui.fill(rect_inflate(page, ui.u(2.).max(1.)), PANEL_EDGE);
        ui.fill(page, PANEL);
        if n == 0 {
            ui.text(page, "No settings pages are registered yet.", TextStyle::new(FontId::Regular, FONT_BODY, TEXT_DIM), HAlign::Center);
        } else {
            let sel = *cx.settings_tab;
            let inner = rect_deflate(page, ui.u(6.), ui.u(6.), ui.u(6.), ui.u(6.));
            let pid = cx.pages[sel].id().to_owned();
            ui.push_id(&pid);
            ui.push_clip(inner);
            cx.pages[sel].ui(ui, inner);
            ui.pop_clip();
            ui.pop_id();
        }
        if ui.button("back", back, "Back", BtnKind::Default, BtnSize::M, true).clicked { leave = true; }
        pages::footer::draw(ui, cx, back, page); // status text + "Reset To Default" + "Apply" (menu/pages)
        if ui.back_pressed() { leave = true; }
        // leaving with unapplied edits asks first (Apply / Discard / Cancel); otherwise just go back one level
        if leave && pages::footer::may_leave(ui, cx) { cx.pop(); }
    }
}

// ---- demo pages (toolkit gallery; used by --menu-test and as a reference for page authors) ------------------------
/// Reference/test pages exercising every widget. They are NOT Fortnite settings: titles say so.
pub struct GalleryPage { kind: u8, toggles: [bool; 4], sliders: [f32; 3], sel: [usize; 3], binds: Vec<Option<u16>>, long: Vec<(bool, f32)> }
impl GalleryPage {
    pub fn widgets() -> Self { Self::new(0) }
    pub fn long_list() -> Self { Self::new(1) }
    pub fn binds() -> Self { Self::new(2) }
    fn new(kind: u8) -> Self {
        Self { kind, toggles: [true, false, true, false], sliders: [50., 0.65, 1.1], sel: [1, 0, 2], binds: vec![Some(0x57), Some(0x41), Some(0x53), Some(0x44), Some(0x20), Some(0x11), None, Some(0x01), Some(0x70)], long: (0..60).map(|i| (i % 3 == 0, (i * 7 % 100) as f32)).collect() }
    }
}
impl SettingsPage for GalleryPage {
    fn id(&self) -> &str { ["gallery.widgets", "gallery.list", "gallery.binds"][self.kind as usize] }
    fn title(&self) -> &str { ["Widget Gallery", "Long List", "Key Binds"][self.kind as usize] }
    fn order(&self) -> i32 { self.kind as i32 }
    fn ui(&mut self, ui: &mut Ui, area: Rect) {
        let kind = self.kind;
        let (toggles, sliders, sel, binds, long) = (&mut self.toggles, &mut self.sliders, &mut self.sel, &mut self.binds, &mut self.long);
        ui.form("page", area, |ui, f| match kind {
            0 => {
                f.header(ui, "Toolkit demo (not Fortnite settings)");
                f.toggle(ui, "Enabled toggle", &mut toggles[0], true);
                f.toggle(ui, "Disabled toggle (checked)", &mut toggles[2], false);
                f.toggle(ui, "A very long label that cannot possibly fit inside its column and must be clipped with an ellipsis", &mut toggles[1], true);
                f.slider(ui, "Integer slider", &mut sliders[0], 0., 100., 1., 0, "", true);
                f.slider(ui, "Fractional slider", &mut sliders[1], 0., 1., 0.05, 2, "", true);
                f.slider(ui, "Disabled slider", &mut sliders[2], 0.5, 2., 0.1, 1, "x", false);
                f.header(ui, "Selectors");
                f.selector(ui, "Arrow-cycle selector", &["Low", "Medium", "High", "Epic"], &mut sel[0], true);
                f.dropdown(ui, "Dropdown", &["Windowed", "Windowed Fullscreen", "Fullscreen", "Borderless", "Exclusive", "Debug", "Test", "More", "Extra"], &mut sel[1], true);
                f.dropdown(ui, "Disabled dropdown", &["Locked"], &mut sel[2], false);
                f.header(ui, "Buttons");
                f.button(ui, "Default button", BtnKind::Default, true);
                f.button(ui, "Emphasis button", BtnKind::Emphasis, true);
                f.button(ui, "Critical button", BtnKind::Critical, true);
                f.button(ui, "Disabled button", BtnKind::Default, false);
                f.note(ui, "Mouse wheel scrolls; sliders ignore the wheel. Esc closes dropdowns first, then goes back.");
            }
            1 => {
                f.header(ui, "60 rows (scroll list)");
                for (i, (on, v)) in long.iter_mut().enumerate() {
                    if i % 2 == 0 { f.toggle(ui, &format!("Option {i}"), on, true); } else { f.slider(ui, &format!("Amount {i}"), v, 0., 100., 5., 0, "%", true); }
                }
            }
            _ => {
                f.header(ui, "Key binds");
                let names = ["Move Forward", "Move Left", "Move Backward", "Move Right", "Jump", "Crouch", "Interact (unbound)", "Fire / Harvest", "Emote"];
                for (i, n) in names.iter().enumerate() { f.keybind(ui, n, &mut binds[i], true); }
                f.note(ui, "Click a bind, then press a key or mouse button. Esc cancels; Backspace/Delete unbinds.");
            }
        });
    }
}
/// Register the three demo pages.
pub fn register_demo_pages(menu: &mut MenuSystem) {
    menu.register_settings_page(Box::new(GalleryPage::widgets()));
    menu.register_settings_page(Box::new(GalleryPage::long_list()));
    menu.register_settings_page(Box::new(GalleryPage::binds()));
}
