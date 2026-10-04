//! Event-driven integration tests: the whole menu + Settings flow driven only through raw, windowing-system-agnostic `InputEvent`s
//! (`MenuSystem::input`), exactly the way a live host (`fn-ui-demo --live`, `fn-app`) feeds it, over the real `fn-settings` model, a
//! `PlatformHost` and an `ApplyBackend`. Mouse positions are taken from the text the menu actually drew, so every test works at any size.
//!
//! Ported flows of the old prototype's `src/menu/tests.rs` / pages tests (open menu, Settings, edit a row, rebind with conflict swap, Apply,
//! Back, keep-video-settings revert) plus hover / press / drag / focus semantics, tab and arrow navigation, Esc back-stack semantics, the
//! key-capture overlay (modifiers L/R, mouse buttons, wheel, Esc) and rapid-input safety.
use super::mainmenu::{self, MenuContext};
use super::pages::{self, NullBackend, UiContext};
use super::*;
use crate::input::vk;
use crate::router::{InputEvent, PadAxis, WheelUnit};
use crate::settings::{Key, NullPlatform, Settings, WindowMode, A_FIRE, A_JUMP};

fn fonts() -> Fonts { crate::testutil::fonts() }

struct Game { s: Settings, platform: NullPlatform, be: NullBackend }
impl Game {
    fn new() -> Self { let mut s = Settings::default(); s.drain_changes(); Game { s, platform: NullPlatform::static_16_9(), be: NullBackend { persist: true, ..Default::default() } } }
    fn cx(&mut self) -> UiContext<'_> { UiContext::new(&mut self.s, &mut self.platform, &mut self.be) }
}

/// A live host in miniature: window size, frame time, the game behind the menu.
struct Live { m: MenuSystem, g: Game, w: f32, h: f32, dt: f32 }
impl Live {
    fn new(w: f32, h: f32) -> Self {
        let mut m = MenuSystem::new(fonts());
        m.fade_time = 0.;
        pages::register_all_into(&mut m);
        Live { m, g: Game::new(), w, h, dt: 1. / 60. }
    }
    /// What a host does with an event: route it, and open the menu on a fresh Escape / Start press while it is closed.
    fn ev(&mut self, e: InputEvent) {
        let r = self.m.input(e);
        if (r.escape_edge || r.start_edge) && !self.m.is_open() { self.m.open(); }
    }
    fn frame(&mut self) { let (w, h, dt) = (self.w, self.h, self.dt); self.m.frame_with(&mut self.g.cx(), w, h, dt); }
    fn frames(&mut self, n: usize) { for _ in 0..n { self.frame(); } }
    /// Let `secs` of game time pass (coarse frames).
    fn advance(&mut self, secs: f32) { let old = self.dt; self.dt = 1. / 30.; for _ in 0..(secs * 30.).ceil() as usize { self.frame(); } self.dt = old; }
    fn key_down(&mut self, code: u16) { self.ev(InputEvent::Key { vk: code, pressed: true, repeat: false }); self.frame(); }
    fn key_up(&mut self, code: u16) { self.ev(InputEvent::Key { vk: code, pressed: false, repeat: false }); self.frame(); }
    fn press(&mut self, code: u16) { self.key_down(code); self.key_up(code); self.frames(2); }
    fn mv(&mut self, x: f32, y: f32) { self.ev(InputEvent::PointerMoved { x, y }); self.frame(); }
    fn button(&mut self, b: MouseButton, pressed: bool) { self.ev(InputEvent::PointerButton { button: b, pressed }); self.frame(); }
    fn click_at(&mut self, x: f32, y: f32) { self.mv(x, y); self.button(MouseButton::Left, true); self.button(MouseButton::Left, false); self.frames(2); }
    fn wheel(&mut self, notches: f32) { self.ev(InputEvent::Wheel { dx: 0., dy: notches, unit: WheelUnit::Lines }); self.frames(2); }
    fn pad(&mut self, b: PadButton) { self.ev(InputEvent::PadButton { button: b, pressed: true }); self.frame(); self.ev(InputEvent::PadButton { button: b, pressed: false }); self.frames(2); }
    fn stack(&self) -> Vec<String> { self.m.stack_names() }
    fn rect_of(&self, label: &str) -> Option<Rect> {
        let fonts = self.m.fonts();
        self.m.parts().0.cmds.iter().find_map(|c| if let Cmd::Text { x, top, box_h, text, font, px, .. } = c { if text == label { Some(Rect::new(*x, *top, fonts.measure(*font, text, *px as f32), *box_h)) } else { None } } else { None })
    }
    fn must(&mut self, label: &str) -> Rect { self.frame(); self.rect_of(label).unwrap_or_else(|| panic!("{label:?} not drawn: {:?}", self.texts())) }
    fn has(&mut self, label: &str) -> bool { self.frame(); self.rect_of(label).is_some() }
    fn texts(&self) -> Vec<String> { self.m.parts().0.text_strings().into_iter().map(str::to_owned).collect() }
    fn all_text(&self) -> String { self.texts().join(" ") }
    fn click_text(&mut self, label: &str) { let r = self.must(label); self.click_at(r.x + r.w * 0.5, r.y + r.h * 0.5); }
    fn scale(&self) -> f32 { crate::layout::Viewport::new(self.w, self.h).scale }
    /// Centre of the right (or left) arrow of the rotator row `label` (see `slate::rotator_row`: label right-aligned in a 300 unit box, 20 gap, 400 control).
    fn arrow_of(&mut self, label: &str, right: bool) -> (f32, f32) {
        use pages::slate::*;
        let r = self.must(label); let s = self.scale();
        let cx = r.x + r.w + LABEL_GAP * s;
        let x = if right { cx + (CONTROL_MIN_W - ARROW_PAD - ARROW_W * 0.5) * s } else { cx + (ARROW_PAD + ARROW_W * 0.5) * s };
        (x, r.y + r.h * 0.5)
    }
    fn click_arrow(&mut self, label: &str, right: bool) { let (x, y) = self.arrow_of(label, right); self.click_at(x, y); }
    /// (track left, track width, y) of the slider row `label` (see `slate::slider_row_ex`).
    fn slider_track(&mut self, label: &str) -> (f32, f32, f32) {
        use pages::slate::*;
        let r = self.must(label); let s = self.scale();
        let cx = r.x + r.w + LABEL_GAP * s;
        (cx + SLIDER_BUTTON_PAD * s, (CONTROL_MIN_W - 2. * SLIDER_BUTTON_PAD) * s, r.y + r.h * 0.5)
    }
    fn open_settings(&mut self) {
        self.key_down(vk::ESCAPE); self.key_up(vk::ESCAPE); self.frames(2);
        assert!(self.m.is_open(), "Esc opens the menu");
        let r = mainmenu::entry_rect(self.w, self.h, MenuContext::Match, "SettingsButton").expect("settings entry");
        self.click_at(r.x + r.w * 0.5, r.y + r.h * 0.5);
        assert_eq!(self.stack(), ["pause", "settings"], "the Settings icon opens the Settings screen");
    }
    fn tab(&mut self, title: &str) { self.click_text(title); self.frames(3); }
    fn working(&self) -> Settings { self.m.session().settings_clone() }
    fn listening(&self) -> bool { self.m.state().is_listening() }
}

const W: f32 = 1920.;
const H: f32 = 1080.;

// ---- open / close ------------------------------------------------------------------------------------------------------
#[test] fn esc_opens_the_menu_and_a_held_esc_cannot_flicker_it() {
    let mut l = Live::new(W, H);
    l.frames(2);
    assert!(!l.m.is_open());
    l.key_down(vk::ESCAPE); l.frames(2);
    assert!(l.m.is_open() && l.stack() == ["pause"]);
    // the OS auto-repeats a held Esc: neither a second open nor a Back may result
    for _ in 0..40 { l.ev(InputEvent::Key { vk: vk::ESCAPE, pressed: true, repeat: true }); l.frame(); }
    assert!(l.m.is_open() && l.stack() == ["pause"], "held Esc does not close the menu again");
    // a key-down that lost its key-up (no repeat flag) is a repeat as well
    for _ in 0..5 { l.key_down(vk::ESCAPE); }
    assert!(l.m.is_open(), "{:?}", l.stack());
    l.key_up(vk::ESCAPE); l.frames(2);
    // a real second press at the root closes it and asks the host to recapture the mouse
    l.m.take_requests();
    l.press(vk::ESCAPE);
    assert!(!l.m.is_open());
    assert_eq!(l.m.take_requests(), [HostRequest::Closed]);
}
#[test] fn pad_start_opens_the_menu_and_b_closes_it() {
    let mut l = Live::new(W, H);
    l.pad(PadButton::Start);
    assert!(l.m.is_open(), "Start opens");
    l.pad(PadButton::B);
    assert!(!l.m.is_open(), "B at the root closes");
    assert_eq!(l.m.take_requests(), [HostRequest::Closed]);
}

// ---- the full flow ------------------------------------------------------------------------------------------------------
#[test] fn open_menu_settings_change_a_row_rebind_with_conflict_swap_apply_and_back() {
    let mut l = Live::new(W, H);
    l.open_settings();
    // Video tab is first: change Vsync with its right arrow
    assert!(l.has("Vsync"));
    let vsync_before = l.g.s.video.vsync;
    l.click_arrow("Vsync", true);
    assert_eq!(l.working().video.vsync, !vsync_before, "the working copy changed");
    assert_eq!(l.g.s.video.vsync, vsync_before, "the game is untouched before Apply");
    assert!(l.has("Apply"), "the footer offers Apply once something changed");
    assert_eq!(l.m.session().changed_ids(), ["video.vsync"]);
    // Input tab: Fire (Left Mouse Button) <- Space Bar, which Jump owns: the keys are swapped (Crouch is the 45th row of the 1.8 list, out of view)
    l.tab("Input");
    assert!(l.has("Left Mouse Button") && l.has("Space Bar"));
    l.click_text("Left Mouse Button");
    assert!(l.listening() && l.has("PRESS ANY KEY..."), "capture overlay is up");
    l.press(vk::SPACE);
    assert!(!l.listening() && !l.has("PRESS ANY KEY..."));
    let b = l.working().input.bindings;
    let (old_crouch, old_jump) = (Key::from_name("LeftMouseButton").unwrap(), Key::from_name("SpaceBar").unwrap());
    assert_eq!(b.get(A_FIRE)[0], old_jump, "Fire now has Space");
    assert_eq!(b.get(A_JUMP)[0], old_crouch, "Jump received Fire's old key (swap, not a silent duplicate)");
    assert!(b.conflicts().is_empty(), "{:?}", b.conflicts());
    let t = l.all_text();
    assert!(!t.contains("was already used") && !t.contains("Swapped"), "the original announces nothing on a swap: {t}");
    assert_eq!(l.g.s.input.bindings.get(A_JUMP)[0], old_crouch, "the Input tab has no Apply handler in 1.8 (TabInputOptions only calls native ChangeBinding): the rebind reaches the game at once");
    // The rebind already applied and saved (Input has no Apply handler); that run also carried the pending Vsync edit, so the footer is clean.
    l.frames(3);
    assert_eq!(l.g.s.video.vsync, !vsync_before, "the immediate apply carried the pending working copy");
    assert_eq!(l.g.s.input.bindings.get(A_FIRE)[0], old_jump);
    assert_eq!(l.g.s.input.bindings.get(A_JUMP)[0], old_crouch);
    let saved = l.g.be.files.last().expect("settings were written");
    assert_eq!(saved.input.bindings.get(A_FIRE)[0], old_jump);
    assert!(!l.m.session().is_dirty());
    assert!(!l.has("Apply"), "Apply disappears once nothing is pending");    // Back (Esc) pops Settings, a second Esc closes the menu
    l.press(vk::ESCAPE);
    assert_eq!(l.stack(), ["pause"]);
    l.press(vk::ESCAPE);
    assert!(!l.m.is_open());
}
#[test] fn restore_discards_the_edit_and_back_drops_pending_changes_without_a_prompt() {
    let mut l = Live::new(W, H);
    l.open_settings();
    l.tab("Audio");
    l.click_arrow("Subtitles", true);
    assert!(l.m.session().is_dirty());
    l.click_text("Restore");
    assert!(!l.m.session().is_dirty(), "Restore dropped the edit");
    l.click_arrow("Subtitles", true);
    assert!(l.m.session().is_dirty());
    l.press(vk::ESCAPE);
    assert_eq!(l.stack(), ["pause"], "Back leaves at once (1.8 has no unsaved-changes prompt)");
    assert!(!l.m.session().is_dirty());
    assert_eq!(l.g.s.accessibility.subtitles, Settings::default().accessibility.subtitles, "nothing was applied");
}

// ---- keep video settings -------------------------------------------------------------------------------------------------
fn risky_apply(l: &mut Live) {
    l.open_settings();
    assert_eq!(l.g.s.video.window_mode, WindowMode::Windowed);
    l.click_arrow("Window Mode", false); // Windowed -> Windowed Fullscreen
    assert_eq!(l.working().video.window_mode, WindowMode::WindowedFullscreen);
    l.click_text("Apply");
    l.frames(4);
    assert_eq!(l.g.s.video.window_mode, WindowMode::WindowedFullscreen, "applied live");
    assert!(l.has("Do you want to keep these video settings?"), "{:?}", l.texts());
    assert_eq!(l.stack().last().map(String::as_str), Some("settings-modal"));
    assert_eq!(l.g.be.files.last().unwrap().video.window_mode, WindowMode::Windowed, "the file keeps the old mode until confirmed");
}
#[test] fn keep_video_dialog_reverts_after_fifteen_seconds() {
    let mut l = Live::new(W, H);
    risky_apply(&mut l);
    l.advance(10.);
    assert!(l.has("Do you want to keep these video settings?"), "still asking after 10 s");
    assert!(l.all_text().contains("Reverting in"), "{:?}", l.texts());
    assert_eq!(l.g.s.video.window_mode, WindowMode::WindowedFullscreen);
    l.advance(5.5);
    l.frames(4);
    assert!(!l.has("Do you want to keep these video settings?"), "dialog closed on its own");
    assert_eq!(l.g.s.video.window_mode, WindowMode::Windowed, "reverted to the previous mode");
    assert_eq!(l.g.be.files.last().unwrap().video.window_mode, WindowMode::Windowed);
    assert_eq!(l.stack(), ["pause", "settings"]);
}
#[test] fn keep_video_dialog_yes_keeps_and_saves() {
    let mut l = Live::new(W, H);
    risky_apply(&mut l);
    l.click_text("Yes");
    l.frames(3);
    assert_eq!(l.g.s.video.window_mode, WindowMode::WindowedFullscreen);
    assert_eq!(l.g.be.files.last().unwrap().video.window_mode, WindowMode::WindowedFullscreen, "now it is persisted");
    l.advance(20.);
    assert_eq!(l.g.s.video.window_mode, WindowMode::WindowedFullscreen, "the countdown is gone");
}
#[test] fn keep_video_dialog_no_and_esc_revert_immediately() {
    for esc in [false, true] {
        let mut l = Live::new(W, H);
        risky_apply(&mut l);
        if esc { l.press(vk::ESCAPE); } else { l.click_text("No"); }
        l.frames(3);
        assert_eq!(l.g.s.video.window_mode, WindowMode::Windowed, "esc={esc}");
        assert_eq!(l.stack(), ["pause", "settings"], "esc={esc}: only the dialog closed");
    }
}
#[test] fn closing_the_menu_with_the_keep_dialog_open_reverts() {
    let mut l = Live::new(W, H);
    risky_apply(&mut l);
    l.m.close();
    l.frames(3);
    assert_eq!(l.g.s.video.window_mode, WindowMode::Windowed, "an unanswered dialog counts as No");
}

// ---- hover / press / drag ------------------------------------------------------------------------------------------------
#[test] fn pressing_a_button_then_leaving_it_cancels_the_click() {
    let mut l = Live::new(W, H);
    l.open_settings();
    l.tab("Audio");
    l.click_arrow("Subtitles", true);
    assert!(l.has("Apply"));
    let r = l.must("Apply");
    // press on Apply, drag away, release elsewhere: nothing is applied
    l.mv(r.x + 4., r.y + r.h * 0.5);
    l.button(MouseButton::Left, true);
    l.mv(r.x + 4., r.y - 400.);
    l.button(MouseButton::Left, false); l.frames(3);
    assert!(l.g.be.applied_calls.is_empty(), "release outside the button is no click");
    assert!(l.m.session().is_dirty());
    // press and release on it applies
    l.click_text("Apply"); l.frames(3);
    assert_eq!(l.g.be.applied_calls, [true]);
}
#[test] fn slider_drag_keeps_following_the_pointer_outside_the_widget_and_the_window() {
    let mut l = Live::new(W, H);
    l.open_settings();
    l.tab("Audio");
    let (tx, tw, ty) = l.slider_track("Music Volume");
    let start = l.working().audio.music_volume;
    l.mv(tx + tw * 0.5, ty); l.button(MouseButton::Left, true);
    let v_mid = l.working().audio.music_volume;
    assert!((v_mid - 0.5).abs() < 0.03, "pressing sets the value from the pointer: {v_mid}");
    let _ = start;
    // far right and well below the row: still dragging
    l.mv(tx + tw + 300., ty + 250.);
    assert!((l.working().audio.music_volume - 1.0).abs() < 1e-6, "{}", l.working().audio.music_volume);
    // the pointer leaves the window while the button is down; the leave is deferred, the drag goes on
    l.ev(InputEvent::PointerLeft); l.frame();
    l.mv(tx - 700., ty - 200.);
    assert!(l.working().audio.music_volume.abs() < 1e-6, "{}", l.working().audio.music_volume);
    l.mv(tx + tw * 0.25, ty + 90.);
    assert!((l.working().audio.music_volume - 0.25).abs() < 0.03);
    l.button(MouseButton::Left, false); l.frames(3);
    let released = l.working().audio.music_volume;
    l.mv(tx + tw, ty); l.frames(2);
    assert_eq!(l.working().audio.music_volume, released, "moving after the release does not drag");
    assert_eq!(l.m.session().changed_ids(), ["audio.music_volume"], "no other row was touched by the drag");
}
#[test] fn focus_loss_cancels_a_drag_a_capture_and_an_open_press() {
    let mut l = Live::new(W, H);
    l.open_settings();
    l.tab("Audio");
    let (tx, tw, ty) = l.slider_track("Music Volume");
    l.mv(tx + tw * 0.5, ty); l.button(MouseButton::Left, true);
    l.ev(InputEvent::FocusLost); l.frames(2);
    let frozen = l.working().audio.music_volume;
    l.mv(tx + tw, ty); l.frames(2);
    assert_eq!(l.working().audio.music_volume, frozen, "alt-tab mid-drag ends the drag");
    l.button(MouseButton::Left, false); l.frames(2);
    assert_eq!(l.working().audio.music_volume, frozen, "the late release does nothing");
    // key capture is cancelled by focus loss, nothing is bound
    l.tab("Input");
    l.click_text("Space Bar");
    assert!(l.listening());
    let before = l.working().input.bindings;
    l.ev(InputEvent::FocusLost); l.frames(2);
    assert!(!l.listening(), "capture cancelled by FocusLost");
    assert_eq!(l.working().input.bindings, before);
}

// ---- keyboard / pad navigation -------------------------------------------------------------------------------------------
fn focus_trail(l: &mut Live, steps: usize, down: bool, tab_key: bool) -> Vec<Option<Id>> {
    let mut v = Vec::new();
    for _ in 0..steps {
        if tab_key {
            if down { l.press(vk::TAB); } else { l.key_down(vk::LSHIFT); l.press(vk::TAB); l.key_up(vk::LSHIFT); l.frames(2); }
        } else { l.press(if down { vk::DOWN } else { vk::UP }); }
        v.push(l.m.state().focused_id());
    }
    v
}
#[test] fn arrow_and_tab_navigation_walk_the_same_focus_order_and_wrap() {
    let mut l = Live::new(W, H);
    l.open_settings();
    l.tab("Audio");
    let n = 24;
    let down = focus_trail(&mut l, n, true, false);
    let first_repeat = (1..n).find(|&i| down[i] == down[0]).expect("focus order wraps");
    assert!(first_repeat >= 5, "a real focus order exists: {first_repeat}");
    assert!(down[..first_repeat].iter().all(|f| f.is_some()));
    let mut l2 = Live::new(W, H);
    l2.open_settings(); l2.tab("Audio");
    let tab_down = focus_trail(&mut l2, first_repeat + 1, true, true);
    assert_eq!(tab_down[..=first_repeat], down[..=first_repeat], "Tab walks the same order as Down");
    // Shift+Tab and Up walk it backwards
    let up = focus_trail(&mut l2, first_repeat, false, true);
    assert_eq!(up[0], down[first_repeat - 1], "Shift+Tab goes back: {:?} vs {:?}", up[0], down[first_repeat - 1]);
    let mut l3 = Live::new(W, H);
    l3.open_settings(); l3.tab("Audio");
    focus_trail(&mut l3, 3, true, false);
    let back = focus_trail(&mut l3, 2, false, false);
    assert_eq!(back[1], down[0], "Up twice from the third stop is the first stop");
}
#[test] fn arrows_adjust_the_focused_rows_and_enter_activates_without_repeating() {
    let mut l = Live::new(W, H);
    l.open_settings();
    l.tab("Audio");
    // walk down until a Right arrow changes something (the first slider / rotator)
    let mut hit = false;
    for _ in 0..12 { l.press(vk::DOWN); l.press(vk::RIGHT); if l.m.session().is_dirty() { hit = true; break; } }
    assert!(hit, "an arrow key edits the focused row");
    let ids = l.m.session().changed_ids();
    assert_eq!(ids.len(), 1, "{ids:?}");
    l.press(vk::LEFT);
    assert!(!l.m.session().is_dirty(), "Left undoes Right");
    // Enter on a focused rotator row advances it once, even when the OS repeats the held key
    l.press(vk::DOWN);
    let before = l.working();
    l.key_down(vk::RETURN);
    for _ in 0..30 { l.ev(InputEvent::Key { vk: vk::RETURN, pressed: true, repeat: true }); l.frame(); }
    l.key_up(vk::RETURN); l.frames(2);
    assert!(before.diff(&l.working()).len() <= 1, "a held Enter activates at most once: {:?}", before.diff(&l.working()));
}#[test] fn ctrl_tab_ctrl_pagedown_and_the_pad_bumpers_switch_settings_tabs() {
    let mut l = Live::new(W, H);
    l.open_settings();
    let n = l.m.pages().len();
    assert_eq!(l.m.settings_tab(), 0);
    l.key_down(vk::LCONTROL); l.press(vk::TAB); assert_eq!(l.m.settings_tab(), 1, "Ctrl+Tab");
    l.press(vk::NEXT); assert_eq!(l.m.settings_tab(), 2, "Ctrl+PageDown");
    l.key_down(vk::RSHIFT); l.press(vk::TAB); l.key_up(vk::RSHIFT); assert_eq!(l.m.settings_tab(), 1, "Ctrl+Shift+Tab goes back");
    l.press(vk::PRIOR); assert_eq!(l.m.settings_tab(), 0, "Ctrl+PageUp");
    l.press(vk::PRIOR); assert_eq!(l.m.settings_tab(), n - 1, "wraps");
    l.key_up(vk::LCONTROL);
    l.pad(PadButton::RB); assert_eq!(l.m.settings_tab(), 0, "RB");
    l.pad(PadButton::LB); assert_eq!(l.m.settings_tab(), n - 1, "LB");
}
#[test] fn gamepad_stick_navigates_with_repeat_a_activates_and_b_goes_back() {
    let mut l = Live::new(W, H);
    l.pad(PadButton::Start);
    assert!(l.m.is_open() && l.stack() == ["pause"]);
    let focus0 = l.m.state().focused_id();
    // one stick push = one step, however long the frame took
    l.ev(InputEvent::PadAxis { axis: PadAxis::LeftY, value: -0.9 }); l.frame();
    let focus1 = l.m.state().focused_id();
    assert_ne!(focus0, focus1, "stick moved focus");
    l.advance(0.3);
    assert_eq!(l.m.state().focused_id(), focus1, "no repeat inside the initial delay");
    l.advance(0.6);
    let focus2 = l.m.state().focused_id();
    assert_ne!(focus2, focus1, "holding the stick repeats after the delay");
    l.ev(InputEvent::PadAxis { axis: PadAxis::LeftY, value: 0. }); l.frames(2);
    let settled = l.m.state().focused_id();
    l.advance(1.0);
    assert_eq!(l.m.state().focused_id(), settled, "releasing the stick stops the repeat");
    // D-pad down + A walks the menu until it reaches Settings; B always steps back
    let mut reached = false;
    for _ in 0..12 {
        l.pad(PadButton::DPadDown); l.pad(PadButton::A);
        if l.stack().last().map(String::as_str) == Some("settings") { reached = true; break; }
        if l.stack().len() > 1 { l.pad(PadButton::B); }
        assert!(l.m.is_open());
    }
    assert!(reached, "A activates the focused entry; Settings is reachable with the pad alone: {:?}", l.stack());
    l.pad(PadButton::B);
    assert_eq!(l.stack(), ["pause"], "B backs out of Settings");
    l.pad(PadButton::B);
    assert!(!l.m.is_open(), "and closes the menu at the root");
}
#[test] fn right_stick_scrolls_the_settings_list() {
    let mut l = Live::new(W, H);
    l.open_settings(); l.tab("Input"); // 40 binding rows overflow the page
    let y0 = l.must("Move Forward").y;
    l.mv(W * 0.5, H * 0.5);
    l.ev(InputEvent::PadAxis { axis: PadAxis::RightY, value: -1. });
    l.advance(1.0);
    l.ev(InputEvent::PadAxis { axis: PadAxis::RightY, value: 0. }); l.frames(2);
    let y1 = l.rect_of("Move Forward").map(|r| r.y);
    assert!(y1.map_or(true, |y| y < y0 - 20.), "the list scrolled: {y0} -> {y1:?}");
}
// ---- Esc back-stack semantics --------------------------------------------------------------------------------------------
#[test] fn esc_pops_exactly_one_screen_per_frame_however_many_presses_arrive() {
    let mut l = Live::new(W, H);
    risky_apply(&mut l); // stack: pause, settings, settings-modal
    assert_eq!(l.stack().len(), 3);
    // three distinct presses inside one frame (down/up/down/up/down/up before the frame runs)
    for _ in 0..3 { l.ev(InputEvent::Key { vk: vk::ESCAPE, pressed: true, repeat: false }); l.ev(InputEvent::Key { vk: vk::ESCAPE, pressed: false, repeat: false }); }
    l.frame();
    assert_eq!(l.stack().len(), 2, "one Back per frame: {:?}", l.stack());
    assert_eq!(l.g.s.video.window_mode, WindowMode::Windowed, "the dialog's Esc is its No");
    l.frames(2);
    assert_eq!(l.stack().len(), 2);
}
#[test] fn esc_while_capturing_cancels_only_the_capture_and_is_never_bound() {
    let mut l = Live::new(W, H);
    l.open_settings(); l.tab("Input");
    let before = l.working().input.bindings;
    l.click_text("Space Bar");
    assert!(l.listening());
    l.press(vk::ESCAPE);
    assert!(!l.listening(), "capture cancelled");
    assert_eq!(l.stack(), ["pause", "settings"], "the same Esc did not also go back");
    assert_eq!(l.working().input.bindings, before, "Esc was not bound");
    assert!(l.working().input.bindings.keys.iter().flatten().all(|k| k.name() != "Escape"));
    l.press(vk::ESCAPE);
    assert_eq!(l.stack(), ["pause"], "the next Esc goes back as usual");
}
// ---- key capture mode ----------------------------------------------------------------------------------------------------
fn capture_with(l: &mut Live, act: impl FnOnce(&mut Live)) -> Key {
    l.click_text("Space Bar");
    assert!(l.listening(), "capture started");
    act(l);
    assert!(!l.listening(), "capture ended");
    l.working().input.bindings.get(A_JUMP)[0]
}
fn input_tab() -> Live { let mut l = Live::new(W, H); l.open_settings(); l.tab("Input"); l }
#[test] fn capture_binds_left_and_right_modifiers_as_separate_keys() {
    for (code, name) in [(vk::LSHIFT, "LeftShift"), (vk::RSHIFT, "RightShift"), (vk::LCONTROL, "LeftControl"), (vk::RCONTROL, "RightControl"), (vk::LMENU, "LeftAlt"), (vk::RMENU, "RightAlt")] {
        let mut l = input_tab();
        let k = capture_with(&mut l, |l| { l.key_down(code); l.key_up(code); l.frames(2); });
        assert_eq!(k.name(), name, "vk {code:#x}");
    }
}
#[test] fn capture_binds_mouse_buttons_and_the_wheel() {
    for (b, name) in [(MouseButton::Right, "RightMouseButton"), (MouseButton::Middle, "MiddleMouseButton"), (MouseButton::X1, "ThumbMouseButton"), (MouseButton::X2, "ThumbMouseButton2")] {
        let mut l = input_tab();
        let k = capture_with(&mut l, |l| { l.button(b, true); l.button(b, false); l.frames(2); });
        assert_eq!(k.name(), name);
    }
    for (d, name) in [(1., "MouseScrollUp"), (-1., "MouseScrollDown")] {
        let mut l = input_tab();
        let k = capture_with(&mut l, |l| l.wheel(d));
        assert_eq!(k.name(), name);
    }
    // touchpad pixel deltas count as wheel notches too
    let mut l = input_tab();
    let k = capture_with(&mut l, |l| { l.ev(InputEvent::Wheel { dx: 0., dy: 96., unit: WheelUnit::Pixels }); l.frames(2); });
    assert_eq!(k.name(), "MouseScrollUp");
}
#[test] fn capture_keys_letters_function_keys_backspace_clear_and_refused_keys() {
    let mut l = input_tab();
    assert_eq!(capture_with(&mut l, |l| l.press(0x47)).name(), "G");
    l.click_text("G"); assert!(l.listening());
    l.press(0x71); assert!(!l.listening());
    assert_eq!(l.working().input.bindings.get(A_JUMP)[0].name(), "F2");
    // Backspace clears the slot
    l.click_text("F2"); assert!(l.listening()); l.press(vk::BACK);
    assert!(!l.listening());
    assert_eq!(l.working().input.bindings.get(A_JUMP)[0], Key::NONE, "Backspace unbinds");
    // a key the table does not know (the Windows key) is refused with a message and binds nothing
    let mut l = input_tab();
    l.click_text("Space Bar"); l.press(0x5B);
    assert!(!l.listening());
    assert_eq!(l.working().input.bindings.get(A_JUMP)[0].name(), "SpaceBar", "unsupported key: unchanged");
    assert!(!l.all_text().contains("cannot be used"), "the original shows no refusal text: {}", l.all_text());
}#[test] fn first_event_wins_when_two_keys_arrive_in_one_frame_while_capturing() {
    let mut l = input_tab();
    l.click_text("Space Bar");
    // 'K' then Esc land before the next frame: K is the key that ended the capture, Esc cannot steal it
    l.ev(InputEvent::Key { vk: 0x4B, pressed: true, repeat: false }); l.ev(InputEvent::Key { vk: vk::ESCAPE, pressed: true, repeat: false });
    l.frames(2);
    assert_eq!(l.working().input.bindings.get(A_JUMP)[0].name(), "K");
    assert_eq!(l.stack(), ["pause", "settings"], "the Esc that arrived during the capture is not a Back");
}
#[test] fn double_click_on_the_listening_button_does_not_bind_the_left_mouse_button() {
    let mut l = input_tab();
    let r = l.must("Space Bar"); let (x, y) = (r.x + r.w * 0.5, r.y + r.h * 0.5);
    l.mv(x, y); l.button(MouseButton::Left, true); l.button(MouseButton::Left, false);
    assert!(l.listening());
    l.button(MouseButton::Left, true); l.button(MouseButton::Left, false);
    assert!(l.listening(), "the accidental second click of a double click is ignored");
    assert_eq!(l.working().input.bindings.get(A_JUMP)[0].name(), "SpaceBar");
    l.advance(0.5);
    l.click_at(x, y);
    assert_eq!(l.working().input.bindings.get(A_JUMP)[0].name(), "LeftMouseButton", "a deliberate click binds the left button");
}
#[test] fn capture_times_out_and_a_click_elsewhere_cancels_it() {
    let mut l = input_tab();
    l.click_text("Space Bar"); assert!(l.listening());
    l.advance(11.);
    assert!(!l.listening(), "gives up after the timeout (pad users cannot press Esc)");
    l.click_text("Space Bar"); assert!(l.listening());
    l.click_at(40., 40.);
    assert!(!l.listening(), "clicking elsewhere cancels");
    assert_eq!(l.working().input.bindings.get(A_JUMP)[0].name(), "SpaceBar");
}

// ---- rapid input safety --------------------------------------------------------------------------------------------------
#[test] fn event_floods_are_bounded_and_harmless() {
    let mut l = Live::new(W, H);
    l.open_settings(); l.tab("Audio");
    for i in 0..50_000 { l.ev(InputEvent::PointerMoved { x: (i % 1900) as f32, y: (i % 1000) as f32 }); }
    for _ in 0..20_000 { l.ev(InputEvent::Wheel { dx: 0., dy: 1., unit: WheelUnit::Lines }); l.ev(InputEvent::Wheel { dx: 0., dy: -1., unit: WheelUnit::Lines }); }
    for _ in 0..10_000 { l.ev(InputEvent::Key { vk: vk::DOWN, pressed: true, repeat: true }); }
    assert!(l.m.state().pending_events() <= MAX_QUEUED_EVENTS, "{}", l.m.state().pending_events());
    l.frames(3);
    assert_eq!(l.m.state().pending_events(), 0);
    assert_eq!(l.stack(), ["pause", "settings"]);
    for _ in 0..200 { l.ev(InputEvent::Key { vk: vk::ESCAPE, pressed: true, repeat: true }); }
    l.frames(2);
    assert_eq!(l.stack(), ["pause", "settings"], "repeats of a key that was never pressed here do not act");
    // garbage values never panic
    for v in [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30] { l.ev(InputEvent::PointerMoved { x: v, y: v }); l.ev(InputEvent::Wheel { dx: v, dy: v, unit: WheelUnit::Pixels }); l.ev(InputEvent::PadAxis { axis: PadAxis::LeftX, value: v }); }
    l.frames(3);
}
#[test] fn out_of_order_events_are_ignored() {
    let mut l = Live::new(W, H);
    l.open_settings(); l.tab("Audio");
    l.button(MouseButton::Left, false); l.key_up(vk::RETURN); l.key_up(vk::ESCAPE); l.frames(2);
    assert_eq!(l.stack(), ["pause", "settings"]);
    // a press before any pointer position is ignored, the later release too
    let mut l = Live::new(W, H);
    l.ev(InputEvent::PointerButton { button: MouseButton::Left, pressed: true }); l.ev(InputEvent::PointerButton { button: MouseButton::Left, pressed: false }); l.frames(2);
    assert!(!l.m.is_open());
    // two presses without a release: one press
    let r = { let mut l2 = Live::new(W, H); l2.open_settings(); l2.tab("Audio"); l2 };
    let mut l = r;
    let (tx, tw, ty) = l.slider_track("Music Volume");
    l.mv(tx + tw * 0.2, ty);
    l.ev(InputEvent::PointerButton { button: MouseButton::Left, pressed: true }); l.ev(InputEvent::PointerButton { button: MouseButton::Left, pressed: true }); l.frame();
    l.button(MouseButton::Left, false); l.frames(2);
    assert!((l.working().audio.music_volume - 0.2).abs() < 0.03);
}
#[test] fn rapid_tab_switching_and_clicking_never_leaves_stuck_state() {
    let mut l = Live::new(W, H);
    l.open_settings();
    for i in 0..60 {
        let title = ["Video", "Game", "Brightness", "Audio", "Input", "Controller", "Live Streaming"][i % 7];
        let r = l.rect_of(title);
        if let Some(r) = r { l.ev(InputEvent::PointerMoved { x: r.x + r.w * 0.5, y: r.y + r.h * 0.5 }); l.ev(InputEvent::PointerButton { button: MouseButton::Left, pressed: true }); l.ev(InputEvent::PointerButton { button: MouseButton::Left, pressed: false }); }
        l.ev(InputEvent::Key { vk: vk::DOWN, pressed: true, repeat: false }); l.ev(InputEvent::Key { vk: vk::DOWN, pressed: false, repeat: false });
        l.frame();
    }
    l.frames(3);
    assert!(l.m.is_open() && l.stack().len() == 2, "{:?}", l.stack());
    assert!(!l.listening() && !l.m.state().dropdown_open());
    l.press(vk::ESCAPE); l.press(vk::ESCAPE);
    assert!(!l.m.is_open(), "two Esc presses always get out");
}
#[test] fn hover_follows_the_pointer_and_clears_when_it_leaves() {
    let mut l = Live::new(W, H);
    l.open_settings();
    assert!(l.has("Back"));
    let r = l.must("Back");
    assert!(l.m.state().hot_id().is_none());
    l.mv(r.x + 3., r.y + r.h * 0.5);
    assert!(l.m.state().hot_id().is_some(), "Back is hot under the pointer");
    l.ev(InputEvent::PointerLeft); l.frames(2);
    assert!(l.m.state().hot_id().is_none(), "nothing is hot after the pointer left the window");
    // hot is not pressed: press activates `active`, release clears it
    l.mv(r.x + 3., r.y + r.h * 0.5);
    l.button(MouseButton::Left, true);
    assert!(l.m.state().active_id().is_some(), "pressed");
    l.button(MouseButton::Left, false);
    assert!(l.m.state().active_id().is_none());
}
#[test] fn menu_open_picks_up_the_pointer_that_was_already_over_a_button() {
    let mut l = Live::new(W, H);
    let r = mainmenu::entry_rect(W, H, MenuContext::Match, "SettingsButton").unwrap();
    l.ev(InputEvent::PointerMoved { x: r.x + r.w * 0.5, y: r.y + r.h * 0.5 }); l.frame();
    l.key_down(vk::ESCAPE); l.key_up(vk::ESCAPE); l.frames(3);
    assert!(l.m.state().hot_id().is_some(), "hover appears without having to wiggle the mouse");
}
#[test] fn resize_while_open_keeps_the_screens_working() {
    let mut l = Live::new(W, H);
    l.open_settings();
    for (w, h) in [(1280., 720.), (800., 600.), (3440., 1440.), (640., 360.), (1920., 1080.)] {
        l.w = w; l.h = h; l.frames(3);
        assert!(l.m.is_open() && l.stack().len() == 2);
    }
    l.press(vk::ESCAPE);
    assert_eq!(l.stack(), ["pause"]);
}
