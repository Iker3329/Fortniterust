//! Keyboard / mouse / gamepad state and the focus + capture policy of the old runtime (`main.rs`: `INPUT_FOCUSED`, `USER_RELEASED`,
//! `CAPTURE_CLICK_HELD`, `gated_key`, `gated_attack`, `may_capture`), turned into one [`fn_game::FrameInput`] per frame.
//!
//! Policy (all of it is the pure functions of `fn_game::host`, the old behaviour pinned by their tests):
//! * keys are only read while the window has focus (`gated_key`): losing focus clears every held key, so nothing sticks and no other app's keys leak in;
//! * the mouse is captured (hidden, confined, raw motion) only while focused, after loading, outside scripted runs (`may_capture`);
//! * Esc opens the menu and releases the mouse; closing the menu re-captures it; a click while released re-captures it **without** swinging
//!   (`gated_attack`: the capture click must be released once before the fire binding counts again);
//! * raw mouse motion (winit `DeviceEvent::MouseMotion`, delta counts) drives the look through `InputLive` (sensitivity, Invert View);
//!   with the mouse free, right-button drag looks like the old prototype.
use crate::keys::*;
use fn_game::host;
use fn_game::inputdev::{pad, wheel};
use fn_game::FrameInput;
use fn_settings::{Bindings, InputLive, A_CROUCH, A_FIRE, A_JUMP, A_MOVE_BACKWARD, A_MOVE_FORWARD, A_MOVE_LEFT, A_MOVE_RIGHT, A_PRIMARY_QUICKBAR_SLOT1, A_RELOAD, A_SPRINT};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct InputState {
    down: [bool; 256],
    /// The game window has keyboard/mouse focus (winit `Focused`).
    pub focused: bool,
    /// The mouse is captured by the game (see module docs).
    pub captured: bool,
    /// Esc released the mouse: no automatic re-capture until the next click (or the menu closing).
    pub user_released: bool,
    /// Set when the mouse is (re)captured: that click must not also swing the pickaxe.
    pub capture_click_held: AtomicBool,
    mouse: (f64, f64),
    pub cursor: (f32, f32),
    drag_last: Option<(f32, f32)>,
    f1_was_down: bool,
    reload_was_down: bool,
    pub help_visible: bool,
    pub shift: bool,
    pub ctrl: bool,
    /// Gameplay has started (loading finished): captures are refused before.
    pub started: bool,
    /// Scripted / diagnostic run: never captures, ignores real devices.
    pub scripted: bool,
    pad_keys: Vec<u16>,
    pad_actions: Vec<usize>,
    pub pad_mapper: fn_game::pad_layout::PadMapper,
    pub pad_last_weapon: usize,
    pub pad_connected: bool,
}

impl Default for InputState { fn default() -> Self { Self::new() } }
impl InputState {
    pub fn new() -> Self {
        InputState { down: [false; 256], focused: true, captured: false, user_released: false, capture_click_held: AtomicBool::new(false), mouse: (0., 0.), cursor: (0., 0.), drag_last: None,
            f1_was_down: false, reload_was_down: false, help_visible: false, shift: false, ctrl: false, started: false, scripted: false, pad_keys: Vec::new(), pad_actions: Vec::new(), pad_mapper: Default::default(), pad_last_weapon: 0, pad_connected: false }
    }

    /// Key state from a window event. Presses that arrive while the window is not focused are ignored (the old `gated_key`): they could only be stale, and
    /// would otherwise appear as held keys the moment focus returns. Releases always go through.
    pub fn set_key(&mut self, vk: u16, pressed: bool) { if pressed && !self.focused { return; } if let Some(k) = self.down.get_mut(vk as usize) { *k = pressed; } }
    pub fn raw_down(&self, vk: u16) -> bool { self.down.get(vk as usize).copied().unwrap_or(false) }
    /// Binding poll: a wheel pseudo-key answers from the notches of this frame, anything else from the held-key table; never while unfocused.
    pub fn poll(&self, code: u16) -> bool { host::gated_key(self.focused, || self.pad_keys.contains(&code) || wheel::poll(code).unwrap_or_else(|| self.raw_down(code))) }
    pub fn set_pad_actions(&mut self, b: &Bindings, actions: &[usize]) {
        self.pad_actions.clear(); self.pad_actions.extend_from_slice(actions);
        self.pad_keys.clear(); self.pad_keys.extend(actions.iter().filter_map(|&a| b.get(a).iter().find_map(|k| k.poll_code())));
    }
    pub fn pad_action(&self, action: usize) -> bool { self.focused && self.pad_actions.contains(&action) }
    pub fn push_wheel(&mut self, delta_notches: f32) { wheel::push((delta_notches * 120.) as i32); }
    /// Latch the wheel notches seen since the previous frame (call once per game frame, before polling).
    pub fn begin_frame(&mut self) { wheel::begin_frame(); }

    /// Raw mouse counts (winit `DeviceEvent::MouseMotion`): only while captured and focused, like the old WM_INPUT arm.
    pub fn raw_motion(&mut self, dx: f64, dy: f64) { if self.captured && self.focused && !self.scripted { self.mouse.0 += dx; self.mouse.1 += dy; } }
    /// Cursor position in client pixels (menu hover, drag look).
    pub fn cursor_moved(&mut self, x: f32, y: f32, menu_open: bool) {
        self.cursor = (x, y);
        if self.raw_down(VK_RBUTTON) && !self.captured && !menu_open && self.focused && !self.scripted {
            if let Some((lx, ly)) = self.drag_last { self.mouse.0 += (x - lx) as f64; self.mouse.1 += (y - ly) as f64; }
            self.drag_last = Some((x, y));
        }
    }
    pub fn right_button(&mut self, pressed: bool) { self.drag_last = if pressed { Some(self.cursor) } else { None }; }
    pub fn take_mouse(&mut self) -> (i32, i32) { let m = (self.mouse.0.round() as i32, self.mouse.1.round() as i32); self.mouse = (self.mouse.0 - m.0 as f64, self.mouse.1 - m.1 as f64); m }

    /// Window lost focus (or was minimised): nothing may stay held, the capture is dropped by the caller.
    pub fn focus_lost(&mut self) {
        self.focused = false; self.down = [false; 256]; self.mouse = (0., 0.); self.drag_last = None; self.shift = false; self.ctrl = false;
        self.capture_click_held.store(false, Ordering::Relaxed);
        self.pad_keys.clear(); self.pad_actions.clear();
    }
    pub fn focus_gained(&mut self) { self.focused = true; }

    /// May the game capture the mouse right now (old `may_capture(window_focused, started, smoke)`).
    pub fn may_capture(&self) -> bool { host::may_capture(self.focused, self.started, self.scripted) }
    /// Call when the capture actually succeeded: the click that did it must not swing.
    pub fn mark_captured(&mut self) { if !self.captured { self.capture_click_held.store(true, Ordering::Relaxed); } self.captured = true; }
    pub fn mark_released(&mut self) { self.captured = false; self.drag_last = None; self.mouse = (0., 0.); }

    /// Dev help card (edge triggered) on F8, not F1: in 1.8 F1 is only `HotbarKeyF1` (wall) / `PickerNull` (ConfigD_Athena ActionMappings:50 / :89,
    /// CookedIniVersion.txt L2677 / L2716); F8 is unbound in 1.8 (see keys::VK_F8). `f1_was_down` = the help key's previous state.
    pub fn update_help(&mut self) { let down = self.poll(VK_F8); self.help_visible = host::help_toggle(self.f1_was_down, down, self.help_visible); self.f1_was_down = down; }

    /// Rising edge of the Reload binding (default R) since the last call.
    pub fn reload_edge(&mut self, b: &Bindings) -> bool { let down = b.down(A_RELOAD, |k| self.poll(k)); let edge = down && !self.reload_was_down; self.reload_was_down = down; edge }

    /// Quickbar slot keys: the last pressed slot among 1..6 wins (old loop).
    pub fn quickbar_press(&self, b: &Bindings) -> Option<usize> { (0..6).filter(|i| b.down(A_PRIMARY_QUICKBAR_SLOT1 + i, |k| self.poll(k))).last() }

    /// INVENTORY HOOK: the SlotUp / SlotDown bindings (default mouse wheel up / down) this frame: +1 = SlotUp, -1 = SlotDown, 0 = neither (both = cancel out).
    pub fn slot_wheel(&self, b: &Bindings) -> i32 { b.down(fn_settings::A_SLOT_UP, |k| self.poll(k)) as i32 - b.down(fn_settings::A_SLOT_DOWN, |k| self.poll(k)) as i32 }

    /// The old `App::update` input gathering (without the scripted additions): bindings, mouse look, gamepad. `fov` = the camera FOV the
    /// exe's mouse term uses (`GetFOVAngle()` of the last view, see `fn_movement::mouse_degrees_per_count`); 0 = unknown (no FOV term).
    /// The arrow keys are NOT look keys in 1.8: they are the second keys of MoveForward / MoveRight (`InputSettings:AxisMappings`,
    /// CookedIniVersion.txt L1989/L1990/L1994/L1995) and reach the movement through the binding table.
    pub fn gather(&mut self, b: &Bindings, live: &InputLive, pad_frame: &pad::Frame, dt: f32, fov: f32) -> FrameInput {
        let (dx, dy) = self.take_mouse();
        let key = |k: u16| self.poll(k);
        let held = |action: usize| b.down(action, key) as i32;
        let (kf, ks) = ((held(A_MOVE_FORWARD) - held(A_MOVE_BACKWARD)) as f32, (held(A_MOVE_RIGHT) - held(A_MOVE_LEFT)) as f32);
        let attack = pad_frame.attack || host::gated_attack(self.captured, &self.capture_click_held, b.down(A_FIRE, key));
        // mouse look only: the right stick goes through `crate::pad::PadLook` (its own targeting multipliers, FUN_140615dd0), added by the app
        let _ = dt;
        let (yaw, pitch) = live.look_delta(dx, dy, fov);
        FrameInput {
            forward: (kf + pad_frame.forward).clamp(-1., 1.), side: (ks + pad_frame.side).clamp(-1., 1.),
            jump: b.down(A_JUMP, key) || pad_frame.jump, crouch_key: b.down(A_CROUCH, key), sprint_key: b.down(A_SPRINT, key) || pad_frame.sprint, attack,
            yaw_delta: yaw, pitch_delta: pitch, reset: key(VK_R), ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_settings::{Settings, A_PRIMARY_QUICKBAR_SLOT1};
    fn setup() -> (InputState, Bindings, InputLive) { let s = Settings::default(); let mut i = InputState::new(); i.started = true; (i, s.input.bindings.clone(), InputLive::from_settings(&s)) }
    fn gather(i: &mut InputState, b: &Bindings, l: &InputLive) -> FrameInput { i.gather(b, l, &pad::Frame::default(), 1. / 60., 0.) }

    #[test] fn default_bindings_move_jump_crouch_sprint() {
        let (mut i, b, l) = setup();
        i.set_key(0x57, true); i.set_key(0x44, true); i.set_key(0x20, true); i.set_key(0xA2, true); i.set_key(0xA0, true);
        let f = gather(&mut i, &b, &l);
        assert_eq!((f.forward, f.side), (1., 1.)); assert!(f.jump && f.crouch_key && f.sprint_key && !f.attack);
        i.set_key(0x53, true); assert_eq!(gather(&mut i, &b, &l).forward, 0., "W+S cancel");
    }
    #[test] fn unfocused_window_reads_no_keys_and_focus_loss_clears_held_keys() {
        let (mut i, b, l) = setup();
        i.set_key(0x57, true); assert_eq!(gather(&mut i, &b, &l).forward, 1.);
        i.focus_lost(); assert_eq!(gather(&mut i, &b, &l).forward, 0.);
        i.focus_gained(); assert_eq!(gather(&mut i, &b, &l).forward, 0., "the held W was dropped, not resumed");
        i.set_key(0x57, true); i.focused = false; assert_eq!(gather(&mut i, &b, &l).forward, 0., "events while unfocused are ignored by the gate");
    }
    #[test] fn capture_policy_matches_the_old_runtime() {
        let (mut i, _, _) = setup();
        assert!(i.may_capture()); i.focused = false; assert!(!i.may_capture()); i.focused = true; i.started = false; assert!(!i.may_capture()); i.started = true; i.scripted = true; assert!(!i.may_capture());
    }
    #[test] fn the_capture_click_does_not_swing_but_the_next_press_does() {
        let (mut i, b, l) = setup();
        i.set_key(VK_LBUTTON, true); assert!(!gather(&mut i, &b, &l).attack, "not captured: no attack");
        i.mark_captured(); // the click that captured the mouse is still down
        assert!(!gather(&mut i, &b, &l).attack); assert!(!gather(&mut i, &b, &l).attack, "still held");
        i.set_key(VK_LBUTTON, false); assert!(!gather(&mut i, &b, &l).attack);
        i.set_key(VK_LBUTTON, true); assert!(gather(&mut i, &b, &l).attack, "a fresh press swings"); assert!(gather(&mut i, &b, &l).attack, "holding keeps swinging");
        i.mark_released(); assert!(!gather(&mut i, &b, &l).attack, "released mouse never attacks");
    }
    #[test] fn raw_motion_only_counts_while_captured_and_focused() {
        let (mut i, b, l) = setup();
        i.raw_motion(10., 4.); assert_eq!(gather(&mut i, &b, &l).yaw_delta, 0.);
        i.mark_captured(); i.raw_motion(10., 4.);
        let f = gather(&mut i, &b, &l);
        assert!((f.yaw_delta - 10. * l.deg_x).abs() < 1e-5 && (f.pitch_delta + 4. * l.deg_y).abs() < 1e-5, "{f:?}");
        i.raw_motion(5., 0.); i.focus_lost(); assert_eq!(gather(&mut i, &b, &l).yaw_delta, 0., "focus loss drops unread motion");
    }
    #[test] fn invert_view_flips_pitch_only() {
        let (mut i, b, mut l) = setup(); i.mark_captured();
        i.raw_motion(0., 10.); let a = gather(&mut i, &b, &l).pitch_delta;
        l.invert_y = true; i.raw_motion(0., 10.); let c = gather(&mut i, &b, &l).pitch_delta;
        assert!(a < 0. && (a + c).abs() < 1e-6);
    }
    #[test] fn fractional_counts_are_carried_not_lost() { let mut i = InputState::new(); i.mark_captured(); for _ in 0..4 { i.raw_motion(0.5, 0.); } let (dx, _) = i.take_mouse(); let (dx2, _) = i.take_mouse(); assert_eq!(dx + dx2, 2); }
    #[test] fn right_button_drag_looks_only_while_the_mouse_is_free() {
        let (mut i, b, l) = setup();
        i.cursor_moved(100., 100., false); i.set_key(VK_RBUTTON, true); i.right_button(true);
        i.cursor_moved(110., 100., false); assert!(gather(&mut i, &b, &l).yaw_delta > 0.);
        i.cursor_moved(120., 100., true); assert_eq!(gather(&mut i, &b, &l).yaw_delta, 0., "menu open: no look");
        i.right_button(false); i.set_key(VK_RBUTTON, false); i.cursor_moved(200., 100., false); assert_eq!(gather(&mut i, &b, &l).yaw_delta, 0.);
    }
    /// 1.8: arrows are the MoveForward / MoveRight second keys (CookedIniVersion.txt L1989-L1995), never look keys.
    #[test] fn arrows_move_and_r_resets() {
        let (mut i, b, l) = setup();
        i.set_key(VK_RIGHT, true); i.set_key(VK_UP, true); i.set_key(VK_R, true);
        let f = gather(&mut i, &b, &l); assert!(f.yaw_delta == 0. && f.pitch_delta == 0. && f.forward == 1. && f.side == 1. && f.reset, "{f:?}");
        i.set_key(VK_RIGHT, false); i.set_key(VK_UP, false); i.set_key(VK_DOWN, true); i.set_key(VK_LEFT, true);
        let f = gather(&mut i, &b, &l); assert!(f.forward == -1. && f.side == -1.);
    }
    /// The 1.8 mouse chain at the 80 deg hold FOV: 100 counts right = 100 x 0.07 x 2.5 x 80 x 0.01111 = 15.55 deg; 100 counts up = 10.89 deg up.
    #[test] fn mouse_counts_turn_like_1_8_at_the_hold_fov() {
        let (mut i, b, l) = setup(); i.mark_captured();
        i.raw_motion(100., -100.);
        let f = i.gather(&b, &l, &pad::Frame::default(), 1. / 60., 80.);
        assert!((f.yaw_delta - 15.554).abs() < 1e-2 && (f.pitch_delta - 10.888).abs() < 1e-2, "{f:?}");
    }
    #[test] fn pad_adds_to_keys_and_clamps() {
        let (mut i, b, l) = setup(); i.set_key(0x57, true);
        let p = pad::Frame { forward: 0.7, side: -0.4, look: (32767, 0), jump: true, attack: true, ..Default::default() };
        let f = i.gather(&b, &l, &p, 0.5, 0.);
        assert_eq!(f.forward, 1.); assert_eq!(f.side, -0.4); assert!(f.yaw_delta == 0. && f.jump && f.attack, "the stick look is added by PadLook, not here");
    }
    #[test] fn quickbar_slots_and_help_toggle() {
        let (mut i, b, _) = setup();
        assert_eq!(i.quickbar_press(&b), None); i.set_key(0x31, true); assert_eq!(i.quickbar_press(&b), Some(0)); i.set_key(0x33, true); assert_eq!(i.quickbar_press(&b), Some(2));
        let _ = A_PRIMARY_QUICKBAR_SLOT1;
        i.set_key(VK_F8, true); i.update_help(); i.update_help(); assert!(i.help_visible, "held F8 toggles once"); i.set_key(VK_F8, false); i.update_help(); i.set_key(VK_F8, true); i.update_help(); assert!(!i.help_visible);
        // F1 is HotbarKeyF1 (wall) only in 1.8 (CookedIniVersion.txt L2677): it never toggles the dev help card
        i.set_key(VK_F8, false); i.update_help(); i.set_key(VK_F1, true); i.update_help(); assert!(!i.help_visible, "F1 must not toggle the help card");
    }
}

