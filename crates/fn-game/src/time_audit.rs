//! `--time-test [seconds]`: wall-clock vs simulation-clock audit of the REAL windowed loop (real frame times, not the fixed 1/60 of the scripted
//! captures). Presses jump on its own every 3 s of simulated time (no input injection) and, when it ends, writes
//! `verification/time-audit-<FNRT_TIME_TAG>.txt`: wall seconds between the first and last update, the dt the simulation was fed (controller / animation /
//! camera after their own clamps), the ratio of the two, how many frames were clamped, and the wall-clock vs simulated airtime of every jump.
//! Modes are selected with the existing diagnostics: `FNRT_NOVSYNC=1` (vsync off), `FNRT_FRAME_CAP=<fps>` (limiter), default = vsync on.
use std::{sync::{atomic::{AtomicBool,Ordering},Mutex},time::Instant};
pub static ON:AtomicBool=AtomicBool::new(false);
/// UE `MaxDeltaTime` stand-in: the longest frame the whole game simulates in one tick; longer frames (a hitch, a window drag) lose the excess.
/// Everything else is exactly real time. (The movement component sub-steps any frame longer than its 0.05 s `MaxSimulationTimeStep`.)
pub const MAX_DELTA_TIME:f32=0.1;
/// The old `App::update` rule for a real frame: `dt = raw.min(MAX_DELTA_TIME)` (scripted runs use a fixed 1/60 instead). Used by `GameSim::step_real`.
pub fn clamp_frame_dt(raw:f32)->f32{raw.min(MAX_DELTA_TIME)}
#[derive(Default)]struct Audit{first:Option<Instant>,last:Option<Instant>,frames:u32,raw:f64,fed:f64,ctrl:f64,cam:f64,clamped:u32,lost:f64,raws:Vec<f32>,secs:f32,ticks:u64,tick_secs:f64,tick_frames:u32,
    air_prev:bool,air_wall:Option<Instant>,air_sim:f64,flights:Vec<(f64,f64)>}
static A:Mutex<Option<Audit>>=Mutex::new(None);
pub fn enable(secs:f32){ON.store(true,Ordering::Relaxed);*A.lock().unwrap()=Some(Audit{secs,..Default::default()});}
pub fn on()->bool{ON.load(Ordering::Relaxed)}
/// One `App::update`: `raw` = real time since the previous update, `fed` = the dt every system received.
pub fn record(raw:f32,fed:f32){
    let mut g=A.lock().unwrap();let Some(a)=g.as_mut() else{return};let now=Instant::now();
    if a.first.is_none(){a.first=Some(now);a.last=Some(now);return;} // the first update's dt spans loading, skip it
    a.frames+=1;a.raw+=raw as f64;a.fed+=fed as f64;a.ctrl+=fed.min(fn_movement::MAX_FRAME_DT) as f64;a.cam+=fed.min(0.1) as f64;
    if raw>fed+1e-6{a.clamped+=1;a.lost+=(raw-fed) as f64;}a.raws.push(raw);a.last=Some(now);
    if a.air_wall.is_some(){a.air_sim+=fed as f64;}
}
/// Fixed-step simulation (src/sim_tick.rs): `ticks` ticks of `step` seconds ran in this frame.
pub fn note_ticks(ticks:u32,step:f32){
    let mut g=A.lock().unwrap();let Some(a)=g.as_mut() else{return};if a.first.is_none(){return;}
    a.ticks+=ticks as u64;a.tick_secs+=ticks as f64*step as f64;if ticks>0{a.tick_frames+=1;}
}
/// Simulated seconds fed so far (drives the scripted jump).
pub fn sim_time()->f64{A.lock().unwrap().as_ref().map_or(0.,|a|a.fed)}
/// Scripted jump key: down for 0.1 s of simulated time out of every 3 s.
pub fn jump_key()->bool{on()&&{let t=sim_time()%3.;t>=1.&&t<1.1}}
/// Called after the controller step with the pawn's airborne state: measures wall vs simulated flight time.
pub fn note_air(air:bool){
    let mut g=A.lock().unwrap();let Some(a)=g.as_mut() else{return};
    if air&&!a.air_prev{a.air_wall=Some(Instant::now());a.air_sim=0.;}
    if !air&&a.air_prev{if let Some(w)=a.air_wall.take(){a.flights.push((w.elapsed().as_secs_f64(),a.air_sim));}}
    a.air_prev=air;
}
/// True once the requested duration has elapsed (the main loop then exits and calls [`report`]).
pub fn done()->bool{on()&&A.lock().unwrap().as_ref().map_or(false,|a|a.first.map_or(false,|f|f.elapsed().as_secs_f32()>=a.secs))}
pub fn report(mode:&str)->String{
    let g=A.lock().unwrap();let Some(a)=g.as_ref() else{return String::new()};
    let wall=match(a.first,a.last){(Some(f),Some(l))=>l.duration_since(f).as_secs_f64(),_=>0.};
    let mut s=a.raws.clone();s.sort_by(|x,y|x.partial_cmp(y).unwrap());let p=|q:f32|s.get(((s.len().max(1)-1) as f32*q) as usize).copied().unwrap_or(0.)*1000.;
    let mut t=format!("time audit, mode: {mode}\nwall {wall:.3} s over {} frames ({:.1} fps)\nsum of raw frame times      {:.3} s\nsim dt fed to App/anim/combat {:.3} s   ratio sim/wall {:.5}\nsim dt fed to Controller::step {:.3} s (its own clamp {} s)   ratio {:.5}\nsim dt fed to camera           {:.3} s (its own clamp 0.1 s)   ratio {:.5}\nframes longer than the {} s clamp: {} (lost {:.3} s of game time)\nraw frame ms: median {:.2} p99 {:.2} max {:.2}\n",
        a.frames,a.frames as f64/wall.max(1e-9),a.raw,a.fed,a.fed/wall.max(1e-9),a.ctrl,fn_movement::MAX_FRAME_DT,a.ctrl/wall.max(1e-9),a.cam,a.cam/wall.max(1e-9),MAX_DELTA_TIME,a.clamped,a.lost,p(0.5),p(0.99),p(1.));
    t+=&format!("simulation ticks: {} ticks = {:.3} s of tick time over {:.3} s wall ({:.2} ticks/s; SIM_HZ {}; per-frame mode when 0), {} frames ran no tick\n",a.ticks,a.tick_secs,wall,a.ticks as f64/wall.max(1e-9),crate::sim_tick::sim_hz(),a.frames.saturating_sub(a.tick_frames));
    for (w,sim) in &a.flights{t+=&format!("flight: wall {w:.3} s, simulated {sim:.3} s (ideal 0.429 s)\n");}
    t
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn the_only_global_clamp_is_the_ue_max_delta_time_and_every_system_accepts_it(){
        // game time == real time except for frames longer than MAX_DELTA_TIME; no system may clamp tighter than that (a tighter per-system clamp = hidden slow motion)
        assert_eq!(MAX_DELTA_TIME,0.1);
        assert!(fn_movement::MAX_FRAME_DT>=MAX_DELTA_TIME,"controller clamp {} < {}",fn_movement::MAX_FRAME_DT,MAX_DELTA_TIME);
        let mut s=fn_combat::Swing::default();s.step(true,MAX_DELTA_TIME);
        assert!((s.time-MAX_DELTA_TIME).abs()<1e-6,"swing montage advances the full {} s of the frame, got {}",MAX_DELTA_TIME,s.time);
    }
}
