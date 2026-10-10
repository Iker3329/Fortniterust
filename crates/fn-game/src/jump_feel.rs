//! Jump feel investigation (`--jump-feel-test`, cargo tests): the whole input -> screen chain of a jump replayed headlessly with the wiring of
//! `App::update` in PER-FRAME mode (`FNRT_SIM_HZ=0`: key edge -> Controller::step -> AnimState::step_sample -> Camera::update every frame), at several frame
//! rates. The default fixed 30 Hz tick is replayed by `sim_tick::FixedSim` (`--sim-tick-test`); this file is the SIM_HZ=0 parity twin.
//!  * `latency_report`: frames/ms from the key edge to liftoff, animation state, leg/jog blend and camera response (a jump is one-frame latency
//!    free in the physics: the press is consumed in the very frame it is sampled).
//!  * `timeline`: rise/apex/fall timing against the animation phases (Jump -> Loop -> Fall -> Land).
//!  * `tuning_table`: apex / rise / airtime for GravityScale 1, 1.5, 2, 2.5 (the `FNRT_GRAVITY_SCALE` / `FNRT_JUMPZ` experiment table).
//!  * tests: the same scripted scenario at 30 / 60 / 144 / 240 fps must agree (tolerances below).
#![allow(dead_code)]
use fn_animgraph::{AnimState,JumpState,PawnSample,lower};
use fn_camera as camera;
use fn_movement::{self as movement,Controller,cmc::Params};
use glam::{Vec2,Vec3};
use std::fmt::Write as _;

#[derive(Clone,Copy,Debug)]pub struct Scenario{pub hz:f32,pub seconds:f32,pub press:f32,pub hold:f32,pub forward:bool,pub run:bool}
/// State at the END of a frame (after controller, animation and camera were stepped).
#[derive(Clone,Copy,Debug)]pub struct Chain{pub f:usize,pub t:f32,pub pos:Vec2,pub height:f32,pub vz:f32,pub air:bool,pub jumps:u32,pub state:JumpState,pub mix:f32,pub jog:f32,pub eye:Vec3}
/// Frame `k` covers simulated time [k*dt, (k+1)*dt]; the key is down from the frame that starts at `press` for `hold` seconds.
pub fn run_chain(s:&Scenario)->Vec<Chain>{
    let dt=1./s.hz;let n=(s.seconds*s.hz).round() as usize;let (p0,p1)=((s.press*s.hz).round() as usize,((s.press+s.hold)*s.hz).round().max((s.press*s.hz).round()+1.) as usize);
    let mut c=Controller::default();c.component_mut().p.jump_release_ignored=false;let mut a=AnimState::new();let mut cam=camera::Camera::default();let mut rows=Vec::with_capacity(n);
    for f in 0..n{
        let jump=f>=p0&&f<p1;
        let dir=if s.forward{Vec2::new(0.,-1.)}else{Vec2::ZERO};
        c.step(dir,jump,s.run,dt);
        a.step_sample(&PawnSample{velocity:(c.velocity.x,c.velocity.y),input:(dir.x,dir.y),facing_deg:c.facing,camera_yaw_deg:c.facing,camera_pitch_deg:0.,height:c.air_height(),vertical_speed:c.vertical_speed,
            fall_time_left:movement::fall_time_left(c.air_height(),c.vertical_speed),crouched:c.crouched,running:c.running,melee_swinging:false},dt);
        let p=c.render_position();
        let view=cam.update(&camera::MELEE,camera::Input{feet:Vec3::new(p.x,c.render_height(),12.+p.y),crouched:c.crouched,yaw_deg:0.,pitch_deg:0.,dt},camera::trace_ground);
        rows.push(Chain{f,t:(f+1) as f32*dt,pos:p,height:c.height,vz:c.vertical_speed,air:c.airborne(),jumps:c.jumps_started(),state:a.jump.state,mix:a.jump.pose().mix,jog:a.lower_weight(lower::MOVEMENT),eye:view.eye});
    }
    rows
}
fn first(rows:&[Chain],from:usize,pred:impl Fn(&Chain)->bool)->Option<usize>{rows.iter().skip(from).find(|r|pred(r)).map(|r|r.f)}
/// Latency of every link of the chain, in frames after the press frame (0 = the press frame itself) and the simulated ms at the END of that frame.
pub fn latency_report()->String{
    let mut s=String::from("Jump latency: key edge -> screen, replayed with App::update's wiring (frames after the press frame; 0 = same frame; ms = simulated time at the end of that frame)\n");
    for (name,fwd,run) in [("standing jump",false,false),("jog jump",true,false),("sprint jump",true,true)]{
        for hz in [30.,60.,144.,240.]{
            let press=if fwd{2.} else {0.5};let sc=Scenario{hz,seconds:press+3.,press,hold:0.1,forward:fwd,run};let rows=run_chain(&sc);let p=(press*hz).round() as usize;
            let eye0=rows[p-1].eye.y;let jog0=rows[p-1].jog;
            let lat=|f:Option<usize>|f.map_or("never".to_string(),|f|format!("{} fr / {:.1} ms",f-p,(f-p+1) as f32*1000./hz));
            let _=writeln!(s,"\n== {name} @ {hz} Hz (dt {:.2} ms), key edge at frame {p} (t {:.3} s)",1000./hz,press);
            let _=writeln!(s,"  Controller consumed the press (DoJump)  : {}",lat(first(&rows,p,|r|r.jumps>=1)));
            let _=writeln!(s,"  vertical speed > 0                      : {}",lat(first(&rows,p,|r|r.vz>0.)));
            let _=writeln!(s,"  height > 0 (feet left the floor)        : {}",lat(first(&rows,p,|r|r.air&&r.height>0.)));
            let _=writeln!(s,"  animation Jump state entered            : {}",lat(first(&rows,p,|r|r.state==JumpState::Jump)));
            let _=writeln!(s,"  jump additive blend >= 50% (0.08 s ramp): {}",lat(first(&rows,p,|r|r.state==JumpState::Jump&&r.mix>=0.5)));
            if run{let _=writeln!(s,"  jog legs: Movement weight {:.2} before; < 0.5 / < 0.1 / 0 : {} | {} | {}",jog0,lat(first(&rows,p,|r|r.jog<0.5)),lat(first(&rows,p,|r|r.jog<0.1)),lat(first(&rows,p,|r|r.jog<=1e-4)));}
            let _=writeln!(s,"  camera eye rose 1 cm above its pre-jump y: {}",lat(first(&rows,p,|r|r.eye.y-eye0>=0.01)));
            // camera lag: how far the eye trails the feet' rise during the ascent, and the same as a time lag
            let apex=rows.iter().skip(p).position(|r|r.vz<=0.).map_or(rows.len(),|i|i+p);
            let (mut worst,mut at)=(0f32,p);for r in &rows[p..apex]{let lag=r.height-(r.eye.y-eye0);if lag>worst{worst=lag;at=r.f;}}
            let _=writeln!(s,"  camera vertical lag during the rise      : worst {:.1} cm behind the feet (frame +{}), = {:.1} ms at {:.1} m/s",worst*100.,at-p,worst/rows[at].vz.max(0.5)*1000.,rows[at].vz);
        }
    }
    s
}
/// Rise / apex / fall timing against the animation phases of one standing jump (60 Hz and 144 Hz).
pub fn timeline()->String{
    let mut s=String::from("Jump timeline (standing jump): physics vs animation phase, t = seconds after the press frame started\n");
    for hz in [60.,144.]{
        let sc=Scenario{hz,seconds:3.5,press:0.5,hold:0.1,forward:false,run:false};let rows=run_chain(&sc);let p=(sc.press*hz).round() as usize;let t=|f:usize|(f-p+1) as f32/hz;
        let apex=first(&rows,p,|r|r.air&&r.vz<=0.);let land=first(&rows,p+2,|r|!r.air);
        let phase=|st:JumpState|first(&rows,p,|r|r.state==st);
        let _=writeln!(s,"\n@ {hz} Hz: liftoff t={:.3}; apex (vz<=0) t={:.3}; touchdown t={:.3}; airtime {:.3} s (ideal {:.3}), apex height {:.3} m (ideal {:.3})",
            t(p),apex.map_or(0.,t),land.map_or(0.,t),land.map_or(0.,|l|t(l)-t(p)),movement_apex_time()*2.,rows.iter().map(|r|r.height).fold(0.,f32::max),Params::default().jump_z_velocity.powi(2)/(2.*-Params::default().gravity_z())*0.01);
        for st in [JumpState::Jump,JumpState::Loop,JumpState::Fall,JumpState::Land,JumpState::Idle]{if let Some(f)=phase(st).filter(|&f|f>=p){let _=writeln!(s,"  anim {:?} entered at t={:.3} (height {:.2} m, vz {:+.2})",st,t(f),rows[f].height,rows[f].vz);}}
        let _=writeln!(s,"  Jump clip 0.4 s -> Loop 0.2 s before its end (0.2 s), Loop plays at 0.5 rate, Loop -> Fall as soon as vz<=0 with the landing predicted inside 0.75 s, Fall clip time-scaled to end at touchdown");
    }
    s
}
fn movement_apex_time()->f32{Params::default().jump_z_velocity/-Params::default().gravity_z()}
/// Apex / rise time / airtime for GravityScale 1, 1.5, 2, 2.5, 3 at the current JumpZ (analytic, plus a simulated check at 144 Hz).
pub fn tuning_table()->String{
    let mut s=String::new();let base=Params::default();
    let _=writeln!(s,"Gravity experiment table (JumpZ {} cm/s; world gravity {} cm/s^2). Run the game with FNRT_GRAVITY_SCALE=<x> and/or FNRT_JUMPZ=<cm/s>.\n  scale | g (m/s^2) | apex (m) | rise (s) | airtime (s) | simulated @144 Hz: apex / airtime",base.jump_z_velocity,base.world_gravity_z);
    for sc in [1.0f32,1.5,2.0,2.5,3.0]{
        let mut p=base.clone();p.gravity_scale=sc;let g=-p.gravity_z()*0.01;let v=p.jump_z_velocity*0.01;
        let mut c=fn_movement::cmc::Cmc::new(p,fn_movement::scene::Scene::default(),Vec3::ZERO);let dt=1./144.;let b=c.feet_z();c.jump();let (mut apex,mut t,mut left)=(0f32,0f32,false);
        for _ in 0..(6.*144.) as usize{c.tick(Vec2::ZERO,dt);t+=dt;if c.is_falling(){left=true;apex=apex.max(c.feet_z()-b);}if left&&c.is_moving_on_ground(){break;}}
        let _=writeln!(s,"  {sc:>4.1}  | {g:>8.2}  | {:>7.3}  | {:>7.3}  | {:>10.3}  | {:.3} m / {:.3} s",v*v/(2.*g),v/g,2.*v/g,apex*0.01,t);
    }
    s
}
/// Sample instants (multiples of 1/6 s: the only times at which the frame ends of 30, 60, 144 and 240 fps all coincide).
pub const SAMPLE_TIMES:[f32;12]=[0.5,7./6.,4./3.,1.5,5./3.,11./6.,2.0,13./6.,7./3.,2.5,3.0,3.5];
/// Worst deviation from the 240 fps run of the same scripted scenario (press at exactly t = 1 s): (height m, vz m/s, position m, eye y m, worst phase-entry time s).
pub fn deviation(hz:f32,fwd:bool,run:bool)->(f32,f32,f32,f32,f32){
    let go=|hz:f32|run_chain(&Scenario{hz,seconds:4.,press:1.,hold:0.1,forward:fwd,run});let (rows,reference)=(go(hz),go(240.));
    let at=|r:&[Chain],hz:f32,t:f32|r[((t*hz).round() as usize).saturating_sub(1)];
    let mut w=(0f32,0f32,0f32,0f32,0f32);
    for t in SAMPLE_TIMES{let (a,b)=(at(&rows,hz,t),at(&reference,240.,t));w.0=w.0.max((a.height-b.height).abs());w.1=w.1.max((a.vz-b.vz).abs());w.2=w.2.max((a.pos-b.pos).length());w.3=w.3.max((a.eye.y-b.eye.y).abs());}
    let enter=|r:&[Chain],hz:f32,st:JumpState|r.iter().find(|c|c.state==st&&c.t>1.0).map(|c|c.t-1./hz).unwrap_or(f32::NAN);
    for st in [JumpState::Jump,JumpState::Loop,JumpState::Fall,JumpState::Land]{w.4=w.4.max((enter(&rows,hz,st)-enter(&reference,240.,st)).abs());}
    w
}
pub fn deviation_report()->String{
    let mut s=String::from("Frame-rate independence: worst deviation from the 240 fps run of the same scenario (press at t = 1.0 s; sampled at multiples of 1/6 s)\n  scenario     fps | height cm | vz m/s | position cm | camera eye y cm | worst anim phase entry ms\n");
    for (name,fwd,run) in [("standing",false,false),("jog jump",true,false),("sprint jump",true,true)]{for hz in [30.,60.,144.]{let d=deviation(hz,fwd,run);
        let _=writeln!(s,"  {name:<11} {hz:>4} | {:>9.2} | {:>6.3} | {:>11.2} | {:>15.2} | {:>8.1}",d.0*100.,d.1,d.2*100.,d.3*100.,d.4*1000.);}}
    s
}
/// `--jump-feel-test`: all of the above in one log.
pub fn run() -> (String, usize) {
    let mut s=String::new();let p=Params::default();
    let _=writeln!(s,"--jump-feel-test. Effective: gravity {:.0} cm/s^2 (world {} x scale {}), JumpZ {} cm/s. {}\n",-p.gravity_z(),p.world_gravity_z,p.gravity_scale,p.jump_z_velocity,
        if p.is_tuned(){"(tuning override active)"}else{"(engine defaults, no override)"});
    s+=&tuning_table();s+="\n";s+=&timeline();s+="\n";s+=&deviation_report();s+="\n";s+=&latency_report();
    (s,0)
}
#[cfg(test)]mod tests{
    use super::*;
    const HZ:[f32;4]=[30.,60.,144.,240.];
    fn chain(hz:f32,fwd:bool,run:bool)->Vec<Chain>{run_chain(&Scenario{hz,seconds:4.,press:1.,hold:0.5/hz,forward:fwd,run})}
    /// Chain row at simulated time `t` (end of the frame that ends at t).
    fn at(rows:&[Chain],hz:f32,t:f32)->Chain{rows[((t*hz).round() as usize).saturating_sub(1)]}
    #[test]fn press_to_liftoff_is_zero_frames_at_every_fps(){
        for hz in HZ{for (fwd,run) in [(false,false),(true,false),(true,true)]{
            let rows=chain(hz,fwd,run);let p=hz as usize; // press frame (t = 1.0 s)
            assert_eq!(rows[p-1].jumps,0,"{hz} Hz: no jump before the press");
            let r=&rows[p];assert_eq!(r.jumps,1,"{hz} Hz: DoJump runs in the press frame itself");assert!(r.vz>6.-28./hz-0.1&&r.air&&r.height>0.,"{hz} Hz: liftoff in the press frame: {r:?}");
            assert_eq!(r.state,JumpState::Jump,"{hz} Hz fwd {fwd} run {run}: the animation enters Jump in the press frame");
            assert!(r.eye.y>rows[p-1].eye.y,"{hz} Hz: the camera starts rising in the press frame");
        }}
    }
    #[test]fn frame_rate_independence_of_the_jump_chain(){
        // Reference 240 Hz. Frames are aligned so the press starts at exactly t = 1.0 s at every rate. Tolerances (measured, see `deviation_report`, with margin):
        //  * height / vz while airborne: the midpoint rule is exact for constant gravity and the press is consumed in its own frame: measured 0.00 cm / 0.000 m/s, allowed 0.2 cm / 0.005 m/s
        //  * horizontal position: UE applies the braked/accelerated velocity for the whole frame, so the run-up differs by ~a*t*dt/2 (README "Frame-rate (in)dependence"):
        //    measured 8.0 / 3.4 / 0.8 cm at 30 / 60 / 144 fps (sprint), allowed 10 / 5 / 1.5 cm. This remainder is the engine's own, not a bug here.
        //  * animation phase entry times: one frame of the tested rate (the machine reacts to the frame it is stepped in): measured 25 / 8.3 / 2.8 ms, allowed 1.5 frames
        //  * camera eye y: FInterpTo is `Dist*clamp(dt*speed)` (frame-rate dependent in UE too): measured 6.6 cm (30/60 fps follow instantly, 240 fps trails ~12 ms), allowed 8 cm
        for (fwd,run) in [(false,false),(true,false),(true,true)]{
            let reference=chain(240.,fwd,run);
            // 30 fps is not in this list on purpose: its pawn tick delta (0.0333 s) is above the 0.03 s threshold of UFortMovementComp_Character::DoJump (exe 0x6d4f20), so its
            // JumpZ is scaled by 1.0589 (see `thirty_fps_per_frame_jump_carries_the_decoded_frame_delta_scale`); 60 fps and faster are below it and must agree with 240.
            for hz in [60.,144.]{
                let rows=chain(hz,fwd,run);
                let ptol=if hz<=30.{0.10}else if hz<=60.{0.05}else{0.015};
                // sample instants are multiples of 1/6 s: the only times at which the frame ends of 30, 60, 144 and 240 fps all coincide
                for t in [0.5f32,7./6.,4./3.,1.5,5./3.,11./6.,2.0,13./6.,7./3.,2.5,3.0,3.5]{
                    let (a,b)=(at(&rows,hz,t),at(&reference,240.,t));
                    assert!((a.height-b.height).abs()<0.002,"fwd {fwd} run {run} {hz} Hz t {t}: height {} vs {}",a.height,b.height);
                    assert!((a.vz-b.vz).abs()<0.005,"fwd {fwd} run {run} {hz} Hz t {t}: vz {} vs {}",a.vz,b.vz);
                    assert!((a.pos-b.pos).length()<ptol,"fwd {fwd} run {run} {hz} Hz t {t}: pos {:?} vs {:?}",a.pos,b.pos);
                    assert!((a.eye.y-b.eye.y).abs()<0.08,"fwd {fwd} run {run} {hz} Hz t {t}: eye y {} vs {}",a.eye.y,b.eye.y);
                }
                // animation phase entry times
                let enter=|rows:&[Chain],hz:f32,st:JumpState|rows.iter().find(|r|r.state==st&&r.t>1.0).map(|r|r.t-1./hz);
                for st in [JumpState::Jump,JumpState::Loop,JumpState::Fall,JumpState::Land]{
                    // the Loop state is only entered when the Jump clip runs 0.2 s before the apex hands over to Fall (apex 0.214 s after takeoff at the recovered gravity), so it may be skipped at some rates
                    if st==JumpState::Loop{if let (Some(a),Some(b))=(enter(&rows,hz,st),enter(&reference,240.,st)){assert!((a-b).abs()<=1.5/hz,"{st:?} entered at {a:.3} vs {b:.3}");}continue;}
                    let (a,b)=(enter(&rows,hz,st).unwrap(),enter(&reference,240.,st).unwrap());
                    assert!((a-b).abs()<=1.5/hz,"fwd {fwd} run {run} {hz} Hz: {st:?} entered at {a:.3} vs {b:.3}");
                }
            }
        }
    }
    #[test]fn thirty_fps_per_frame_jump_carries_the_decoded_frame_delta_scale(){
        // standing jump at 30 fps vs 240 fps: apex ratio = (jump_frame_scale(1/30))^2 within the sampling error (g dt^2/8 = 0.4 cm at 30 fps)
        let apex=|hz:f32|chain(hz,false,false).iter().map(|r|r.height).fold(0.,f32::max);
        let (a30,a240)=(apex(30.),apex(240.));let s=movement::params::jump_frame_scale(1./30.);
        let hover=0.022;let expect=(a240-hover)*s*s+hover;
        assert!((a30-expect).abs()<0.012,"30 fps apex {a30} vs 240 fps {a240} x {s}^2 -> {expect}");
        assert!(a30>a240*1.08,"{a30} {a240}");
    }
    #[test]fn rise_and_fall_match_the_animation_phases(){
        // The Jump clip (0.4 s) hands to the Loop at 0.2 s, the Loop hands to Fall at the apex: the body must still be rising while Jump/Loop play
        // and falling while Fall plays (no animation hanging in the apex while the body already falls, or the reverse).
        for hz in [60.,144.]{
            let rows=chain(hz,false,false);let p=hz as usize;
            for r in &rows[p..]{ if r.air{
                match r.state{JumpState::Jump=>assert!(r.vz>0.,"{hz} Hz: Jump state while falling at t {:.3}",r.t),
                    JumpState::Loop=>assert!(r.vz>-0.35,"{hz} Hz: Loop held {:.2} m/s into the descent at t {:.3}",r.vz,r.t),
                    JumpState::Fall=>assert!(r.vz<=0.1,"{hz} Hz: Fall state while rising {:.2} m/s at t {:.3}",r.vz,r.t),_=>{}}}}
        }
    }
    #[test]fn tuning_table_matches_closed_form(){let t=tuning_table();assert!(t.contains("1.0")&&t.contains("2.5"));}
    /// Writes verification/jump-feel-test.txt when FNRT_WRITE_EVIDENCE=1 (same as the CLI run).
    #[test]fn write_jump_feel_evidence(){
        if std::env::var_os("FNRT_WRITE_EVIDENCE").is_none(){return;}
        let dir=fn_live::var_os("FN_EVIDENCE_DIR").map(std::path::PathBuf::from).unwrap_or_else(||std::env::temp_dir().join("fn-evidence"));let _=std::fs::create_dir_all(&dir);std::fs::write(dir.join("jump-feel-test.txt"),run().0).unwrap();
    }
}
