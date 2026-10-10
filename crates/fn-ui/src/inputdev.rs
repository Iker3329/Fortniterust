//! Pure parts of the old runtime's `inputdev.rs`: gamepad gameplay mapping and the mouse-wheel pulse latch.
//!
//! Mirror of `fn_game::inputdev::pad` (keep in step). RECOVERED there: the stick dead zones (gpad.* cvars, CookedIniVersion.txt L1927-L1930,
//! ApplyGamepadStickDeadzone FUN_1405fb290, then the AxisConfig DeadZone 0.25 of L1981-L1984) and the ConfigD_Athena buttons (B = switch mode /
//! edit, no gamepad crouch). The right-stick look (curves, scales, acceleration from the cooked AthenaGamepadSettings, FUN_140615dd0) lives in
//! `fn_game::gamepad_look`; [`pad::map`] only hands the raw stick on. ASSUMED: XInput layout as the only backend.
//!
//! No XInput loading and no thread-local gamepad: the platform layer polls the device (it implements [`crate::input::PadSource`] for the
//! menu and fills a [`pad::PadStatus`]), this module only decodes and maps.
#![allow(dead_code)]

/// Mouse-wheel notches as pollable "keys". The platform calls `push` from its wheel event, the game calls `begin_frame` once per update;
/// `up()`/`down()` then answer for that whole frame (a notch is a one-frame press, like a tap). An owned value (the old runtime used
/// process-global atomics).
#[derive(Clone, Copy, Debug, Default)]
pub struct WheelLatch { pending: i32, frame: i32 }
impl WheelLatch {
    /// Wheel delta (a multiple of 120, positive = away from the user).
    pub fn push(&mut self, delta: i32) { self.pending += delta; }
    /// Latch the notches seen since the previous frame.
    pub fn begin_frame(&mut self) { self.frame = std::mem::take(&mut self.pending); }
    pub fn up(&self) -> bool { self.frame > 0 }
    pub fn down(&self) -> bool { self.frame < 0 }
    /// Answer a `Key::poll_code` for the wheel pseudo-keys; None for real virtual keys.
    pub fn poll(&self, code: u16) -> Option<bool> {
        match code { crate::settings::WHEEL_UP_CODE => Some(self.up()), crate::settings::WHEEL_DOWN_CODE => Some(self.down()), _ => None }
    }
    pub fn reset(&mut self) { *self = Self::default(); }
}

/// Gamepad gameplay input (XInput layout).
pub mod pad {
    pub const A: u16 = 0x1000; pub const B: u16 = 0x2000; pub const X: u16 = 0x4000; pub const Y: u16 = 0x8000;
    pub const LB: u16 = 0x0100; pub const RB: u16 = 0x0200; pub const L3: u16 = 0x0040; pub const R3: u16 = 0x0080;
    pub const START: u16 = 0x0010; pub const BACK: u16 = 0x0020;
    /// `gpad.LeftStickInnerDeadzone` / `gpad.RightStickInnerDeadzone` (CookedIniVersion.txt L1927 / L1929).
    pub const LEFT_DEADZONE: f32 = 0.24;
    pub const RIGHT_DEADZONE: f32 = 0.27;
    /// `gpad.LeftStickOuterDeadzone` = `gpad.RightStickOuterDeadzone` (L1928 / L1930).
    pub const OUTER_DEADZONE: f32 = 0.01;
    /// `InputSettings:AxisConfig` DeadZone of Gamepad_LeftX/LeftY/RightX/RightY (L1981-L1984; Exponent 1, Sensitivity 1).
    pub const AXIS_DEADZONE: f32 = 0.25;
    /// Trigger counts above which RT counts as pressed (stock XINPUT_GAMEPAD_TRIGGER_THRESHOLD is 30; a firmer pull avoids ghost swings).
    pub const TRIGGER_ON: u8 = 100;

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct State { pub buttons: u16, pub lt: u8, pub rt: u8, pub lx: i16, pub ly: i16, pub rx: i16, pub ry: i16 }
    impl State {
        /// Decode the 16 raw bytes of XINPUT_STATE (packet, buttons+triggers, LX+LY, RX+RY).
        pub fn from_raw(raw: [u32; 4]) -> State {
            State { buttons: (raw[1] & 0xffff) as u16, lt: ((raw[1] >> 16) & 0xff) as u8, rt: (raw[1] >> 24) as u8,
                lx: (raw[2] & 0xffff) as u16 as i16, ly: (raw[2] >> 16) as u16 as i16, rx: (raw[3] & 0xffff) as u16 as i16, ry: (raw[3] >> 16) as u16 as i16 }
        }
    }

    /// What the Controller settings tab shows about the device: filled by the platform layer (`menu::pages::set_pad_status`).
    /// `available` = a pad backend exists on this system, `connected` = a controller answered the last poll, `last` = its last state.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct PadStatus { pub available: bool, pub connected: bool, pub last: Option<State> }

    /// `ApplyGamepadStickDeadzone` (FUN_1405fb290) with inner dead zone `dz` and the 0.01 outer dead zone: magnitude clamped to [inner, 1 - outer],
    /// rescaled to 0..1 by (m - inner) / (1 - inner - outer), along the stick direction. Returns x/y in -1..1 (y positive = stick pushed up/forward).
    pub fn radial_deadzone(x: i16, y: i16, dz: f32) -> (f32, f32) {
        let (fx, fy) = (x as f32 / 32767., y as f32 / 32767.);
        let span = 1. - dz - OUTER_DEADZONE;
        if !(span > 0. && span <= 1.) { return (0., 0.); } // the exe logs "Invalid deadzones" and skips; never reached with the ini values
        let m = (fx * fx + fy * fy).sqrt();
        let (dx, dy) = if m > 1e-8 { (fx / m, fy / m) } else { (0., 0.) };
        let k = (m.clamp(dz, 1. - OUTER_DEADZONE) - dz) / span;
        (dx * k, dy * k)
    }
    /// The per-axis AxisConfig dead zone of `vfunc_71`: |v| - 0.25 rescaled to 0..1 (Exponent 1 and Sensitivity 1 leave it unchanged).
    pub fn axis_deadzone(v: f32) -> f32 { (v.abs() - AXIS_DEADZONE).max(0.) / (1. - AXIS_DEADZONE) * v.signum() }
    /// A stick as the game reads it: [`radial_deadzone`] then [`axis_deadzone`] on each axis.
    pub fn stick(x: i16, y: i16, dz: f32) -> (f32, f32) { let (x, y) = radial_deadzone(x, y, dz); (axis_deadzone(x), axis_deadzone(y)) }
    /// The Controller Sensitivity slider (0..1) to `GamepadLookSensitivity` (1..10): value x (10 - 1) + 1 (`GamepadLookSensitivityXChanged` @142f044a0,
    /// Max 10 from the UFortGameOptions ctor @1430fd3f0); the getter @142f0a300 shows (sens - 1) / 9, so the default 5 reads 0.44.
    pub fn slider_to_sensitivity(v: f32) -> f32 { v * 9. + 1. }

    /// One frame of gameplay input from the pad. `look` = the raw right stick, turned into rotation by `fn_game::gamepad_look`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    /// `switch_mode_or_edit`: B (`GamepadSwitchModeOrEditBuilding`, ConfigD_Athena); no gameplay consumer yet.
    pub struct Frame { pub forward: f32, pub side: f32, pub look: (i16, i16), pub jump: bool, pub switch_mode_or_edit: bool, pub sprint: bool, pub attack: bool, pub menu: bool }

    /// Start button -> pause menu, on RELEASE of a press that began while the game was running. The menu itself reads Start as "back"
    /// on press, so opening on press would close it again at once; and after the menu closes with Start still held, the release must not
    /// reopen it (`reset` is called every frame the menu is open, so a press has to be seen again first). Owned by the caller (the old
    /// runtime kept one in a thread-local).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct StartLatch { state: u8 } // 0 = disarmed (waiting for a released button), 1 = armed, 2 = pressed
    impl StartLatch {
        pub fn reset(&mut self) { self.state = 0; }
        /// Feed the current Start state once per game frame; true on the frame the button is released after a clean press.
        pub fn update(&mut self, down: bool) -> bool {
            match (self.state, down) {
                (0, false) => { self.state = 1; false }
                (1, true) => { self.state = 2; false }
                (2, false) => { self.state = 1; true }
                _ => false,
            }
        }
    }

    /// Map a pad state to gameplay input: left stick move, right stick look (raw), A jump, B switch mode / edit, left-stick click sprint,
    /// right trigger fire/swing (ConfigD_Athena).
    pub fn map(s: &State) -> Frame {
        let (mx, my) = stick(s.lx, s.ly, LEFT_DEADZONE);
        Frame {
            forward: my, side: mx, look: (s.rx, s.ry),
            jump: s.buttons & A != 0, switch_mode_or_edit: s.buttons & B != 0, sprint: s.buttons & L3 != 0, attack: s.rt >= TRIGGER_ON, menu: s.buttons & START != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pad::*;
    use super::WheelLatch;
    fn map(s: &State, _: &()) -> Frame { super::pad::map(s) }
    fn cfg() {}

    #[test] fn raw_state_decodes_every_field() {
        let s = State::from_raw([7, 0xC840_1040, 0xF000_1234, 0x8000_7FFF]);
        assert_eq!(s.buttons, A | L3);
        assert_eq!((s.lt, s.rt), (0x40, 0xC8));
        assert_eq!((s.lx, s.ly), (0x1234, 0xF000u16 as i16));
        assert_eq!((s.rx, s.ry), (0x7FFF, 0x8000u16 as i16));
    }
    #[test] fn deadzone_zeroes_inside_and_reaches_one_outside() {
        assert_eq!(radial_deadzone(0, 0, LEFT_DEADZONE), (0., 0.));
        assert_eq!(radial_deadzone(7000, 0, LEFT_DEADZONE), (0., 0.), "7000 / 32767 < 0.24");
        assert_eq!(radial_deadzone(5000, 5000, LEFT_DEADZONE), (0., 0.), "radial: 7071 magnitude is still inside");
        let (x, y) = radial_deadzone(32767, 0, LEFT_DEADZONE); assert!((x - 1.).abs() < 1e-4 && y == 0.);
        let (x, _) = radial_deadzone(32440, 0, LEFT_DEADZONE); assert!((x - 1.).abs() < 1e-3, "the 0.01 outer dead zone reaches 1 early: {x}");
        let (x, _) = radial_deadzone(16384, 0, RIGHT_DEADZONE); assert!((x - (0.5 - 0.27) / 0.72).abs() < 1e-3, "{x}");
        assert_eq!(axis_deadzone(0.25), 0.); assert!((axis_deadzone(-0.625) + 0.5).abs() < 1e-6); assert_eq!(axis_deadzone(1.), 1.);
        let (x, y) = radial_deadzone(-32768, 32767, LEFT_DEADZONE); assert!(x >= -1. && y <= 1.);
        let (x, _) = radial_deadzone(16000, 0, LEFT_DEADZONE); assert!(x > 0. && x < 0.6, "{x}");
        let (x1, _) = radial_deadzone(9000, 0, LEFT_DEADZONE); let (x2, _) = radial_deadzone(20000, 0, LEFT_DEADZONE); assert!(x2 > x1 && x1 > 0.);
    }
    #[test] fn deadzone_preserves_direction() {
        let (x, y) = radial_deadzone(20000, 10000, LEFT_DEADZONE);
        assert!((y / x - 0.5).abs() < 1e-3, "{x} {y}");
    }
    #[test] fn idle_pad_does_nothing() { assert_eq!(map(&State::default(), &cfg()), Frame::default()); }
    #[test] fn left_stick_moves_right_stick_looks() {
        let f = map(&State { ly: 32767, ..Default::default() }, &cfg());
        assert!(f.forward > 0.99 && f.side == 0. && f.look == (0, 0));
        let f = map(&State { lx: -32768, ..Default::default() }, &cfg());
        assert!(f.side < -0.99 && f.forward == 0.);
        let f = map(&State { ly: 32767, lx: 32767, ..Default::default() }, &cfg());
        let d = (0.5f32.sqrt() - 0.25) / 0.75; // radial: unit circle, then the per-axis 0.25 AxisConfig dead zone
        assert!((f.forward - f.side).abs() < 1e-4 && (f.forward - d).abs() < 1e-3, "full diagonal: {f:?}");
        let f = map(&State { rx: 32767, ry: -5, ..Default::default() }, &cfg());
        assert_eq!(f.look, (32767, -5));
        assert_eq!((f.forward, f.side), (0., 0.));
    }
    #[test] fn sensitivity_slider_maps_to_one_through_ten() {
        assert_eq!((slider_to_sensitivity(0.), slider_to_sensitivity(1.)), (1., 10.)); assert!((slider_to_sensitivity(4. / 9.) - 5.).abs() < 1e-6);
    }
    #[test] fn buttons_map_to_actions() {
        let f = map(&State { buttons: A, ..Default::default() }, &cfg()); assert!(f.jump && !f.switch_mode_or_edit && !f.sprint && !f.attack);
        let f = map(&State { buttons: B, ..Default::default() }, &cfg()); assert!(f.switch_mode_or_edit && !f.jump, "B = GamepadSwitchModeOrEditBuilding, not crouch (ConfigD_Athena L2659)");
        let f = map(&State { buttons: L3, ..Default::default() }, &cfg()); assert!(f.sprint);
        let f = map(&State { rt: 255, ..Default::default() }, &cfg()); assert!(f.attack);
        assert!(!map(&State { rt: 99, ..Default::default() }, &cfg()).attack && map(&State { rt: 100, ..Default::default() }, &cfg()).attack);
        let f = map(&State { buttons: X | Y | LB | RB | R3 | BACK, lt: 255, ..Default::default() }, &cfg()); assert_eq!(f, Frame::default(), "other buttons are unused");
        assert_eq!(map(&State { buttons: START, ..Default::default() }, &cfg()), Frame { menu: true, ..Default::default() }, "Start only asks for the pause menu");
    }
    #[test] fn start_opens_the_menu_on_release_only() {
        let mut l = StartLatch::default();
        // starts disarmed (e.g. the game just resumed from the menu with Start held): the release must not reopen it
        assert!(!l.update(true)); assert!(!l.update(true)); assert!(!l.update(false), "release of a press the game never saw: no menu");
        assert!(!l.update(false));
        assert!(!l.update(true), "press: nothing yet"); assert!(!l.update(true), "hold: nothing yet");
        assert!(l.update(false), "release: open the menu"); assert!(!l.update(false));
        // quick tap
        assert!(!l.update(true)); assert!(l.update(false));
        // the menu opening mid-press cancels it
        assert!(!l.update(true)); l.reset(); assert!(!l.update(false), "reset disarms until a released frame is seen");
        assert!(!l.update(true)); assert!(l.update(false));
    }
    #[test] fn wheel_and_mouse_codes_are_polled_through_bindings() {
        use crate::settings::{Bindings, Key, A_CROUCH, A_FIRE, A_JUMP, WHEEL_DOWN_CODE, WHEEL_UP_CODE};
        let mut b = Bindings::default();
        b.keys[A_JUMP] = [Key::from_name("MouseScrollUp").unwrap(), Key::from_name("ThumbMouseButton2").unwrap()];
        b.keys[A_CROUCH] = [Key::from_name("MouseScrollDown").unwrap(), Key::NONE];
        b.keys[A_FIRE] = [Key::NONE, Key::from_name("RightAlt").unwrap()];
        assert!(b.down(A_JUMP, |c| c == WHEEL_UP_CODE) && !b.down(A_JUMP, |c| c == WHEEL_DOWN_CODE));
        assert!(b.down(A_JUMP, |c| c == 0x06), "thumb button 2 = VK_XBUTTON2");
        assert!(b.down(A_CROUCH, |c| c == WHEEL_DOWN_CODE) && !b.down(A_CROUCH, |c| c == 0x20));
        assert!(b.down(A_FIRE, |c| c == 0xA5) && !b.down(A_FIRE, |c| c == WHEEL_UP_CODE));
    }
    #[test] fn wheel_notches_are_one_frame_pulses() {
        let mut w = WheelLatch::default();
        assert!(!w.up() && !w.down());
        w.push(120); w.push(120);
        assert!(!w.up(), "not visible until the next frame begins");
        w.begin_frame(); assert!(w.up() && !w.down());
        w.begin_frame(); assert!(!w.up(), "pulse lasts exactly one frame");
        w.push(-120); w.begin_frame(); assert!(w.down() && !w.up());
        w.push(120); w.push(-120); w.begin_frame(); assert!(!w.up() && !w.down(), "cancelling notches in one frame net to nothing");
        assert_eq!(w.poll(crate::settings::WHEEL_UP_CODE), Some(false)); assert_eq!(w.poll(0x20), None);
    }
}
