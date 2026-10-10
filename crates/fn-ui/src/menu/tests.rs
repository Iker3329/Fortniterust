//! Toolkit tests: pure helpers, input translation, interaction semantics (headless: no GL, real NotoSans fonts).
use super::pause::*;
use super::theme::WHITE;
use super::widgets::*;
use super::*;
use std::{cell::{Cell, RefCell}, rc::Rc};

use crate::input::{vk, XI_A, XI_DPAD_DOWN};
fn fonts() -> Fonts { crate::testutil::fonts() }

struct H { m: MenuSystem, w: f32, h: f32 }
impl H {
    fn new(w: f32, h: f32) -> Self { let mut m = MenuSystem::new(fonts()); m.fade_time = 0.; m.open(); let mut s = H { m, w, h }; s.frame(); s.frame(); s }
    fn frame(&mut self) -> Vec<Cmd> { self.m.frame(self.w, self.h, 1. / 60.).cmds.clone() }
    fn mv(&mut self, x: f32, y: f32) { self.m.event(Event::MouseMove(x, y)); self.frame(); }
    fn down(&mut self, x: f32, y: f32) { self.m.event(Event::MouseDown(MouseButton::Left, x, y)); self.frame(); }
    fn up(&mut self, x: f32, y: f32) { self.m.event(Event::MouseUp(MouseButton::Left, x, y)); self.frame(); }
    fn click(&mut self, x: f32, y: f32) { self.mv(x, y); self.down(x, y); self.up(x, y); self.frame(); }
    fn key(&mut self, k: NavKey) { self.m.event(Event::Nav(k)); self.frame(); }
    fn resize(&mut self, w: f32, h: f32) { self.w = w; self.h = h; self.frame(); self.frame(); }
}
fn center(r: Rect) -> (f32, f32) { (r.x + r.w * 0.5, r.y + r.h * 0.5) }
fn buttons(cmds: &[Cmd]) -> Vec<(BtnKind, BtnSize, Vis, Rect)> { cmds.iter().filter_map(|c| if let Cmd::Brush { brush: BrushId::Button(k, s, v), rect, .. } = c { Some((*k, *s, *v, *rect)) } else { None }).collect() }
/// Pixel rect of a MainMenu entry (match context, mouse layout).
fn entry(h: &H, widget: &str) -> Rect { mainmenu::entry_rect(h.w, h.h, mainmenu::MenuContext::Match, widget).unwrap_or_else(|| panic!("{widget} is in the match menu")) }

const SIZES: [(f32, f32); 6] = [(1920., 1080.), (2560., 1080.), (1024., 768.), (640., 480.), (800., 1280.), (3440., 1440.)];

// ---- pure helpers --------------------------------------------------------------------------------------------------
#[test] fn ids_are_stable_and_distinct() {
    assert_eq!(id_of("resume"), id_of("resume")); assert_ne!(id_of("resume"), id_of("settings"));
    assert_ne!(id_with(1, 2), id_with(2, 1)); assert_ne!(id_with(id_of("a"), id_of("x")), id_with(id_of("b"), id_of("x")));
}
#[test] fn snap_value_clamps_snaps_and_survives_garbage() {
    assert_eq!(snap_value(0.26, 0., 1., 0.25), 0.25); assert_eq!(snap_value(5., 0., 1., 0.25), 1.); assert_eq!(snap_value(-5., 0., 1., 0.25), 0.);
    assert_eq!(snap_value(f32::NAN, 2., 9., 1.), 2.); assert_eq!(snap_value(f32::INFINITY, 2., 9., 1.), 2.);
    assert_eq!(snap_value(3., 5., 5., 1.), 5., "degenerate range collapses to min"); assert_eq!(snap_value(3., 6., 5., 1.), 6.);
    assert_eq!(snap_value(0.333, 0., 1., 0.), 0.333, "step 0 = continuous");
    assert_eq!(snap_value(7.4, 5., 10., 2.), 7., "snaps relative to min");
}
#[test] fn value_format_has_no_negative_zero() { assert_eq!(format_value(-0., 1, ""), "0.0"); assert_eq!(format_value(0.5, 2, ""), "0.50"); assert_eq!(format_value(75., 0, "%"), "75%"); }
#[test] fn tab_widths_shrink_only_when_needed() {
    assert_eq!(tab_widths(&[100., 100.], 500.), vec![100., 100.]);
    let w = tab_widths(&[300., 300., 400.], 500.); assert!((w.iter().sum::<f32>() - 500.).abs() < 1e-3); assert!((w[2] / w[0] - 4. / 3.).abs() < 1e-4);
}
#[test] fn fit_text_never_overflows_and_marks_truncation() {
    let f = fonts();
    for px in [8., 12., 18., 30.] {
        let s = "A rather long label that will certainly not fit in a narrow column";
        let full = f.measure(FontId::Bold, s, px);
        assert_eq!(fit_text(&f, FontId::Bold, s, px, full + 1.), s);
        for frac in [0.9, 0.5, 0.2, 0.1] {
            let w = full * frac; let t = fit_text(&f, FontId::Bold, s, px, w);
            assert!(f.measure(FontId::Bold, &t, px) <= w + 0.01, "{px} {frac}: {t:?}");
            if !t.is_empty() { assert!(t.ends_with("..."), "{t:?}"); }
        }
        assert_eq!(fit_text(&f, FontId::Bold, s, px, 1.), "", "not even the dots fit");
    }
    assert_eq!(sanitize("a\u{e9}b\n"), "a?b ", "non-ASCII is drawn as ? so measuring matches the atlas");
}
#[test] fn wrap_text_respects_width_and_splits_long_words() {
    let f = fonts();
    for w in [60., 120., 300.] {
        let lines = wrap_text(&f, FontId::Regular, "Are you sure you want to exit to the desktop? Supercalifragilisticexpialidocious", 16., w);
        assert!(lines.len() >= 2);
        for l in &lines { assert!(f.measure(FontId::Regular, l, 16.) <= w + 0.01, "{w}: {l:?}"); }
        assert_eq!(lines.join("").replace(' ', ""), "Areyousureyouwanttoexittothedesktop?Supercalifragilisticexpialidocious");
    }
    assert_eq!(wrap_text(&f, FontId::Regular, "a\nb", 16., 200.), vec!["a", "b"]);
    assert_eq!(wrap_text(&f, FontId::Regular, "", 16., 200.), vec![""]);
}

// ---- input translation --------------------------------------------------------------------------------------------
// DROPPED (inherently Win32): `win32_mouse_coordinates_are_signed` parsed WM_* lParam/wParam words (translate_win32); the window layer that owns
// those messages now builds `Event`s itself. The keyboard half is covered by `keys_map_to_nav_and_raw` over the platform independent
// `input::translate_key_down`.
#[test] fn keys_map_to_nav_and_raw() {
    use crate::input::translate_key_down;
    let k = |code: u16, repeat: bool, shift: bool, ctrl: bool| translate_key_down(code, repeat, shift, ctrl);
    assert_eq!(k(vk::ESCAPE, false, false, false), vec![Event::Nav(NavKey::Back), Event::RawKey(vk::ESCAPE)]);
    assert_eq!(k(vk::RETURN, true, false, false), vec![], "held Enter must not re-click and is not a fresh key press");
    assert_eq!(k(vk::DOWN, true, false, false), vec![Event::Nav(NavKey::Down)], "movement keys auto-repeat");
    assert!(k(vk::TAB, false, false, false).contains(&Event::Nav(NavKey::Down)));
    assert!(k(vk::TAB, false, true, false).contains(&Event::Nav(NavKey::Up)));
    assert!(k(vk::TAB, false, false, true).contains(&Event::Nav(NavKey::TabNext)));
    assert!(k(vk::TAB, false, true, true).contains(&Event::Nav(NavKey::TabPrev)));
    assert!(k(vk::NEXT, false, false, true).contains(&Event::Nav(NavKey::TabNext)));
    assert!(k(vk::NEXT, false, false, false).contains(&Event::Nav(NavKey::PageDown)));
    assert!(k(vk::SPACE, false, false, false).contains(&Event::Nav(NavKey::Activate)));
    assert_eq!(k(0x41, false, false, false), vec![Event::RawKey(0x41)], "letters only produce a raw key (keybind capture)");
}#[test] fn key_names_are_readable() {
    assert_eq!(key_name(0x57), "W"); assert_eq!(key_name(0x31), "1"); assert_eq!(key_name(0x70), "F1"); assert_eq!(key_name(0x7B), "F12");
    assert_eq!(key_name(0x20), "Space Bar"); assert_eq!(key_name(0x01), "Left Mouse Button"); assert_eq!(key_name(0xA0 + 7), "Key 167");
    assert_eq!(key_name(0x60), "Num 0");
}
#[test] fn gamepad_edges_and_repeat() {
    let mut p = PadState::default();
    assert_eq!(p.step(XI_A, 0, 0, 0.016), vec![PadButton::A]);
    assert!(p.step(XI_A, 0, 0, 0.016).is_empty(), "held button fires once");
    assert_eq!(p.step(0, 0, 0, 0.016), vec![]);
    assert_eq!(p.step(XI_DPAD_DOWN, 0, 0, 0.0), vec![PadButton::DPadDown]);
    assert!(p.step(XI_DPAD_DOWN, 0, 0, 0.3).is_empty(), "before repeat delay");
    assert_eq!(p.step(XI_DPAD_DOWN, 0, 0, 0.15), vec![PadButton::DPadDown], "first repeat after 0.4 s");
    assert!(p.step(XI_DPAD_DOWN, 0, 0, 0.05).is_empty()); assert_eq!(p.step(XI_DPAD_DOWN, 0, 0, 0.1), vec![PadButton::DPadDown]);
    assert!(p.step(0, 0, 0, 0.016).is_empty()); assert_eq!(p.step(XI_DPAD_DOWN, 0, 0, 0.0), vec![PadButton::DPadDown], "release resets repeat");
    let mut s = PadState::default();
    assert_eq!(s.step(0, 0, 20000, 0.016), vec![PadButton::DPadUp], "left stick acts as D-pad");
    assert!(s.step(0, 0, 5000, 0.016).is_empty(), "inside dead zone");
    assert_eq!(pad_to_nav(PadButton::A), NavKey::Activate); assert_eq!(pad_to_nav(PadButton::B), NavKey::Back); assert_eq!(pad_to_nav(PadButton::RB), NavKey::TabNext);
}

// ---- layout (pure) --------------------------------------------------------------------------------------------------
// SPEC CHANGE (pause menu rebuilt from the 1.10 MainMenu evidence): the old test asserted three centred L buttons
// (Resume/Settings/Exit Game). The recovered menu is right-hugging with two columns; the invariants that survive are
// "everything on screen" and "rows are disjoint", asserted here for the match context at every test size.
#[test] fn pause_menu_entries_stay_on_screen_and_disjoint() {
    use mainmenu::*;
    for (w, h) in SIZES.into_iter().chain([(300., 200.), (200., 600.)]) {
        let vp = Viewport::new(w, h); let (vw, vh) = vp.slate_size();
        for wide in [false, true] {
            let l = layout(vw, vh, vp.scale, MenuContext::Match, wide);
            assert_eq!(l.items.len(), 7, "4 text + 3 icon entries in a match");
            for (i, r) in &l.items { assert!(r.x >= -0.01 && r.y >= -0.01 && r.x + r.w <= w + 0.01 && r.y + r.h <= h + 0.01, "{w}x{h} wide={wide} {} {r:?}", ENTRIES[*i].widget); }
            for (a, (_, ra)) in l.items.iter().enumerate() { for (b, (_, rb)) in l.items.iter().enumerate() {
                if a < b { assert!(ra.x + ra.w <= rb.x + 0.01 || rb.x + rb.w <= ra.x + 0.01 || ra.y + ra.h <= rb.y + 0.01 || rb.y + rb.h <= ra.y + 0.01, "{w}x{h} entries {a}/{b} overlap"); }
            } }
            assert!(l.help.x + l.help.w <= w + 0.01, "{w}x{h} help box inside the client");
        }
    }
}
#[test] fn dialog_and_settings_layout_fit_every_aspect() {
    for (w, h) in SIZES.into_iter().chain([(300., 200.)]) {
        let vp = Viewport::new(w, h); let (vw, vh) = vp.slate_size();
        for body in [20., 60., 400.] {
            let (panel, bodyr, c, x) = dialog_layout(vw, vh, vp.scale, body);
            for r in [panel, c, x] { assert!(r.x >= -0.01 && r.y >= -0.01 && r.x + r.w <= w + 0.01 && r.y + r.h <= h + 0.01, "{w}x{h} body {body} {r:?}"); }
            assert!(c.x + c.w <= x.x + 0.01, "confirm left of cancel"); assert!(bodyr.x >= panel.x);
        }
        let (title, close, tabs, page, back) = settings_layout(vw, vh, vp.scale);
        for r in [title, close, tabs, page, back] { assert!(r.x >= -0.01 && r.y >= -0.01 && r.x + r.w <= w + 0.01 && r.y + r.h <= h + 0.01, "{w}x{h} {r:?}"); }
        assert!(tabs.y >= title.y + title.h - 0.01 && page.y >= tabs.y + tabs.h - 0.01 && back.y >= page.y + page.h - 0.01, "stacked top to bottom {w}x{h}");
    }
}

// ---- manager behaviour -----------------------------------------------------------------------------------------------
#[test] fn open_close_requests_and_rapid_toggle() {
    let mut h = H::new(1280., 720.);
    assert!(h.m.is_open()); assert_eq!(h.m.stack_names(), ["pause"]);
    h.m.open(); assert_eq!(h.m.depth(), 1, "double open must not stack roots");
    h.m.close(); h.m.close(); assert!(!h.m.is_open());
    assert_eq!(h.m.take_requests(), vec![HostRequest::Closed], "Closed is emitted once");
    for _ in 0..50 { h.m.toggle(); }
    assert!(!h.m.is_open(), "even number of toggles ends closed"); h.m.toggle(); assert!(h.m.is_open());
    assert_eq!(h.m.take_requests().iter().filter(|r| **r == HostRequest::Closed).count(), 25);
    assert_eq!(h.m.depth(), 1);
}
#[test] fn events_while_closed_are_dropped_and_do_not_leak_into_next_open() {
    let mut h = H::new(1280., 720.);
    h.m.close(); h.m.take_requests();
    h.m.event(Event::MouseDown(MouseButton::Left, 100., 100.)); h.m.event(Event::Nav(NavKey::Activate));
    assert_eq!(h.m.state().pending_events(), 0);
    assert!(h.frame().is_empty() || !h.m.is_open());
    h.m.open(); let cmds = h.frame(); assert!(!cmds.is_empty());
    assert!(h.m.take_requests().is_empty(), "stale Activate must not click Resume");
}
#[test] fn fade_out_is_visual_only() {
    let mut m = MenuSystem::new(fonts()); m.open();
    for _ in 0..20 { m.frame(1280., 720., 0.016); } assert!((m.visibility() - 1.).abs() < 1e-6, "{}", m.visibility());
    m.close();
    let n = m.frame(1280., 720., 0.016).cmds.len(); assert!(n > 0, "still fading out");
    for _ in 0..20 { m.frame(1280., 720., 0.016); } assert_eq!(m.frame(1280., 720., 0.016).cmds.len(), 0, "fully faded -> nothing drawn");
    m.event(Event::MouseDown(MouseButton::Left, 5., 5.)); assert_eq!(m.state().pending_events(), 0, "no input while fading out");
}
// SPEC CHANGE: there is no "Resume" button in 1.10 (no such string/widget); the menu closes with Escape / pad Start/B.
// The Resume-based click/hover/press tests below were re-pointed at the real entries (Settings = enabled icon button,
// Leave Match = enabled text button, Feedback = disabled text button) and the Esc toggle.
#[test] fn escape_closes_the_menu_and_requests_recapture() {
    for (w, h_) in SIZES {
        let mut h = H::new(w, h_); h.key(NavKey::Back);
        assert!(!h.m.is_open(), "{w}x{h_}"); assert_eq!(h.m.take_requests(), vec![HostRequest::Closed]);
    }
}
fn procs(cmds: &[Cmd]) -> Vec<(super::outline::ProcStyle, Vis, Rect)> { cmds.iter().filter_map(|c| if let Cmd::Procedural { style, vis, rect, .. } = c { Some((*style, *vis, *rect)) } else { None }).collect() }
/// Vis of the procedural box drawn at `widget`'s rectangle.
fn vis_at(h: &mut H, widget: &str) -> Vis {
    let r = entry(h, widget); let cmds = h.frame();
    procs(&cmds).into_iter().find(|(_, _, pr)| (pr.x - r.x).abs() < 0.5 && (pr.y - r.y).abs() < 0.5).unwrap_or_else(|| panic!("{widget} drawn")).1
}
#[test] fn hover_pressed_and_normal_looks() {
    let mut h = H::new(1280., 720.);
    assert_eq!(vis_at(&mut h, "SettingsButton"), Vis::Normal); assert_eq!(vis_at(&mut h, "Feedback"), Vis::Disabled, "Feedback is not available in this build");
    let (x, y) = center(entry(&h, "SettingsButton")); h.mv(x, y);
    assert_eq!(vis_at(&mut h, "SettingsButton"), Vis::Hover); assert_eq!(vis_at(&mut h, "LeaveButton"), Vis::Normal);
    h.down(x, y); assert_eq!(vis_at(&mut h, "SettingsButton"), Vis::Pressed);
    h.mv(x, 2.); assert_eq!(vis_at(&mut h, "SettingsButton"), Vis::Normal, "pressed look only while the pointer is over the button");
    h.mv(x, y); assert_eq!(vis_at(&mut h, "SettingsButton"), Vis::Pressed, "returns to pressed when dragged back");
    h.up(x, y); assert_eq!(h.m.depth(), 2, "Settings opened");
}
#[test] fn press_inside_release_outside_does_not_click() {
    let mut h = H::new(1280., 720.); let (x, y) = center(entry(&h, "LeaveButton"));
    h.mv(x, y); h.down(x, y); h.up(2., 2.); h.frame();
    assert_eq!(h.m.stack_names(), ["pause"]);
    // and press outside, release inside must not click either
    h.down(2., 2.); h.up(x, y); h.frame(); assert_eq!(h.m.stack_names(), ["pause"]);
}
#[test] fn press_and_release_in_one_frame_still_clicks() {
    let mut h = H::new(1280., 720.); let (x, y) = center(entry(&h, "LeaveButton"));
    h.m.event(Event::MouseMove(x, y)); h.m.event(Event::MouseDown(MouseButton::Left, x, y)); h.m.event(Event::MouseUp(MouseButton::Left, x, y)); h.frame();
    assert_eq!(h.m.stack_names(), ["pause", "confirm"], "fast click between two frames");
}
#[test] fn right_and_middle_clicks_do_nothing() {
    let mut h = H::new(1280., 720.); let (x, y) = center(entry(&h, "LeaveButton"));
    h.m.event(Event::MouseMove(x, y)); h.frame(); h.m.event(Event::MouseDown(MouseButton::Right, x, y)); h.m.event(Event::MouseUp(MouseButton::Right, x, y)); h.m.event(Event::MouseDown(MouseButton::Middle, x, y)); h.m.event(Event::MouseUp(MouseButton::Middle, x, y)); h.frame();
    assert_eq!(h.m.stack_names(), ["pause"]);
}
/// Widget names of the focused look (hover-looking) entries.
fn focused_entries(h: &mut H) -> Vec<&'static str> {
    let cmds = h.frame(); let p = procs(&cmds);
    mainmenu::ENTRIES.iter().filter(|e| e.visible(mainmenu::MenuContext::Match)).filter(|e| {
        let r = entry(h, e.widget); p.iter().any(|(_, v, pr)| (pr.x - r.x).abs() < 0.5 && (pr.y - r.y).abs() < 0.5 && *v == Vis::Hover)
    }).map(|e| e.widget).collect()
}
#[test] fn keyboard_navigation_wraps_over_all_seven_entries_including_disabled_ones() {
    let mut h = H::new(1280., 720.);
    assert!(focused_entries(&mut h).is_empty(), "focus ring hidden until a key is pressed");
    h.key(NavKey::Down);
    // Settings is the initial focus; the focus order is text column then icon column: Settings -> Legal (disabled: shown as Disabled but focus-framed)
    let order = ["Feedback", "AthenaPartyPrivacyButton", "CodeOfConduct", "LeaveButton", "SettingsButton", "Legal", "LogoutButton"];
    let mut seen = Vec::new();
    for _ in 0..7 { let f = h.m.state().focused_id(); seen.push(f.expect("one focus")); h.key(NavKey::Down); }
    let mut uniq = seen.clone(); uniq.sort(); uniq.dedup(); assert_eq!(uniq.len(), 7, "7 distinct stops in {order:?}");
    assert_eq!(h.m.state().focused_id(), Some(seen[0]), "7 steps wrap");
    h.key(NavKey::Up); assert_eq!(h.m.state().focused_id(), Some(seen[6]), "Up wraps backwards");
    // mouse movement hides the focus ring
    h.mv(3., 3.); assert!(focused_entries(&mut h).is_empty());
}
#[test] fn left_right_hop_between_columns() {
    let mut h = H::new(1280., 720.);
    let id = |h: &H, w: &str| { let _ = h; mainmenu::entry_index(w) };
    assert!(id(&h, "SettingsButton").is_some());
    h.key(NavKey::Left); // initial focus = Settings (icon column) -> hops to the text column
    let f = h.m.state().focused_id(); h.key(NavKey::Activate);
    assert_eq!(h.m.stack_names(), ["pause"], "the left column entry the focus landed on is Feedback (disabled): Enter does nothing, still no dialog/settings");
    h.key(NavKey::Right); assert_ne!(h.m.state().focused_id(), f, "Right hops back to the icon column");
}
#[test] fn enter_activates_settings_first_and_escape_or_pad_back_close() {
    let mut h = H::new(1280., 720.); h.key(NavKey::Activate); assert_eq!(h.m.stack_names(), ["pause", "settings"], "initial focus is the Settings icon (ASSUMED first stop)");
    let mut h = H::new(1280., 720.); h.key(NavKey::Back); assert!(!h.m.is_open()); assert_eq!(h.m.take_requests(), vec![HostRequest::Closed]);
    let mut h = H::new(1280., 720.); h.m.event(Event::Pad(PadButton::B)); h.frame(); assert!(!h.m.is_open(), "controller B backs out");
    let mut h = H::new(1280., 720.); h.m.event(Event::Pad(PadButton::Start)); h.frame(); assert!(!h.m.is_open(), "controller Start (MainMenu action) closes");
    let mut h = H::new(1280., 720.); h.m.event(Event::Pad(PadButton::A)); h.frame(); assert_eq!(h.m.stack_names(), ["pause", "settings"], "controller A activates the focused Settings icon");
}
fn open_confirm(h: &mut H) { let (x, y) = center(entry(h, "LeaveButton")); h.click(x, y); }
#[test] fn leave_match_requires_confirmation_and_leave_is_default_like_the_original() {
    let mut h = H::new(1280., 720.); open_confirm(&mut h);
    assert_eq!(h.m.stack_names(), ["pause", "confirm"]); assert!(h.m.take_requests().is_empty(), "no quit before confirming");
    // CenterFirstButton: the first button of the ButtonBox (Leave) is focused, Cancel second.
    h.key(NavKey::Right); h.key(NavKey::Activate); assert_eq!(h.m.stack_names(), ["pause"], "Right moves to Cancel, Enter cancels"); assert!(h.m.is_open());
    open_confirm(&mut h); h.key(NavKey::Back); assert_eq!(h.m.stack_names(), ["pause"], "Esc = Cancel/GenericBack cancels the dialog (not the menu)");
    open_confirm(&mut h); h.key(NavKey::Activate);
    assert_eq!(h.m.take_requests(), vec![HostRequest::QuitToDesktop], "Leave quits to desktop in this standalone runtime"); assert!(!h.m.is_open());
}
#[test] fn confirm_dialog_via_mouse_and_modal_blocking() {
    for (w, h_) in SIZES {
        let mut h = H::new(w, h_); open_confirm(&mut h);
        let vp = Viewport::new(w, h_); let (vw, vh) = vp.slate_size();
        let (panel, _, _, yes, no) = mainmenu::confirm_layout(vw, vh, vp.scale, 48.);
        // a click on where Settings is (outside the panel or on it) must not reach the paused screen
        let r0 = entry(&h, "SettingsButton"); let (x0, y0) = center(r0);
        if !rect_contains(panel, x0, y0) { h.click(x0, y0); assert!(h.m.is_open() && h.m.depth() == 2, "{w}x{h_}: modal swallows the click"); }
        let (cx, cy) = center(no); h.click(cx, cy); assert_eq!(h.m.depth(), 1, "{w}x{h_} cancel");
        open_confirm(&mut h); let (yx, yy) = center(yes); h.click(yx, yy);
        assert_eq!(h.m.take_requests(), vec![HostRequest::QuitToDesktop], "{w}x{h_}"); assert!(!h.m.is_open());
    }
}
#[test] fn dialog_never_draws_text_outside_the_screen() {
    for (w, h_) in SIZES.into_iter().chain([(300., 200.)]) {
        let mut h = H::new(w, h_); open_confirm(&mut h);
        let f = fonts();
        for c in h.frame() { if let Cmd::Text { x, top, box_h, text, font, px, .. } = c {
            let tw = f.measure(font, &text, px as f32);
            assert!(x >= -0.5 && x + tw <= w + 0.5 && top >= -0.5 && top + box_h <= h_ + 0.5, "{w}x{h_}: {text:?} x {x} w {tw} top {top}");
        } }
    }
}
#[test] fn resize_while_open_rebuilds_hit_regions() {
    let mut h = H::new(1920., 1080.);
    h.resize(640., 480.);
    let r = entry(&h, "Feedback"); assert!((r.w - mainmenu::LEFT_W * 0.5).abs() < 1., "layout follows the new DPI scale (480 px high: the curve holds 0.5 below 540)");
    let (x, y) = center(entry(&h, "SettingsButton")); h.click(x, y); assert_eq!(h.m.stack_names(), ["pause", "settings"], "click lands on the NEW Settings rect");
    h.resize(800., 1280.); h.resize(3440., 1440.); h.key(NavKey::Back); assert_eq!(h.m.stack_names(), ["pause"]);
    h.resize(1., 1.); h.resize(2., 3.); let _ = h.frame(); assert!(h.m.is_open(), "degenerate sizes must not panic");
}
#[test] fn focus_lost_clears_drag_listening_and_popups() {
    let mut h = H::new(1280., 720.); let (x, y) = center(entry(&h, "SettingsButton"));
    h.mv(x, y); h.down(x, y); assert!(h.m.state().active_id().is_some());
    h.m.event(Event::FocusLost); h.frame();
    assert!(h.m.state().active_id().is_none(), "alt-tab mid-press cancels the press");
    h.up(x, y); assert_eq!(h.m.depth(), 1, "the stray release after refocus must not click");
}

// ---- widgets through a custom screen ------------------------------------------------------------------------------------
struct TestScreen { f: Box<dyn FnMut(&mut Ui, &mut Ctx)>, opaque: bool }
impl Screen for TestScreen { fn name(&self) -> &str { "test" } fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) { (self.f)(ui, cx); } fn opaque(&self) -> bool { self.opaque } }
fn test_menu(w: f32, h: f32, f: impl FnMut(&mut Ui, &mut Ctx) + 'static) -> H {
    let mut s = H::new(w, h); s.m.push(Box::new(TestScreen { f: Box::new(f), opaque: true })); s.frame(); s.frame(); s
}
#[test] fn disabled_widgets_block_clicks_to_widgets_underneath() {
    let hits = Rc::new(RefCell::new(Vec::<&'static str>::new())); let hh = hits.clone();
    let mut h = test_menu(1000., 700., move |ui, _| {
        let under = Rect::new(100., 100., 300., 80.); let over = Rect::new(250., 100., 300., 80.);
        if ui.button("under", under, "under", BtnKind::Default, BtnSize::M, true).clicked { hh.borrow_mut().push("under"); }
        if ui.button("over", over, "over", BtnKind::Default, BtnSize::M, false).clicked { hh.borrow_mut().push("over"); }
    });
    h.click(300., 140.); assert!(hits.borrow().is_empty(), "overlap point: topmost is the DISABLED widget; nothing fires");
    h.click(150., 140.); assert_eq!(*hits.borrow(), vec!["under"], "non-overlapped part of the lower widget still works");
}
#[test] fn topmost_enabled_widget_wins_on_overlap() {
    let hits = Rc::new(RefCell::new(Vec::<&'static str>::new())); let hh = hits.clone();
    let mut h = test_menu(1000., 700., move |ui, _| {
        if ui.button("a", Rect::new(100., 100., 300., 80.), "a", BtnKind::Default, BtnSize::M, true).clicked { hh.borrow_mut().push("a"); }
        if ui.button("b", Rect::new(250., 100., 300., 80.), "b", BtnKind::Default, BtnSize::M, true).clicked { hh.borrow_mut().push("b"); }
    });
    h.mv(300., 140.); let b = buttons(&h.frame()); assert_eq!((b[0].2, b[1].2), (Vis::Normal, Vis::Hover), "only the topmost is hot");
    h.click(300., 140.); assert_eq!(*hits.borrow(), vec!["b"]);
}
#[test] fn disabled_button_looks_disabled_and_is_not_focusable() {
    let clicks = Rc::new(Cell::new(0)); let c2 = clicks.clone();
    let mut h = test_menu(1000., 700., move |ui, _| {
        if ui.button("off", Rect::new(100., 100., 300., 60.), "Disabled", BtnKind::Emphasis, BtnSize::M, false).clicked { c2.set(c2.get() + 1); }
        if ui.button("on", Rect::new(100., 200., 300., 60.), "Enabled", BtnKind::Default, BtnSize::M, true).clicked { c2.set(c2.get() + 100); }
    });
    assert_eq!(buttons(&h.frame())[0].2, Vis::Disabled);
    h.click(200., 130.); h.key(NavKey::Activate); assert_eq!(clicks.get(), 100, "Enter hits the only focusable widget, never the disabled one");
    h.mv(200., 130.); assert_eq!(buttons(&h.frame())[0].2, Vis::Disabled, "hover does not change a disabled button");
}
#[test] fn slider_drag_keys_clamp_snap_and_garbage() {
    let v = Rc::new(Cell::new(50f32)); let v2 = v.clone(); let en = Rc::new(Cell::new(true)); let en2 = en.clone();
    let mut h = test_menu(1280., 720., move |ui, _| {
        let mut x = v2.get(); ui.slider("s", Rect::new(100., 100., 900., 50.), "Volume", &mut x, 0., 100., 5., 0, "", en2.get()); v2.set(x);
    });
    // drag from the middle of the track to far past the right edge, then to far left
    h.mv(700., 125.); h.down(700., 125.); let mid = v.get(); assert!(mid > 0. && mid < 100. && (mid % 5.).abs() < 1e-3, "snapped: {mid}");
    h.mv(5000., 125.); assert_eq!(v.get(), 100., "drag past the end clamps"); h.mv(-5000., 125.); assert_eq!(v.get(), 0.); h.up(-5000., 125.);
    // keyboard
    h.key(NavKey::Down); h.key(NavKey::Right); h.key(NavKey::Right); assert_eq!(v.get(), 10.);
    h.key(NavKey::End); assert_eq!(v.get(), 100.); h.key(NavKey::Right); assert_eq!(v.get(), 100.); h.key(NavKey::Home); assert_eq!(v.get(), 0.); h.key(NavKey::Left); assert_eq!(v.get(), 0.);
    // wheel never changes sliders
    h.mv(500., 125.); h.m.event(Event::Wheel(3.)); h.frame(); assert_eq!(v.get(), 0., "wheel does not adjust sliders");
    // garbage in
    v.set(f32::NAN); h.frame(); assert_eq!(v.get(), 0., "NaN repaired to min"); v.set(1e9); h.frame(); assert_eq!(v.get(), 100.);
    // disabled: neither drag nor keys change it
    en.set(false); h.frame(); v.set(20.); h.mv(700., 125.); h.down(700., 125.); h.up(700., 125.); h.key(NavKey::Right); assert_eq!(v.get(), 20.);
}
#[test] fn slider_value_text_and_label_are_drawn_and_clipped() {
    let mut h = test_menu(1280., 720., move |ui, _| {
        let mut x = 0.75f32; ui.slider("s", Rect::new(100., 100., 700., 50.), "A very long slider label that definitely overflows the label column", &mut x, 0., 1., 0.05, 2, "", true);
    });
    let c = h.frame(); let t: Vec<&str> = c.iter().filter_map(|c| if let Cmd::Text { text, .. } = c { Some(text.as_str()) } else { None }).collect();
    assert!(t.contains(&"0.75"), "{t:?}"); assert!(t.iter().any(|s| s.ends_with("...")), "long label ellipsised: {t:?}");
}
#[test] fn toggle_and_selector_and_dropdown() {
    let on = Rc::new(Cell::new(false)); let sel = Rc::new(Cell::new(0usize)); let dd = Rc::new(Cell::new(1usize)); let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let (on2, sel2, dd2, log2) = (on.clone(), sel.clone(), dd.clone(), log.clone());
    let mut h = test_menu(1280., 720., move |ui, _| {
        let mut o = on2.get(); if ui.toggle("t", Rect::new(100., 100., 800., 44.), "Toggle", &mut o, true).changed { log2.borrow_mut().push("toggle".into()); } on2.set(o);
        let mut s = sel2.get(); if ui.selector("s", Rect::new(100., 160., 800., 44.), "Selector", &["A", "B", "C"], &mut s, true).changed { log2.borrow_mut().push("selector".into()); } sel2.set(s);
        let mut d = dd2.get(); if ui.dropdown("d", Rect::new(100., 220., 800., 44.), "Dropdown", &["x", "y", "z", "w"], &mut d, true).changed { log2.borrow_mut().push("dropdown".into()); } dd2.set(d);
    });
    h.click(300., 122.); assert!(on.get()); h.click(300., 122.); assert!(!on.get());
    h.key(NavKey::Activate); assert!(on.get(), "keyboard toggles the focused (last clicked) row");
    // selector arrows (control area is on the right half): find them from the drawn sprites
    let c = h.frame(); let arrows: Vec<Rect> = c.iter().filter_map(|c| if let Cmd::Sprite { brush: BrushId::Arrow, rect, .. } = c { Some(*rect) } else { None }).collect();
    assert_eq!(arrows.len(), 2); let (lx, ly) = center(arrows[0]); let (rx, ry) = center(arrows[1]);
    h.click(rx, ry); assert_eq!(sel.get(), 1); h.click(rx, ry); h.click(rx, ry); assert_eq!(sel.get(), 0, "wraps forward");
    h.click(lx, ly); assert_eq!(sel.get(), 2, "wraps backward");
    // dropdown: open, pick the last item
    let c = h.frame(); let boxes: Vec<Rect> = buttons(&c).iter().map(|b| b.3).collect(); let dbox = boxes[boxes.len() - 1];
    let (dx, dy) = center(dbox); h.click(dx, dy); assert!(h.m.state().dropdown_open());
    let c = h.frame(); assert!(c.iter().any(|c| matches!(c, Cmd::Text { text, .. } if text == "w")), "popup lists all options");
    h.key(NavKey::Back); assert!(!h.m.state().dropdown_open(), "Esc closes the popup only"); assert_eq!(h.m.stack_names(), ["pause", "test"]);
    h.click(dx, dy); h.key(NavKey::End); h.key(NavKey::Activate); h.frame(); h.frame(); assert_eq!(dd.get(), 3); assert!(log.borrow().contains(&"dropdown".to_string()));
    // click-outside closes and does not activate what is underneath
    let before = on.get(); h.click(dx, dy); assert!(h.m.state().dropdown_open()); h.click(300., 122.); assert!(!h.m.state().dropdown_open()); assert_eq!(on.get(), before, "outside click was swallowed by the popup blocker");
}
#[test] fn dropdown_mouse_selection_and_dead_popup_cleanup() {
    let dd = Rc::new(Cell::new(0usize)); let d2 = dd.clone(); let show = Rc::new(Cell::new(true)); let s2 = show.clone();
    let mut h = test_menu(1280., 720., move |ui, _| {
        if s2.get() { let mut d = d2.get(); ui.dropdown("d", Rect::new(100., 100., 800., 44.), "Mode", &["one", "two", "three"], &mut d, true); d2.set(d); }
    });
    let boxes = buttons(&h.frame()); let (x, y) = center(boxes[boxes.len() - 1].3);
    h.click(x, y); assert!(h.m.state().dropdown_open());
    // second item sits one row below the anchor
    let item_h = Viewport::new(1280., 720.).scale * 36.; let (ax, ay) = (x, y + 44. * 0.5 * Viewport::new(1280., 720.).scale + item_h * 1.5 + 2.);
    h.mv(ax, ay); h.down(ax, ay); h.up(ax, ay); h.frame(); h.frame(); assert_eq!(dd.get(), 1, "clicking the second row selects it"); assert!(!h.m.state().dropdown_open());
    h.click(x, y); assert!(h.m.state().dropdown_open()); show.set(false); h.frame(); h.frame(); assert!(!h.m.state().dropdown_open(), "popup of a vanished dropdown is dropped");
}
#[test] fn keybind_capture_flow() {
    let b = Rc::new(Cell::new(Some(0x57u16))); let b2 = b.clone(); let changes = Rc::new(Cell::new(0)); let c2 = changes.clone();
    let mut h = test_menu(1280., 720., move |ui, _| {
        let mut v = b2.get(); if ui.keybind("k", Rect::new(100., 100., 800., 44.), "Forward", &mut v, true).changed { c2.set(c2.get() + 1); } b2.set(v);
        let mut other = None; ui.keybind("k2", Rect::new(100., 200., 800., 44.), "Other", &mut other, true);
    });
    let c = h.frame(); assert!(c.iter().any(|c| matches!(c, Cmd::Text { text, .. } if text == "W")));
    let (x, y) = (700., 122.);
    h.click(x, y); assert!(h.m.state().is_listening()); assert!(h.frame().iter().any(|c| matches!(c, Cmd::Text { text, .. } if text == "Press a key...")));
    h.m.event(Event::Nav(NavKey::Back)); h.m.event(Event::RawKey(0x1B)); h.frame(); assert!(!h.m.state().is_listening()); assert_eq!(b.get(), Some(0x57), "Esc cancels and does NOT close the menu"); assert_eq!(h.m.stack_names(), ["pause", "test"]);
    h.click(x, y); h.m.event(Event::Nav(NavKey::Down)); h.m.event(Event::RawKey(0x45)); h.frame(); assert_eq!(b.get(), Some(0x45)); assert_eq!(changes.get(), 1);
    h.click(x, y); h.m.event(Event::RawKey(0x2E)); h.frame(); assert_eq!(b.get(), None, "Delete unbinds"); assert!(h.frame().iter().any(|c| matches!(c, Cmd::Text { text, .. } if text == "Unbound")));
    h.click(x, y); h.m.event(Event::MouseDown(MouseButton::Right, x, y)); h.frame(); assert_eq!(b.get(), Some(0x02), "mouse buttons bind");
    h.click(x, y); assert!(h.m.state().is_listening()); h.m.event(Event::FocusLost); h.frame(); assert!(!h.m.state().is_listening(), "alt-tab cancels capture"); assert_eq!(b.get(), Some(0x02));
    h.click(x, y); h.click(x, y); assert!(!h.m.state().is_listening(), "clicking the listening button again cancels");
    h.click(x, y); h.click(x, 222.); assert!(h.m.state().is_listening() && h.m.state().is_listening(), "clicking another bind cancels the first and starts the second");
}
#[test] fn scroll_area_wheel_clip_and_hit_testing() {
    let clicked = Rc::new(RefCell::new(Vec::<usize>::new())); let c2 = clicked.clone();
    let mut h = test_menu(1280., 720., move |ui, _| {
        let area = Rect::new(100., 100., 600., 300.);
        ui.scroll_area("list", area, |ui, v| {
            let mut y = v.y;
            for i in 0..40usize { let r = Rect::new(v.x, y, v.w, 40.); if ui.button(&format!("row{i}"), r, &format!("Row {i}"), BtnKind::Default, BtnSize::M, true).clicked { c2.borrow_mut().push(i); } y += 44.; }
            y - v.y
        });
        // a button OUTSIDE the area, directly below it, overlapping nothing
        if ui.button("below", Rect::new(100., 420., 200., 40.), "Below", BtnKind::Default, BtnSize::M, true).clicked { c2.borrow_mut().push(999); }
    });
    h.click(200., 120.); assert_eq!(*clicked.borrow(), vec![0]);
    h.click(200., 405.); assert_eq!(*clicked.borrow(), vec![0], "row partly scrolled out is clipped: a click below the clip rect must not hit it");
    h.mv(300., 200.); h.m.event(Event::Wheel(-2.)); h.frame(); h.frame();
    clicked.borrow_mut().clear(); h.click(200., 120.); assert_eq!(*clicked.borrow(), vec![2], "wheel down by 2 notches scrolled 128 units = 2.9 rows");
    h.mv(300., 200.); for _ in 0..200 { h.m.event(Event::Wheel(-5.)); } h.frame(); h.frame();
    let st = h.m.state().scroll.values().next().copied().unwrap(); assert!((st.offset - (st.content_h - st.view_h)).abs() < 1e-2, "clamped at the bottom: {st:?}");
    for _ in 0..200 { h.m.event(Event::Wheel(5.)); } h.frame(); h.frame(); assert_eq!(h.m.state().scroll.values().next().unwrap().offset, 0., "clamped at the top");
    // wheel outside the area does not scroll it
    h.mv(900., 600.); h.m.event(Event::Wheel(-3.)); h.frame(); h.frame(); assert_eq!(h.m.state().scroll.values().next().unwrap().offset, 0.);
    clicked.borrow_mut().clear(); h.click(150., 440.); assert_eq!(*clicked.borrow(), vec![999]);
}
#[test] fn scroll_follows_keyboard_focus_and_pagedown() {
    let mut h = test_menu(1280., 720., move |ui, _| {
        ui.scroll_area("list", Rect::new(100., 100., 600., 300.), |ui, v| {
            let mut y = v.y; for i in 0..40usize { let r = Rect::new(v.x, y, v.w, 40.); ui.button(&format!("row{i}"), r, &format!("Row {i}"), BtnKind::Default, BtnSize::M, true); y += 44.; } y - v.y
        });
    });
    for _ in 0..12 { h.key(NavKey::Down); } for _ in 0..3 { h.frame(); }
    let off = h.m.state().scroll.values().next().unwrap().offset; assert!(off > 0., "focus moved below the fold -> area scrolled ({off})");
    for _ in 0..39 { h.key(NavKey::Down); } for _ in 0..3 { h.frame(); } // wraps to the top
    let off2 = h.m.state().scroll.values().next().unwrap().offset; assert!(off2 < off + 400., "{off2}");
    h.mv(300., 200.); h.key(NavKey::PageDown); h.frame(); assert!(h.m.state().scroll.values().next().unwrap().offset > 0.);
}
#[test] fn scrollbar_track_drags() {
    let mut h = test_menu(1280., 720., move |ui, _| {
        ui.scroll_area("list", Rect::new(100., 100., 600., 300.), |ui, v| { let mut y = v.y; for i in 0..40usize { ui.label(Rect::new(v.x, y, v.w, 40.), &format!("Row {i}"), 17., WHITE, HAlign::Left); y += 44.; } y - v.y });
    });
    let x = 100. + 600. - 4.; h.mv(x, 110.); h.down(x, 110.); h.mv(x, 395.); h.up(x, 395.); h.frame();
    let st = *h.m.state().scroll.values().next().unwrap(); assert!(st.offset > (st.content_h - st.view_h) * 0.9, "dragging the thumb to the bottom scrolls to the end: {st:?}");
}
#[test] fn content_shrink_clamps_offset() {
    let n = Rc::new(Cell::new(40usize)); let n2 = n.clone();
    let mut h = test_menu(1280., 720., move |ui, _| {
        ui.scroll_area("list", Rect::new(100., 100., 600., 300.), |ui, v| { let mut y = v.y; for i in 0..n2.get() { ui.label(Rect::new(v.x, y, v.w, 40.), &format!("Row {i}"), 17., WHITE, HAlign::Left); y += 44.; } y - v.y });
    });
    h.mv(300., 200.); for _ in 0..100 { h.m.event(Event::Wheel(-5.)); } h.frame(); h.frame(); assert!(h.m.state().scroll.values().next().unwrap().offset > 100.);
    n.set(3); h.frame(); h.frame(); assert_eq!(h.m.state().scroll.values().next().unwrap().offset, 0., "short content cannot stay scrolled");
}

// ---- settings container -----------------------------------------------------------------------------------------------
#[derive(Default)]
struct Counts { show: Cell<u32>, hide: Cell<u32>, close: Cell<u32>, frames: Cell<u32> }
struct CountPage { name: &'static str, order: i32, c: Rc<Counts> }
impl SettingsPage for CountPage {
    fn id(&self) -> &str { self.name } fn title(&self) -> &str { self.name } fn order(&self) -> i32 { self.order }
    fn ui(&mut self, ui: &mut Ui, area: Rect) { self.c.frames.set(self.c.frames.get() + 1); ui.label(area, self.name, 20., WHITE, HAlign::Center); }
    fn on_show(&mut self) { self.c.show.set(self.c.show.get() + 1); } fn on_hide(&mut self) { self.c.hide.set(self.c.hide.get() + 1); } fn on_close(&mut self) { self.c.close.set(self.c.close.get() + 1); }
}
fn open_settings(h: &mut H) { let (x, y) = center(entry(h, "SettingsButton")); h.click(x, y); }
#[test] fn settings_pages_register_sort_switch_and_notify() {
    let (a, b, c) = (Rc::new(Counts::default()), Rc::new(Counts::default()), Rc::new(Counts::default()));
    let mut m = MenuSystem::new(fonts()); m.fade_time = 0.;
    m.register_settings_page(Box::new(CountPage { name: "Late", order: 5, c: c.clone() }));
    m.register_settings_page(Box::new(CountPage { name: "Second", order: 1, c: b.clone() }));
    m.register_settings_page(Box::new(CountPage { name: "First", order: 0, c: a.clone() }));
    m.open(); assert_eq!(m.pages().iter().map(|p| p.title().to_owned()).collect::<Vec<_>>(), ["First", "Second", "Late"], "sorted by order()");
    let mut h = H { m, w: 1280., h: 720. }; h.frame(); h.frame();
    open_settings(&mut h); h.frame(); assert_eq!(h.m.stack_names(), ["pause", "settings"]);
    assert_eq!((a.show.get(), a.frames.get() > 0, b.frames.get()), (1, true, 0), "only the selected page runs");
    h.m.event(Event::Nav(NavKey::TabNext)); h.frame(); h.frame(); assert_eq!((a.hide.get(), b.show.get(), h.m.settings_tab()), (1, 1, 1));
    h.m.event(Event::Pad(PadButton::RB)); h.frame(); assert_eq!(h.m.settings_tab(), 2); h.m.event(Event::Nav(NavKey::TabNext)); h.frame(); assert_eq!(h.m.settings_tab(), 0, "wraps");
    h.m.event(Event::Pad(PadButton::LB)); h.frame(); assert_eq!(h.m.settings_tab(), 2);
    // mouse on a tab
    // (1.8 tabs are drawn with the shader-ported materials; locate them by their title text)
    let tabs: Vec<Rect> = h.frame().iter().filter_map(|c| if let Cmd::Text { x, top, box_h, text, font, px, .. } = c { if text == "Second" { Some(Rect::new(*x, *top, h.m.fonts().measure(*font, text, *px as f32), *box_h)) } else { None } } else { None }).collect(); assert!(!tabs.is_empty(), "the Second tab title is drawn");
    let (x, y) = center(tabs[0]); h.click(x, y); assert_eq!(h.m.settings_tab(), 1);
    h.key(NavKey::Back); assert_eq!(h.m.stack_names(), ["pause"]); assert_eq!((a.close.get(), b.close.get(), c.close.get()), (1, 1, 1), "every page told the screen closed");
    open_settings(&mut h); h.frame(); assert_eq!(h.m.settings_tab(), 1, "last tab is remembered across openings");
}
#[test] fn settings_screen_is_opaque_and_hides_the_pause_buttons() {
    let mut h = H::new(1280., 720.); h.m.register_settings_page(Box::new(CountPage { name: "P", order: 0, c: Rc::new(Counts::default()) }));
    let before = procs(&h.frame()).len(); assert!(before >= 7, "the menu draws its seven buttons");
    open_settings(&mut h); h.frame();
    let after = procs(&h.frame()); assert!(after.is_empty(), "no MainMenu material buttons are drawn under the opaque settings screen: {before} -> {}", after.len());
}
#[test] fn empty_settings_shows_an_honest_placeholder() {
    let mut h = H::new(1280., 720.); open_settings(&mut h); assert!(h.frame().iter().any(|c| matches!(c, Cmd::Text { text, .. } if text.starts_with("No settings pages"))));
    h.key(NavKey::Back); assert_eq!(h.m.stack_names(), ["pause"]);
}
#[test] fn demo_gallery_pages_run_at_every_size_without_overflowing_text() {
    let f = fonts();
    // (no 300x300 case: `UIScaleRule=Vertical` holds 0.5 below 540 px (CookedIniVersion.txt L531-532), so a 300 px client is 600 Slate units wide, narrower than the fixed 1.8 layout)
    for (w, h_) in SIZES {
        let mut h = H::new(w, h_); register_demo_pages(&mut h.m); open_settings(&mut h);
        for tab in 0..3 {
            for _ in 0..3 { h.frame(); }
            for c in h.frame() { if let Cmd::Text { x, text, font, px, .. } = c {
                let tw = f.measure(font, &text, px as f32);
                assert!(x >= -0.5 && x + tw <= w + 0.5, "{w}x{h_} tab {tab}: {text:?} x {x} tw {tw}");
            } }
            h.m.event(Event::Nav(NavKey::TabNext));
        }
    }
}
#[test] fn gallery_text_is_clipped_to_its_scroll_viewport() {
    let mut h = H::new(1280., 720.); register_demo_pages(&mut h.m); open_settings(&mut h); for _ in 0..4 { h.frame(); }
    let c = h.frame(); let mut clip: Vec<Rect> = Vec::new(); let mut in_scroll_text = 0;
    for cmd in &c { match cmd { Cmd::PushClip(r) => clip.push(*r), Cmd::PopClip => { clip.pop(); } Cmd::Text { .. } if !clip.is_empty() => in_scroll_text += 1, _ => {} } }
    assert!(clip.is_empty(), "clip stack balanced"); assert!(in_scroll_text > 10);
}

// ---- assets ------------------------------------------------------------------------------------------------------------
#[test] fn every_brush_texture_exists_with_the_recovered_size() {
    let root = crate::assets::DirAssets::default_root();
    if !root.is_dir() { eprintln!("skipping: asset tree {} not found", root.display()); return; }
    let all = crate::assets::DirAssets::new(root);
    let find = |n: &str| all.path_of(n).cloned();
    for b in BrushId::all() {
        let n = b.file_name(); let p = find(&n).unwrap_or_else(|| panic!("missing exported texture {n}"));
        let (w, h) = image::image_dimensions(&p).unwrap();
        let expect = match b {
            BrushId::Button(_, BtnSize::M, _) | BrushId::Close(_) => (36, 36), BrushId::Button(_, BtnSize::L, _) => (60, 60),
            BrushId::Tab(_) => (42, 36), BrushId::Check(..) => (28, 28), BrushId::Arrow => (16, 16), BrushId::KeyFrame => (40, 40),
            // 1.8: T-Icon-{Config,Logout,Exit,Feedback}-XL and T-Icon-{LegalInfo,Credits}-52 are all 52x52 (reports/menu-textures.json width/height; 1.10 used 64x64 -64 files).
            BrushId::Icon(_) => (52, 52),
            BrushId::Calibration => (512, 512), // T_BrightnessCalibration (Brightness tab logo)
            // 1.8 settings textures: GamepadMappingInfo Image_Gamepad brush size [1024,672] (both platforms + line overlays), remove-icon brush 32x32
            BrushId::Tex(t) => match t { TexId::Xb1Config | TexId::Xb1Lines | TexId::Ps4Config | TexId::Ps4Lines => (1024, 672), TexId::RemoveIcon => (32, 32), TexId::EpicLogo => (214, 244), TexId::ModeCombat | TexId::ModeBuild => (w, h) },
        };
        assert_eq!((w, h), expect, "{n}");
    }
}
#[test] fn pause_menu_uses_original_style_asset_values() {
    // Button-Default-M style: ImageSize 36, box margin 0.5, padding 12 (L: 60 / 20); read with tools/uasset-props.cjs.
    assert_eq!(BrushId::Button(BtnKind::Default, BtnSize::M, Vis::Normal).box_margin(), Some([0.5; 4]));
    assert_eq!(BrushId::Button(BtnKind::Emphasis, BtnSize::L, Vis::Disabled).file_name(), "Button-Default-L-_disabled_.png", "Disabled maps to the Default disabled texture (as the Emphasis style asset does)");
    assert_eq!(BrushId::Tab(TabVis::Passive).file_name(), "Tab1-M-_passive_.png");
    assert_eq!(BrushId::Check(true, Vis::Hover).file_name(), "CheckBoxCheckedHovered.png");
    assert_eq!(BrushId::Check(false, Vis::Normal).file_name(), "CheckBoxUnchecked.png");
}
#[test] fn draw_list_alpha_scales_with_fade() {
    let mut m = MenuSystem::new(fonts()); m.open(); m.fade_time = 1.;
    let cmds = m.frame(1280., 720., 0.5).cmds.clone();
    let a = cmds.iter().find_map(|c| if let Cmd::Procedural { alpha, .. } = c { Some(*alpha) } else { None }).unwrap();
    assert!((a - 0.5).abs() < 1e-5, "half-way through the fade-in tints are half transparent: {a}");
    // 1.8: no dim Fill any more (MainMenu.Border_0 is NoDrawType); the ShellUberBG backdrop panel fades with the menu instead.
    let bd = cmds.iter().find_map(|c| if let Cmd::Panel { alpha, .. } = c { Some(*alpha) } else { None }).unwrap();
    assert!((bd - 0.5).abs() < 1e-5, "the backdrop panel fades with the menu: {bd}");
}
