//! Raw, windowing-system-agnostic input -> toolkit [`Event`]s.
//!
//! `fn-ui` never sees winit (or Win32): the host translates its own events into [`InputEvent`] (pointer move / button / wheel, key down / up
//! with the Windows virtual-key code and an auto-repeat flag, gamepad buttons and sticks, focus changes) and feeds them to an
//! [`InputRouter`] (owned by `MenuSystem`, see `MenuSystem::input`). The router
//!
//! * tracks the held keys, the left/right modifiers, the pressed mouse buttons and the pad state itself, so it can be robust against what real
//!   window systems do: key auto-repeat (Esc / Enter / Backspace never repeat their action), lost key-ups and double key-downs (a key
//!   that is already held is a repeat), releases without a press, a pointer that leaves the window while a button is down (the leave is
//!   deferred until the release, so a slider drag outside the window keeps working), focus loss (everything is released, the UI cancels its
//!   drags / key capture / popups), event floods (consecutive moves and wheel deltas coalesce);
//! * resolves keyboard navigation (arrows, Tab / Shift+Tab, Ctrl+Tab / Ctrl+PageUp/PageDown, Enter / Space, Esc = back, Home / End,
//!   Backspace / Delete = clear) and forwards every non-repeat key as `Event::RawKey` for the key-capture mode ("PRESS ANY KEY...": left and
//!   right Shift / Ctrl / Alt are separate keys; mouse buttons and the wheel arrive as their own events);
//! * turns pad buttons into immediate press edges, the D-pad and the left stick into repeating directions (initial delay + rate, with
//!   hysteresis and a dominant axis) and the right stick into wheel scrolling.
//!
//! It is text-free on purpose: the menu has no text fields.
use crate::input::{self, vk, Event, MouseButton, NavKey, PadButton};

/// Unit of a wheel delta.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WheelUnit {
    /// Notches (mouse wheel "lines").
    Lines,
    /// Device pixels (touchpads): about [`PIXELS_PER_NOTCH`] make one notch.
    Pixels,
}
/// Pixel deltas per wheel notch.
pub const PIXELS_PER_NOTCH: f32 = 48.;

/// Gamepad axes. Sticks run -1..1 with y positive = pushed up / forward; triggers 0..1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PadAxis { LeftX, LeftY, RightX, RightY, LeftTrigger, RightTrigger }

/// Left / right modifier state.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Modifiers { pub lshift: bool, pub rshift: bool, pub lctrl: bool, pub rctrl: bool, pub lalt: bool, pub ralt: bool }
impl Modifiers {
    pub fn shift(&self) -> bool { self.lshift || self.rshift }
    pub fn ctrl(&self) -> bool { self.lctrl || self.rctrl }
    pub fn alt(&self) -> bool { self.lalt || self.ralt }
    pub fn any(&self) -> bool { self.shift() || self.ctrl() || self.alt() }
    fn slot(vkc: u16) -> Option<usize> {
        match vkc { vk::LSHIFT | vk::SHIFT => Some(0), vk::RSHIFT => Some(1), vk::LCONTROL | vk::CONTROL => Some(2), vk::RCONTROL => Some(3), vk::LMENU | vk::MENU => Some(4), vk::RMENU => Some(5), _ => None }
    }
    fn set(&mut self, slot: usize, on: bool) {
        match slot { 0 => self.lshift = on, 1 => self.rshift = on, 2 => self.lctrl = on, 3 => self.rctrl = on, 4 => self.lalt = on, _ => self.ralt = on }
    }
    fn get(&self, slot: usize) -> bool { [self.lshift, self.rshift, self.lctrl, self.rctrl, self.lalt, self.ralt][slot] }
    const KEYS: [u16; 6] = [vk::LSHIFT, vk::RSHIFT, vk::LCONTROL, vk::RCONTROL, vk::LMENU, vk::RMENU];
}

/// What a window system reports, reduced to what the UI needs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InputEvent {
    /// Pointer position in client pixels (top-left origin).
    PointerMoved { x: f32, y: f32 },
    /// The pointer left the client area.
    PointerLeft,
    /// A mouse button changed state at the last known pointer position.
    PointerButton { button: MouseButton, pressed: bool },
    /// Wheel / touchpad scroll; `dy` positive = away from the user (scroll up). `dx` is ignored (the menu has no horizontal lists).
    Wheel { dx: f32, dy: f32, unit: WheelUnit },
    /// Key press / release by Windows virtual-key code (left / right modifiers as `VK_LSHIFT`.. `VK_RMENU`). `repeat` = OS auto-repeat.
    Key { vk: u16, pressed: bool, repeat: bool },
    /// Re-sync of the modifier state (window systems report it on focus changes): a modifier that is held here but not in `Modifiers` is released.
    Modifiers(Modifiers),
    /// Gamepad button press / release.
    PadButton { button: PadButton, pressed: bool },
    /// Gamepad axis value (sticks -1..1, y up; triggers 0..1).
    PadAxis { axis: PadAxis, value: f32 },
    /// The window lost focus / was minimised: releases everything and cancels drags, key capture and popups.
    FocusLost,
    /// The window gained focus again (state is already clean after `FocusLost`).
    FocusGained,
}

/// What the host needs to know about an input event beyond the toolkit events (the menu swallows everything while it is open).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct InputOutcome {
    /// A fresh (non-repeat, not already held) Escape press: the host opens the menu with it when the menu is closed.
    pub escape_edge: bool,
    /// A fresh press of the pad Start button.
    pub start_edge: bool,
}

const DIR_MASKS: [(u16, PadButton); 4] = [(input::XI_DPAD_UP, PadButton::DPadUp), (input::XI_DPAD_DOWN, PadButton::DPadDown), (input::XI_DPAD_LEFT, PadButton::DPadLeft), (input::XI_DPAD_RIGHT, PadButton::DPadRight)];
fn pad_mask(b: PadButton) -> u16 {
    match b {
        PadButton::DPadUp => input::XI_DPAD_UP, PadButton::DPadDown => input::XI_DPAD_DOWN, PadButton::DPadLeft => input::XI_DPAD_LEFT, PadButton::DPadRight => input::XI_DPAD_RIGHT,
        PadButton::A => input::XI_A, PadButton::B => input::XI_B, PadButton::X => input::XI_X, PadButton::Y => input::XI_Y,
        PadButton::LB => input::XI_LB, PadButton::RB => input::XI_RB, PadButton::Start => input::XI_START, PadButton::Back => input::XI_BACK,
    }
}
fn mouse_slot(b: MouseButton) -> usize { match b { MouseButton::Left => 0, MouseButton::Right => 1, MouseButton::Middle => 2, MouseButton::X1 => 3, MouseButton::X2 => 4 } }

/// See the module docs.
#[derive(Clone, Debug, Default)]
pub struct InputRouter {
    mods: Modifiers,
    held: Vec<u16>,
    buttons: [bool; 5],
    pos: Option<(f32, f32)>,
    leave_pending: bool,
    pad_buttons: u16,
    stick: (f32, f32),
    right_y: f32,
    dir_now: u16,
    dir_hold: [f32; 4],
}

impl InputRouter {
    /// Stick magnitude that switches a direction on / off (hysteresis).
    pub const STICK_ON: f32 = 0.49;
    pub const STICK_OFF: f32 = 0.35;
    /// Right-stick scroll speed at full deflection (wheel notches per second) and its dead zone.
    pub const SCROLL_NOTCHES_PER_SEC: f32 = 8.;
    pub const SCROLL_DEADZONE: f32 = 0.25;
    /// Directional repeat (seconds), same feel as `input::PadState`.
    pub const REPEAT_DELAY: f32 = 0.4;
    pub const REPEAT_RATE: f32 = 0.12;

    pub fn new() -> Self { Self::default() }

    pub fn modifiers(&self) -> Modifiers { self.mods }
    pub fn is_held(&self, vkc: u16) -> bool { self.held.contains(&vkc) }
    pub fn held_keys(&self) -> &[u16] { &self.held }
    pub fn pointer(&self) -> Option<(f32, f32)> { self.pos }
    pub fn mouse_down(&self, b: MouseButton) -> bool { self.buttons[mouse_slot(b)] }
    pub fn any_mouse_down(&self) -> bool { self.buttons.iter().any(|b| *b) }
    pub fn pad_down(&self, b: PadButton) -> bool { self.pad_buttons & pad_mask(b) != 0 }

    /// Drop all held state without producing events (used when the owner is torn down).
    pub fn clear(&mut self) { *self = Self { pos: self.pos, ..Self::default() }; }

    /// Feed one raw event; the resulting toolkit events are appended to `out`.
    pub fn push(&mut self, e: InputEvent, out: &mut Vec<Event>) -> InputOutcome {
        let mut res = InputOutcome::default();
        match e {
            InputEvent::PointerMoved { x, y } => {
                if !(x.is_finite() && y.is_finite()) { return res; }
                self.pos = Some((x, y)); self.leave_pending = false;
                out.push(Event::MouseMove(x, y));
            }
            InputEvent::PointerLeft => {
                if self.any_mouse_down() { self.leave_pending = true; } // a drag continues outside the window
                else { self.pos = None; out.push(Event::MouseLeave); }
            }
            InputEvent::PointerButton { button, pressed } => {
                let i = mouse_slot(button);
                if pressed {
                    let Some((x, y)) = self.pos else { return res }; // nothing to hit without a known position
                    if self.buttons[i] { return res; }
                    self.buttons[i] = true;
                    out.push(Event::MouseDown(button, x, y));
                } else {
                    if !self.buttons[i] { return res; } // release without a press (e.g. the press happened before the menu existed)
                    self.buttons[i] = false;
                    let (x, y) = self.pos.unwrap_or((0., 0.));
                    out.push(Event::MouseUp(button, x, y));
                    if self.leave_pending && !self.any_mouse_down() { self.leave_pending = false; self.pos = None; out.push(Event::MouseLeave); }
                }
            }
            InputEvent::Wheel { dy, unit, .. } => {
                if !dy.is_finite() { return res; }
                let notches = match unit { WheelUnit::Lines => dy, WheelUnit::Pixels => dy / PIXELS_PER_NOTCH }.clamp(-20., 20.);
                if notches != 0. { out.push(Event::Wheel(notches)); }
            }
            InputEvent::Key { vk: code, pressed, repeat } => {
                if pressed {
                    let is_repeat = repeat || self.held.contains(&code);
                    if !is_repeat { self.held.push(code); }
                    if let Some(s) = Modifiers::slot(code) { self.mods.set(s, true); }
                    out.extend(input::translate_key_down(code, is_repeat, self.mods.shift(), self.mods.ctrl()));
                    if code == vk::ESCAPE && !is_repeat { res.escape_edge = true; }
                } else if let Some(p) = self.held.iter().position(|k| *k == code) {
                    self.held.remove(p);
                    if let Some(s) = Modifiers::slot(code) { if !self.held.contains(&Modifiers::KEYS[s]) { self.mods.set(s, false); } }
                } else if let Some(s) = Modifiers::slot(code) { self.mods.set(s, false); } // generic VK_SHIFT release etc.
            }
            InputEvent::Modifiers(m) => {
                for s in 0..6 {
                    if self.mods.get(s) && !m.get(s) { self.mods.set(s, false); self.held.retain(|k| *k != Modifiers::KEYS[s]); }
                }
            }
            InputEvent::PadButton { button, pressed } => {
                let m = pad_mask(button);
                let was = self.pad_buttons & m != 0;
                if pressed { self.pad_buttons |= m; } else { self.pad_buttons &= !m; }
                if DIR_MASKS.iter().any(|(dm, _)| *dm == m) { self.update_dirs(out); }
                else if pressed && !was {
                    out.push(Event::Pad(button));
                    if button == PadButton::Start { res.start_edge = true; }
                }
            }
            InputEvent::PadAxis { axis, value } => {
                if !value.is_finite() { return res; }
                let v = value.clamp(-1., 1.);
                match axis {
                    PadAxis::LeftX => { self.stick.0 = v; self.update_dirs(out); }
                    PadAxis::LeftY => { self.stick.1 = v; self.update_dirs(out); }
                    PadAxis::RightY => self.right_y = v,
                    PadAxis::RightX | PadAxis::LeftTrigger | PadAxis::RightTrigger => {}
                }
            }
            InputEvent::FocusLost => {
                *self = Self { pos: self.pos, ..Self::default() };
                out.push(Event::FocusLost);
            }
            InputEvent::FocusGained => {}
        }
        res
    }

    /// Per-frame step: directional repeats and right-stick scrolling.
    pub fn tick(&mut self, dt: f32, out: &mut Vec<Event>) {
        let dt = dt.clamp(0., 0.1);
        for (i, (mask, b)) in DIR_MASKS.iter().enumerate() {
            if self.dir_now & mask == 0 { continue; }
            let before = self.dir_hold[i];
            self.dir_hold[i] += dt;
            if self.dir_hold[i] >= Self::REPEAT_DELAY {
                let a = ((before - Self::REPEAT_DELAY).max(0.) / Self::REPEAT_RATE).floor();
                let c = ((self.dir_hold[i] - Self::REPEAT_DELAY) / Self::REPEAT_RATE).floor();
                if before < Self::REPEAT_DELAY || c > a { out.push(Event::Pad(*b)); }
            }
        }
        if self.right_y.abs() > Self::SCROLL_DEADZONE {
            let k = (self.right_y.abs() - Self::SCROLL_DEADZONE) / (1. - Self::SCROLL_DEADZONE);
            out.push(Event::Wheel(self.right_y.signum() * k * Self::SCROLL_NOTCHES_PER_SEC * dt));
        }
    }

    /// Recompute the pressed directions (D-pad + left stick) and emit the new press edges immediately.
    fn update_dirs(&mut self, out: &mut Vec<Event>) {
        let mut dirs = self.pad_buttons & (input::XI_DPAD_UP | input::XI_DPAD_DOWN | input::XI_DPAD_LEFT | input::XI_DPAD_RIGHT);
        let (x, y) = self.stick;
        let on = |v: f32, was: bool| v.abs() > if was { Self::STICK_OFF } else { Self::STICK_ON };
        let (xw, yw) = (self.dir_now & (input::XI_DPAD_LEFT | input::XI_DPAD_RIGHT) != 0, self.dir_now & (input::XI_DPAD_UP | input::XI_DPAD_DOWN) != 0);
        // dominant axis only: a diagonal stick must not navigate two ways at once
        let (use_x, use_y) = if x.abs() >= y.abs() { (on(x, xw), on(y, yw) && y.abs() > x.abs()) } else { (on(x, xw) && x.abs() >= y.abs(), on(y, yw)) };
        if use_x { dirs |= if x > 0. { input::XI_DPAD_RIGHT } else { input::XI_DPAD_LEFT }; }
        if use_y { dirs |= if y > 0. { input::XI_DPAD_UP } else { input::XI_DPAD_DOWN }; }
        for (i, (mask, b)) in DIR_MASKS.iter().enumerate() {
            let (was, is) = (self.dir_now & mask != 0, dirs & mask != 0);
            if is && !was { self.dir_hold[i] = 0.; out.push(Event::Pad(*b)); }
            if !is { self.dir_hold[i] = 0.; }
        }
        self.dir_now = dirs;
    }
}

/// Navigation action of a key (exposed for hosts that want to pre-filter): the same table the router uses.
pub fn nav_of_key(vkc: u16, shift: bool, ctrl: bool) -> Option<NavKey> {
    input::translate_key_down(vkc, false, shift, ctrl).into_iter().find_map(|e| if let Event::Nav(n) = e { Some(n) } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(r: &mut InputRouter, evs: &[InputEvent]) -> Vec<Event> { let mut out = Vec::new(); for e in evs { r.push(*e, &mut out); } out }
    fn key(vk: u16, pressed: bool) -> InputEvent { InputEvent::Key { vk, pressed, repeat: false } }

    #[test] fn pointer_events_carry_the_last_position_and_unknown_positions_are_ignored() {
        let mut r = InputRouter::new();
        assert!(run(&mut r, &[InputEvent::PointerButton { button: MouseButton::Left, pressed: true }]).is_empty(), "no position yet");
        let o = run(&mut r, &[InputEvent::PointerMoved { x: 10., y: 20. }, InputEvent::PointerButton { button: MouseButton::Left, pressed: true }, InputEvent::PointerMoved { x: 30., y: 40. }, InputEvent::PointerButton { button: MouseButton::Left, pressed: false }]);
        assert_eq!(o, vec![Event::MouseMove(10., 20.), Event::MouseDown(MouseButton::Left, 10., 20.), Event::MouseMove(30., 40.), Event::MouseUp(MouseButton::Left, 30., 40.)]);
        assert!(run(&mut r, &[InputEvent::PointerButton { button: MouseButton::Left, pressed: false }]).is_empty(), "release without press");
        assert!(run(&mut r, &[InputEvent::PointerMoved { x: f32::NAN, y: 1. }]).is_empty(), "garbage ignored");
    }
    #[test] fn pointer_leave_is_deferred_while_a_button_is_down() {
        let mut r = InputRouter::new();
        let o = run(&mut r, &[InputEvent::PointerMoved { x: 5., y: 5. }, InputEvent::PointerButton { button: MouseButton::Left, pressed: true }, InputEvent::PointerLeft]);
        assert!(!o.contains(&Event::MouseLeave), "drag continues outside the window");
        let o = run(&mut r, &[InputEvent::PointerMoved { x: 900., y: 5. }, InputEvent::PointerButton { button: MouseButton::Left, pressed: false }]);
        assert_eq!(o.last(), Some(&Event::MouseUp(MouseButton::Left, 900., 5.)));
        let mut r2 = InputRouter::new();
        let o = run(&mut r2, &[InputEvent::PointerMoved { x: 5., y: 5. }, InputEvent::PointerButton { button: MouseButton::Left, pressed: true }, InputEvent::PointerLeft, InputEvent::PointerButton { button: MouseButton::Left, pressed: false }]);
        assert_eq!(o.last(), Some(&Event::MouseLeave), "the leave is delivered after the release");
        assert_eq!(run(&mut InputRouter::new(), &[InputEvent::PointerLeft]), vec![Event::MouseLeave]);
    }
    #[test] fn wheel_units_and_garbage() {
        let mut r = InputRouter::new();
        assert_eq!(run(&mut r, &[InputEvent::Wheel { dx: 0., dy: 2., unit: WheelUnit::Lines }]), vec![Event::Wheel(2.)]);
        assert_eq!(run(&mut r, &[InputEvent::Wheel { dx: 0., dy: -PIXELS_PER_NOTCH, unit: WheelUnit::Pixels }]), vec![Event::Wheel(-1.)]);
        assert_eq!(run(&mut r, &[InputEvent::Wheel { dx: 0., dy: 1e9, unit: WheelUnit::Lines }]), vec![Event::Wheel(20.)], "clamped");
        assert!(run(&mut r, &[InputEvent::Wheel { dx: 0., dy: f32::NAN, unit: WheelUnit::Lines }, InputEvent::Wheel { dx: 3., dy: 0., unit: WheelUnit::Lines }]).is_empty());
    }
    #[test] fn keys_map_to_navigation_and_raw_keys_without_repeating_actions() {
        let mut r = InputRouter::new();
        assert_eq!(run(&mut r, &[key(vk::DOWN, true)]), vec![Event::Nav(NavKey::Down), Event::RawKey(vk::DOWN)]);
        assert_eq!(run(&mut r, &[InputEvent::Key { vk: vk::DOWN, pressed: true, repeat: true }]), vec![Event::Nav(NavKey::Down)], "arrows repeat, the raw key does not");
        assert_eq!(run(&mut r, &[key(vk::DOWN, true)]), vec![Event::Nav(NavKey::Down)], "a second down without an up is a repeat");
        run(&mut r, &[key(vk::DOWN, false)]);
        assert_eq!(run(&mut r, &[key(vk::ESCAPE, true)]), vec![Event::Nav(NavKey::Back), Event::RawKey(vk::ESCAPE)]);
        for _ in 0..5 { assert!(!run(&mut r, &[InputEvent::Key { vk: vk::ESCAPE, pressed: true, repeat: true }]).iter().any(|e| matches!(e, Event::Nav(NavKey::Back))), "held Esc never repeats Back"); }
        assert!(run(&mut r, &[key(vk::ESCAPE, true)]).iter().all(|e| !matches!(e, Event::Nav(NavKey::Back))), "nor does a lost key-up");
        run(&mut r, &[key(vk::ESCAPE, false)]);
        assert_eq!(run(&mut r, &[key(vk::RETURN, true), key(vk::RETURN, false), key(vk::SPACE, true)]), vec![Event::Nav(NavKey::Activate), Event::RawKey(vk::RETURN), Event::Nav(NavKey::Activate), Event::RawKey(vk::SPACE)]);
        assert!(run(&mut r, &[key(0x41, false)]).is_empty(), "release without press");
    }
    #[test] fn tab_uses_the_tracked_modifiers() {
        let mut r = InputRouter::new();
        assert_eq!(run(&mut r, &[key(vk::TAB, true)])[0], Event::Nav(NavKey::Down)); run(&mut r, &[key(vk::TAB, false)]);
        run(&mut r, &[key(vk::LSHIFT, true)]);
        assert_eq!(run(&mut r, &[key(vk::TAB, true)])[0], Event::Nav(NavKey::Up)); run(&mut r, &[key(vk::TAB, false)]);
        run(&mut r, &[key(vk::RCONTROL, true)]);
        assert_eq!(run(&mut r, &[key(vk::TAB, true)])[0], Event::Nav(NavKey::TabPrev), "Ctrl+Shift+Tab"); run(&mut r, &[key(vk::TAB, false), key(vk::LSHIFT, false)]);
        assert_eq!(run(&mut r, &[key(vk::TAB, true)])[0], Event::Nav(NavKey::TabNext), "Ctrl+Tab"); run(&mut r, &[key(vk::TAB, false)]);
        assert_eq!(run(&mut r, &[key(vk::NEXT, true)])[0], Event::Nav(NavKey::TabNext), "Ctrl+PageDown");
        run(&mut r, &[key(vk::RCONTROL, false)]);
        assert!(!r.modifiers().any());
    }
    #[test] fn left_and_right_modifiers_are_distinct_raw_keys_and_tracked() {
        let mut r = InputRouter::new();
        let o = run(&mut r, &[key(vk::LSHIFT, true), key(vk::RSHIFT, true), key(vk::LMENU, true), key(vk::RCONTROL, true)]);
        assert_eq!(o, vec![Event::RawKey(vk::LSHIFT), Event::RawKey(vk::RSHIFT), Event::RawKey(vk::LMENU), Event::RawKey(vk::RCONTROL)]);
        let m = r.modifiers(); assert!(m.lshift && m.rshift && m.lalt && !m.ralt && m.rctrl && !m.lctrl);
        run(&mut r, &[key(vk::LSHIFT, false)]);
        assert!(!r.modifiers().lshift && r.modifiers().rshift && r.modifiers().shift(), "right shift still holds shift");
        run(&mut r, &[key(vk::RSHIFT, false)]);
        assert!(!r.modifiers().shift());
    }
    #[test] fn modifier_resync_releases_stuck_modifiers_and_focus_loss_releases_everything() {
        let mut r = InputRouter::new();
        run(&mut r, &[key(vk::LCONTROL, true), key(0x41, true), InputEvent::PointerMoved { x: 1., y: 1. }, InputEvent::PointerButton { button: MouseButton::Right, pressed: true }]);
        assert!(r.modifiers().lctrl && r.is_held(0x41) && r.mouse_down(MouseButton::Right));
        run(&mut r, &[InputEvent::Modifiers(Modifiers::default())]);
        assert!(!r.modifiers().ctrl() && !r.is_held(vk::LCONTROL) && r.is_held(0x41));
        let o = run(&mut r, &[InputEvent::FocusLost]);
        assert_eq!(o, vec![Event::FocusLost]);
        assert!(r.held_keys().is_empty() && !r.any_mouse_down());
        assert!(run(&mut r, &[InputEvent::PointerButton { button: MouseButton::Right, pressed: false }]).is_empty(), "the late release after focus loss is ignored");
        assert_eq!(run(&mut r, &[key(0x41, true)]), vec![Event::RawKey(0x41)], "keys work again after focus returns");
    }
    #[test] fn escape_edge_is_reported_once_per_physical_press() {
        let mut r = InputRouter::new(); let mut out = Vec::new();
        assert!(r.push(key(vk::ESCAPE, true), &mut out).escape_edge);
        assert!(!r.push(InputEvent::Key { vk: vk::ESCAPE, pressed: true, repeat: true }, &mut out).escape_edge);
        assert!(!r.push(key(vk::ESCAPE, true), &mut out).escape_edge);
        r.push(key(vk::ESCAPE, false), &mut out);
        assert!(r.push(key(vk::ESCAPE, true), &mut out).escape_edge);
        assert!(!r.push(key(vk::RETURN, true), &mut out).escape_edge);
    }
    #[test] fn pad_buttons_press_edges_are_immediate_and_not_duplicated() {
        let mut r = InputRouter::new();
        let o = run(&mut r, &[InputEvent::PadButton { button: PadButton::A, pressed: true }, InputEvent::PadButton { button: PadButton::A, pressed: true }, InputEvent::PadButton { button: PadButton::A, pressed: false }, InputEvent::PadButton { button: PadButton::B, pressed: true }]);
        assert_eq!(o, vec![Event::Pad(PadButton::A), Event::Pad(PadButton::B)], "a press and release inside one frame still counts once");
        let mut out = Vec::new();
        assert!(r.push(InputEvent::PadButton { button: PadButton::Start, pressed: true }, &mut out).start_edge);
    }
    #[test] fn dpad_and_stick_repeat_after_the_delay_and_stop_on_release() {
        let mut r = InputRouter::new(); let mut out = Vec::new();
        r.push(InputEvent::PadButton { button: PadButton::DPadDown, pressed: true }, &mut out);
        assert_eq!(out, vec![Event::Pad(PadButton::DPadDown)]); out.clear();
        for _ in 0..20 { r.tick(0.016, &mut out); } // 0.32 s: still inside the delay
        assert!(out.is_empty());
        for _ in 0..60 { r.tick(0.016, &mut out); }
        assert!(out.len() >= 5 && out.iter().all(|e| *e == Event::Pad(PadButton::DPadDown)), "repeats: {}", out.len());
        out.clear();
        r.push(InputEvent::PadButton { button: PadButton::DPadDown, pressed: false }, &mut out);
        for _ in 0..60 { r.tick(0.016, &mut out); }
        assert!(out.is_empty());
        // left stick = directions with hysteresis and a dominant axis
        r.push(InputEvent::PadAxis { axis: PadAxis::LeftY, value: -0.8 }, &mut out);
        assert_eq!(out, vec![Event::Pad(PadButton::DPadDown)]); out.clear();
        r.push(InputEvent::PadAxis { axis: PadAxis::LeftY, value: -0.4 }, &mut out); // between OFF and ON: stays on, no new edge
        r.push(InputEvent::PadAxis { axis: PadAxis::LeftX, value: 0.45 }, &mut out); // x below ON and weaker than y: nothing
        assert!(out.is_empty(), "{out:?}");
        r.push(InputEvent::PadAxis { axis: PadAxis::LeftY, value: 0. }, &mut out);
        r.push(InputEvent::PadAxis { axis: PadAxis::LeftX, value: 0.9 }, &mut out);
        assert_eq!(out, vec![Event::Pad(PadButton::DPadRight)]);
        out.clear();
        r.push(InputEvent::PadAxis { axis: PadAxis::LeftY, value: 0.95 }, &mut out); // y now dominates
        assert_eq!(out, vec![Event::Pad(PadButton::DPadUp)], "the dominant axis wins, right is released");
    }
    #[test] fn right_stick_scrolls_like_the_wheel() {
        let mut r = InputRouter::new(); let mut out = Vec::new();
        r.push(InputEvent::PadAxis { axis: PadAxis::RightY, value: 0.1 }, &mut out); r.tick(0.1, &mut out);
        assert!(out.is_empty(), "inside the dead zone");
        r.push(InputEvent::PadAxis { axis: PadAxis::RightY, value: -1. }, &mut out); r.tick(0.1, &mut out);
        match out[..] { [Event::Wheel(d)] => assert!((d + 0.8).abs() < 1e-4, "{d}"), _ => panic!("{out:?}") }
        out.clear();
        r.push(InputEvent::PadAxis { axis: PadAxis::RightY, value: 0. }, &mut out); r.tick(0.1, &mut out);
        assert!(out.is_empty());
        r.push(InputEvent::PadAxis { axis: PadAxis::RightY, value: f32::NAN }, &mut out); r.tick(0.1, &mut out);
        assert!(out.is_empty());
    }
}
