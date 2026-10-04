//! Gamepad backends for `fn_game::inputdev::pad::PadBackend` (raw XInput-style state of controller 0).
//!
//! * [`GilrsPad`] (feature `gamepad`, default): `gilrs` (pure Rust; XInput on Windows). Events are pumped every poll so button/axis state stays
//!   current, and the first connected pad is encoded as the 16 raw bytes of an `XINPUT_STATE`, which is exactly what `pad::State::from_raw` decodes.
//! * [`NoPad`]: no controller (`--no-gamepad`, or gilrs could not start).
//! * [`ScriptedPad`]: a settable raw state (tests).
use fn_game::inputdev::pad::{self, PadBackend};

pub fn encode(buttons: u16, lt: u8, rt: u8, lx: i16, ly: i16, rx: i16, ry: i16, packet: u32) -> [u32; 4] {
    [packet, buttons as u32 | (lt as u32) << 16 | (rt as u32) << 24, lx as u16 as u32 | (ly as u16 as u32) << 16, rx as u16 as u32 | (ry as u16 as u32) << 16]
}
/// Axis value -1..1 -> XInput i16.
pub fn axis_i16(v: f32) -> i16 { (v.clamp(-1., 1.) * 32767.).round() as i16 }
pub fn trigger_u8(v: f32) -> u8 { (v.clamp(0., 1.) * 255.).round() as u8 }

pub struct NoPad;
impl PadBackend for NoPad { fn read_raw(&mut self) -> Option<[u32; 4]> { None } fn available(&self) -> bool { false } }

/// Settable state for tests.
#[derive(Default)]
pub struct ScriptedPad { pub state: Option<[u32; 4]> }
impl PadBackend for ScriptedPad { fn read_raw(&mut self) -> Option<[u32; 4]> { self.state } }

#[cfg(feature = "gamepad")]
pub use gilrs_backend::GilrsPad;

#[cfg(feature = "gamepad")]
mod gilrs_backend {
    use super::*;
    use gilrs::{Axis, Button, Gilrs};

    pub struct GilrsPad { gilrs: Gilrs, packet: u32 }
    impl GilrsPad {
        /// None when the platform backend cannot start (the game then runs keyboard/mouse only).
        pub fn new() -> Option<GilrsPad> {
            match Gilrs::new() { Ok(g) => Some(GilrsPad { gilrs: g, packet: 0 }), Err(e) => { eprintln!("[pad] gamepad support unavailable: {e}"); None } }
        }
        pub fn connected_names(&self) -> Vec<String> { self.gilrs.gamepads().map(|(_, g)| g.name().to_owned()).collect() }
    }
    impl PadBackend for GilrsPad {
        fn read_raw(&mut self) -> Option<[u32; 4]> {
            while self.gilrs.next_event().is_some() {} // keep the cached state current
            let (_, gp) = self.gilrs.gamepads().find(|(_, g)| g.is_connected())?;
            let mut b = 0u16;
            for (btn, mask) in [(Button::South, pad::A), (Button::East, pad::B), (Button::West, pad::X), (Button::North, pad::Y), (Button::LeftTrigger, pad::LB), (Button::RightTrigger, pad::RB),
                (Button::LeftThumb, pad::L3), (Button::RightThumb, pad::R3), (Button::Start, pad::START), (Button::Select, pad::BACK),
                (Button::DPadUp, 0x1), (Button::DPadDown, 0x2), (Button::DPadLeft, 0x4), (Button::DPadRight, 0x8)] {
                if gp.is_pressed(btn) { b |= mask; }
            }
            let trig = |btn: Button| gp.button_data(btn).map(|d| d.value()).unwrap_or_else(|| if gp.is_pressed(btn) { 1. } else { 0. });
            self.packet = self.packet.wrapping_add(1);
            Some(encode(b, trigger_u8(trig(Button::LeftTrigger2)), trigger_u8(trig(Button::RightTrigger2)),
                axis_i16(gp.value(Axis::LeftStickX)), axis_i16(gp.value(Axis::LeftStickY)), axis_i16(gp.value(Axis::RightStickX)), axis_i16(gp.value(Axis::RightStickY)), self.packet))
        }
    }
}

/// The pad as the app uses it: a boxed backend behind `Gamepad`.
pub struct DynPad(pub Box<dyn PadBackend>);
impl PadBackend for DynPad { fn read_raw(&mut self) -> Option<[u32; 4]> { self.0.read_raw() } fn available(&self) -> bool { self.0.available() } }

/// Best available backend.
pub fn open(enabled: bool) -> DynPad {
    #[cfg(feature = "gamepad")]
    if enabled { if let Some(p) = GilrsPad::new() { return DynPad(Box::new(p)); } }
    let _ = enabled;
    DynPad(Box::new(NoPad))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_game::inputdev::pad::State;
    #[test] fn encoding_round_trips_through_state_from_raw() {
        let raw = encode(pad::A | pad::L3, 12, 200, -1234, 32767, 5, -32768, 9);
        let s = State::from_raw(raw);
        assert_eq!(s.buttons, pad::A | pad::L3); assert_eq!((s.lt, s.rt), (12, 200)); assert_eq!((s.lx, s.ly, s.rx, s.ry), (-1234, 32767, 5, -32768));
    }
    #[test] fn axis_conversion_clamps() { assert_eq!(axis_i16(2.), 32767); assert_eq!(axis_i16(-1.), -32767); assert_eq!(axis_i16(0.), 0); assert_eq!(trigger_u8(1.5), 255); assert_eq!(trigger_u8(-1.), 0); }
    #[test] fn no_pad_is_unavailable_and_scripted_pad_reports_its_state() {
        assert!(!NoPad.available() && NoPad.read_raw().is_none());
        let mut p = fn_game::inputdev::pad::Gamepad::new(ScriptedPad { state: Some(encode(pad::START, 0, 0, 0, 0, 0, 0, 1)) });
        assert!(p.poll(0.016).is_some() && p.connected);
    }
}
