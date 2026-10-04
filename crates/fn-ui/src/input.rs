//! Abstract input model for the UI: mouse / keyboard / gamepad events with no OS dependency.
//!
//! Key codes are Windows virtual-key codes stored as `u16` (the settings file and `settings::Key` use them); the constants below are the
//! plain numbers, so no Win32 binding is needed. Platform layers (winit in `fn-ui-demo`, the game's window code) translate their events here.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseButton { Left, Right, Middle, X1, X2 }

/// Logical navigation keys (keyboard, D-pad/stick and face buttons all map here).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavKey { Up, Down, Left, Right, Activate, Back, TabPrev, TabNext, Home, End, PageUp, PageDown, Clear }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PadButton { DPadUp, DPadDown, DPadLeft, DPadRight, A, B, X, Y, LB, RB, Start, Back }

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Event {
    MouseMove(f32, f32),
    MouseDown(MouseButton, f32, f32),
    MouseUp(MouseButton, f32, f32),
    MouseLeave,
    /// Wheel notches, positive = away from the user (scroll up).
    Wheel(f32),
    Nav(NavKey),
    /// Raw virtual-key code of a key press (non-repeat); only consumed by a listening keybind button.
    RawKey(u16),
    Pad(PadButton),
    /// Window lost focus / was deactivated / minimised: cancels drags, key capture and open popups.
    FocusLost,
}

pub fn pad_to_nav(b: PadButton) -> NavKey {
    match b {
        PadButton::DPadUp => NavKey::Up, PadButton::DPadDown => NavKey::Down, PadButton::DPadLeft => NavKey::Left, PadButton::DPadRight => NavKey::Right,
        PadButton::A => NavKey::Activate, PadButton::B | PadButton::Start | PadButton::Back => NavKey::Back,
        PadButton::LB => NavKey::TabPrev, PadButton::RB => NavKey::TabNext, PadButton::X => NavKey::Clear, PadButton::Y => NavKey::Home,
    }
}

/// Windows virtual-key codes used by the UI.
pub mod vk {
    pub const LBUTTON: u16 = 0x01; pub const RBUTTON: u16 = 0x02; pub const MBUTTON: u16 = 0x04; pub const XBUTTON1: u16 = 0x05; pub const XBUTTON2: u16 = 0x06;
    pub const BACK: u16 = 0x08; pub const TAB: u16 = 0x09; pub const RETURN: u16 = 0x0D; pub const SHIFT: u16 = 0x10; pub const CONTROL: u16 = 0x11; pub const MENU: u16 = 0x12;
    pub const PAUSE: u16 = 0x13; pub const CAPITAL: u16 = 0x14; pub const ESCAPE: u16 = 0x1B; pub const SPACE: u16 = 0x20; pub const PRIOR: u16 = 0x21; pub const NEXT: u16 = 0x22;
    pub const END: u16 = 0x23; pub const HOME: u16 = 0x24; pub const LEFT: u16 = 0x25; pub const UP: u16 = 0x26; pub const RIGHT: u16 = 0x27; pub const DOWN: u16 = 0x28;
    pub const INSERT: u16 = 0x2D; pub const DELETE: u16 = 0x2E;
    pub const MULTIPLY: u16 = 0x6A; pub const ADD: u16 = 0x6B; pub const SUBTRACT: u16 = 0x6D; pub const DECIMAL: u16 = 0x6E; pub const DIVIDE: u16 = 0x6F;
    pub const NUMLOCK: u16 = 0x90; pub const SCROLL: u16 = 0x91;
    pub const LSHIFT: u16 = 0xA0; pub const RSHIFT: u16 = 0xA1; pub const LCONTROL: u16 = 0xA2; pub const RCONTROL: u16 = 0xA3; pub const LMENU: u16 = 0xA4; pub const RMENU: u16 = 0xA5;
}

/// Display name of a virtual-key code (keybind buttons).
pub fn key_name(code: u16) -> String {
    use vk::*;
    let n = match code {
        LBUTTON => "Left Mouse Button", RBUTTON => "Right Mouse Button", MBUTTON => "Middle Mouse Button",
        XBUTTON1 => "Thumb Mouse Button", XBUTTON2 => "Thumb Mouse Button 2",
        BACK => "Backspace", TAB => "Tab", RETURN => "Enter", SHIFT => "Shift", CONTROL => "Ctrl", MENU => "Alt",
        LSHIFT => "Left Shift", RSHIFT => "Right Shift", LCONTROL => "Left Ctrl", RCONTROL => "Right Ctrl", LMENU => "Left Alt", RMENU => "Right Alt",
        PAUSE => "Pause", CAPITAL => "Caps Lock", ESCAPE => "Escape", SPACE => "Space Bar", PRIOR => "Page Up", NEXT => "Page Down",
        END => "End", HOME => "Home", LEFT => "Left", UP => "Up", RIGHT => "Right", DOWN => "Down", INSERT => "Insert", DELETE => "Delete",
        MULTIPLY => "Num *", ADD => "Num +", SUBTRACT => "Num -", DECIMAL => "Num .", DIVIDE => "Num /", NUMLOCK => "Num Lock", SCROLL => "Scroll Lock",
        0xBA => ";", 0xBB => "=", 0xBC => ",", 0xBD => "-", 0xBE => ".", 0xBF => "/", 0xC0 => "`", 0xDB => "[", 0xDC => "\\", 0xDD => "]", 0xDE => "'",
        _ => "",
    };
    if !n.is_empty() { return n.to_owned(); }
    match code {
        0x30..=0x39 | 0x41..=0x5A => (code as u8 as char).to_string(),
        0x60..=0x69 => format!("Num {}", code - 0x60),
        0x70..=0x87 => format!("F{}", code - 0x6F),
        _ => format!("Key {code}"),
    }
}

/// Keyboard -> toolkit events for a key press, given the virtual-key code and modifier state (platform independent form of the old
/// `translate_win32` WM_KEYDOWN arm). `repeat` = auto-repeat. Activate/Back/Clear never auto-repeat; the raw key is only sent for non-repeats.
pub fn translate_key_down(vkc: u16, repeat: bool, shift: bool, ctrl: bool) -> Vec<Event> {
    use vk::*;
    let nav = match vkc {
        UP => Some(NavKey::Up), DOWN => Some(NavKey::Down), LEFT => Some(NavKey::Left), RIGHT => Some(NavKey::Right),
        RETURN | SPACE => Some(NavKey::Activate), ESCAPE => Some(NavKey::Back),
        TAB => Some(if ctrl { if shift { NavKey::TabPrev } else { NavKey::TabNext } } else if shift { NavKey::Up } else { NavKey::Down }),
        PRIOR => Some(if ctrl { NavKey::TabPrev } else { NavKey::PageUp }),
        NEXT => Some(if ctrl { NavKey::TabNext } else { NavKey::PageDown }),
        HOME => Some(NavKey::Home), END => Some(NavKey::End), DELETE | BACK => Some(NavKey::Clear),
        _ => None,
    };
    let mut out = Vec::new();
    if let Some(n) = nav { if !(repeat && matches!(n, NavKey::Activate | NavKey::Back | NavKey::Clear)) { out.push(Event::Nav(n)); } }
    if !repeat { out.push(Event::RawKey(vkc)); }
    out
}

// ---- gamepad (abstract) -------------------------------------------------------------------------------------
pub const XI_DPAD_UP: u16 = 0x1; pub const XI_DPAD_DOWN: u16 = 0x2; pub const XI_DPAD_LEFT: u16 = 0x4; pub const XI_DPAD_RIGHT: u16 = 0x8;
pub const XI_START: u16 = 0x10; pub const XI_BACK: u16 = 0x20; pub const XI_LB: u16 = 0x100; pub const XI_RB: u16 = 0x200;
pub const XI_A: u16 = 0x1000; pub const XI_B: u16 = 0x2000; pub const XI_X: u16 = 0x4000; pub const XI_Y: u16 = 0x8000;

/// Gamepad button bitmask (XInput layout) + left stick -> edge/repeat `PadButton`s. Pure: the platform feeds raw state each frame.
#[derive(Default, Clone, Debug)]
pub struct PadState { prev: u16, hold: [f32; 4], dir_prev: u16 }
impl PadState {
    pub const REPEAT_DELAY: f32 = 0.4;
    pub const REPEAT_RATE: f32 = 0.12;
    pub const STICK_ON: i16 = 16000;
    pub fn step(&mut self, buttons: u16, lx: i16, ly: i16, dt: f32) -> Vec<PadButton> {
        let mut dirs = buttons & (XI_DPAD_UP | XI_DPAD_DOWN | XI_DPAD_LEFT | XI_DPAD_RIGHT);
        if ly > Self::STICK_ON { dirs |= XI_DPAD_UP; } if ly < -Self::STICK_ON { dirs |= XI_DPAD_DOWN; }
        if lx < -Self::STICK_ON { dirs |= XI_DPAD_LEFT; } if lx > Self::STICK_ON { dirs |= XI_DPAD_RIGHT; }
        let mut out = Vec::new();
        let edge = buttons & !self.prev;
        for (mask, b) in [(XI_A, PadButton::A), (XI_B, PadButton::B), (XI_X, PadButton::X), (XI_Y, PadButton::Y), (XI_LB, PadButton::LB), (XI_RB, PadButton::RB), (XI_START, PadButton::Start), (XI_BACK, PadButton::Back)] {
            if edge & mask != 0 { out.push(b); }
        }
        for (i, (mask, b)) in [(XI_DPAD_UP, PadButton::DPadUp), (XI_DPAD_DOWN, PadButton::DPadDown), (XI_DPAD_LEFT, PadButton::DPadLeft), (XI_DPAD_RIGHT, PadButton::DPadRight)].into_iter().enumerate() {
            if dirs & mask == 0 { self.hold[i] = 0.; continue; }
            if self.dir_prev & mask == 0 { self.hold[i] = 0.; out.push(b); continue; }
            let before = self.hold[i]; self.hold[i] += dt;
            if self.hold[i] >= Self::REPEAT_DELAY {
                let (a, c) = (((before - Self::REPEAT_DELAY).max(0.) / Self::REPEAT_RATE).floor(), ((self.hold[i] - Self::REPEAT_DELAY) / Self::REPEAT_RATE).floor());
                if before < Self::REPEAT_DELAY || c > a { out.push(b); }
            }
        }
        self.prev = buttons; self.dir_prev = dirs;
        out
    }
}

/// A gamepad provider. The old runtime polled XInput controller 0 here; platform crates implement this (none ships in fn-ui).
pub trait PadSource { fn poll(&mut self, dt: f32) -> Vec<Event>; }
/// No controller.
pub struct NoPad;
impl PadSource for NoPad { fn poll(&mut self, _dt: f32) -> Vec<Event> { Vec::new() } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn key_names() { assert_eq!(key_name(vk::SPACE), "Space Bar"); assert_eq!(key_name(0x41), "A"); assert_eq!(key_name(0x70), "F1"); assert_eq!(key_name(0x61), "Num 1"); }
    #[test] fn tab_navigation() {
        assert_eq!(translate_key_down(vk::TAB, false, false, true), vec![Event::Nav(NavKey::TabNext), Event::RawKey(vk::TAB)]);
        assert_eq!(translate_key_down(vk::RETURN, true, false, false), vec![]);
        assert_eq!(translate_key_down(vk::DOWN, true, false, false), vec![Event::Nav(NavKey::Down)]);
    }
    #[test] fn dpad_repeats_after_delay() {
        let mut p = PadState::default();
        assert_eq!(p.step(XI_DPAD_DOWN, 0, 0, 0.016), vec![PadButton::DPadDown]);
        assert!(p.step(XI_DPAD_DOWN, 0, 0, 0.1).is_empty());
        let mut n = 0; for _ in 0..40 { n += p.step(XI_DPAD_DOWN, 0, 0, 0.05).len(); } assert!(n >= 10);
    }
}
