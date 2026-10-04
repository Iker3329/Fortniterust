//! Footer of the Settings screen: `Back` (drawn by the screen) ... status text ... `Reset To Default` `Apply`.
//! Labels "Apply" / "Reset To Default" / "Back" are the original InputActionDataTable display names (recovered); the original
//! draws them in the UIManager bottom bar with keyboard shortcuts A / R, which are NOT implemented here (assumption: mouse,
//! Tab/arrows + Enter and the pad cover the same actions).
use super::{bridge, dialogs};
use crate::menu::theme::*;
use crate::menu::*;

/// Pixel rects of the footer buttons and status area given the screen's Back button and page rect. Pure for tests.
pub fn footer_layout(back: Rect, page: Rect, scale: f32) -> (Rect, Rect, Rect) {
    let u = |v: f32| v * scale;
    let gap = u(12.);
    let right = page.x + page.w;
    let avail = (right - (back.x + back.w) - 2. * gap).max(0.);
    // buttons shrink on narrow screens: apply 150, reset 220 units, but never more than 45% / 55% of the free width.
    let apply_w = u(150.).min(avail * 0.4);
    let reset_w = u(220.).min(avail * 0.6 - gap).max(0.);
    let apply = Rect::new(right - apply_w, back.y, apply_w, back.h);
    let reset = Rect::new(apply.x - gap - reset_w, back.y, reset_w, back.h);
    let status_x = back.x + back.w + gap;
    let status = Rect::new(status_x, back.y, (reset.x - gap - status_x).max(0.), back.h);
    (apply, reset, status)
}

/// Draw the footer; handles Apply / Reset To Default. Call from the settings screen after its own widgets.
pub fn draw(ui: &mut Ui, cx: &mut Ctx, back: Rect, page: Rect) {
    let (apply, reset, status) = footer_layout(back, page, ui.vp.scale);
    if !ui.is_inert() { let dt = ui.dt; ui.session.tick_status(dt); }
    let dirty = ui.session.is_dirty();
    let busy = ui.session.busy();
    let sel = *cx.settings_tab;
    let reset_prompt = cx.pages.get(sel).and_then(|p| p.reset_prompt());
    if reset.w > ui.u(60.) {
        if ui.button("reset", reset, "Reset To Default", BtnKind::Default, BtnSize::M, reset_prompt.is_some()).clicked {
            match reset_prompt {
                Some(Some(text)) => cx.push(Box::new(dialogs::reset_dialog(&text))),
                Some(None) => { if let Some(p) = cx.pages.get_mut(sel) { p.reset_to_default(ui.session); } }
                None => {}
            }
        }
    }
    if apply.w > ui.u(40.) {
        let kind = if dirty { BtnKind::Emphasis } else { BtnKind::Default };
        if ui.button("apply", apply, "Apply", kind, BtnSize::M, dirty && !busy).clicked { ui.session.apply_button(); }
    }
    // status line: pending-change count, or the result of the last apply
    let (text, color) = match ui.session.status() {
        Some(s) => (s.text, match s.kind { bridge::StatusKind::Ok => TEAL_BRIGHT, bridge::StatusKind::Info => TEXT_DIM, bridge::StatusKind::Warn => [1., 0.82, 0.3, 1.], bridge::StatusKind::Error => [1., 0.45, 0.4, 1.] }),
        None => if dirty { let n = ui.session.changed_ids().len().max(1); (format!("{n} change{} not applied", if n == 1 { "" } else { "s" }), [1., 0.82, 0.3, 1.]) } else { (String::new(), TEXT_DIM) },
    };
    // an unapplied-change count always wins over a stale "applied" message
    let (text, color) = if dirty && !busy && !matches!(ui.session.status().map(|s| s.kind), Some(bridge::StatusKind::Error) | Some(bridge::StatusKind::Warn)) { let n = ui.session.changed_ids().len().max(1); (format!("{n} change{} not applied", if n == 1 { "" } else { "s" }), [1., 0.82, 0.3, 1.]) } else { (text, color) };
    if status.w > ui.u(60.) && !text.is_empty() { ui.text(status, &text, TextStyle::new(FontId::Regular, 15., color), HAlign::Left); }
}

/// Leaving the screen (Back / X / Esc). Returns true when it is fine to pop; with unapplied edits it pushes the prompt instead.
pub fn may_leave(ui: &mut Ui, cx: &mut Ctx) -> bool {
    if !ui.session.is_dirty() { return true; }
    cx.push(Box::new(dialogs::unsaved_dialog()));
    false
}
