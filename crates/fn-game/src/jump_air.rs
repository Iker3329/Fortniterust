//! Air-jump investigation + regression harness (`--jump-air-test` in-app series lives in `pick_test.rs` air mode; this file is the
//! headless twin that drives the SAME controller + animation graph with the per-frame wiring of `App::update` in `FNRT_SIM_HZ=0` mode; the default
//! 30 Hz tick runs the same step from `sim_tick`, whose `FixedSim` twin is tested in `sim_tick.rs`: air presses are ignored by CanJump either way).
//! Fortnite 1.8 / UE4.16 rules: ACharacter::CanJumpInternal allows a jump only while JumpCurrentCount < JumpMaxCount (1), and
//! JumpMaxCount is never overridden by any package (reports/movement-full.json). So: no double jump, no air jump, and a jump
//! press while airborne must not change velocity or restart the jump animation.
//! The animation side is structural: the AdditiveJumps machine (anim_graph::JumpMachine) reads only the movement component's
//! airborne / vertical-speed / predicted-landing state (the graph's bIsJumping / bIsFalling / bLandingPredicted), never the key, and
//! the state log below proves every takeoff produces exactly one plain Idle -> Jump -> Loop -> Fall -> Land -> Idle cycle.
#![allow(dead_code)] // compiled in normal builds too: movement_series.rs runs it for `--jump-air-test`
use fn_animgraph::{AnimState,JumpState,PawnSample};
use fn_movement::{self as movement,Controller};
use glam::Vec2;
#[derive(Clone,Copy,Default,Debug)]pub struct In{pub jump:bool,pub crouch:bool,pub forward:bool,pub run:bool}
#[derive(Clone,Copy,Debug)]pub struct Row{pub frame:usize,pub t:f32,pub inp:In,pub height:f32,pub vz:f32,pub crouched:bool,pub state:JumpState,pub mix:f32,pub takeoffs:u32,pub jump_entries:u32}
/// One scripted frame stream at `hz`, wired exactly like App::update (crouch recomputed before the step, anim stepped after it).
/// Returns the per-frame rows and the final animation state (for its state log).
pub fn simulate_full(hz:f32,frames:usize,script:impl Fn(usize)->In)->(Vec<Row>,AnimState){
    let dt=1./hz;let mut c=Controller::default();let mut a=AnimState::new();let mut rows=vec![];
    let (mut takeoffs,mut entries)=(0u32,0u32);let mut prev_air=false;let mut prev_state=JumpState::Idle;
    for f in 0..frames{
        let i=script(f);
        // same crouch wiring as App::update: the crouch REQUEST (bWantsToCrouch) is dropped on the jump-press frame (CanJump refuses while it is set)
        c.wants_crouch=i.crouch&&!c.jump_edge(i.jump);
        let dir=if i.forward{Vec2::Y}else{Vec2::ZERO};
        c.step(dir,i.jump,i.run,dt);
        let air=c.airborne();if air&&!prev_air{takeoffs+=1;}prev_air=air;
        a.step_sample(&PawnSample{velocity:(c.velocity.x,c.velocity.y),input:(dir.x,dir.y),facing_deg:c.facing,camera_yaw_deg:c.facing,camera_pitch_deg:0.,height:c.air_height(),vertical_speed:c.vertical_speed,
            fall_time_left:movement::fall_time_left(c.air_height(),c.vertical_speed),crouched:c.crouched,running:c.running,melee_swinging:false},dt);
        if a.jump.state==JumpState::Jump&&prev_state!=JumpState::Jump{entries+=1;}prev_state=a.jump.state;
        rows.push(Row{frame:f,t:(f+1) as f32*dt,inp:i,height:c.height,vz:c.vertical_speed,crouched:c.crouched,state:a.jump.state,mix:a.jump.pose().mix,takeoffs,jump_entries:entries});
    }(rows,a)
}
pub fn simulate(hz:f32,frames:usize,script:impl Fn(usize)->In)->Vec<Row>{simulate_full(hz,frames,script).0}
pub fn trace(rows:&[Row])->String{rows.iter().map(|r|format!("f{:03} t{:.3} jump{} crouch{} h{:.3} vz{:+.2} crouched{} {:?} mix{:.2} takeoffs{} jumpAnimEntries{}",r.frame,r.t,r.inp.jump as u8,r.inp.crouch as u8,r.height,r.vz,r.crouched as u8,r.state,r.mix,r.takeoffs,r.jump_entries)).collect::<Vec<_>>().join("\n")}
/// The state log as text ("Idle->Jump Jump->Loop ...").
pub fn state_log(a:&AnimState)->String{a.jump.log.iter().map(|(_,f,t)|format!("{f:?}->{t:?}")).collect::<Vec<_>>().join(" ")}
/// Every consecutive pair of the log must be an edge of the baked AdditiveJumps machine and consecutive entries must chain.
pub fn log_is_a_clean_walk(a:&AnimState)->Result<(),String>{
    use JumpState::*;
    let edges=[(Idle,Jump),(Idle,Fall),(Jump,Loop),(Jump,Fall),(Loop,Fall),(Fall,Land),(Land,Idle)];
    let mut at=Idle;
    for (tick,from,to) in &a.jump.log{
        if *from!=at{return Err(format!("tick {tick}: log does not chain ({from:?} after {at:?})"));}
        if !edges.contains(&(*from,*to)){return Err(format!("tick {tick}: {from:?}->{to:?} is not a baked AdditiveJumps transition"));}
        at=*to;
    }
    Ok(())
}
#[cfg(test)]mod tests{
    use super::*;
    fn pulse(frames:&'static [usize])->impl Fn(usize)->In{move|f|In{jump:frames.contains(&f),..Default::default()}}
    fn held(from:usize,to:usize)->impl Fn(usize)->In{move|f|In{jump:f>=from&&f<to,..Default::default()}}
    const PLAIN:[JumpState;6]=[JumpState::Idle,JumpState::Jump,JumpState::Loop,JumpState::Fall,JumpState::Land,JumpState::Idle];
    /// The plain cycle. With the project gravity (-2800, PROVENANCE: decrypted config, not independently verified) the apex comes 0.214 s after takeoff at pawn ticks of 0.03 s or shorter (0.227 s at 30 fps: the exe DoJump frame-delta scale), so Jump -> Loop (0.2 s into the 0.4 s clip) may be skipped by Jump -> Fall.
    fn is_plain(a:&AnimState)->bool{let v=a.jump.visited();v==PLAIN.to_vec()||v==vec![JumpState::Idle,JumpState::Jump,JumpState::Fall,JumpState::Land,JumpState::Idle]}
    /// Frame (at `hz`) in which a jump pressed at frame `10 * hz / 60` touches down again.
    fn landing(hz:f32)->usize{let k=hz/60.;let t=(10.*k) as usize;let rows=simulate(hz,(300.*k) as usize,move|f|In{jump:f==t,..Default::default()});rows.iter().enumerate().skip(t+2).find(|(_,r)|r.height==0.).map(|(i,_)|i).unwrap()}
    #[test]fn air_presses_at_every_phase_do_nothing(){
        // takeoff at frame 10 (60 Hz: apex ~ +13 frames, landing ~ +26); presses during rise, apex, fall and the last frames before touchdown
        for hz in [30.,60.,144.,240.]{
            let k=hz/60.;let at=|s:f32|(s*k) as usize;let (t,l)=(at(10.),landing(hz));
            let mut script=vec![t];for fr in [0.1f32,0.3,0.5,0.7,0.85]{script.push(t+((l-t) as f32*fr) as usize);}script.push(l-3);script.push(l-2); // later presses would be legitimate ground jumps
            let script=std::sync::Arc::new(script);let s2=script.clone();
            let (rows,a)=simulate_full(hz,(300.*k) as usize,move|f|In{jump:s2.contains(&f),..Default::default()});
            let last=rows.last().unwrap();
            assert_eq!(last.takeoffs,1,"hz {hz}: exactly one takeoff, got {}\n{}",last.takeoffs,trace(&rows));
            assert_eq!(last.jump_entries,1,"hz {hz}: jump animation must start once, got {}; state log: {}",last.jump_entries,state_log(&a));
            // the state log is the plain cycle: Idle->Jump->Loop->Fall->Land->Idle, nothing else, no re-entry
            assert!(is_plain(&a),"hz {hz}: state log {}",state_log(&a));
            assert!(matches!(a.jump.log.len(),4|5),"hz {hz}: exactly four or five transitions, got: {}",state_log(&a));
            log_is_a_clean_walk(&a).unwrap();
            // no frame after takeoff may gain vertical speed
            for w in rows.windows(2){if w[0].height>0.&&w[1].height>0.{assert!(w[1].vz<=w[0].vz+1e-4,"hz {hz}: vertical speed increased in the air at frame {}",w[1].frame);}}
        }
    }
    #[test]fn spamming_the_key_every_frame_through_flight_and_landing_never_replays_the_rise(){
        // jump key mashed on EVERY frame from takeoff for 6 s: the pawn may only re-jump after touching down (a legitimate ground jump), and
        // every takeoff is one plain cycle: Jump entries == takeoffs, the log chains through baked edges only, no Jump->Jump / Land->Jump.
        for hz in [60.,144.] {
            let (rows,a)=simulate_full(hz,(360.*hz/60.) as usize,|f|In{jump:f>=10&&f%2==0,..Default::default()});
            let last=rows.last().unwrap();
            assert!(last.takeoffs>=2,"the mashing pawn re-jumps from the ground, got {} takeoffs",last.takeoffs);
            assert_eq!(last.jump_entries,last.takeoffs,"hz {hz}: every takeoff starts the jump clip exactly once; log {}",state_log(&a));
            log_is_a_clean_walk(&a).unwrap();
            // while airborne the machine must be in Jump/Loop/Fall (or a cross-fade from them) -- never Idle with the key down
            for r in rows.iter().filter(|r|r.height>0.&&r.inp.jump){assert!(r.state!=JumpState::Land,"hz {hz}: Land state while airborne at frame {}",r.frame);}
        }
    }
    #[test]fn holding_through_flight_and_landing_does_not_rejump_or_replay(){
        let (rows,a)=simulate_full(60.,400,held(10,300));let last=rows.last().unwrap();
        assert_eq!(last.takeoffs,1,"{}",trace(&rows));assert_eq!(last.jump_entries,1);assert_eq!(last.state,JumpState::Idle);
        assert!(is_plain(&a),"{}",state_log(&a));
    }
    #[test]fn press_during_landing_recovery_is_an_ordinary_ground_jump(){
        // touchdown ~f36; press 4 frames later (Land state, ground): a legitimate jump from the ground, animation restarts exactly once for it
        let l=landing(60.);let (rows,a)=simulate_full(60.,400,move|f|In{jump:f==10||f==l+4,..Default::default()});let last=rows.last().unwrap();
        assert_eq!(last.takeoffs,2,"{}",trace(&rows));assert_eq!(last.jump_entries,2);
        log_is_a_clean_walk(&a).unwrap();
        // the Land clip is interrupted by the new jump (Land->Idle 0.08 s, then Idle->Jump), never Land->Jump
        assert!(a.jump.log.iter().all(|(_,f,t)|!(*f==JumpState::Land&&*t==JumpState::Jump)),"{}",state_log(&a));
        // but a press 2 frames BEFORE touchdown (still airborne) must not queue a second jump (no input buffering in UE4.16)
        let (rows,a)=simulate_full(60.,400,move|f|In{jump:f==10||f==l-2,..Default::default()});assert_eq!(rows.last().unwrap().takeoffs,1,"{}",trace(&rows));
        assert!(is_plain(&a),"{}",state_log(&a));
    }
    #[test]fn crouched_press_stands_and_jumps_once_and_air_press_is_ignored(){
        let rows=simulate(60.,400,|f|In{crouch:f<10||(f>14&&f<200),jump:f==10||f==11||f==20,..Default::default()});let last=rows.last().unwrap();
        assert_eq!(last.takeoffs,1,"{}",trace(&rows));
        // 1.8 exe CanCrouchInCurrentState (0x20702e0) = IsFalling || IsMovingOnGround: a crouch key still held in the air crouches the airborne pawn again (the press tick itself stands it up)
        assert!(rows.iter().filter(|r|r.height>0.).skip(2).any(|r|r.crouched),"crouch key held in the air re-crouches the capsule");
    }
    #[test]fn sprint_jump_air_press_is_ignored(){
        let (rows,a)=simulate_full(60.,400,|f|In{forward:f>=0,run:true,jump:f==30||f==38||f==48,..Default::default()});assert_eq!(rows.last().unwrap().takeoffs,1,"{}",trace(&rows));
        assert!(is_plain(&a),"{}",state_log(&a));
    }
    #[test]fn single_frame_press_at_high_fps_is_not_lost(){
        // At 240 Hz a frame (4.2 ms) is shorter than a 120 Hz physics step: roughly every other frame runs no substep.
        // A 1-frame press on a no-substep frame must still jump (the request has to be latched until a substep consumes it).
        for press in 5..9usize{let rows=simulate(240.,600,move|f|In{jump:f==press,..Default::default()});assert_eq!(rows.last().unwrap().takeoffs,1,"press at frame {press}: {}",trace(&rows[..40.min(rows.len())]));}
    }
    /// Writes the evidence log `verification/jump-air-state-log.txt` (state log of the spam scenarios) when asked via FNRT_WRITE_EVIDENCE=1.
    #[test]fn write_state_log_evidence(){
        if std::env::var_os("FNRT_WRITE_EVIDENCE").is_none(){return;}
        let mut out=String::from("Jump-in-the-air state log (headless twin of App::update, 1.8 AdditiveJumps machine). Plain cycle = Idle->Jump->Loop->Fall->Land->Idle.\n");
        for (name,script) in [("press only at takeoff",Box::new(|f:usize|In{jump:f==10,..Default::default()}) as Box<dyn Fn(usize)->In>),
            ("air presses at f15 f30 f47 f60 f75 f80 f81 f82",Box::new(|f:usize|In{jump:[10,15,30,47,60,75,80,81,82].contains(&f),..Default::default()})),
            ("key mashed every other frame from takeoff for 3 s",Box::new(|f:usize|In{jump:f>=10&&f%2==0,..Default::default()})),
            ("press during landing recovery (f90)",Box::new(|f:usize|In{jump:f==10||f==90,..Default::default()}))]{
            let (rows,a)=simulate_full(60.,300,script);let last=rows.last().unwrap();
            out+=&format!("\n== {name}\ntakeoffs {} jump-clip entries {}\nlog: {}\nplain cycles only: {}\n",last.takeoffs,last.jump_entries,state_log(&a),log_is_a_clean_walk(&a).is_ok());
            for w in rows.windows(2).filter(|w|w[0].state!=w[1].state){out+=&format!("  f{:03} t{:.3} {:?}->{:?} h{:.3} vz{:+.2} jumpkey{}\n",w[1].frame,w[1].t,w[0].state,w[1].state,w[1].height,w[1].vz,w[1].inp.jump as u8);}
        }
        let dir=fn_live::var_os("FN_EVIDENCE_DIR").map(std::path::PathBuf::from).unwrap_or_else(||std::env::temp_dir().join("fn-evidence"));let _=std::fs::create_dir_all(&dir);std::fs::write(dir.join("jump-air-state-log.txt"),out).unwrap();
    }
}
