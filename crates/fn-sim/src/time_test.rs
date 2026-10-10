//! Deterministic twin of `--time-test` (old time_audit.rs): the real windowed loop's wall-clock vs simulation-clock audit, but with a VIRTUAL
//! clock. The old run measured real frame times of the OpenGL loop (non-reproducible, so no golden log exists for it); this one feeds a
//! synthetic list of raw frame times through `GameSim::step_real` (the only clamp is `fn_game::time_audit::MAX_DELTA_TIME`, 0.1 s) with the
//! same self-pressed jump (down for one frame out of every 3 s of simulated time), and reports in the old file's layout (`time-audit-<tag>.txt`).
//! "wall" here is the sum of the raw frame times, so `sim/wall` = 1 means the game ran in real time, < 1 means hidden slow motion.
use crate::scenarios::Outputs;
use fn_game::time_audit::{clamp_frame_dt, MAX_DELTA_TIME};
use fn_game::{FrameInput, GameSim};
use std::fmt::Write as _;

/// Raw frame times of `seconds` of a steady `hz` loop, with extra `(at second, length s)` hitches inserted as one long frame each.
pub fn frame_times(hz: f32, seconds: f32, hitches: &[(f32, f32)]) -> Vec<f32> {
    let dt = 1. / hz; let n = (seconds * hz).round() as usize; let mut out = Vec::with_capacity(n + hitches.len()); let mut t = 0f32; let mut pending: Vec<(f32, f32)> = hitches.to_vec();
    for _ in 0..n {
        if let Some(i) = pending.iter().position(|h| t >= h.0) { out.push(pending.remove(i).1); } else { out.push(dt); }
        t += *out.last().unwrap();
    }
    out
}

/// Result of one audit.
#[derive(Clone, Debug, Default)]
pub struct Audit { pub frames: u32, pub raw: f64, pub fed: f64, pub ctrl: f64, pub cam: f64, pub clamped: u32, pub lost: f64, pub flights: Vec<f64> }
impl Audit { pub fn ratio(&self) -> f64 { self.fed / self.raw.max(1e-9) } }

/// Runs the loop over `raw_frames` (the first frame's dt spans loading in the old run and is skipped, as `time_audit::record` did).
pub fn run(raw_frames: &[f32]) -> Audit {
    let mut g = GameSim::new(); g.controller.component_mut().p.jump_release_ignored = false; let mut a = Audit::default();
    let (mut air_prev, mut air_active, mut air_sim, mut sim_time) = (false, false, 0f64, 0f64);
    for (i, &raw) in raw_frames.iter().enumerate() {
        let fed = clamp_frame_dt(raw);
        if i > 0 {
            a.frames += 1; a.raw += raw as f64; a.fed += fed as f64; a.ctrl += fed.min(fn_movement::MAX_FRAME_DT) as f64; a.cam += fed.min(0.1) as f64;
            if raw > fed + 1e-6 { a.clamped += 1; a.lost += (raw - fed) as f64; }
            sim_time += fed as f64; if air_active { air_sim += fed as f64; }
        }
        let t = sim_time % 3.; let jump = i > 0 && t >= 1. && t < 1. + fed as f64;
        g.step_real(&FrameInput { script_jump: jump, ..Default::default() }, raw);
        let air = g.controller.airborne();
        if air && !air_prev { air_active = true; air_sim = 0.; }
        if !air && air_prev && air_active { a.flights.push(air_sim); air_active = false; }
        air_prev = air;
    }
    a
}

/// `time-audit-<tag>.txt` in the old layout (virtual wall clock).
pub fn report(mode: &str, a: &Audit) -> String {
    let wall = a.raw;
    let mut t = format!("time audit (virtual clock), mode: {mode}\nwall {wall:.3} s over {} frames ({:.1} fps)\nsum of raw frame times      {:.3} s\nsim dt fed to App/anim/combat {:.3} s   ratio sim/wall {:.5}\nsim dt fed to Controller::step {:.3} s (its own clamp {} s)   ratio {:.5}\nsim dt fed to camera           {:.3} s (its own clamp 0.1 s)   ratio {:.5}\nframes longer than the {} s clamp: {} (lost {:.3} s of game time)\n",
        a.frames, a.frames as f64 / wall.max(1e-9), a.raw, a.fed, a.fed / wall.max(1e-9), a.ctrl, fn_movement::MAX_FRAME_DT, a.ctrl / wall.max(1e-9), a.cam, a.cam / wall.max(1e-9), MAX_DELTA_TIME, a.clamped, a.lost);
    for s in &a.flights { let _ = writeln!(t, "flight: simulated {s:.3} s (ideal 0.429 s)"); }
    t
}

/// The scenario's output: the audit of several frame rates, 20 s each.
pub fn time_test() -> Outputs {
    let mut out = Outputs::default(); let mut all = String::new();
    for (tag, hz, hitches) in [("240fps", 240., vec![]), ("144fps", 144., vec![]), ("60fps", 60., vec![]), ("30fps", 30., vec![]), ("15fps", 15., vec![]), ("60fps-hitch", 60., vec![(5., 0.25), (12., 0.5)])] {
        let a = run(&frame_times(hz, 20., &hitches)); let r = report(&format!("{tag} (fixed {hz} Hz{})", if hitches.is_empty() { "" } else { ", with 0.25 s and 0.5 s hitches" }), &a);
        out.add(&format!("time-audit-{tag}.txt"), r.clone()); all += &r; all += "\n";
    }
    out.add("time-audit-all.txt", all); out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn game_time_is_real_time_at_every_frame_rate_up_to_the_clamp() {
        let _g = crate::scenarios::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for hz in [240., 144., 60., 30., 15., 10.] {
            let a = run(&frame_times(hz, 20., &[]));
            assert!((a.ratio() - 1.).abs() < 1e-6, "{hz} Hz: sim/wall {}", a.ratio()); assert_eq!(a.clamped, 0, "{hz} Hz"); assert!((a.ctrl - a.fed).abs() < 1e-9 && (a.cam - a.fed).abs() < 1e-9, "no system clamps tighter than MAX_DELTA_TIME");
            assert!(a.flights.len() >= 5, "{hz} Hz: {} jumps", a.flights.len());
            // airtime of the decoded jump: JumpZ x DoJump frame-delta scale (1.0 at 60 Hz and faster, 1.0589 at 30 fps, larger below; exe 0x6d4f20) over g\r\n            let want = fn_movement::params::jump_airtime_at(1. / hz) as f64;\r\n            for f in &a.flights { assert!((f - want).abs() < 1.5 / hz as f64 + 0.01, "{hz} Hz: simulated airtime {f} vs decoded {want}"); }
        }
    }
    #[test] fn only_frames_longer_than_max_delta_time_lose_time() {
        let _g = crate::scenarios::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let a = run(&frame_times(60., 20., &[(5., 0.25), (12., 0.5)]));
        assert_eq!(a.clamped, 2); assert!((a.lost - ((0.25 - MAX_DELTA_TIME as f64) + (0.5 - MAX_DELTA_TIME as f64))).abs() < 1e-4, "{}", a.lost);
        assert!(a.ratio() < 1. && a.ratio() > 0.95, "{}", a.ratio());
    }
    #[test] fn the_audit_is_deterministic_and_the_report_lists_every_flight() {
        let _g = crate::scenarios::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (a, b) = (time_test(), time_test()); assert_eq!(a.files, b.files); assert!(a.files.iter().any(|f| f.0 == "time-audit-all.txt"));
        assert!(a.files[0].1.contains("ratio sim/wall 1.00000") && a.files[0].1.contains("flight: simulated 0.4"), "{}", a.files[0].1);
    }
}
