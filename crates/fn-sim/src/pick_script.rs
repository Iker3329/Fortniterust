//! `--pickaxe-test`: scripted fixed-60Hz run that drives every equipped-pickaxe state (idle, jog, sprint,
//! crouch idle/walk, jump, swing windup, TriggerAbility hit -> Bounce, held auto-repeat until the
//! harvestable is destroyed, a miss swing) and then re-captures at several window sizes/aspects.
//! Writes verification/pick-*.bmp plus verification/pickaxe-test.txt. Pure data so it can be unit-tested.
use std::sync::atomic::{AtomicBool,Ordering};
static ACTIVE:AtomicBool=AtomicBool::new(false);
pub fn set_active(v:bool){ACTIVE.store(v,Ordering::Relaxed);}
pub fn active()->bool{ACTIVE.load(Ordering::Relaxed)}
#[derive(Default,Clone,Copy,Debug,PartialEq)]pub struct Step{pub forward:f32,pub side:f32,pub run:bool,pub crouch:bool,pub jump:bool,pub attack:bool,
 /// Scripted camera (`--anim-test` only): yaw as an offset in degrees from the spawn yaw, pitch absolute in degrees (positive = up).
 pub yaw:Option<f32>,pub pitch:Option<f32>}
static ANIM:AtomicBool=AtomicBool::new(false);
static DIRS:AtomicBool=AtomicBool::new(false);
/// `--dir-test`: locomotion direction series (8 directions x jog / sprint / crouch with a fixed camera, then a camera turn while holding left).
pub fn set_dir_series(v:bool){DIRS.store(v,Ordering::Relaxed);if v{ANIM.store(true,Ordering::Relaxed);}}
pub fn dir_series()->bool{DIRS.load(Ordering::Relaxed)}
pub const DIR_NAMES:[&str;8]=["F","B","L","R","FL","FR","BL","BR"];
pub const DIR_KEYS:[(f32,f32);8]=[(1.,0.),(-1.,0.),(0.,-1.),(0.,1.),(1.,-1.),(1.,1.),(-1.,-1.),(-1.,1.)];
pub const DIR_MODES:[&str;3]=["jog","sprint","crouch"];
pub const DIR_CASE:u32=100;
pub const DIR_END:u32=DIR_CASE*25+20;
pub fn dir_step(f:u32)->Step{
 let c=(f/DIR_CASE) as usize;let k=f%DIR_CASE;
 if c>=24{ // camera turning while holding strafe left
  let t=f-DIR_CASE*24;return Step{forward:0.,side:-1.,run:false,crouch:false,jump:false,attack:false,yaw:Some(if t<30{0.}else{(t-30) as f32*2.}),pitch:Some(0.)};}
 let (mode,dir)=(c/8,c%8);let move_=k>=10&&k<95;
 Step{forward:if move_{DIR_KEYS[dir].0}else{0.},side:if move_{DIR_KEYS[dir].1}else{0.},run:mode==1&&move_,crouch:mode==2,jump:false,attack:false,yaw:Some(0.),pitch:Some(0.)}}
pub fn dir_capture(f:u32)->Option<&'static str>{
 let c=(f/DIR_CASE) as usize;let k=f%DIR_CASE;
 if c<24&&k==90{return Some(Box::leak(format!("dir-{}-{}",DIR_MODES[c/8],DIR_NAMES[c%8]).into_boxed_str()));}
 if c==24&&matches!(k,25|60|99){return Some(Box::leak(format!("dir-turn-{k}").into_boxed_str()));}
 None}
/// `--anim-test`: the animation-graph series (aim / head look, turn in place, jump from idle and from a jog, key mashing in the air).
pub fn set_anim_series(v:bool){ANIM.store(v,Ordering::Relaxed);}
pub fn anim_series()->bool{ANIM.load(Ordering::Relaxed)}
pub fn clear_frame()->u32{if anim_series(){2}else{CLEAR_FRAME}}
pub fn target_frame()->u32{if anim_series(){u32::MAX}else{TARGET_FRAME}}
pub fn end_frame()->u32{if dir_series(){DIR_END}else if anim_series(){ANIM_END}else{END_FRAME}}
/// --anim-test script (60 Hz). Camera: pitch up 0..40 (f20-40), down to -35 (f60-80), level (f100-120); yaw right 55 (f120-140), left 55 (f160-180),
/// 110 (f200-215, past the 70 degree turn threshold), back to 0 (f290-310). Jump from idle f400, W held f520-690 with a jog jump at f580,
/// jump key mashed every other frame f720-800 (the pawn re-jumps only after touching down).
pub const ANIM_JUMP_IDLE:u32=400;pub const ANIM_JUMP_JOG:u32=580;pub const ANIM_JOG:(u32,u32)=(520,690);pub const ANIM_SPAM:(u32,u32)=(720,800);pub const ANIM_END:u32=860;
fn ramp(f:u32,a:u32,b:u32,x:f32,y:f32)->f32{if f<=a{x}else if f>=b{y}else{x+(y-x)*(f-a) as f32/(b-a) as f32}}
fn anim_pitch(f:u32)->f32{if f<40{ramp(f,20,40,0.,40.)}else if f<80{ramp(f,60,80,40.,-35.)}else{ramp(f,100,120,-35.,0.)}}
fn anim_yaw(f:u32)->f32{if f<160{ramp(f,120,140,0.,55.)}else if f<200{ramp(f,160,180,55.,-55.)}else if f<290{ramp(f,200,215,-55.,110.)}else{ramp(f,290,310,110.,0.)}}
pub fn anim_step(f:u32)->Step{if dir_series(){return dir_step(f);}Step{side:0.,forward:if (ANIM_JOG.0..ANIM_JOG.1).contains(&f){1.}else{0.},run:false,crouch:false,
 jump:f==ANIM_JUMP_IDLE||f==ANIM_JUMP_JOG||((ANIM_SPAM.0..ANIM_SPAM.1).contains(&f)&&f%2==0),attack:false,yaw:Some(anim_yaw(f)),pitch:Some(anim_pitch(f))}}
pub fn anim_capture(f:u32)->Option<&'static str>{if dir_series(){return dir_capture(f);}Some(match f{
 20=>"anim-aim-center",58=>"anim-aim-pitch-up",98=>"anim-aim-pitch-down",158=>"anim-aim-yaw-right",198=>"anim-aim-yaw-left",
 212=>"anim-turn-start",236=>"anim-turn-mid",270=>"anim-turn-end",
 410=>"anim-jump-idle-rise",437=>"anim-jump-idle-apex",455=>"anim-jump-idle-fall",470=>"anim-jump-idle-fall-late",477=>"anim-jump-idle-land",488=>"anim-jump-idle-recover",
 565=>"anim-jog-steady",590=>"anim-jump-jog-rise",617=>"anim-jump-jog-apex",635=>"anim-jump-jog-fall",650=>"anim-jump-jog-fall-late",657=>"anim-jump-jog-land",668=>"anim-jump-jog-recover",
 728=>"anim-jump-spam-rise",756=>"anim-jump-spam-fall",796=>"anim-jump-spam-late",
 _=>return None})}
/// Frame at which the player is returned to spawn and a tree is planted 1.8 m ahead.
pub const TARGET_FRAME:u32=350;
/// Frame at which the placeholder tree is removed so the jump / sprint / crouch / equip series runs on an empty baseplate.
pub const CLEAR_FRAME:u32=745;
/// Equip montage trigger (Melee_Equip, 0.7718 s) and the animation-graph series that follows it (all at fixed 60 Hz):
/// standing jump 830 (airtime 1.224 s -> touchdown ~903), sprint 950..1130 with a sprint jump at 1040, crouch 1180..1260.
pub const EQUIP_FRAME:u32=760;
pub const JUMP_FRAMES:[u32;2]=[830,1040];
pub const SPRINT_RANGE:(u32,u32)=(950,1130);
pub const CROUCH_RANGE:(u32,u32)=(1180,1260);
/// Frame after which the player is stationary again (resolution sweep).
pub const SIZE_SWEEP_START:u32=1340;
pub fn equip(f:u32)->bool{active()&&f==if anim_series(){3}else{EQUIP_FRAME}}
pub const SIZES:[(i32,i32);6]=[(1280,720),(1920,1080),(2560,1080),(1024,768),(640,480),(3440,1440)];
pub const END_FRAME:u32=SIZE_SWEEP_START+SIZES.len() as u32*14+10;
pub fn step(f:u32)->Step{
 if !active(){return Step::default();}
 if anim_series(){return anim_step(f);}
 Step{
  forward:if (30..90).contains(&f)||(100..150).contains(&f)||(210..250).contains(&f)||(SPRINT_RANGE.0..SPRINT_RANGE.1).contains(&f){1.}else{0.},
  run:(100..150).contains(&f)||(SPRINT_RANGE.0..SPRINT_RANGE.1).contains(&f),
  crouch:(170..260).contains(&f)||(CROUCH_RANGE.0..CROUCH_RANGE.1).contains(&f),
  jump:f==280||JUMP_FRAMES.contains(&f),
  side:0.,attack:attack(f),yaw:None,pitch:None,
 }
}
/// Left-mouse script. Single press 370..420 (hit at ~380, bounce, then release), held 450..720.
pub fn attack(f:u32)->bool{active()&&((370..420).contains(&f)||(450..720).contains(&f))}
/// Screenshot name for the frame (after the frame counter incremented), if any.
pub fn capture(f:u32)->Option<&'static str>{if !active(){return None;}if anim_series(){return anim_capture(f);}Some(match f{
 20=>"pick-idle",80=>"pick-jog",140=>"pick-sprint",200=>"pick-crouch-idle",245=>"pick-crouch-walk",292=>"pick-jump",
 374=>"pick-swing-windup",379=>"pick-swing-pre-hit",383=>"pick-hit-bounce-start",392=>"pick-bounce-late",
 470=>"pick-harvest-held",560=>"pick-harvest-held-2",640=>"pick-harvest-held-3",
 700=>"pick-swing-miss",740=>"pick-idle-after",
 // equip montage (t = (f-760)/60 s): blend-in, mid, late blend-out, done
 763=>"pick-equip-early",770=>"pick-equip-mid",785=>"pick-equip-late",815=>"pick-equip-done",
 // standing jump (takeoff 830): rise, apex (0.61 s), descent, late fall, touchdown (~903), landing recovery
 836=>"pick-jump-rise-early",846=>"pick-jump-rise",866=>"pick-jump-apex",880=>"pick-jump-fall",896=>"pick-jump-fall-late",905=>"pick-jump-land",915=>"pick-jump-land-mid",935=>"pick-jump-recover",
 // sprint (steady after the 0.5 s blend) and a jump while sprinting (takeoff 1040, touchdown ~1113)
 1030=>"pick-sprint-steady",1047=>"pick-sprintjump-rise",1067=>"pick-sprintjump-apex",1090=>"pick-sprintjump-fall",1116=>"pick-sprintjump-land",1170=>"pick-stand-before-crouch",
 // crouch transitions: enter 0.25 s, exit 0.35 s
 1183=>"pick-crouch-enter-early",1190=>"pick-crouch-enter-mid",1205=>"pick-crouch-entered",1263=>"pick-crouch-exit-early",1272=>"pick-crouch-exit-mid",1295=>"pick-crouch-exited",
 _=>return None})}
/// Window client size to apply on this frame (resolution sweep), if any.
pub fn resize(f:u32)->Option<(i32,i32)>{if active()&&f==1{return Some(SIZES[0]);} // whole series at 1280x720
 if !active()||anim_series()||f<SIZE_SWEEP_START{return None;}let i=((f-SIZE_SWEEP_START)/14) as usize;if (f-SIZE_SWEEP_START)%14==0&&i<SIZES.len(){Some(SIZES[i])}else{None}}
/// Capture once the resized frame has rendered (11 frames after the resize).
pub fn size_capture(f:u32)->bool{active()&&!anim_series()&&f>=SIZE_SWEEP_START&&(f-SIZE_SWEEP_START)%14==11&&(((f-SIZE_SWEEP_START)/14) as usize)<SIZES.len()}
#[cfg(test)]mod tests{use super::*;
 use crate::scenarios::TEST_LOCK as LOCK;
 #[test]fn script_covers_every_state_exactly_once(){let _g=LOCK.lock().unwrap_or_else(|e|e.into_inner());set_active(true);
  let names:Vec<_>=(0..END_FRAME).filter_map(capture).collect();let mut d=names.clone();d.sort();d.dedup();assert_eq!(names.len(),d.len());assert!(names.len()>=15);
  assert!((0..END_FRAME).filter(|f|resize(*f).is_some()).count()==SIZES.len()+1); // +1: the initial 1280x720 window for the whole seriesassert!((0..END_FRAME).filter(|f|size_capture(*f)).count()==SIZES.len());
  assert!(step(100).run&&step(100).forward==1.);assert!(step(200).crouch&&!step(200).run);assert!(step(280).jump&&!step(281).jump);
  // the single press ends before the held press starts; target is planted while idle
  assert!(attack(370)&&!attack(430)&&attack(450));assert!(!step(TARGET_FRAME).attack&&step(TARGET_FRAME).forward==0.);
  set_active(false);assert_eq!(step(100),Step::default());}
 #[test]fn graph_series_is_ordered_and_precedes_the_size_sweep(){let _g=LOCK.lock().unwrap_or_else(|e|e.into_inner());set_active(true);
  assert!(equip(EQUIP_FRAME)&&!equip(EQUIP_FRAME+1));assert!(CLEAR_FRAME<EQUIP_FRAME&&EQUIP_FRAME+60<JUMP_FRAMES[0]);
  // standing jump (1.224 s = ~74 frames of flight + 0.4 s recovery) finishes before the sprint run starts; sprint jump before the sprint run ends
  assert!(JUMP_FRAMES[0]+74+36<SPRINT_RANGE.0&&SPRINT_RANGE.0<JUMP_FRAMES[1]&&JUMP_FRAMES[1]+74<SPRINT_RANGE.1);
  assert!(SPRINT_RANGE.1+40<CROUCH_RANGE.0&&CROUCH_RANGE.1+40<SIZE_SWEEP_START);
  assert!(step(JUMP_FRAMES[0]).jump&&step(JUMP_FRAMES[1]).jump&&!step(JUMP_FRAMES[0]+1).jump);
  assert!(step(SPRINT_RANGE.0+10).run&&step(SPRINT_RANGE.0+10).forward==1.&&!step(SPRINT_RANGE.1).run);
  assert!(step(CROUCH_RANGE.0+1).crouch&&!step(CROUCH_RANGE.1).crouch);
  for f in [836,846,866,880,896,905,915,935,1030,1047,1067,1090,1116,1183,1190,1205,1263,1272,1295,763,770,785,815]{assert!(capture(f).is_some(),"{f}");assert!(f<SIZE_SWEEP_START);}
  set_active(false);}
 #[test]fn anim_series_script_is_unique_ordered_and_covers_the_requested_states(){let _g=LOCK.lock().unwrap_or_else(|e|e.into_inner());set_active(true);set_anim_series(true);
  let names:Vec<_>=(0..ANIM_END).filter_map(capture).collect();let mut d=names.clone();d.sort();d.dedup();assert_eq!(names.len(),d.len());
  for want in ["anim-aim-pitch-up","anim-aim-pitch-down","anim-aim-yaw-right","anim-aim-yaw-left","anim-turn-start","anim-jump-idle-apex","anim-jump-idle-land","anim-jump-jog-rise","anim-jump-jog-apex","anim-jump-jog-fall","anim-jump-jog-land","anim-jump-spam-fall"]{assert!(names.contains(&want),"{want}");}
  assert!(step(ANIM_JUMP_IDLE).jump&&step(ANIM_JUMP_JOG).jump&&step(ANIM_JUMP_JOG).forward==1.&&step(ANIM_JUMP_IDLE).forward==0.);
  assert!((ANIM_SPAM.0..ANIM_SPAM.1).filter(|f|step(*f).jump).count()>=35);
  // standing jump finishes (1.224 s airtime + recovery) before the jog starts; the jog jump lands before the key-mashing block
  assert!(ANIM_JUMP_IDLE+74+40<ANIM_JOG.0&&ANIM_JOG.0<ANIM_JUMP_JOG&&ANIM_JUMP_JOG+74<ANIM_JOG.1&&ANIM_JOG.1<ANIM_SPAM.0&&ANIM_SPAM.1+40<ANIM_END);
  assert!(step(0).yaw==Some(0.)&&step(150).yaw.unwrap()>50.&&step(190).yaw.unwrap()< -50.&&step(240).yaw.unwrap()>100.);
  set_anim_series(false);set_active(false);}
 #[test]fn size_sweep_has_wide_narrow_low_and_high_resolutions(){let ratio=|s:(i32,i32)|s.0 as f32/s.1 as f32;assert!(SIZES.iter().any(|s|(ratio(*s)-16./9.).abs()<0.01));assert!(SIZES.iter().any(|s|ratio(*s)>2.3));assert!(SIZES.iter().any(|s|(ratio(*s)-4./3.).abs()<0.01));assert!(SIZES.iter().any(|s|s.0<=640));assert!(SIZES.iter().any(|s|s.0>=3000));}
}

