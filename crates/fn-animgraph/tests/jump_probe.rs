//! Per-tick probe of a full jump (standing / walking / jogging / sprinting) at 60 Hz: pawn velocity z, the native-split flags, the state
//! weights of every machine that matters in the air and the dominant clips of the pose tree. Run with
//! `cargo test --release -p fn-animgraph --test jump_probe -- --nocapture` (env `FN_PROBE_GRAVITY` = cm/s^2, default 2800 = fn-movement WORLD_GRAVITY_Z;
//! the test also runs 980 for the flight-length comparison). The asserts pin the 1.8 rules (JUMP_AIR.md) for the first airborne tick.
use fn_animgraph::*;

fn leaf(n:&Node,w:f32,out:&mut Vec<(String,f32)>){
    let mut add=|name:&str,w:f32|{if w>1e-4{match out.iter_mut().find(|(c,_)|c==name){Some(o)=>o.1+=w,None=>out.push((name.to_owned(),w))}}};
    match n{
        Node::Clip{name,..}=>add(name,w),
        Node::Mix(v)=>{let t:f32=v.iter().map(|(_,x)|*x).sum();if t>1e-6{for (c,x) in v{leaf(c,w*x/t,out);}}}
        Node::MixProfile{from,to,w:t,..}=>{leaf(from,w*(1.-t),out);leaf(to,w*t,out);}
        Node::ApplyAdditive{base,add:a,alpha}|Node::ApplyMeshAdditive{base,add:a,alpha}=>{leaf(base,w,out);leaf(a,w*alpha,out);}
        Node::Layered{base,blend,weight,..}=>{leaf(base,w*(1.-weight),out);leaf(blend,w*weight,out);}
        Node::AimOffset{base,..}|Node::RotateRoot{base,..}|Node::TwoBoneIk{base,..}|Node::HandRetarget{base,..}|Node::CopyBone{base,..}|Node::ModifyBone{base,..}|Node::HeadTrack{base,..}=>leaf(base,w,out),
        Node::Ref=>{}}
}
/// Clip leaves grouped by family (the BlendSpace samples of one family are summed).
fn families(a:&AnimState)->Vec<(String,f32)>{
    let mut lw=vec![];leaf(&a.plan(&[]),1.,&mut lw);
    let mut out:Vec<(String,f32)>=vec![];
    for (n,w) in lw{
        let fam=if n.starts_with("M_Jog_AssaultRifle_")||n.starts_with("M_Jog_AR_"){"jog-core(legs)"}
            else if n.starts_with("M_Jog_Melee_Twohanded_")||n.starts_with("M_Jog_Melee_TH"){"jog-additive"}
            else if n.starts_with("Sprint_"){"Sprint"}
            else if n.starts_with("JogAR_Lean"){"lean-add"}
            else if n.starts_with("M_Walk_")||n.starts_with("M_Locomotion2D"){"walk"}
            else if n=="M_Melee_TwoHanded"{"M_Melee_TwoHanded(idle lower pose)"}
            else if n=="Idle_Noise"||n=="Idle_Noise_AR_Downsights"{"Idle_Noise(add)"}
            else{n.as_str()};
        match out.iter_mut().find(|(c,_)|c==fam){Some(o)=>o.1+=w,None=>out.push((fam.to_owned(),w))}
    }
    out.sort_by(|a,b|b.1.partial_cmp(&a.1).unwrap());out
}
struct Row{t:f32,vz:f32}
fn run(label:&str,vel:f32,sprint:bool,g:f32,print:bool)->(Vec<Row>,AnimState){
    let dt=1./60.;
    let acc=if vel>0.{MAX_ACCELERATION_CM}else{0.};
    let ground=PawnInput{sprinting:sprint,velocity:[vel,0.,0.],accel:[acc,0.],..PawnInput::melee()};
    let mut a=AnimState::new();for _ in 0..120{a.step(&ground,dt);}
    // takeoff: JumpZ 600 cm/s, MOVE_Falling from the takeoff tick (DoJump sets velocity + mode in the same tick)
    let (mut h,mut vz)=(0f32,600f32);let mut rows=vec![];
    if print{println!("=== {label}: vel {vel} cm/s sprint {sprint} gravity {g} (flight {:.3} s, apex {:.3} s) ===",2.*600./g,600./g);
        println!(" tick  t(s)   vz   |J F LP| lowerIdle lowerMov full:Spr addIdle addMov | jump:state  Jmp  Loop Fall Land Idle | clips");}
    let mut n=0;
    loop{n+=1;
        let left=(vz+(vz*vz+2.*g*h.max(0.)).sqrt())/g;
        a.step(&PawnInput{falling:true,on_ground:false,velocity:[vel,0.,vz],fall_time_left:left,..ground},dt);
        let t=n as f32*dt;
        if print{
            let j=&a.jump;let fam=families(&a);let top:Vec<String>=fam.iter().take(5).map(|(n,w)|format!("{n}={w:.2}")).collect();
            println!("{n:5} {t:5.3} {vz:6.0} |{} {} {}| {:7.3}  {:7.3}  {:7.3}  {:6.3} {:6.3} | {:?} {:5.2} {:5.2} {:5.2} {:5.2} {:5.2} | {}",
                a.vars.b_is_jumping as u8,a.vars.b_is_falling as u8,a.vars.b_landing_predicted as u8,
                a.lower_weight(lower::IDLES),a.lower_weight(lower::MOVEMENT),a.full_weight(fullbody::SPRINTING),a.additive_weight(additive::IDLE),a.additive_weight(additive::MOVEMENT),
                j.state,j.state_weight(JumpState::Jump),j.state_weight(JumpState::Loop),j.state_weight(JumpState::Fall),j.state_weight(JumpState::Land),j.state_weight(JumpState::Idle),top.join(" "));
        }
        rows.push(Row{t,vz});
        h+=vz*dt-0.5*g*dt*dt;vz-=g*dt;
        if h<=0.||n>240{break;}
    }
    (rows,a)
}
#[test]fn jump_probe_all_speeds(){
    let g:f32=std::env::var("FN_PROBE_GRAVITY").ok().and_then(|s|s.parse().ok()).unwrap_or(2800.);
    let smooth=|x:f32|{let x=x.clamp(0.,1.);x*x*(3.-2.*x)};
    for (label,vel,sprint) in [("standing",0.,false),("walking",150.,false),("jogging",365.,false),("sprint",600.,true)]{
        let (rows,_)=run(label,vel,sprint,g,true);
        // first airborne tick: the rules of JUMP_AIR.md section 1 (exe 0x40e8a0 split, AnimBP transitions)
        let (_,first)=run(label,vel,sprint,g,false);let _=first;
        let dt=1./60.;let acc=if vel>0.{MAX_ACCELERATION_CM}else{0.};
        let ground=PawnInput{sprinting:sprint,velocity:[vel,0.,0.],accel:[acc,0.],..PawnInput::melee()};
        let mut a=AnimState::new();for _ in 0..120{a.step(&ground,dt);}
        a.step(&PawnInput{falling:true,on_ground:false,velocity:[vel,0.,600.],fall_time_left:2.*600./g,..ground},dt);
        assert!(a.vars.b_is_jumping&&!a.vars.b_is_falling,"{label}: rising = bIsJumping (exe 0x40e8a0)");
        if vel>=6.{ // jog / sprint: Movement -> Idles and MovementAdditive -> IdleAdditive fire on the takeoff tick
            assert_eq!(a.lower_state(),lower::IDLES,"{label}");assert_eq!(a.additive_state(),additive::IDLE,"{label}");
            assert!((a.lower_weight(lower::MOVEMENT)-(1.-smooth(dt/0.4))).abs()<2e-3,"{label}");
        }
        if sprint{assert_eq!(a.full_state(),fullbody::SPLIT);assert!((a.full_weight(fullbody::SPRINTING)-(1.-smooth(dt/0.3))).abs()<2e-3);}
        assert!(rows.len()>10);
    }
}
#[test]fn jump_probe_flight_980(){
    // same probe with the engine-default gravity (flight 1.22 s): the fade-outs (0.4 s / 0.3 s) finish well inside the flight
    let (rows,a)=run("jogging g=980",365.,false,980.,true);
    assert!(rows.last().unwrap().t>1.1);let _=a;
}
/// UE 4.16 transition BlendProfile weight (exe 0x1fe9e20 + NormalizeDataWeight 0x2004740): a s^2 / (a s^2 + 1 - a), not clamp(a s).
#[test]fn blend_profile_weight_is_the_normalised_scaled_pair(){
    assert_eq!(blend_profile_weight(0.,2.),0.);assert!((blend_profile_weight(1.,2.)-1.).abs()<1e-6);assert!((blend_profile_weight(0.37,1.)-0.37).abs()<1e-6);
    assert!((blend_profile_weight(0.5,2.)-0.8).abs()<1e-6,"FastFeet legs at the middle of the fade: 0.8, the old clamp(a*s) gave 1.0");
    assert!((blend_profile_weight(0.1,3.)-0.5).abs()<1e-6);
    assert!((blend_profile_weight(0.5,0.8)-0.390_243_9).abs()<1e-5,"SlowTorso spine_03: slower than the plain alpha");
    for s in [0.5f32,0.6,0.8,2.,3.]{let mut last=0.;for i in 0..=100{let w=blend_profile_weight(i as f32/100.,s);assert!(w>=last-1e-6&&(0.0..=1.0).contains(&w));last=w;}}
}
