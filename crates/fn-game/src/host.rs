//! Window/focus/cursor policy and small input helpers the old main.rs kept next to its Win32 code. All pure functions: the front-end
//! (`fn-app`) feeds them facts (is the window foreground, did the capture click release, ...) and acts on the answers through
//! `fn_settings::PlatformHost`.
use std::sync::atomic::{AtomicBool, Ordering};

/// The game window owns input only while it is the foreground window (`window` 0 = no window, never focused).
pub fn window_focused(foreground: isize, window: isize) -> bool { window != 0 && foreground == window }
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FocusChange { Unchanged, Gained, Lost }
pub fn focus_change(was: bool, now: bool) -> FocusChange { match (was, now) { (true, false) => FocusChange::Lost, (false, true) => FocusChange::Gained, _ => FocusChange::Unchanged } }
/// What the front-end must do after re-deriving focus from the real foreground window (the old `App::sync_focus`, run once per frame; WM_ACTIVATE
/// alone is not trusted, it can be missed across the splash -> loading -> game window switch).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct FocusActions {
    pub change: FocusChange,
    /// New value of the app's `active` flag.
    pub active: bool,
    /// Gate for raw key polling (`gated_key`): true only while the game window is foreground.
    pub input_focused: bool,
    /// Release the cursor clip and show the arrow (focus lost, or "belt and braces": never stay clipped while not foreground).
    pub release_mouse: bool,
    /// Tell the pause menu it lost focus (`menu::host::focus_lost`): only on the Lost transition.
    pub menu_focus_lost: bool,
}
/// `was_active`: the app's current flag; `now`: `window_focused(GetForegroundWindow(), window)`; `mouse_locked`: the cursor is currently clipped.
pub fn focus_step(was_active: bool, now: bool, mouse_locked: bool) -> FocusActions {
    let change = focus_change(was_active, now);
    match change {
        FocusChange::Lost => FocusActions { change, active: false, input_focused: now, release_mouse: true, menu_focus_lost: true },
        FocusChange::Gained => FocusActions { change, active: true, input_focused: now, release_mouse: false, menu_focus_lost: false },
        FocusChange::Unchanged => FocusActions { change, active: was_active, input_focused: now, release_mouse: !now && mouse_locked, menu_focus_lost: false },
    }
}
/// The mouse may only be captured (clipped + hidden) while the window is really foreground, gameplay has started and this is not a scripted run.
pub fn may_capture(focused: bool, started: bool, scripted: bool) -> bool { focused && started && !scripted }
/// Raw key read gated on focus: `raw` is not even called while another app is in front, so its keys can never leak into the game.
pub fn gated_key(focused: bool, raw: impl FnOnce() -> bool) -> bool { focused && raw() }
/// Gameplay capture point: raw-input look, so the hidden cursor is pinned to the client centre (left, top, right, bottom in screen pixels).
/// None for an empty/minimised client rect.
pub fn capture_center(r: (i32, i32, i32, i32)) -> Option<(i32, i32)> { if r.2 <= r.0 || r.3 <= r.1 { None } else { Some((r.0 + (r.2 - r.0) / 2, r.1 + (r.3 - r.1) / 2)) } }
/// `HTCLIENT` hit-test code (Win32), the only region where the captured cursor is hidden.
pub const HT_CLIENT: u32 = 1;
/// Cursor policy over the client area: hidden only while captured; menus/splash/loading use the stock arrow.
pub fn cursor_hidden(captured: bool, hit_test: u32) -> bool { captured && hit_test == HT_CLIENT }
/// Attack input = captured && fire down && not the still-held capture click. `capture_click_held` is set when the mouse is (re)captured: the
/// click that captured it must not also count as an attack; it is cleared the first time the fire button is seen released afterwards.
pub fn gated_attack(locked: bool, capture_click_held: &AtomicBool, fire_down: bool) -> bool {
    if !fire_down { capture_click_held.store(false, Ordering::Relaxed); }
    locked && fire_down && !capture_click_held.load(Ordering::Relaxed)
}
/// Edge-triggered toggle: flips `visible` on the frame the key goes down, not while it is held (the F8 help overlay; F1 is HotbarKeyF1 in 1.8, CookedIniVersion.txt L2677).
pub fn help_toggle(was_down: bool, down: bool, visible: bool) -> bool { if down && !was_down { !visible } else { visible } }
/// Look with the 1.8 default `MouseSensitivity` 0.07 at the 80 deg hold FOV (`fn_movement::mouse_degrees_per_count`: 0.1556 deg/count yaw,
/// 0.1089 pitch). Gameplay uses `fn_settings::InputLive::look` with the live camera FOV.
pub fn mouse_look(yaw: f32, pitch: f32, dx: i32, dy: i32) -> (f32, f32) {
    let (ky, kp) = fn_movement::mouse_degrees_per_count(fn_movement::MOUSE_SENSITIVITY_DEFAULT, fn_camera::MELEE.fov);
    ((yaw + dx as f32 * ky).rem_euclid(360.), fn_camera::clamp_pitch(pitch - dy as f32 * kp))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn help_panel_toggles_on_f1_press_edge_only() {
        let mut v = false; let mut was = false;
        for down in [false, true, true, true, false] { v = help_toggle(was, down, v); was = down; }
        assert!(v, "one press shows it; holding does not flicker");
        for down in [true, true, false] { v = help_toggle(was, down, v); was = down; }
        assert!(!v, "second press hides it");
    }
    #[test] fn capture_point_is_client_centre_for_all_aspects_and_sizes() {
        for (w, h) in [(1280, 800), (1920, 1080), (2560, 1080), (1024, 768), (640, 360), (3840, 2160), (1, 1)] {
            let (ox, oy) = (-1920 + 37, 53); let (cx, cy) = capture_center((ox, oy, ox + w, oy + h)).unwrap();
            assert!(cx >= ox && cx < ox + w && cy >= oy && cy < oy + h); assert!((cx - ox - w / 2).abs() <= 1 && (cy - oy - h / 2).abs() <= 1);
        }
    }
    #[test] fn capture_click_is_not_an_attack_until_the_button_is_released_once() {
        let held = AtomicBool::new(false);
        assert!(!gated_attack(false, &held, true), "not captured: never an attack");
        held.store(true, Ordering::Relaxed); // the click that captured the mouse
        assert!(!gated_attack(true, &held, true), "capture click still down");
        assert!(!gated_attack(true, &held, true), "still held several frames later");
        assert!(!gated_attack(true, &held, false) && !held.load(Ordering::Relaxed), "release clears the guard");
        assert!(gated_attack(true, &held, true), "the next press attacks");
        assert!(gated_attack(true, &held, true), "holding keeps attacking (combo hold)");
    }
    #[test] fn focus_requires_the_game_window_to_be_the_foreground_window() {
        assert!(window_focused(0x10, 0x10));
        assert!(!window_focused(0x20, 0x10), "another app (or the splash HWND) is in front");
        assert!(!window_focused(0, 0x10), "no foreground window"); assert!(!window_focused(0, 0), "null game window is never focused");
    }
    #[test] fn focus_transitions_detect_a_missed_activate() {
        assert_eq!(focus_change(true, false), FocusChange::Lost); assert_eq!(focus_change(false, true), FocusChange::Gained);
        assert_eq!(focus_change(true, true), FocusChange::Unchanged); assert_eq!(focus_change(false, false), FocusChange::Unchanged);
    }
    #[test] fn focus_step_releases_the_mouse_on_loss_and_never_stays_clipped_unfocused() {
        let lost = focus_step(true, false, true); assert_eq!(lost, FocusActions { change: FocusChange::Lost, active: false, input_focused: false, release_mouse: true, menu_focus_lost: true });
        let gained = focus_step(false, true, false); assert!(gained.active && gained.input_focused && !gained.release_mouse && !gained.menu_focus_lost);
        let steady = focus_step(true, true, true); assert!(steady.active && !steady.release_mouse && steady.change == FocusChange::Unchanged);
        let belt = focus_step(false, false, true); assert!(belt.release_mouse && !belt.menu_focus_lost && !belt.active, "unfocused but still clipped: release");
        assert!(!focus_step(false, false, false).release_mouse);
    }
    #[test] fn mouse_is_never_captured_without_focus_or_during_scripted_runs() {
        assert!(may_capture(true, true, false));
        assert!(!may_capture(false, true, false), "not foreground"); assert!(!may_capture(true, false, false), "still loading"); assert!(!may_capture(true, true, true), "smoke test");
    }
    #[test] fn keys_are_not_even_polled_while_unfocused() {
        let mut polled = false; assert!(!gated_key(false, || { polled = true; true })); assert!(!polled, "raw key polling must not run for another app's keys");
        assert!(gated_key(true, || true)); assert!(!gated_key(true, || false));
    }
    #[test] fn capture_refused_for_empty_or_minimised_client() { assert_eq!(capture_center((10, 10, 10, 50)), None); assert_eq!(capture_center((0, 0, 0, 0)), None); assert_eq!(capture_center((5, 5, 4, 4)), None); }
    #[test] fn cursor_hidden_only_when_captured_over_client() {
        assert!(cursor_hidden(true, HT_CLIENT)); assert!(!cursor_hidden(false, HT_CLIENT));
        assert!(!cursor_hidden(true, 2 /* HTCAPTION */)); assert!(!cursor_hidden(true, 17 /* HTBOTTOMRIGHT */));
    }
    #[test] fn menu_open_cursor_is_the_free_os_arrow() {
        for locked in [false, true] { assert!(!cursor_hidden(locked && !true, HT_CLIENT), "menu open: arrow visible (locked={locked})"); }
        assert!(cursor_hidden(true && !false, HT_CLIENT), "gameplay capture still hides it over the client");
        assert!(!cursor_hidden(false, HT_CLIENT), "released = arrow");
    }
    #[test] fn raw_mouse_look_wraps_and_clamps() { let k = 0.07 * 2.5 * 80. * 0.01111; let (y, p) = mouse_look(359., 0., 20, 2000); assert!((y - (359. + 20. * k - 360.)).abs() < 1e-4 && p == fn_camera::PITCH_MIN); let (y, p) = mouse_look(0., 0., -10, -2000); assert!((y - (360. - 10. * k)).abs() < 1e-4 && p == fn_camera::PITCH_MAX); }
    #[test] fn mouse_look_is_additive_so_batching_cannot_lose_or_double_count() { let mut a = (10., 0.); for _ in 0..100 { a = mouse_look(a.0, a.1, 3, 1); } let b = mouse_look(10., 0., 300, 100); assert!((a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3); }
    #[test] fn mouse_yaw_never_leaves_0_360() { let mut y = 0.; for i in 0..10000 { y = mouse_look(y, 0., if i % 2 == 0 { -7 } else { 3 }, 0).0; assert!((0.0..360.).contains(&y)); } }
    #[test] fn pitch_is_clamped_short_of_gimbal_flip() { let (_, p) = mouse_look(0., 0., 0, -100000); assert!(p < 90. && p == fn_camera::PITCH_MAX); let (_, p) = mouse_look(0., 0., 0, 100000); assert_eq!(p, fn_camera::PITCH_MIN); }
}
