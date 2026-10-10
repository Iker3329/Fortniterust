use fn_game::{FrameInput, GameSim, TickDriver, EYE, Z_OFFSET};
use glam::{Vec2, Vec3};
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Step { Hold(f32, f32, u32), Jump, StopJump, Crouch, UnCrouch, Sprint(bool), Yaw(f32), Pitch(f32), Lift(f32), Tp }

#[derive(Clone, Debug, Default)]
pub struct Plan { pub frames: u32, pub steps: Vec<(u32, Step)> }

impl Plan {
    pub fn parse(text: &str) -> Result<Plan, String> {
        let mut p = Plan::default();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() { continue; }
            let t: Vec<&str> = line.split_whitespace().collect();
            let bad = || format!("line {}: {raw:?}", n + 1);
            if t[0] == "frames" { p.frames = t.get(1).and_then(|v| v.parse().ok()).ok_or_else(bad)?; continue; }
            let f: u32 = t[0].parse().map_err(|_| bad())?;
            let num = |i: usize| -> Result<f32, String> { t.get(i).and_then(|v| v.parse().ok()).ok_or_else(bad) };
            let s = match *t.get(1).ok_or_else(bad)? {
                "hold" => { let (a, b) = t.get(2).and_then(|v| v.split_once(',')).ok_or_else(bad)?; Step::Hold(a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?, num(3)? as u32) }
                "jump" => Step::Jump, "stopjump" => Step::StopJump, "crouch" => Step::Crouch, "uncrouch" => Step::UnCrouch,
                "sprint" => Step::Sprint(num(2)? != 0.), "yaw" => Step::Yaw(num(2)?), "pitch" => Step::Pitch(num(2)?), "lift" => Step::Lift(num(2)?), "tp" => Step::Tp,
                _ => return Err(bad()),
            };
            p.steps.push((f, s));
        }
        if p.frames == 0 { p.frames = p.steps.iter().map(|s| s.0).max().unwrap_or(0) + 120; }
        Ok(p)
    }
}

pub const HEADER: &str = "frame,ms,dt,fwd,right,up,v_fwd,v_right,v_up,mode,crouched,halfh,ctrl_pitch,ctrl_yaw,cam_fwd,cam_right,cam_up,cam_pitch,cam_yaw,fov,held";

pub fn run(plan: &Plan, fps: f32, hz: u32) -> String {
    let mut sim = GameSim::new();
    sim.world.props.clear();
    let mut d = TickDriver::new(sim, hz);
    let dt = 1. / fps;
    let settle = FrameInput { set_yaw: Some(0.), set_pitch: Some(0.), ..Default::default() };
    for _ in 0..30 { d.frame(&settle, dt, &mut |_| Vec::new()); }
    let start = d.sim.controller.render_position();
    let start_h = d.sim.controller.render_height();
    let feet0 = d.sim.controller.component().loc.z - d.sim.controller.capsule_half_height_cm();
    let mut out = String::from(HEADER);
    out.push('\n');
    let (mut hold, mut jump, mut crouch, mut sprint, mut yaw, mut pitch) = ((0f32, 0f32, 0u32), false, false, false, 0f32, 0f32);
    for fr in 1..=plan.frames {
        let (mut lift, mut tp) = (None, false);
        for (_, s) in plan.steps.iter().filter(|s| s.0 == fr) {
            match *s {
                Step::Hold(a, b, n) => hold = (a, b, n), Step::Jump => jump = true, Step::StopJump => jump = false,
                Step::Crouch => crouch = true, Step::UnCrouch => crouch = false, Step::Sprint(on) => sprint = on,
                Step::Yaw(v) => yaw = v, Step::Pitch(v) => pitch = v, Step::Lift(m) => lift = Some(m), Step::Tp => tp = true,
            }
        }
        if tp { d.sim.controller.position = start; d.sim.controller.height = start_h; d.sim.controller.velocity = Vec2::ZERO; d.sim.controller.vertical_speed = 0.; d.snap(); }
        if let Some(m) = lift { d.sim.controller.height += m * 0.01; d.sim.controller.vertical_speed = 0.; }
        let held = hold.2 > 0;
        let inp = FrameInput { forward: if held { hold.0 } else { 0. }, side: if held { hold.1 } else { 0. }, script_jump: jump, force_crouch: crouch, force_run: sprint, set_yaw: Some(yaw), set_pitch: Some(pitch), ..Default::default() };
        if held { hold.2 -= 1; }
        d.sim.controller.component_mut().frame_dt_override = Some(dt);
        d.frame(&inp, dt, &mut |_| Vec::new());
        let c = &d.sim.controller;
        let p = c.render_position() - start;
        let hh = c.capsule_half_height_cm();
        let up = c.component().loc.z - feet0;
        let fwd = -p.y * 100.; let right = p.x * 100.;
        let v = c.velocity * 100.;
        let v = Vec2::new(-v.y, v.x);
        let feet = Vec3::new(d.sim.x, d.sim.y - EYE, d.sim.z);
        let root = feet + Vec3::Y * hh * 0.01;
        let e = d.sim.cam_view.eye - root;
        let (cf, cr) = (-e.z * 100., e.x * 100.);
        let _ = Z_OFFSET;
        let mode = if c.airborne() { 3 } else { 1 };
        let _ = writeln!(out, "{fr},{:.3},{dt:.6},{fwd:.3},{right:.3},{up:.3},{:.3},{:.3},{:.3},{mode},{},{hh:.3},{:.3},{:.3},{cf:.3},{cr:.3},{:.3},{:.3},{:.3},80,{}",
            fr as f32 * dt * 1000., v.x, v.y, c.vertical_speed * 100., c.crouched as u8, d.sim.pitch, d.sim.yaw, e.y * 100., d.sim.cam_view.pitch_deg, d.sim.cam_view.yaw_deg, held as u8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn plan_parses_every_step() {
        let p = Plan::parse("frames 50\n1 hold 1,0 10\n5 jump\n6 stopjump\n7 crouch\n8 uncrouch\n9 sprint 1\n10 yaw 90\n11 pitch -10\n12 lift 1500").unwrap();
        assert_eq!(p.frames, 50); assert_eq!(p.steps.len(), 9); assert_eq!(p.steps[0], (1, Step::Hold(1., 0., 10)));
    }
    #[test] fn standing_still_stays_put_and_forward_moves_forward() {
        let s = run(&Plan::parse("frames 60\n1 hold 1,0 60").unwrap(), 60., 0);
        let last: Vec<f32> = s.lines().last().unwrap().split(',').map(|v| v.parse().unwrap()).collect();
        assert!(last[3] > 100. && last[4].abs() < 1., "{last:?}");
    }
}
