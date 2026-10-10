//! Interaction prompt (`InteractionIndicator_C`; Settings > HUD row "Interaction Info", tag `HUD.TopLeft.PlayerInfo` = `HudVisibility::interaction_info`):
//! the key cap + description that appears next to something the player can use.
//!
//! SOURCES (fn-inspect / old Kismet dump `InteractionIndicator.txt`, `BasicInteractionWidget.txt`, KeybindWidget bytecode):
//! * `InteractionIndicator_C` (parent `FortActorIndicatorWidget`): `bUseScreenSpacePosition = true`, `ScreenSpaceRelativeOffset = (65, -65)`; it binds
//!   `FortHUDContext.OnPlayerCanInteract` / `OnInteractUpdated` and `FortGlobalUIContext.OnKeybindsChanged`. `HandleInteractionChanged(interaction)`: Visible(4) when the
//!   interaction is valid else Collapsed; the Pickup / Basic / DefenderBeacon children start Collapsed; a `FortPickup` shows `HUD-PickupItemWidget` (item info card), a
//!   defender trap the beacon widget, anything else `ShowBasicInteractionWidget`: `BasicInteractionWidget.UpdateDescription(ContextText)`, the text under the keybind
//!   collapsed, the keybind hidden when `bDisplayTextOnly`, otherwise `SetForcedHoldKeybindStatus(RequiredDuration > 0)` and `SetShowProgressCountDown(RequiredDuration > 0
//!   && bShowCountDown)`. `UpdateKeybinds`: `KeybindWidget.SetBoundAction(Use)`.
//! * Tree: root `CommonBorder` (style `Border-KeyBase-Center-M`: Box brush `Keybase-Center-L` 64x64, margin L/R 0.5, tint (0,0,0,0.5)) -> overlay slot Padding 5 ->
//!   HorizontalBox [VerticalBox{ `KeybindWidget` (Brush Size Medium, BoundAction Use), HorizontalBox{ Spacer 8x1, `TextUnderKeybindText` (18) } } (Left/Top)] [Spacer 8x1]
//!   [Pickup | `BasicInteractionWidget` | Defender beacon] [Spacer 8x1].
//! * `BasicInteractionWidget`: SizeBox (MinDesiredWidth 50, MaxDesiredWidth 300; its MinDesiredHeight 128 is not enabled) -> `CommonTextBlock` style `TextStyle-Base-S-B`
//!   (NotoSans-Bold 18), `WrapTextAt 300`, Left / Center.
//! * `KeybindWidget` Medium: `UpdateFrameSize` bytecode tables (index = `EFortBrushSize` VeryVerySmall..VeryLarge = [24, 24, 24, 32, 64, 64]) -> Medium = 32: `TextSizeMinBox`
//!   32 x 32, key font NotoSans-Bold `Round(32 * 1.5 * 96 / 300)` = 15, `BorderFrame` padding `32 * 0.1` = 3.2 with the `Brush_M` frame (`Keybind-Frame-M`, 40x40, margin 0.25)
//!   at brush alpha 0.6 (as the quickbar keycaps).
//!
//! NOT RECOVERED / APPROXIMATE: the placement pivot (the native indicator canvas): the panel's CENTRE is put on `projected anchor + (65, -65) * dpi` (default anchor = screen
//! centre); the hold-to-use ring (`HoldKeybindImage` / `SetShowProgressCountDown`) and the pickup / defender variants are not drawn; the key text comes from the producer
//! (`GetKeyTextForAction(Use)`); text wrapping is greedy by measured advance (the native `SWrapBox` metrics are not recovered).
use crate::draw::RenderList;
use crate::layout::{dpi_scale, Rect};
use crate::scene::UiContext;
use crate::screens::reticle_state::InteractionPrompt;
use crate::screens::{art, ascent, atlas_px, line_height, put_text, text_width, Snap, WHITE};
use crate::text::FontId;

pub const OFFSET: (f32, f32) = (65., -65.); // ScreenSpaceRelativeOffset
pub const BORDER_PAD: f32 = 5.; // OverlaySlot_0 Padding (5,5,5,5)
pub const SPACER: f32 = 8.; // Spacer_0/1/4 Size (8, 1)
pub const KEYCAP_MIN: f32 = 32.; // Medium: TextSizeMinBox min width/height (UpdateFrameSize table, index 3)
pub const KEYCAP_FONT: f32 = 15.; // Round(32 * 1.5 * 96 / 300)
pub const KEYCAP_PAD: f32 = 3.2; // BorderFrame padding = 32 * 0.1
pub const FRAME_ALPHA: f32 = 0.6; // BorderFrame BrushColor (1,1,1,0.6)
pub const FRAME_MARGIN: f32 = 0.25; // Brush_M margin
pub const DESC_FONT: f32 = 18.; // TextStyle-Base-S-B
pub const DESC_MIN_W: f32 = 50.; // SizeBox_0 MinDesiredWidth (bOverride true)
pub const DESC_MAX_W: f32 = 300.; // SizeBox_0 MaxDesiredWidth / WrapTextAt
pub const BORDER_MARGIN: [f32; 4] = [0.5, 0., 0.5, 0.]; // Border-KeyBase-Center-M Background Margin (Left, Top, Right, Bottom)
pub const BORDER_TINT: [f32; 4] = [0., 0., 0., 0.5]; // Border-KeyBase-Center-M Background TintColor
pub const BORDER_TEXTURE: &str = "Keybase-Center-L.png";
pub const FRAME_TEXTURE: &str = "Keybind-Frame-M.png";
/// `KeybindWidget.UpdateFrameSize` size table indexed by `EFortBrushSize` (VeryVerySmall, VerySmall, Small, Medium, Large, VeryLarge).
pub const KEYCAP_SIZES: [f32; 6] = [24., 24., 24., 32., 64., 64.];

/// Greedy word wrap of `text` to `max_w` pixels with the measure function.
pub fn wrap(text: &str, max_w: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split_whitespace() {
            let cand = if cur.is_empty() { word.to_owned() } else { format!("{cur} {word}") };
            if !cur.is_empty() && measure(&cand) > max_w { lines.push(std::mem::take(&mut cur)); cur = word.to_owned(); } else { cur = cand; }
        }
        lines.push(cur);
    }
    lines
}

/// Pixel layout of the panel: (panel, keycap frame, description text origin x, first line top, line height, lines).
#[derive(Clone, Debug, PartialEq)]
pub struct Layout { pub scale: f32, pub panel: Rect, pub keycap: Rect, pub text_x: f32, pub text_top: f32, pub line_h: f32, pub lines: Vec<String> }

pub fn layout(ctx: &mut UiContext, w: f32, h: f32, p: &InteractionPrompt) -> Layout { layout_ex(ctx, w, h, p, 0.) }

/// CONTAINERS: [`layout`] of a hold keybind: the KeybindWidget is its `SizeBoxBackplate` (`backplate` Slate units square, `hold_ring::backplate_units`)
/// with the keycap centred in it (`OverlaySlot_2` Center / Center); `backplate` 0 = not a hold keybind (backplate Collapsed). `Layout::keycap` stays the
/// keycap frame; the backplate rect is [`backplate_rect`].
pub fn layout_ex(ctx: &mut UiContext, w: f32, h: f32, p: &InteractionPrompt, backplate: f32) -> Layout {
    let s = dpi_scale(w, h);
    let kpx = atlas_px(KEYCAP_FONT, s); let dpx = atlas_px(DESC_FONT, s);
    let key_w = if crate::screens::target_info::key_icon(&p.key).is_some() { 0. } else { text_width(ctx, FontId::Bold, &crate::screens::target_info::hud_key_text(&p.key), kpx) };
    let key_h = line_height(ctx, FontId::Bold, kpx);
    let cap = ((key_w + 2. * KEYCAP_PAD * s).max(KEYCAP_MIN * s), (key_h + 2. * KEYCAP_PAD * s).max(KEYCAP_MIN * s));
    let keycap = (cap.0.max(backplate * s), cap.1.max(backplate * s));
    let line_h = line_height(ctx, FontId::Bold, dpx);
    let measure = |t: &str| ctx.fonts.measure(FontId::Bold, t, dpx as f32);
    let lines = wrap(&p.text, DESC_MAX_W * s, &measure);
    let desc_w = lines.iter().map(|l| ctx.fonts.measure(FontId::Bold, l, dpx as f32)).fold(0f32, f32::max).max(DESC_MIN_W * s).min(DESC_MAX_W * s);
    let desc_h = line_h * lines.len() as f32;
    // HorizontalBox: [keycap column (top aligned)] [Spacer 8] [description (centred vertically)] [Spacer 8]
    let (row_w, row_h) = (keycap.0 + SPACER * s + desc_w + SPACER * s, keycap.1.max(desc_h));
    let (pw, ph) = (row_w + 2. * BORDER_PAD * s, row_h + 2. * BORDER_PAD * s);
    let anchor = p.screen.unwrap_or((w * 0.5, h * 0.5));
    let (cx, cy) = (anchor.0 + OFFSET.0 * s, anchor.1 + OFFSET.1 * s);
    let panel = Rect::new((cx - pw * 0.5).round(), (cy - ph * 0.5).round(), pw, ph);
    let col = Rect::new(panel.x + BORDER_PAD * s, panel.y + BORDER_PAD * s, keycap.0, keycap.1);
    // backplate 0: exactly the old keycap rect (the column is the keycap)
    let kc = if backplate > 0. { Rect::new((col.x + (col.w - cap.0) * 0.5).round(), (col.y + (col.h - cap.1) * 0.5).round(), cap.0, cap.1) } else { col };
    Layout { scale: s, panel, keycap: kc, text_x: col.x + col.w + SPACER * s, text_top: panel.y + BORDER_PAD * s + (row_h - desc_h) * 0.5, line_h, lines }
}

/// CONTAINERS: the hold backplate (pixels) of a [`layout_ex`] layout: `backplate` units square, centred on the keycap.
pub fn backplate_rect(l: &Layout, backplate: f32) -> Rect {
    let b = backplate * l.scale;
    Rect::new((l.keycap.x + l.keycap.w * 0.5 - b * 0.5).round(), (l.keycap.y + l.keycap.h * 0.5 - b * 0.5).round(), b.round(), b.round())
}

pub fn build(ctx: &mut UiContext, w: f32, h: f32, p: &InteractionPrompt, out: &mut RenderList) { build_hold(ctx, w, h, p, None, out) }

/// CONTAINERS: [`build`] of a hold interaction: `hold` = (`Percentage` 0..1, the ring drawer); the ring is drawn under the keycap (`Overlay_612` order:
/// backplate first, then the `Switcher`).
pub fn build_hold(ctx: &mut UiContext, w: f32, h: f32, p: &InteractionPrompt, hold: Option<(f32, &mut crate::screens::hold_ring::HoldRing)>, out: &mut RenderList) {
    let bp = if hold.is_some() { crate::screens::hold_ring::backplate_units(crate::screens::hold_ring::MEDIUM, false) } else { 0. };
    let l = layout_ex(ctx, w, h, p, bp); let s = l.scale;
    let bg = art(ctx, BORDER_TEXTURE);
    out.nine_slice(bg, l.panel, BORDER_MARGIN, s, BORDER_TINT);
    if let Some((pct, ring)) = hold { let r = backplate_rect(&l, bp); ring.draw(ctx, r, pct, out); }
    if let Some(icon) = crate::screens::target_info::key_icon(&p.key) { crate::screens::target_info::draw_key_icon(ctx, out, icon, l.keycap, s); } else {
    let key = crate::screens::target_info::hud_key_text(&p.key);
    let frame = art(ctx, FRAME_TEXTURE);
    out.nine_slice(frame, l.keycap, [FRAME_MARGIN; 4], s, [1., 1., 1., FRAME_ALPHA]);
    let kpx = atlas_px(KEYCAP_FONT, s);
    let kw = text_width(ctx, FontId::Bold, &key, kpx);
    let (lh, asc) = (line_height(ctx, FontId::Bold, kpx), ascent(ctx, FontId::Bold, kpx));
    put_text(ctx, out, FontId::Bold, kpx, l.keycap.x + (l.keycap.w - kw) * 0.5, (l.keycap.y + (l.keycap.h - lh) * 0.5 + asc).round(), &key, WHITE, None, Snap::PerGlyph);
    }
    let dpx = atlas_px(DESC_FONT, s); let dasc = ascent(ctx, FontId::Bold, dpx);
    for (i, line) in l.lines.iter().enumerate() {
        put_text(ctx, out, FontId::Bold, dpx, l.text_x, (l.text_top + l.line_h * i as f32 + dasc).round(), line, WHITE, None, Snap::PerGlyph);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::testutil::ctx;
    fn prompt(text: &str) -> InteractionPrompt { InteractionPrompt { key: "E".into(), text: text.into(), screen: Some((960., 540.)), pickup: None } }

    #[test] fn constants_are_the_decoded_widget_numbers() {
        assert_eq!((OFFSET, BORDER_PAD, SPACER, DESC_MIN_W, DESC_MAX_W, DESC_FONT), ((65., -65.), 5., 8., 50., 300., 18.));
        assert_eq!(KEYCAP_SIZES, [24., 24., 24., 32., 64., 64.]);
        assert_eq!(KEYCAP_SIZES[3], KEYCAP_MIN, "Medium");
        assert_eq!((KEYCAP_MIN * 1.5 * 96. / 300.).round(), KEYCAP_FONT, "KeybindWidget font size formula");
        assert!((KEYCAP_MIN * 0.1 - KEYCAP_PAD).abs() < 1e-6);
        assert_eq!(BORDER_TINT, [0., 0., 0., 0.5]);
        let root = crate::screens::testutil::old_root().join("reports").join("hud-bytecode").join("InteractionIndicator.txt");
        if let Ok(t) = std::fs::read_to_string(&root) { assert!(t.contains("prop ScreenSpaceRelativeOffset = [65,-65]") && t.contains("prop WrapTextAt = 300") && t.contains("Border-KeyBase-Center-M_C")); }
    }
    #[test] fn wrap_is_greedy_and_keeps_explicit_newlines() {
        let m = |s: &str| s.len() as f32 * 10.;
        assert_eq!(wrap("aa bb cc", 50., &m), vec!["aa bb".to_owned(), "cc".to_owned()]);
        assert_eq!(wrap("aa\nbb", 500., &m), vec!["aa".to_owned(), "bb".to_owned()]);
        assert_eq!(wrap("loooooooong", 30., &m), vec!["loooooooong".to_owned()], "a single long word is never split");
        assert_eq!(wrap("", 50., &m), vec![String::new()]);
    }
    #[test] fn panel_is_centred_on_the_anchor_plus_the_screen_space_offset() {
        let Some(mut c) = ctx() else { return };
        let l = layout(&mut c, 1920., 1080., &prompt("Open"));
        let (cx, cy) = (l.panel.x + l.panel.w * 0.5, l.panel.y + l.panel.h * 0.5);
        assert!((cx - (960. + 65.)).abs() <= 0.5 && (cy - (540. - 65.)).abs() <= 0.5, "{cx} {cy}");
        assert!(l.keycap.w >= 32. && l.keycap.h >= 32., "keycap min size 32 (Medium)");
        assert_eq!(l.keycap.x, l.panel.x + 5.); assert_eq!(l.keycap.y, l.panel.y + 5.);
        assert!(l.text_x >= l.keycap.x + l.keycap.w + 8. - 1e-3);
        assert_eq!(l.lines, vec!["Open".to_owned()]);
        // long text wraps at 300 units and the panel grows; the description is never wider than 300
        let long = layout(&mut c, 1920., 1080., &prompt("Hold to open the supply chest and take everything that is inside of it right now"));
        assert!(long.lines.len() >= 2 && long.panel.h > l.panel.h);
        let dpx = atlas_px(DESC_FONT, 1.); assert!(long.lines.iter().all(|t| c.fonts.measure(FontId::Bold, t, dpx as f32) <= 300. + 1e-3));
        // no screen anchor: the screen centre
        let mut p = prompt("Use"); p.screen = None; let d = layout(&mut c, 1280., 720., &p); assert!((d.panel.x + d.panel.w * 0.5 - (640. + 65. * 0.666)).abs() <= 0.6);
    }
    #[test] fn build_draws_background_keycap_frame_and_text() {
        let Some(mut c) = ctx() else { return };
        let mut out = RenderList::new(); build(&mut c, 1920., 1080., &prompt("Open"), &mut out);
        // 9 + 9 nine-slice quads + 'E' + 'Open' glyphs (some textures may be absent from a sparse asset source: only demand the text)
        assert!(out.quad_count() >= 1 + 4, "{}", out.quad_count());
    }
}
