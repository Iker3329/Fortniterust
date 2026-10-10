//! Input devices beyond the keyboard poll: mouse-wheel pulses (so the wheel can be a key binding) and XInput gamepad play input.
//!
//! RECOVERED: nothing here comes from the 1.8 pak. The pak only provides the controller option NAMES/ranges (sensitivity 0.1..10,
//! vibration, "Configuration" and "Controller Platform" rotators; see reports/menu-layout-settings.json, settings-catalog.json).
//! RECOVERED (CookedIniVersion.txt = the cooker's ini value list in pakchunk0, and the 1.8 exe):
//! * stick dead zones: `ApplyGamepadStickDeadzone` (FortPlayerInput.cpp, FUN_1405fb290, called per stick by FUN_140615c60) with the cvars
//!   `gpad.LeftStickInnerDeadzone` 0.24 / `gpad.LeftStickOuterDeadzone` 0.01 / `gpad.RightStickInnerDeadzone` 0.27 / `gpad.RightStickOuterDeadzone` 0.01
//!   (ini L1927-L1930), then the per-axis `InputSettings:AxisConfig` DeadZone 0.25 / Exponent 1 / Sensitivity 1 of Gamepad_LeftX/LeftY/RightX/RightY
//!   (ini L1981-L1984) in `UFortPlayerInputAthena::vfunc_71` @1404392c0 (the MassageAxisInput override).
//! * buttons (ConfigD_Athena "Standard", the Athena default preset, ini L2627-2761): Jump = A (FaceButton_Bottom), Sprint = left-stick click,
//!   Fire = right trigger, B (FaceButton_Right) = `GamepadSwitchModeOrEditBuilding` / `CompleteBuildingEditInteraction` (L2659/L2660); there is
//!   NO gamepad Crouch mapping in Standard (Crouch is LeftControl only).
//! * right-stick look: `crate::gamepad_look` (FUN_140615dd0 with the cooked `/Game/Gamepad/AthenaGamepadSettings` curves and scales); [`pad::map`]
//!   only hands the raw right stick on.
//! ASSUMED: XInput as the only backend.
#![allow(dead_code)]
use std::sync::atomic::{AtomicI32, Ordering};

/// Mouse-wheel notches as pollable "keys". `push` is called from the window procedure, `begin_frame` once per `App::update`;
/// `up()`/`down()` then answer for that whole frame (a notch is a one-frame press, like a tap).
pub mod wheel {
    use super::*;
    static PENDING: AtomicI32 = AtomicI32::new(0);
    static FRAME: AtomicI32 = AtomicI32::new(0);
    /// WM_MOUSEWHEEL delta (a multiple of 120, positive = away from the user).
    pub fn push(delta: i32) { PENDING.fetch_add(delta, Ordering::Relaxed); }
    /// Latch the notches seen since the previous frame.
    pub fn begin_frame() { FRAME.store(PENDING.swap(0, Ordering::Relaxed), Ordering::Relaxed); }
    pub fn up() -> bool { FRAME.load(Ordering::Relaxed) > 0 }
    pub fn down() -> bool { FRAME.load(Ordering::Relaxed) < 0 }
    /// Answer a `Key::poll_code` for the wheel pseudo-keys; None for real virtual keys.
    pub fn poll(code: u16) -> Option<bool> {
        match code { fn_settings::WHEEL_UP_CODE => Some(up()), fn_settings::WHEEL_DOWN_CODE => Some(down()), _ => None }
    }
    #[cfg(test)] pub fn reset() { PENDING.store(0, Ordering::Relaxed); FRAME.store(0, Ordering::Relaxed); }
}

/// XInput (controller 0) gameplay input.
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
    /// The 1.8 Controller Sensitivity slider (0..1) to `GamepadLookSensitivity` (1..10): `UFortGameOptions::GamepadLookSensitivityXChanged`
    /// @142f044a0 stores value x (Max - 1) + 1 with Max = +0x27c = 10 (ctor @1430fd3f0, DAT_143fe3714); the getter @142f0a300 shows (sens - 1) / 9.
    pub fn slider_to_sensitivity(v: f32) -> f32 { v * (SENSITIVITY_MAX - 1.) + 1. }
    pub const SENSITIVITY_MAX: f32 = 10.;

    /// One frame of gameplay input from the pad. `look` = the raw right stick (x, y; y positive = pushed up), turned into view rotation by
    /// `crate::gamepad_look`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    /// `switch_mode_or_edit`: B (`GamepadSwitchModeOrEditBuilding`, ConfigD_Athena); no gameplay consumer yet.
    pub struct Frame { pub forward: f32, pub side: f32, pub look: (i16, i16), pub jump: bool, pub switch_mode_or_edit: bool, pub sprint: bool, pub attack: bool, pub menu: bool }

    /// Start button -> pause menu, on RELEASE of a press that began while the game was running. The menu itself reads Start as "back"
    /// on press, so opening on press would close it again at once; and after the menu closes with Start still held, the release must not
    /// reopen it (`reset` is called every frame the menu is open, so a press has to be seen again first).
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
    thread_local! { static START_LATCH: std::cell::RefCell<StartLatch> = std::cell::RefCell::new(StartLatch::default()); }
    /// True when Start was just released (open the pause menu). Call once per game frame with `Frame::menu`.
    pub fn start_released(down: bool) -> bool { START_LATCH.with(|s| s.borrow_mut().update(down)) }
    /// The menu is open: forget any Start press in progress.
    pub fn menu_opened() { START_LATCH.with(|s| s.borrow_mut().reset()); }

    /// Map a pad state to gameplay input: left stick move, right stick look (raw, see `crate::gamepad_look`), A jump,
    /// B switch mode / edit, left-stick click sprint, right trigger fire/swing (ConfigD_Athena).
    pub fn map(s: &State) -> Frame {
        let (mx, my) = stick(s.lx, s.ly, LEFT_DEADZONE);
        Frame {
            forward: my, side: mx, look: (s.rx, s.ry),
            jump: s.buttons & A != 0, switch_mode_or_edit: s.buttons & B != 0, sprint: s.buttons & L3 != 0, attack: s.rt >= TRIGGER_ON, menu: s.buttons & START != 0,
        }
    }

    /// Raw XInput-style backend: the 16 bytes of an `XINPUT_STATE` of controller 0 as four u32, or None when there is no controller / no API.
    /// (The old code loaded xinput1_4 / 1_3 / 9_1_0 dynamically with `LoadLibraryA`; that platform code lives in fn-app now.)
    pub trait PadBackend { fn read_raw(&mut self) -> Option<[u32; 4]>; fn available(&self) -> bool { true } }
    /// Gamepad poller: an empty slot is re-probed once per second because a read on a disconnected slot can be slow.
    pub struct Gamepad<B: PadBackend> { pub backend: B, retry: f32, pub connected: bool, pub last: State }
    impl<B: PadBackend> Gamepad<B> {
        pub fn new(backend: B) -> Self { Self { backend, retry: 0., connected: false, last: State::default() } }
        pub fn available(&self) -> bool { self.backend.available() }
        pub fn poll(&mut self, dt: f32) -> Option<State> {
            if !self.backend.available() { return None; }
            if self.retry > 0. { self.retry -= dt; return None; }
            let Some(raw) = self.backend.read_raw() else { self.retry = 1.; self.connected = false; self.last = State::default(); return None; };
            self.connected = true; self.last = State::from_raw(raw);
            Some(self.last)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pad::*;
    use super::wheel;
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
        assert_eq!(f.look, (32767, -5), "the right stick goes to gamepad_look raw");
        assert_eq!((f.forward, f.side), (0., 0.));
    }
    /// @142f044a0 / @142f0a300: slider 0..1 <-> sensitivity 1..10; the 5 default shows as 0.44.
    #[test] fn sensitivity_slider_maps_to_one_through_ten() {
        assert_eq!((slider_to_sensitivity(0.), slider_to_sensitivity(1.)), (1., 10.));
        assert!((slider_to_sensitivity(4. / 9.) - 5.).abs() < 1e-6);
    }
    #[test] fn buttons_map_to_actions() {
        let f = map(&State { buttons: A, ..Default::default() }, &cfg()); assert!(f.jump && !f.switch_mode_or_edit && !f.sprint && !f.attack);
        let f = map(&State { buttons: B, ..Default::default() }, &cfg()); assert!(f.switch_mode_or_edit && !f.jump, "B = GamepadSwitchModeOrEditBuilding, not crouch (ConfigD_Athena L2659)");
        let f = map(&State { buttons: L3, ..Default::default() }, &cfg()); assert!(f.sprint);
        let f = map(&State { rt: 255, ..Default::default() }, &cfg()); assert!(f.attack);
        assert!(!map(&State { rt: 99, ..Default::default() }, &cfg()).attack && map(&State { rt: 100, ..Default::default() }, &cfg()).attack);
        let f = map(&State { buttons: X | Y | LB | RB | R3 | BACK, lt: 255, ..Default::default() }, &cfg()); assert_eq!(f, Frame::default(), "the other buttons belong to pad_layout::PadMapper, not to the movement frame");
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
        use fn_settings::{Bindings, Key, A_CROUCH, A_FIRE, A_JUMP, WHEEL_DOWN_CODE, WHEEL_UP_CODE};
        let mut b = Bindings::default();
        b.keys[A_JUMP] = [Key::from_name("MouseScrollUp").unwrap(), Key::from_name("ThumbMouseButton2").unwrap()];
        b.keys[A_CROUCH] = [Key::from_name("MouseScrollDown").unwrap(), Key::NONE];
        b.keys[A_FIRE] = [Key::NONE, Key::from_name("RightAlt").unwrap()];
        assert!(b.down(A_JUMP, |c| c == WHEEL_UP_CODE) && !b.down(A_JUMP, |c| c == WHEEL_DOWN_CODE));
        assert!(b.down(A_JUMP, |c| c == 0x06), "thumb button 2 = VK_XBUTTON2");
        assert!(b.down(A_CROUCH, |c| c == WHEEL_DOWN_CODE) && !b.down(A_CROUCH, |c| c == 0x20));
        assert!(b.down(A_FIRE, |c| c == 0xA5) && !b.down(A_FIRE, |c| c == WHEEL_UP_CODE));
        // (the global wheel latch itself is exercised by `wheel_notches_are_one_frame_pulses`; tests run in parallel)
    }
    #[test] fn wheel_notches_are_one_frame_pulses() {
        wheel::reset();
        assert!(!wheel::up() && !wheel::down());
        wheel::push(120); wheel::push(120);
        assert!(!wheel::up(), "not visible until the next frame begins");
        wheel::begin_frame(); assert!(wheel::up() && !wheel::down());
        wheel::begin_frame(); assert!(!wheel::up(), "pulse lasts exactly one frame");
        wheel::push(-120); wheel::begin_frame(); assert!(wheel::down() && !wheel::up());
        wheel::push(120); wheel::push(-120); wheel::begin_frame(); assert!(!wheel::up() && !wheel::down(), "cancelling notches in one frame net to nothing");
        assert_eq!(wheel::poll(fn_settings::WHEEL_UP_CODE), Some(false)); assert_eq!(wheel::poll(0x20), None);
        wheel::reset();
    }
}

#[cfg(test)]
mod gamepad_tests {
    use super::pad::*;
    struct Fake { present: bool, reads: u32 }
    impl PadBackend for Fake { fn read_raw(&mut self) -> Option<[u32; 4]> { self.reads += 1; if self.present { Some([1, 0x0000_1000, 0, 0]) } else { None } } }
    #[test] fn an_empty_slot_is_reprobed_once_per_second_and_a_present_pad_every_frame() {
        let mut g = Gamepad::new(Fake { present: false, reads: 0 });
        for _ in 0..30 { assert!(g.poll(1. / 60.).is_none()); }
        assert_eq!(g.backend.reads, 1, "30 frames (0.5 s) after a failed read: no further probe"); assert!(!g.connected);
        for _ in 0..40 { g.poll(1. / 60.); } assert_eq!(g.backend.reads, 2, "re-probed after a second");
        g.backend.present = true; for _ in 0..70 { g.poll(1. / 60.); }
        assert!(g.connected && g.poll(1. / 60.).map(|s| s.buttons & A != 0) == Some(true));
        let reads = g.backend.reads; g.poll(1. / 60.); assert_eq!(g.backend.reads, reads + 1, "a connected pad is read every frame");
    }
}