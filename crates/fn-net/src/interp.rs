use crate::proto::State;
use glam::Vec3;
use std::collections::VecDeque;

pub const DELAY_MS: f64 = 110.;
pub const MAX_EXTRAPOLATE_MS: f64 = 150.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample { pub pos: Vec3, pub vel: Vec3, pub yaw: f32, pub pitch: f32, pub flags: u16, pub held: u8, pub air_m: f32 }

#[derive(Default)]
pub struct Interp { snaps: VecDeque<State>, offset_ms: Option<f64>, last_seq: Option<u32> }

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + 540.).rem_euclid(360.) - 180.;
    (a + d * t).rem_euclid(360.)
}

impl Interp {
    pub fn push(&mut self, s: State, recv_ms: f64) {
        if let Some(l) = self.last_seq { if s.seq <= l && l - s.seq < 1 << 30 { return; } }
        self.last_seq = Some(s.seq);
        let off = recv_ms - s.t_ms as f64;
        self.offset_ms = Some(match self.offset_ms { Some(o) if off >= o => o + (off - o).min(2.) * 0.02, _ => off });
        if self.snaps.back().map_or(false, |b| s.t_ms < b.t_ms) { self.snaps.clear(); self.offset_ms = Some(off); }
        self.snaps.push_back(s);
        while self.snaps.len() > 40 { self.snaps.pop_front(); }
    }
    pub fn snap_to(&mut self, pos: Vec3) { for s in self.snaps.iter_mut() { s.pos = pos; s.vel = Vec3::ZERO; } }
    pub fn reset(&mut self) { self.snaps.clear(); self.offset_ms = None; }
    pub fn latest(&self) -> Option<&State> { self.snaps.back() }
    pub fn sample(&self, now_ms: f64) -> Option<Sample> {
        let off = self.offset_ms?;
        let t = now_ms - off - DELAY_MS;
        let last = self.snaps.back()?;
        let mk = |s: &State, pos: Vec3, yaw: f32, pitch: f32| Sample { pos, vel: s.vel, yaw, pitch, flags: s.flags, held: s.held, air_m: s.air_dm as f32 * 0.1 };
        if t >= last.t_ms as f64 {
            let dt = ((t - last.t_ms as f64).min(MAX_EXTRAPOLATE_MS) / 1000.) as f32;
            return Some(mk(last, last.pos + last.vel * dt, last.yaw, last.pitch));
        }
        let first = self.snaps.front()?;
        if t <= first.t_ms as f64 { return Some(mk(first, first.pos, first.yaw, first.pitch)); }
        for w in self.snaps.iter().zip(self.snaps.iter().skip(1)) {
            let (a, b) = w;
            if t >= a.t_ms as f64 && t <= b.t_ms as f64 {
                let span = (b.t_ms - a.t_ms).max(1) as f64;
                let k = ((t - a.t_ms as f64) / span) as f32;
                return Some(mk(b, a.pos.lerp(b.pos, k), lerp_angle(a.yaw, b.yaw, k), a.pitch + (b.pitch - a.pitch) * k));
            }
        }
        Some(mk(last, last.pos, last.yaw, last.pitch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn st(seq: u32, t: u32, x: f32) -> State { State { seq, t_ms: t, pos: Vec3::new(x, 2., 0.), vel: Vec3::new(5., 0., 0.), yaw: 350., ..Default::default() } }
    #[test] fn interpolates_between_snapshots_behind_the_delay() {
        let mut i = Interp::default();
        for k in 0..5 { i.push(st(k, k * 50, k as f32 * 0.25), 1000. + k as f64 * 50.); }
        let s = i.sample(1000. + 125. + DELAY_MS).unwrap();
        assert!((s.pos.x - 0.625).abs() < 1e-3, "{}", s.pos.x);
    }
    #[test] fn extrapolates_a_little_then_holds() {
        let mut i = Interp::default();
        i.push(st(0, 0, 0.), 0.); i.push(st(1, 50, 0.25), 50.);
        let s = i.sample(50. + DELAY_MS + 1000.).unwrap();
        assert!((s.pos.x - (0.25 + 5. * 0.15)).abs() < 1e-3);
    }
    #[test] fn old_and_duplicate_packets_are_ignored() {
        let mut i = Interp::default();
        i.push(st(5, 250, 1.), 0.); i.push(st(4, 200, 9.), 1.); i.push(st(5, 250, 9.), 2.);
        assert_eq!(i.latest().unwrap().pos.x, 1.);
    }
    #[test] fn angles_wrap() { assert!((lerp_angle(350., 10., 0.5) - 0.).abs() < 1e-3 || (lerp_angle(350., 10., 0.5) - 360.).abs() < 1e-3); }
}
