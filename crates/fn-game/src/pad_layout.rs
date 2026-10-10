use crate::inputdev::pad::{self, State, A, B, BACK, L3, LB, R3, RB, X, Y};
use fn_settings::{A_BUILDING_EDIT_RESET, A_CHANGE_MATERIAL, A_FIRE, A_PERFORM_BUILDING_EDIT_INTERACTION, A_PERFORM_BUILDING_IMPROVEMENT_INTERACTION, A_PRIMARY_QUICKBAR_SLOT1,
    A_RELOAD, A_ROTATE, A_SECONDARY_QUICKBAR_SLOT_F1, A_SLOT_DOWN, A_SLOT_UP, A_SWITCH_QUICKBAR, A_TARGET, A_USE};

pub const DPAD_UP: u16 = 0x1;
pub const DPAD_DOWN: u16 = 0x2;
pub const DPAD_LEFT: u16 = 0x4;
pub const DPAD_RIGHT: u16 = 0x8;
pub const HOLD_INPUT_TIME: f32 = 0.25;
const LT_BIT: u32 = 1 << 16;
const RT_BIT: u32 = 1 << 17;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout { #[default] Standard, QuickBuilder, CombatPro }
impl Layout {
    pub const ALL: [Layout; 3] = [Layout::Standard, Layout::QuickBuilder, Layout::CombatPro];
    pub fn from_index(i: usize) -> Layout { Self::ALL[i.min(2)] }
    pub fn preset(self) -> &'static str { match self { Layout::Standard => "ConfigD_Athena", Layout::QuickBuilder => "ConfigE_Athena", Layout::CombatPro => "ConfigF_Athena" } }
    pub fn from_settings(s: &fn_settings::Settings) -> Layout { Self::from_index(fn_settings::extras::configuration(s)) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode { #[default] Combat, Build, Edit }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Actions {
    pub held: Vec<usize>,
    pub crouch: bool,
    pub crouch_press: bool,
    pub toggle_harvest: bool,
    pub map: bool,
    pub inventory: bool,
}
impl Actions {
    pub fn has(&self, action: usize) -> bool { self.held.contains(&action) }
    fn push(&mut self, action: usize) { if !self.held.contains(&action) { self.held.push(action); } }
}

#[derive(Clone, Debug, Default)]
pub struct PadMapper {
    prev: u32,
    block: u32,
    b_timer: Option<f32>,
    b_edit: bool,
    y_timer: Option<f32>,
    x_use: bool,
    crouched: bool,
}

fn bits(s: &State) -> u32 { s.buttons as u32 | if s.lt >= pad::TRIGGER_ON { LT_BIT } else { 0 } | if s.rt >= pad::TRIGGER_ON { RT_BIT } else { 0 } }

impl PadMapper {
    pub fn new() -> Self { Self::default() }
    pub fn crouched(&self) -> bool { self.crouched }
    pub fn suppress(&mut self, s: Option<&State>) {
        let crouched = self.crouched;
        *self = PadMapper { block: s.map_or(0, bits), crouched, ..Default::default() };
        self.prev = self.block;
    }
    pub fn uncrouch(&mut self) { self.crouched = false; }

    pub fn update(&mut self, s: Option<&State>, layout: Layout, mode: Mode, interact: bool, dt: f32) -> Actions {
        let Some(s) = s else { *self = PadMapper::default(); return Actions::default(); };
        let raw = bits(s);
        self.block &= raw;
        let cur = raw & !self.block;
        let pressed = cur & !self.prev;
        let released = !cur & self.prev;
        self.prev = cur;
        let down = |m: u32| cur & m != 0;
        let press = |m: u32| pressed & m != 0;
        let release = |m: u32| released & m != 0;
        let (b, x, y, lb, rb, l3, r3, a_btn) = (B as u32, X as u32, Y as u32, LB as u32, RB as u32, L3 as u32, R3 as u32, A as u32);
        let mut a = Actions::default();

        if mode == Mode::Edit {
            self.b_timer = None;
            if down(b) { a.push(A_PERFORM_BUILDING_EDIT_INTERACTION); }
            if !down(b) { self.b_edit = false; }
        } else if self.b_edit {
            if down(b) { a.push(A_PERFORM_BUILDING_EDIT_INTERACTION); } else { self.b_edit = false; }
        } else {
            if press(b) { self.b_timer = Some(0.); }
            if let Some(t) = self.b_timer {
                if down(b) {
                    let t = t + dt.max(0.);
                    if t >= HOLD_INPUT_TIME { self.b_timer = None; self.b_edit = true; a.push(A_PERFORM_BUILDING_EDIT_INTERACTION); } else { self.b_timer = Some(t); }
                } else { self.b_timer = None; a.push(A_SWITCH_QUICKBAR); }
            }
        }

        if press(a_btn) { self.crouched = false; }

        match mode {
            Mode::Combat => {
                if down(LT_BIT) { a.push(A_TARGET); }
                if press(x) { self.x_use = interact; }
                if down(x) && self.x_use { a.push(A_USE); }
                if release(x) { if !self.x_use { a.push(A_RELOAD); } self.x_use = false; }
                match layout {
                    Layout::Standard | Layout::QuickBuilder => {
                        if press(y) { self.y_timer = Some(0.); }
                        if let Some(t) = self.y_timer {
                            if down(y) {
                                let t = t + dt.max(0.);
                                if t >= HOLD_INPUT_TIME { self.y_timer = None; a.push(A_PRIMARY_QUICKBAR_SLOT1); } else { self.y_timer = Some(t); }
                            } else { self.y_timer = None; a.push(A_SLOT_UP); }
                        }
                        if press(lb) { self.toggle_crouch(&mut a); }
                    }
                    Layout::CombatPro => {
                        self.y_timer = None;
                        if press(y) { a.toggle_harvest = true; }
                        if press(rb) { a.push(A_SLOT_UP); }
                        if press(lb) { a.push(A_SLOT_DOWN); }
                        if press(r3) { self.toggle_crouch(&mut a); }
                    }
                }
                if down(r3) { a.push(A_PERFORM_BUILDING_IMPROVEMENT_INTERACTION); }
            }
            Mode::Build => {
                self.y_timer = None; self.x_use = false;
                if down(RT_BIT) { a.push(A_FIRE); }
                if down(x) { a.push(A_USE); }
                match layout {
                    Layout::Standard => {
                        if press(y) { a.push(A_SLOT_UP); }
                        if down(lb) { a.push(A_CHANGE_MATERIAL); }
                        if down(rb) { a.push(A_ROTATE); }
                        if down(r3) { a.push(A_PERFORM_BUILDING_IMPROVEMENT_INTERACTION); }
                    }
                    Layout::QuickBuilder => {
                        if press(y) { a.push(A_SECONDARY_QUICKBAR_SLOT_F1 + 4); }
                        if press(lb) { a.push(A_SLOT_DOWN); }
                        if press(rb) { a.push(A_SLOT_UP); }
                        if down(r3) { a.push(A_ROTATE); }
                        if down(l3) { a.push(A_CHANGE_MATERIAL); }
                    }
                    Layout::CombatPro => {
                        if press(y) { a.toggle_harvest = true; }
                        if press(lb) { a.push(A_SLOT_DOWN); }
                        if press(rb) { a.push(A_SLOT_UP); }
                        if down(r3) { a.push(A_ROTATE); }
                        if down(LT_BIT) { a.push(A_CHANGE_MATERIAL); }
                    }
                }
            }
            Mode::Edit => {
                self.y_timer = None; self.x_use = false;
                if down(RT_BIT) { a.push(A_FIRE); }
                if down(r3) { a.push(A_BUILDING_EDIT_RESET); }
            }
        }

        let (map_btn, inv_btn) = if layout == Layout::CombatPro { (BACK as u32, DPAD_UP as u32) } else { (DPAD_UP as u32, BACK as u32) };
        a.map = press(map_btn);
        a.inventory = press(inv_btn);
        a.crouch = self.crouched;
        a
    }

    fn toggle_crouch(&mut self, a: &mut Actions) { self.crouched = !self.crouched; a.crouch_press = true; }
}

pub fn rescale_axis_pair(x: i16, y: i16, user_dz: f32, base_dz: f32) -> (i16, i16) {
    if (user_dz - base_dz).abs() < 1e-6 { return (x, y); }
    let (fx, fy) = (x as f32 / 32767., y as f32 / 32767.);
    let m = (fx * fx + fy * fy).sqrt();
    if m <= 1e-8 { return (x, y); }
    let o = pad::OUTER_DEADZONE;
    let user_span = 1. - user_dz - o;
    let k = if user_span > 0. { ((m.clamp(user_dz, 1. - o) - user_dz) / user_span).clamp(0., 1.) } else { 0. };
    let m2 = if k <= 0. { 0. } else { base_dz + k * (1. - base_dz - o) };
    let s = m2 / m;
    (((fx * s) * 32767.).round().clamp(-32768., 32767.) as i16, ((fy * s) * 32767.).round().clamp(-32768., 32767.) as i16)
}

pub fn apply_deadzones(s: State, left_dz: f32, right_dz: f32) -> State {
    let (lx, ly) = rescale_axis_pair(s.lx, s.ly, left_dz, pad::LEFT_DEADZONE);
    let (rx, ry) = rescale_axis_pair(s.rx, s.ry, right_dz, pad::RIGHT_DEADZONE);
    State { lx, ly, rx, ry, ..s }
}

#[cfg(test)]
mod tests {
    use super::*;
    const DT: f32 = 1. / 60.;
    fn st(buttons: u16) -> State { State { buttons, ..Default::default() } }
    fn run(m: &mut PadMapper, s: &State, l: Layout, mode: Mode, n: usize) -> Vec<Actions> { (0..n).map(|_| m.update(Some(s), l, mode, false, DT)).collect() }

    #[test] fn b_tap_switches_quickbar_and_b_hold_begins_edit() {
        let mut m = PadMapper::new();
        let a = m.update(Some(&st(B)), Layout::Standard, Mode::Combat, false, DT); assert!(a.held.is_empty(), "nothing on the press itself");
        let a = m.update(Some(&st(0)), Layout::Standard, Mode::Combat, false, DT); assert_eq!(a.held, vec![A_SWITCH_QUICKBAR], "release before HoldInputTime = SwitchQuickbar");
        let mut m = PadMapper::new();
        let frames = run(&mut m, &st(B), Layout::Standard, Mode::Combat, 20);
        let first = frames.iter().position(|a| a.has(A_PERFORM_BUILDING_EDIT_INTERACTION)).unwrap();
        assert!((first as f32 + 1.) * DT >= HOLD_INPUT_TIME && (first as f32) * DT < HOLD_INPUT_TIME + DT, "edit fires once the 0.25 s hold elapsed (frame {first})");
        assert!(frames[first..].iter().all(|a| a.has(A_PERFORM_BUILDING_EDIT_INTERACTION)), "held until B is released");
        assert!(frames.iter().all(|a| !a.has(A_SWITCH_QUICKBAR)));
        let a = m.update(Some(&st(B)), Layout::Standard, Mode::Edit, false, DT); assert!(a.has(A_PERFORM_BUILDING_EDIT_INTERACTION));
        let a = m.update(Some(&st(0)), Layout::Standard, Mode::Edit, false, DT); assert!(!a.has(A_PERFORM_BUILDING_EDIT_INTERACTION) && !a.has(A_SWITCH_QUICKBAR), "the release after a hold-edit is not a quickbar switch");
    }
    #[test] fn b_in_edit_mode_is_the_complete_key_directly() {
        let mut m = PadMapper::new();
        let a = m.update(Some(&st(B)), Layout::CombatPro, Mode::Edit, false, DT); assert_eq!(a.held, vec![A_PERFORM_BUILDING_EDIT_INTERACTION], "CompleteBuildingEditInteraction on the press");
        let a = m.update(Some(&st(0)), Layout::CombatPro, Mode::Edit, false, DT); assert!(a.held.is_empty());
    }
    #[test] fn y_tap_is_next_weapon_and_y_hold_is_the_pickaxe() {
        let mut m = PadMapper::new();
        m.update(Some(&st(Y)), Layout::Standard, Mode::Combat, false, DT);
        assert_eq!(m.update(Some(&st(0)), Layout::Standard, Mode::Combat, false, DT).held, vec![A_SLOT_UP]);
        let frames = run(&mut m, &st(Y), Layout::QuickBuilder, Mode::Combat, 30);
        assert_eq!(frames.iter().filter(|a| a.has(A_PRIMARY_QUICKBAR_SLOT1)).count(), 1, "HotbarKey1 once");
        assert!(m.update(Some(&st(0)), Layout::QuickBuilder, Mode::Combat, false, DT).held.is_empty(), "no next weapon after a hold");
    }
    #[test] fn standard_combat_buttons() {
        let mut m = PadMapper::new();
        let a = m.update(Some(&State { lt: 255, ..Default::default() }), Layout::Standard, Mode::Combat, false, DT); assert!(a.has(A_TARGET));
        let a = m.update(Some(&st(LB)), Layout::Standard, Mode::Combat, false, DT); assert!(a.crouch_press && a.crouch, "GamepadAbility1 = Crouch / UnCrouch (Athena vfunc_640 -> vfunc_808)");
        m.update(Some(&st(0)), Layout::Standard, Mode::Combat, false, DT);
        let a = m.update(Some(&st(LB)), Layout::Standard, Mode::Combat, false, DT); assert!(a.crouch_press && !a.crouch, "second press uncrouches");
        m.update(Some(&st(0)), Layout::Standard, Mode::Combat, false, DT);
        let a = m.update(Some(&st(RB)), Layout::Standard, Mode::Combat, false, DT); assert!(a.held.is_empty() && !a.crouch_press, "GamepadAbility2 is the STW gadget");
        m.update(Some(&st(0)), Layout::Standard, Mode::Combat, false, DT);
        let a = m.update(Some(&st(DPAD_UP)), Layout::Standard, Mode::Combat, false, DT); assert!(a.map && !a.inventory);
        let a = m.update(Some(&st(BACK)), Layout::Standard, Mode::Combat, false, DT); assert!(a.inventory && !a.map);
        let a = m.update(Some(&st(BACK)), Layout::Standard, Mode::Combat, false, DT); assert!(!a.inventory, "press only");
    }
    #[test] fn x_uses_with_a_target_and_reloads_on_release_without_one() {
        let mut m = PadMapper::new();
        let a = m.update(Some(&st(X)), Layout::Standard, Mode::Combat, true, DT); assert!(a.has(A_USE));
        let a = m.update(Some(&st(X)), Layout::Standard, Mode::Combat, false, DT); assert!(a.has(A_USE), "held while the button is");
        let a = m.update(Some(&st(0)), Layout::Standard, Mode::Combat, false, DT); assert!(!a.has(A_RELOAD) && !a.has(A_USE));
        let a = m.update(Some(&st(X)), Layout::Standard, Mode::Combat, false, DT); assert!(a.held.is_empty());
        let a = m.update(Some(&st(0)), Layout::Standard, Mode::Combat, false, DT); assert_eq!(a.held, vec![A_RELOAD]);
    }
    #[test] fn combat_pro_bumpers_cycle_and_y_toggles_harvest() {
        let mut m = PadMapper::new();
        assert!(m.update(Some(&st(RB)), Layout::CombatPro, Mode::Combat, false, DT).has(A_SLOT_UP));
        assert!(!m.update(Some(&st(RB)), Layout::CombatPro, Mode::Combat, false, DT).has(A_SLOT_UP), "one slot per press");
        assert!(m.update(Some(&st(LB)), Layout::CombatPro, Mode::Combat, false, DT).has(A_SLOT_DOWN));
        assert!(m.update(Some(&st(Y)), Layout::CombatPro, Mode::Combat, false, DT).toggle_harvest);
        let a = m.update(Some(&st(R3)), Layout::CombatPro, Mode::Combat, false, DT); assert!(a.crouch_press, "GamepadAbility1 = right stick click in Combat Pro");
        let a = m.update(Some(&st(BACK)), Layout::CombatPro, Mode::Combat, false, DT); assert!(a.map && !a.inventory);
        let a = m.update(Some(&st(DPAD_UP)), Layout::CombatPro, Mode::Combat, false, DT); assert!(a.inventory && !a.map);
    }
    #[test] fn build_mode_per_layout() {
        let mut m = PadMapper::new();
        let a = m.update(Some(&State { buttons: LB | RB | X, rt: 255, ..Default::default() }), Layout::Standard, Mode::Build, false, DT);
        for x in [A_FIRE, A_CHANGE_MATERIAL, A_ROTATE, A_USE] { assert!(a.has(x), "{x}"); }
        assert!(m.update(Some(&st(Y)), Layout::Standard, Mode::Build, false, DT).has(A_SLOT_UP), "GamepadNextBuilding");
        let mut m = PadMapper::new();
        let a = m.update(Some(&st(RB | R3 | L3)), Layout::QuickBuilder, Mode::Build, false, DT); for x in [A_SLOT_UP, A_ROTATE, A_CHANGE_MATERIAL] { assert!(a.has(x), "{x}"); }
        assert!(m.update(Some(&st(Y)), Layout::QuickBuilder, Mode::Build, false, DT).has(A_SECONDARY_QUICKBAR_SLOT_F1 + 4), "GamepadTrapPicker");
        let mut m = PadMapper::new();
        let a = m.update(Some(&State { buttons: LB | R3, lt: 255, ..Default::default() }), Layout::CombatPro, Mode::Build, false, DT); for x in [A_SLOT_DOWN, A_ROTATE, A_CHANGE_MATERIAL] { assert!(a.has(x), "{x}"); }
        assert!(!a.has(A_TARGET));
    }
    #[test] fn edit_mode_fire_selects_and_right_stick_click_resets() {
        let mut m = PadMapper::new();
        let a = m.update(Some(&State { buttons: R3, rt: 255, ..Default::default() }), Layout::Standard, Mode::Edit, false, DT);
        assert!(a.has(A_FIRE) && a.has(A_BUILDING_EDIT_RESET) && !a.has(A_PERFORM_BUILDING_IMPROVEMENT_INTERACTION));
    }
    #[test] fn unplug_mid_hold_clears_every_latch_without_firing() {
        let mut m = PadMapper::new();
        run(&mut m, &st(B | Y), Layout::Standard, Mode::Combat, 5);
        assert_eq!(m.update(None, Layout::Standard, Mode::Combat, false, DT), Actions::default());
        let a = m.update(Some(&st(0)), Layout::Standard, Mode::Combat, false, DT); assert!(a.held.is_empty(), "the reconnected idle pad releases nothing: {a:?}");
        let mut m = PadMapper::new();
        run(&mut m, &st(B), Layout::Standard, Mode::Combat, 30);
        m.update(None, Layout::Standard, Mode::Edit, false, DT);
        assert!(m.update(Some(&st(0)), Layout::Standard, Mode::Edit, false, DT).held.is_empty());
    }
    #[test] fn suppressed_buttons_stay_dead_until_released() {
        let mut m = PadMapper::new();
        m.suppress(Some(&st(B)));
        assert!(run(&mut m, &st(B), Layout::Standard, Mode::Combat, 30).iter().all(|a| a.held.is_empty()), "B held over the menu closing does nothing");
        m.update(Some(&st(0)), Layout::Standard, Mode::Combat, false, DT);
        m.update(Some(&st(B)), Layout::Standard, Mode::Combat, false, DT);
        assert_eq!(m.update(Some(&st(0)), Layout::Standard, Mode::Combat, false, DT).held, vec![A_SWITCH_QUICKBAR]);
    }
    #[test] fn jump_uncrouches_the_pad_crouch() {
        let mut m = PadMapper::new();
        assert!(m.update(Some(&st(LB)), Layout::Standard, Mode::Combat, false, DT).crouch);
        assert!(!m.update(Some(&st(A)), Layout::Standard, Mode::Combat, false, DT).crouch);
    }
    #[test] fn default_deadzones_leave_the_stick_untouched_and_custom_ones_rescale() {
        let s = State { lx: 9000, ly: -4000, rx: 20000, ry: 100, ..Default::default() };
        assert_eq!(apply_deadzones(s, pad::LEFT_DEADZONE, pad::RIGHT_DEADZONE), s);
        let t = apply_deadzones(s, 0.1, 0.65);
        let (a, _) = pad::radial_deadzone(t.lx, t.ly, pad::LEFT_DEADZONE);
        let m = ((9000f32 / 32767.).powi(2) + (4000f32 / 32767.).powi(2)).sqrt();
        let want = (m - 0.1) / (1. - 0.1 - pad::OUTER_DEADZONE) * (9000. / 32767.) / m;
        assert!((a - want).abs() < 2e-3, "{a} vs {want}");
        assert_eq!(pad::radial_deadzone(t.rx, t.ry, pad::RIGHT_DEADZONE), (0., 0.), "a 0.61 deflection is inside a 0.65 dead zone");
    }
}
