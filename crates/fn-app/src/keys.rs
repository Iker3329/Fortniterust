//! winit key / mouse button -> the Windows virtual-key codes the settings bindings (`fn_settings::Key::poll_code`) and the UI (`fn_ui::input::vk`) use.
//! The codes are plain numbers: no Win32 binding is involved.
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

pub const VK_LBUTTON: u16 = 0x01;
pub const VK_RBUTTON: u16 = 0x02;
pub const VK_MBUTTON: u16 = 0x04;
pub const VK_XBUTTON1: u16 = 0x05;
pub const VK_XBUTTON2: u16 = 0x06;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_F1: u16 = 0x70;
pub const VK_R: u16 = 0x52;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_SHIFT_L: u16 = 0xA0;
pub const VK_SHIFT_R: u16 = 0xA1;
pub const VK_CTRL_L: u16 = 0xA2;
pub const VK_CTRL_R: u16 = 0xA3;

pub fn mouse_vk(b: MouseButton) -> Option<u16> {
    match b {
        MouseButton::Left => Some(VK_LBUTTON),
        MouseButton::Right => Some(VK_RBUTTON),
        MouseButton::Middle => Some(VK_MBUTTON),
        MouseButton::Back => Some(VK_XBUTTON1),
        MouseButton::Forward => Some(VK_XBUTTON2),
        MouseButton::Other(_) => None,
    }
}

/// Virtual-key code of a physical key (layout independent, like `GetAsyncKeyState` on the key's scan position).
pub fn vk(code: KeyCode) -> Option<u16> {
    use KeyCode::*;
    Some(match code {
        KeyA => 0x41, KeyB => 0x42, KeyC => 0x43, KeyD => 0x44, KeyE => 0x45, KeyF => 0x46, KeyG => 0x47, KeyH => 0x48, KeyI => 0x49, KeyJ => 0x4A,
        KeyK => 0x4B, KeyL => 0x4C, KeyM => 0x4D, KeyN => 0x4E, KeyO => 0x4F, KeyP => 0x50, KeyQ => 0x51, KeyR => 0x52, KeyS => 0x53, KeyT => 0x54,
        KeyU => 0x55, KeyV => 0x56, KeyW => 0x57, KeyX => 0x58, KeyY => 0x59, KeyZ => 0x5A,
        Digit0 => 0x30, Digit1 => 0x31, Digit2 => 0x32, Digit3 => 0x33, Digit4 => 0x34, Digit5 => 0x35, Digit6 => 0x36, Digit7 => 0x37, Digit8 => 0x38, Digit9 => 0x39,
        Numpad0 => 0x60, Numpad1 => 0x61, Numpad2 => 0x62, Numpad3 => 0x63, Numpad4 => 0x64, Numpad5 => 0x65, Numpad6 => 0x66, Numpad7 => 0x67, Numpad8 => 0x68, Numpad9 => 0x69,
        NumpadMultiply => 0x6A, NumpadAdd => 0x6B, NumpadSubtract => 0x6D, NumpadDecimal => 0x6E, NumpadDivide => 0x6F, NumpadEnter => 0x0D,
        F1 => 0x70, F2 => 0x71, F3 => 0x72, F4 => 0x73, F5 => 0x74, F6 => 0x75, F7 => 0x76, F8 => 0x77, F9 => 0x78, F10 => 0x79, F11 => 0x7A, F12 => 0x7B,
        Space => 0x20, Tab => 0x09, Enter => 0x0D, Escape => 0x1B, Backspace => 0x08, CapsLock => 0x14, Pause => 0x13, NumLock => 0x90, ScrollLock => 0x91,
        ShiftLeft => 0xA0, ShiftRight => 0xA1, ControlLeft => 0xA2, ControlRight => 0xA3, AltLeft => 0xA4, AltRight => 0xA5,
        ArrowUp => 0x26, ArrowDown => 0x28, ArrowLeft => 0x25, ArrowRight => 0x27, PageUp => 0x21, PageDown => 0x22, End => 0x23, Home => 0x24, Insert => 0x2D, Delete => 0x2E,
        Semicolon => 0xBA, Equal => 0xBB, Comma => 0xBC, Minus => 0xBD, Period => 0xBE, Slash => 0xBF, Backquote => 0xC0, BracketLeft => 0xDB, Backslash => 0xDC, BracketRight => 0xDD, Quote => 0xDE,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_settings::Key;
    #[test] fn letters_digits_and_function_keys() { assert_eq!(vk(KeyCode::KeyW), Some(0x57)); assert_eq!(vk(KeyCode::Digit1), Some(0x31)); assert_eq!(vk(KeyCode::F1), Some(VK_F1)); assert_eq!(vk(KeyCode::Escape), Some(VK_ESCAPE)); }
    #[test] fn every_code_is_a_bindable_key_or_a_known_special() {
        // whatever the player can press that maps to a vk should round-trip through the settings key table (modifiers, mouse, letters, digits, F1-F12)
        for (code, name) in [(KeyCode::KeyA, "A"), (KeyCode::Space, "SpaceBar"), (KeyCode::ShiftLeft, "LeftShift"), (KeyCode::ControlLeft, "LeftControl"), (KeyCode::Digit3, "Three"), (KeyCode::F5, "F5"), (KeyCode::ArrowUp, "Up"), (KeyCode::Tab, "Tab")] {
            let k = Key::from_vk(vk(code).unwrap()).unwrap_or_else(|| panic!("{name} has no key"));
            assert_eq!(k.name(), name);
        }
    }
    #[test] fn mouse_buttons_use_the_mouse_codes() { assert_eq!(mouse_vk(MouseButton::Left), Some(1)); assert_eq!(Key::from_vk(VK_LBUTTON).map(|k| k.name()), Some("LeftMouseButton")); assert_eq!(mouse_vk(MouseButton::Other(9)), None); }
}
