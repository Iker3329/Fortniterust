use crate::cmc::{Cmc, Mode};
use glam::Vec3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Aerial { #[default] None, Skydive, Parachute }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirControlParams { pub max_acceleration: f32, pub lateral_friction: f32, pub max_lateral_speed: f32, pub terminal_velocity: f32, pub gravity_scalar: f32 }

pub const SKYDIVING_PASSIVE: AirControlParams = AirControlParams { max_acceleration: 4096., lateral_friction: 0.8, max_lateral_speed: 1200., terminal_velocity: 1200., gravity_scalar: 1. };
pub const SKYDIVING_ACTIVE: AirControlParams = AirControlParams { max_acceleration: 4096., lateral_friction: 1.5, max_lateral_speed: 2400., terminal_velocity: 6000., gravity_scalar: 1. };
pub const PARACHUTE_SKYDIVE: AirControlParams = AirControlParams { max_acceleration: 2048., lateral_friction: 1.2, max_lateral_speed: 2000., terminal_velocity: 500., gravity_scalar: 1. };
pub const PARACHUTE_JUMP: AirControlParams = AirControlParams { max_acceleration: 2048., lateral_friction: 1.3, max_lateral_speed: 800., terminal_velocity: 500., gravity_scalar: 1. };

pub const SKYDIVING_DIRECTIONAL_CURVE: [(f32, f32); 3] = [(0., 1.), (70., 0.8), (110., 0.65)];
pub const PARACHUTE_DIRECTIONAL_CURVE: [(f32, f32); 3] = [(0., 1.), (70., 0.8), (110., 0.65)];

pub const ACTIVE_INPUT_EPSILON: f32 = 1e-4;
pub const ACTIVE_DIVE_Z: f32 = -0.1;
pub const ACTIVE_DIVE_DOT: f32 = 0.4;
pub const STRAFING_DOT: f32 = 0.8;
pub const FALL_SUBSTEP: f32 = 1. / 33.;
pub const FALL_MIN_STEP: f32 = 1e-6;

pub const REQUEST_NONE: u8 = 0;
pub const REQUEST_OPEN: u8 = 1;
pub const REQUEST_CLOSE: u8 = 2;

pub fn curve_constant(keys: &[(f32, f32)], t: f32) -> f32 {
    let mut v = keys.first().map_or(1., |k| k.1);
    for k in keys { if t >= k.0 { v = k.1; } }
    v
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AerialState {
    pub mode: Aerial,
    pub is_skydiving: bool,
    pub forced_open: bool,
    pub from_launch_pad: bool,
    pub request: u8,
    pub control_forward: Vec3,
    pub input: Vec3,
    pub accel_snapshot: Vec3,
    pub skydives_ended: u32,
}

impl Cmc {
    pub fn aerial_mode(&self) -> Aerial { self.air.mode }
    pub fn is_skydiving(&self) -> bool { self.air.is_skydiving }
    pub fn is_parachute_open(&self) -> bool { self.air.mode == Aerial::Parachute }
    pub fn is_parachute_forced_open(&self) -> bool { self.air.forced_open }
    pub fn in_aerial_mode(&self) -> bool { self.air.mode != Aerial::None }

    pub fn start_skydiving(&mut self) {
        self.air.is_skydiving = true;
        self.air.forced_open = false;
        self.air.request = REQUEST_NONE;
        self.pressed_jump = false;
        self.set_aerial_mode(Aerial::Skydive);
    }
    pub fn end_skydiving(&mut self) {
        if self.air.is_skydiving { self.air.skydives_ended += 1; }
        self.air.is_skydiving = false;
        self.air.forced_open = false;
        self.air.from_launch_pad = true;
    }
    pub fn open_parachute(&mut self) { self.set_aerial_mode(Aerial::Parachute); }
    pub fn close_parachute(&mut self) {
        if self.air.is_skydiving { self.set_aerial_mode(Aerial::Skydive); } else { self.set_aerial_mode(Aerial::None); }
    }
    pub fn force_parachute_open(&mut self) {
        if self.air.forced_open { return; }
        self.air.forced_open = true;
        self.air.request = REQUEST_OPEN;
        if self.air.mode != Aerial::Parachute { self.open_parachute(); }
    }
    fn set_aerial_mode(&mut self, m: Aerial) {
        self.air.mode = m;
        if m != Aerial::None && self.mode != Mode::Falling { self.start_falling_externally(); }
    }
    pub fn on_jump_pressed_aerial(&mut self) -> bool {
        if !(self.air.is_skydiving || self.air.mode == Aerial::Parachute) || self.air.forced_open { return false; }
        self.air.request = if self.air.mode == Aerial::Parachute { REQUEST_CLOSE } else { REQUEST_OPEN };
        true
    }
    pub(crate) fn aerial_check_jump_input(&mut self) -> bool {
        if !(self.air.is_skydiving || self.air.mode == Aerial::Parachute) { return false; }
        match self.air.request {
            REQUEST_OPEN if self.air.mode != Aerial::Parachute => self.open_parachute(),
            REQUEST_CLOSE if self.air.mode == Aerial::Parachute && !self.air.forced_open => self.close_parachute(),
            _ => {}
        }
        self.air.request = REQUEST_NONE;
        self.pressed_jump = false;
        true
    }
    pub fn set_aerial_input(&mut self, input: Vec3, control_forward: Vec3) { self.air.input = input; self.air.control_forward = control_forward; }
    pub(crate) fn aerial_constrained_input(&self) -> Vec3 {
        let i = self.air.input;
        match self.air.mode {
            Aerial::Skydive => Vec3::new(i.x, i.y, i.z.min(0.)),
            _ => Vec3::new(i.x, i.y, 0.),
        }
    }
    pub fn is_actively_skydiving(&self) -> bool {
        if self.air.mode != Aerial::Skydive { return false; }
        let a = self.air.accel_snapshot;
        if !(a.x.abs() > ACTIVE_INPUT_EPSILON || a.y.abs() > ACTIVE_INPUT_EPSILON || a.z.abs() > ACTIVE_INPUT_EPSILON) { return false; }
        let d = a.normalize_or_zero();
        d.z < ACTIVE_DIVE_Z && self.air.control_forward.dot(d) > ACTIVE_DIVE_DOT
    }
    pub fn is_actively_strafing_in_air(&self) -> bool {
        if self.air.mode != Aerial::Skydive { return false; }
        let d = self.air.accel_snapshot.normalize_or_zero();
        if d == Vec3::ZERO { return false; }
        self.air.control_forward.dot(d).abs() <= STRAFING_DOT
    }
    pub fn air_control_params(&self) -> Option<AirControlParams> {
        match self.air.mode {
            Aerial::None => None,
            Aerial::Skydive => Some(if self.is_actively_skydiving() { SKYDIVING_ACTIVE } else { SKYDIVING_PASSIVE }),
            Aerial::Parachute => Some(if self.air.is_skydiving { PARACHUTE_SKYDIVE } else { PARACHUTE_JUMP }),
        }
    }
    pub(crate) fn aerial_max_speed(&self, p: &AirControlParams) -> f32 {
        let v2 = Vec3::new(self.velocity.x, self.velocity.y, 0.);
        let n = if v2.length_squared() <= 1e-8 { Vec3::ZERO } else { v2.normalize() };
        let angle = self.facing.dot(n).clamp(-1., 1.).acos().to_degrees();
        let curve = if self.air.mode == Aerial::Parachute { &PARACHUTE_DIRECTIONAL_CURVE } else { &SKYDIVING_DIRECTIONAL_CURVE };
        let mut s = p.max_lateral_speed * curve_constant(curve, angle);
        if self.is_actively_skydiving() {
            let d = self.air.accel_snapshot.normalize_or_zero();
            s *= (d.x * d.x + d.y * d.y).sqrt();
        }
        s
    }
    pub(crate) fn aerial_terminal_velocity(&self, p: &AirControlParams) -> f32 {
        if self.is_actively_skydiving() {
            let d = self.air.accel_snapshot.normalize_or_zero();
            SKYDIVING_PASSIVE.terminal_velocity + d.z.min(0.).abs() * (SKYDIVING_ACTIVE.terminal_velocity - SKYDIVING_PASSIVE.terminal_velocity)
        } else { p.terminal_velocity }
    }
    pub(crate) fn aerial_new_fall_velocity(&self, p: &AirControlParams, initial: Vec3, gravity: Vec3, dt: f32) -> Vec3 {
        let t = self.aerial_terminal_velocity(p);
        let g = gravity.z * p.gravity_scalar;
        let mut r = initial;
        if t > 0. && initial.z < -t {
            let mut vz = initial.z;
            let mut remaining = dt;
            while remaining >= FALL_MIN_STEP {
                let step = if remaining <= FALL_SUBSTEP { remaining } else { (remaining * 0.5).min(FALL_SUBSTEP) };
                vz += (g / t) * vz * step;
                remaining -= step;
            }
            r.z = vz.min(-t);
        } else {
            r.z = (initial.z + g * dt).max(-t);
        }
        r
    }
    pub(crate) fn aerial_landed(&mut self) {
        if self.air.mode == Aerial::None && !self.air.is_skydiving { return; }
        self.pressed_jump = false;
        self.air.request = REQUEST_NONE;
        if self.air.is_skydiving { self.end_skydiving(); }
        self.air.mode = Aerial::None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmc::Params;
    use crate::scene::Scene;
    use glam::{Vec2, Vec3};
    fn air(z: f32) -> Cmc { let mut c = Cmc::new(Params::default(), Scene::default(), Vec3::new(0., 0., z)); c.velocity = Vec3::new(6000., 0., 0.); c.facing = Vec3::X; c.start_skydiving(); c }
    fn run(c: &mut Cmc, input: Vec3, fwd: Vec3, secs: f32) { let dt = 1. / 30.; for _ in 0..(secs / dt) as usize { c.set_aerial_input(input, fwd); c.tick(Vec2::ZERO, dt); } }
    #[test] fn passive_skydive_settles_at_its_terminal_velocity_and_lateral_cap() {
        let mut c = air(80000.);
        assert_eq!(c.aerial_mode(), Aerial::Skydive);
        run(&mut c, Vec3::ZERO, Vec3::X, 8.);
        assert!((c.velocity.z + 1200.).abs() < 2., "vz {}", c.velocity.z);
        assert!(Vec3::new(c.velocity.x, c.velocity.y, 0.).length() < 1300., "bus speed bleeds off: {:?}", c.velocity);
    }
    #[test] fn diving_raises_the_terminal_velocity() {
        let mut c = air(80000.);
        let p = (-70f32).to_radians();
        let fwd = Vec3::new(p.cos(), 0., p.sin());
        run(&mut c, fwd, fwd, 10.);
        assert!(c.is_actively_skydiving());
        let want = 1200. + 4800. * p.sin().abs();
        assert!((c.velocity.z + want).abs() < 30., "vz {} want {}", c.velocity.z, -want);
    }
    #[test] fn the_glider_falls_at_500_and_lands_into_walking() {
        let mut c = air(3000.);
        c.on_jump_pressed_aerial(); c.check_jump_input(1. / 30.);
        assert!(c.is_parachute_open());
        run(&mut c, Vec3::ZERO, Vec3::X, 3.);
        assert!((c.velocity.z + 500.).abs() < 2., "vz {}", c.velocity.z);
        c.on_jump_pressed_aerial(); c.check_jump_input(1. / 30.);
        assert_eq!(c.aerial_mode(), Aerial::Skydive, "the glider closes back to skydive");
        c.force_parachute_open();
        assert!(!c.on_jump_pressed_aerial(), "a forced glider cannot be closed");
        run(&mut c, Vec3::ZERO, Vec3::X, 12.);
        assert!(c.is_moving_on_ground() && c.aerial_mode() == Aerial::None && !c.is_skydiving(), "{:?} {:?}", c.mode, c.air);
        assert_eq!(c.air.skydives_ended, 1);
    }
}
