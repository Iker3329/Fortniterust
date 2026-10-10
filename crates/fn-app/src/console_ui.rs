//! CONSOLE HOOK: the overlay of the F2 command bar. A dark translucent panel across the top of the screen with the log, the input row (caret, ghost usage hint) and the
//! suggestion list, set in the 1.8 UI fonts through `fn_ui` (the same text path the HUD and the lobby console use). There is no 1.8 UMG widget for it: the
//! engine's console input box is a Slate widget of the engine (`SConsoleInputBox`) with no asset in the paks, so this is the port's own draw, not a recreation of a
//! shipped widget.
use fn_console::{LineKind, View};
use fn_ui::draw::RenderList;
use fn_ui::layout::Rect;
use fn_ui::scene::UiContext;
use fn_ui::screens::{ascent, atlas_px, put_text, Snap};
use fn_ui::text::FontId;

pub const LOG_LINES: usize = 12;
const MAX_SUGGESTIONS: usize = 9;

/// Shortens `s` with an ellipsis until it fits `max_w` pixels.
fn fit(uic: &UiContext, font: FontId, px: u32, s: &str, max_w: f32) -> String {
    if uic.fonts.measure(font, s, px as f32) <= max_w { return s.to_owned(); }
    let mut t: Vec<char> = s.chars().collect();
    while !t.is_empty() { t.pop(); let c: String = t.iter().collect::<String>() + "..."; if uic.fonts.measure(font, &c, px as f32) <= max_w { return c; } }
    String::new()
}

/// The bar as a draw list (pixels). `time` drives the caret blink.
pub fn draw(uic: &mut UiContext, view: &View, w: f32, h: f32, time: f32) -> RenderList {
    let mut out = RenderList::new();
    let scale = fn_ui::layout::dpi_scale(w, h);
    let px = atlas_px(18.0, scale);
    let line_h = (uic.fonts.line_height(FontId::Regular, px as f32) * 1.15).ceil();
    let pad = 10.0 * scale;
    let max_w = w - pad * 2.0;
    let shown: Vec<&fn_console::Suggestion> = view.items.iter().take(MAX_SUGGESTIONS).collect();
    let more = view.items.len() > MAX_SUGGESTIONS;
    let log_rows = view.log.len() + usize::from(view.scrolled > 0);
    let height = pad * 2.0 + line_h * (log_rows as f32 + 1.0) + if shown.is_empty() { 0.0 } else { line_h * (shown.len() as f32 + if more { 1.0 } else { 0.0 }) + pad * 0.5 };
    // dark translucent backing + a thin accent line under it
    out.fill(Rect::new(0.0, 0.0, w, height), [0.012, 0.016, 0.045, 0.93]);
    out.fill(Rect::new(0.0, height - 2.0, w, 2.0), [0.35, 0.55, 0.85, 1.0]);
    let colour = |k: LineKind| match k { LineKind::Input => [0.75, 0.85, 1.0, 1.0], LineKind::Ok => [0.55, 0.95, 0.55, 1.0], LineKind::Error => [1.0, 0.45, 0.45, 1.0], LineKind::Info => [0.85, 0.85, 0.85, 1.0] };
    let asc = ascent(uic, FontId::Regular, px);
    let shadow = Some(([1.0, 1.0], [0.0, 0.0, 0.0, 0.8]));
    let mut y = pad;
    for (k, line) in view.log {
        let t = fit(uic, FontId::Regular, px, line, max_w);
        put_text(uic, &mut out, FontId::Regular, px, pad, y + asc, &t, colour(*k), shadow, Snap::Pen);
        y += line_h;
    }
    if view.scrolled > 0 {
        put_text(uic, &mut out, FontId::Regular, px, pad, y + asc, &format!("-- {} newer lines below (PageDown) --", view.scrolled), [0.55, 0.6, 0.7, 1.0], None, Snap::Pen);
        y += line_h;
    }
    // input row
    let prompt = "> ";
    let pw = uic.fonts.measure(FontId::Bold, prompt, px as f32);
    put_text(uic, &mut out, FontId::Bold, px, pad, y + asc, prompt, [1.0, 0.85, 0.2, 1.0], shadow, Snap::Pen);
    put_text(uic, &mut out, FontId::Regular, px, pad + pw, y + asc, view.input, [1.0, 1.0, 1.0, 1.0], shadow, Snap::Pen);
    let before: String = view.input.chars().take(view.cursor).collect();
    let cx = pad + pw + uic.fonts.measure(FontId::Regular, &before, px as f32);
    if (time * 2.0).fract() < 0.6 { out.fill(Rect::new(cx.round(), y + 2.0, (2.0 * scale).max(1.0), line_h - 4.0), [1.0, 1.0, 1.0, 1.0]); }
    if !view.hint.is_empty() {
        let iw = uic.fonts.measure(FontId::Regular, view.input, px as f32);
        let gap = if view.input.ends_with(' ') || view.input.is_empty() { 0.0 } else { uic.fonts.measure(FontId::Regular, " ", px as f32) };
        put_text(uic, &mut out, FontId::Regular, px, pad + pw + iw + gap + 4.0, y + asc, view.hint, [0.55, 0.6, 0.7, 1.0], None, Snap::Pen);
    }
    y += line_h + pad * 0.5;
    // suggestions
    let label_w = shown.iter().map(|s| uic.fonts.measure(FontId::Bold, &s.label, px as f32)).fold(0.0f32, f32::max).min(w * 0.4);
    for (i, s) in shown.iter().enumerate() {
        let sel = view.selected == Some(i);
        if sel { out.fill(Rect::new(pad * 0.5, y, w - pad, line_h), [0.25, 0.45, 0.8, 0.55]); }
        let label = fit(uic, FontId::Bold, px, &s.label, w * 0.4);
        put_text(uic, &mut out, FontId::Bold, px, pad, y + asc + 1.0, &label, if sel { [1.0, 1.0, 1.0, 1.0] } else { [0.8, 0.9, 1.0, 1.0] }, None, Snap::Pen);
        let hx = pad + label_w + 18.0 * scale;
        let hint = fit(uic, FontId::Regular, px, &s.hint, (w - hx - pad).max(0.0));
        put_text(uic, &mut out, FontId::Regular, px, hx, y + asc + 1.0, &hint, [0.62, 0.68, 0.78, 1.0], None, Snap::Pen);
        y += line_h;
    }
    if more { put_text(uic, &mut out, FontId::Regular, px, pad, y + asc, &format!("... {} more (keep typing to narrow, Tab cycles)", view.items.len() - MAX_SUGGESTIONS), [0.55, 0.6, 0.7, 1.0], None, Snap::Pen); }
    out
}
