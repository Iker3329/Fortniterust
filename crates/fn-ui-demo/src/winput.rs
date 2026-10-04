//! winit -> `fn_ui::router::InputEvent`: the only place in the demo that knows both vocabularies (fn-ui itself is windowing-agnostic).
//! `fn-app` needs the same translation; this table is the reference (Windows virtual-key codes, left / right modifiers resolved from the physical key).
use fn_ui::input::{vk, MouseButton};
use fn_ui::router::{InputEvent, Modifiers, WheelUnit};
use winit::event::{ElementState, MouseButton as WMouse, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

/// Windows virtual-key code of a physical key (None = a key the UI has no use for).
pub fn vk_of(code: KeyCode) -> Option<u16> {
    use KeyCode::*;
    Some(match code {
        KeyA => 0x41, KeyB => 0x42, KeyC => 0x43, KeyD => 0x44, KeyE => 0x45, KeyF => 0x46, KeyG => 0x47, KeyH => 0x48, KeyI => 0x49, KeyJ => 0x4A, KeyK => 0x4B, KeyL => 0x4C, KeyM => 0x4D,
        KeyN => 0x4E, KeyO => 0x4F, KeyP => 0x50, KeyQ => 0x51, KeyR => 0x52, KeyS => 0x53, KeyT => 0x54, KeyU => 0x55, KeyV => 0x56, KeyW => 0x57, KeyX => 0x58, KeyY => 0x59, KeyZ => 0x5A,
        Digit0 => 0x30, Digit1 => 0x31, Digit2 => 0x32, Digit3 => 0x33, Digit4 => 0x34, Digit5 => 0x35, Digit6 => 0x36, Digit7 => 0x37, Digit8 => 0x38, Digit9 => 0x39,
        Numpad0 => 0x60, Numpad1 => 0x61, Numpad2 => 0x62, Numpad3 => 0x63, Numpad4 => 0x64, Numpad5 => 0x65, Numpad6 => 0x66, Numpad7 => 0x67, Numpad8 => 0x68, Numpad9 => 0x69,
        NumpadMultiply => vk::MULTIPLY, NumpadAdd => vk::ADD, NumpadSubtract => vk::SUBTRACT, NumpadDecimal => vk::DECIMAL, NumpadDivide => vk::DIVIDE, NumpadEnter => vk::RETURN,
        F1 => 0x70, F2 => 0x71, F3 => 0x72, F4 => 0x73, F5 => 0x74, F6 => 0x75, F7 => 0x76, F8 => 0x77, F9 => 0x78, F10 => 0x79, F11 => 0x7A, F12 => 0x7B,
        ArrowLeft => vk::LEFT, ArrowUp => vk::UP, ArrowRight => vk::RIGHT, ArrowDown => vk::DOWN,
        Escape => vk::ESCAPE, Enter => vk::RETURN, Space => vk::SPACE, Tab => vk::TAB, Backspace => vk::BACK, Delete => vk::DELETE, Insert => vk::INSERT,
        Home => vk::HOME, End => vk::END, PageUp => vk::PRIOR, PageDown => vk::NEXT,
        ShiftLeft => vk::LSHIFT, ShiftRight => vk::RSHIFT, ControlLeft => vk::LCONTROL, ControlRight => vk::RCONTROL, AltLeft => vk::LMENU, AltRight => vk::RMENU,
        CapsLock => vk::CAPITAL, Pause => vk::PAUSE, NumLock => vk::NUMLOCK, ScrollLock => vk::SCROLL,
        Semicolon => 0xBA, Equal => 0xBB, Comma => 0xBC, Minus => 0xBD, Period => 0xBE, Slash => 0xBF, Backquote => 0xC0, BracketLeft => 0xDB, Backslash => 0xDC, BracketRight => 0xDD, Quote => 0xDE,
        SuperLeft => 0x5B, SuperRight => 0x5C, ContextMenu => 0x5D,
        _ => return None,
    })
}
pub fn mouse_button(b: WMouse) -> Option<MouseButton> {
    Some(match b { WMouse::Left => MouseButton::Left, WMouse::Right => MouseButton::Right, WMouse::Middle => MouseButton::Middle, WMouse::Back => MouseButton::X1, WMouse::Forward => MouseButton::X2, WMouse::Other(_) => return None })
}
/// Modifier state from per-key flags (`ModifiersChanged`: a side the window system cannot tell apart is reported on both).
pub fn modifiers(lshift: bool, rshift: bool, lctrl: bool, rctrl: bool, lalt: bool, ralt: bool) -> InputEvent { InputEvent::Modifiers(Modifiers { lshift, rshift, lctrl, rctrl, lalt, ralt }) }

/// Translate one winit window event (None = not an input event).
pub fn translate(e: &WindowEvent) -> Option<InputEvent> {
    Some(match e {
        WindowEvent::CursorMoved { position, .. } => InputEvent::PointerMoved { x: position.x as f32, y: position.y as f32 },
        WindowEvent::CursorLeft { .. } => InputEvent::PointerLeft,
        WindowEvent::MouseInput { state, button, .. } => InputEvent::PointerButton { button: mouse_button(*button)?, pressed: *state == ElementState::Pressed },
        WindowEvent::MouseWheel { delta, .. } => match delta {
            MouseScrollDelta::LineDelta(x, y) => InputEvent::Wheel { dx: *x, dy: *y, unit: WheelUnit::Lines },
            MouseScrollDelta::PixelDelta(p) => InputEvent::Wheel { dx: p.x as f32, dy: p.y as f32, unit: WheelUnit::Pixels },
        },
        WindowEvent::KeyboardInput { event, .. } => {
            let PhysicalKey::Code(code) = event.physical_key else { return None };
            InputEvent::Key { vk: vk_of(code)?, pressed: event.state == ElementState::Pressed, repeat: event.repeat }
        }
        WindowEvent::ModifiersChanged(m) => {
            use winit::keyboard::ModifiersKeyState::Pressed;
            let s = m.state();
            let side = |l: winit::keyboard::ModifiersKeyState, r: winit::keyboard::ModifiersKeyState, both: bool| match (l == Pressed, r == Pressed) { (false, false) => (both, both), (a, b) => (a, b) };
            let (ls, rs) = side(m.lshift_state(), m.rshift_state(), s.shift_key());
            let (lc, rc) = side(m.lcontrol_state(), m.rcontrol_state(), s.control_key());
            let (la, ra) = side(m.lalt_state(), m.ralt_state(), s.alt_key());
            modifiers(ls, rs, lc, rc, la, ra)
        }
        WindowEvent::Focused(false) => InputEvent::FocusLost,
        WindowEvent::Focused(true) => InputEvent::FocusGained,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_ui::settings::Key;
    #[test] fn letters_digits_function_keys_and_modifiers_map_to_windows_codes() {
        assert_eq!(vk_of(KeyCode::KeyA), Some(0x41)); assert_eq!(vk_of(KeyCode::KeyZ), Some(0x5A));
        assert_eq!(vk_of(KeyCode::Digit0), Some(0x30)); assert_eq!(vk_of(KeyCode::Digit9), Some(0x39));
        assert_eq!(vk_of(KeyCode::F1), Some(0x70)); assert_eq!(vk_of(KeyCode::F12), Some(0x7B));
        assert_eq!(vk_of(KeyCode::ShiftLeft), Some(vk::LSHIFT)); assert_eq!(vk_of(KeyCode::ShiftRight), Some(vk::RSHIFT));
        assert_eq!(vk_of(KeyCode::ControlLeft), Some(vk::LCONTROL)); assert_eq!(vk_of(KeyCode::AltRight), Some(vk::RMENU));
        assert_eq!(vk_of(KeyCode::Escape), Some(vk::ESCAPE)); assert_eq!(vk_of(KeyCode::Space), Some(vk::SPACE));
        assert_eq!(vk_of(KeyCode::MediaPlayPause), None);
    }
    #[test] fn every_bindable_key_of_the_settings_table_has_a_winit_key() {
        // each key the binding table can hold (except the wheel pseudo keys) must be reachable from a physical key, or it could never be bound
        use KeyCode::*;
        let all = [KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
            Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9, Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
            NumpadMultiply, NumpadAdd, NumpadSubtract, NumpadDecimal, NumpadDivide, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, ArrowLeft, ArrowUp, ArrowRight, ArrowDown,
            Escape, Enter, Space, Tab, Backspace, Delete, Insert, Home, End, PageUp, PageDown, ShiftLeft, ShiftRight, ControlLeft, ControlRight, AltLeft, AltRight, CapsLock, Pause, NumLock, ScrollLock,
            Semicolon, Equal, Comma, Minus, Period, Slash, Backquote, BracketLeft, Backslash, BracketRight, Quote];
        let reachable: std::collections::HashSet<u16> = all.iter().filter_map(|c| vk_of(*c)).collect();
        for name in Key::all_names() {
            let k = Key::from_name(name).unwrap();
            // (mouse buttons, vk 1..6, arrive as pointer events, not keys)
            if let Some(code) = k.vk().filter(|c| *c > 6) { assert!(reachable.contains(&code), "{name} (vk {code:#x}) cannot be typed on a winit keyboard"); }
        }
    }
    #[test] fn mouse_buttons() {
        assert_eq!(mouse_button(WMouse::Left), Some(MouseButton::Left)); assert_eq!(mouse_button(WMouse::Back), Some(MouseButton::X1)); assert_eq!(mouse_button(WMouse::Forward), Some(MouseButton::X2));
        assert_eq!(mouse_button(WMouse::Other(7)), None);
    }
}
