//! Modal dialogs of the Settings screen: Reset confirmation, unsaved-changes prompt and the 15 s "keep these video
//! settings?" dialog.
//!
//! PROVENANCE: the original strings "Do you want to keep these video settings?" (Yes/No), "Reset game options to default?"
//! and "Reset HUD options to default?" (Yes/No) are recovered (menu-layout-settings.json `confirmations`). The unsaved-changes
//! prompt text/buttons and the 15 second timeout are NOT original (assumed; the original has no such prompt that we could find).
use super::bridge::{self, Session};
use crate::menu::pause::dialog_width;
use crate::menu::theme::*;
use crate::menu::*;
use crate::settings::Value;

type Callback = Box<dyn FnMut(usize, &mut Ctx, &mut Session)>;

/// A modal with 1..=3 buttons. Esc / pad B = `cancel` button. Pops itself BEFORE running the callback (so the callback may push).
pub struct Modal {
    title: String, body: String, buttons: Vec<(String, BtnKind)>,
    default_focus: usize, cancel: usize,
    /// (seconds left, button index chosen on timeout, suffix shown under the body; `{}` = whole seconds left)
    countdown: Option<(f32, usize, String)>,
    /// Result used if the screen is torn down without an answer (menu closed).
    on_leave_result: Option<usize>,
    on_result: Option<Callback>,
    done: bool,
}
impl Modal {
    pub fn new(title: &str, body: &str, buttons: &[(&str, BtnKind)], default_focus: usize, cancel: usize, on_result: impl FnMut(usize, &mut Ctx, &mut Session) + 'static) -> Self {
        Self { title: title.into(), body: body.into(), buttons: buttons.iter().map(|(l, k)| (l.to_string(), *k)).collect(), default_focus, cancel, countdown: None, on_leave_result: None, on_result: Some(Box::new(on_result)), done: false }
    }
    pub fn with_countdown(mut self, seconds: f32, result: usize, suffix: &str) -> Self { self.countdown = Some((seconds, result, suffix.into())); self.on_leave_result = Some(result); self }
    pub fn seconds_left(&self) -> Option<f32> { self.countdown.as_ref().map(|c| c.0) }
    fn finish(&mut self, idx: usize, cx: &mut Ctx, sx: &mut Session) {
        if self.done { return; }
        self.done = true;
        cx.pop();
        if let Some(mut f) = self.on_result.take() { f(idx, cx, sx); }
    }
}
/// Geometry in pixels: (panel, body, button rects). Pure so it is unit-tested.
pub fn modal_layout(vw: f32, vh: f32, scale: f32, body_h: f32, n_buttons: usize) -> (Rect, Rect, Vec<Rect>) { modal_layout_t(vw, vh, scale, body_h, n_buttons, true) }
/// [`modal_layout`] with or without the title band (the original overlay messages have no title, only the message text).
pub fn modal_layout_t(vw: f32, vh: f32, scale: f32, body_h: f32, n_buttons: usize, titled: bool) -> (Rect, Rect, Vec<Rect>) {
    let w = dialog_width(vw);
    let (pad, title_h, gap) = (24., if titled { 44. } else { 0. }, 12.);
    let btn_h = BTN_M_H + 4.;
    let h = (pad + title_h + body_h + 20. + btn_h + pad).min(vh - 16.);
    let (x, y) = ((vw - w) * 0.5, (vh - h) * 0.5);
    let n = n_buttons.max(1) as f32;
    let bw = ((w - 2. * pad - gap * (n - 1.)) / n).min(200.).max(40.);
    let total = bw * n + gap * (n - 1.);
    let bx = x + (w - total) * 0.5;
    let by = y + h - pad - btn_h;
    let rects = (0..n_buttons).map(|i| Rect::new((bx + i as f32 * (bw + gap)) * scale, by * scale, bw * scale, btn_h * scale)).collect();
    (Rect::new(x * scale, y * scale, w * scale, h * scale), Rect::new((x + pad) * scale, (y + pad + title_h) * scale, (w - 2. * pad) * scale, body_h * scale), rects)
}
impl Screen for Modal {
    fn name(&self) -> &str { "settings-modal" }
    fn on_leave(&mut self, cx: &mut Ctx) {
        if self.done { return; }
        self.done = true;
        if let (Some(i), Some(mut f), Some(sx)) = (self.on_leave_result, self.on_result.take(), cx.session.take()) { f(i, cx, sx); }
    }
    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        // OptionsMenu `Message`: CommonBorder Border-Bottom-Box-Rounded-DkBlue (BrushColor 0,0,0,0.7, padding 25) > VerticalBox (padding 25) >
        // [SizeBox max width 1000 { OverlayText TextStyle-Header-L = Burbank 48, centred, auto wrap }, HorizontalBox (centred) { Yes (pad R 50, Fill, left),
        // No (pad L 50, Fill, right) }]; IconTextButtons use ButtonStyle-Primary-M. The screen behind is NOT dimmed (the message is just an overlay).
        let s = ui.vp.scale;
        let body_text = self.body.clone();
        let px = ui.text_px(48.) as f32;
        let max_w = 1000. * s;
        let lines: Vec<String> = wrap_text(ui.fonts, FontId::Title, &body_text, px, max_w);
        let lh = ui.fonts.line_height(FontId::Title, px);
        let text_w = lines.iter().map(|l| ui.fonts.measure(FontId::Title, l, px)).fold(0f32, f32::max);
        let n = self.buttons.len().max(1);
        let btn_w = 160. * s;
        // HorizontalBox desired width: two buttons + 50 + 50 padding (one button: button + 50)
        let hbox_w = if n >= 2 { 2. * btn_w + 100. * s } else { btn_w + 50. * s };
        let content_w = text_w.max(hbox_w);
        let btn_h = 48. * s;
        let extra = match &self.countdown { Some((left, _, suffix)) => suffix.replace("{}", &(left.ceil().max(0.) as i64).to_string()), None => String::new() };
        let extra_h = if extra.is_empty() { 0. } else { 30. * s };
        let (pw, ph) = (content_w + 100. * s, 50. * s + lines.len() as f32 * lh + extra_h + btn_h + 50. * s);
        let panel = Rect::new((ui.vp.w - pw) * 0.5, (ui.vp.h - ph) * 0.5, pw, ph);
        ui.material(super::uimat::Mat::BottomBoxRounded, super::uimat::MatState::Base, panel, 0., [0., 0., 0., 0.7]);
        let mut y = panel.y + 50. * s;
        for l in &lines { ui.text(Rect::new(panel.x + 50. * s, y, content_w, lh), l, TextStyle::new(FontId::Title, 48., [1., 1., 1., 1.]), HAlign::Center); y += lh; }
        if !extra.is_empty() { ui.text(Rect::new(panel.x + 50. * s, y + 4. * s, content_w, 26. * s), &extra, TextStyle::new(FontId::Regular, 18., [1., 1., 1., 0.8]), HAlign::Center); y += extra_h; }
        let hx = panel.x + 50. * s + (content_w - hbox_w) * 0.5;
        let ids: Vec<Id> = (0..self.buttons.len()).map(|i| ui.id(&format!("modal_btn{i}"))).collect();
        if let Some(id) = ids.get(self.default_focus) { ui.default_focus(*id); }
        let mut chosen: Option<usize> = None;
        for (i, (label, _kind)) in self.buttons.iter().enumerate() {
            let bx = if i == 0 { hx } else { hx + hbox_w - btn_w };
            let r = Rect::new(bx, y, btn_w, btn_h);
            let it = ui.interact(ids[i], r, true, true);
            let st = if it.down { super::uimat::MatState::Pressed } else if it.hot || it.focused { super::uimat::MatState::Hovered } else { super::uimat::MatState::Base };
            ui.material(super::uimat::Mat::RadioM, st, r, 0., [1., 1., 1., 1.]);
            ui.text(r, label, TextStyle::new(FontId::Bold, 18., crate::menu::outline::srgba([0.008023, 0.01096, 0.026241], 1.)), HAlign::Center);
            if it.clicked { chosen = Some(i); }
        }
        ui.horizontal_focus(&ids);
        if !ui.is_inert() {
            if let Some((left, res, _)) = self.countdown.as_mut() { *left -= ui.dt; if *left <= 0. && chosen.is_none() { chosen = Some(*res); } }
        }
        if chosen.is_none() && ui.back_pressed() { chosen = Some(self.cancel); }
        if let Some(i) = chosen { self.finish(i, cx, &mut *ui.session); }
    }
}

/// The original video-settings confirmation (Yes keeps; No / Esc / timeout / menu closed reverts).
pub struct KeepVideoDialog;
impl KeepVideoDialog {
    pub fn new(revert: Vec<(&'static str, Value)>) -> Modal {
        Modal::new("", "Do you want to keep these video settings?",&[("Yes", BtnKind::Emphasis), ("No", BtnKind::Default)], 0, 1, move |idx, _cx, sx| { // original focus: CenterWidget(VideoSettingsAccept) = Yes
            if idx == 0 { sx.confirm_keep(); } else { sx.revert_values(&revert); }
        }).with_countdown(bridge::REVERT_SECONDS, 1, "Reverting in {} seconds.")
    }
}
/// "Reset ...?" confirmation: Yes resets the CURRENT tab's working values (nothing is applied until Apply).
pub fn reset_dialog(prompt: &str) -> Modal {
    // original: overlay text only, Yes / No buttons, focus on No (`CenterWidget(ResetGameOptionsToDefaultNo)`)
    Modal::new("", prompt, &[("Yes", BtnKind::Emphasis), ("No", BtnKind::Default)], 1, 1, |idx, cx, sx| {
        if idx == 0 { let t = *cx.settings_tab; if let Some(p) = cx.pages.get_mut(t) { p.reset_to_default(sx); } }
    })
}
/// `Handle Apply` when `NeedsLanguageChangeConfirmation()`: the message only, one Ok button (recovered).
pub fn language_restart_dialog() -> Modal {
    Modal::new("", "Restart the game to show all language changes.", &[("Ok", BtnKind::Emphasis)], 0, 0, |_idx, _cx, _sx| {})
}
/// Unsaved-changes prompt when leaving Settings with unapplied edits (wording is an approximation, not original).
pub fn unsaved_dialog() -> Modal {
    Modal::new("Unapplied Changes", "You have changes that have not been applied.", &[("Apply", BtnKind::Emphasis), ("Discard", BtnKind::Critical), ("Cancel", BtnKind::Default)], 2, 2, |idx, cx, sx| {
        match idx {
            0 => sx.apply_button(),
            1 => { sx.discard_changes(); cx.pop(); }
            _ => {}
        }
    })
}
