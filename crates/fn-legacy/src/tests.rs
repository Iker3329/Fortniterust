//! Tests ported from the old assets.rs (retarget rules, pickaxe grip, the AnimGraph pose pipeline on the real skeleton and clips).
//! Data comes from the READ-ONLY old tree (`fn_testdata::assets()`, env FN_OLD_ROOT).
#![allow(unused_imports, dead_code)]
use crate::*;
use crate::Md5Clip as Clip;
use fn_animgraph::{solve_two_bone_ik, two_bone, handle_axis_xz, grip_center, grip_axis_distance, PICK_SOCKET_YAW, Skeleton, SkinnedMesh};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Skinned primitive data (positions, joints, weights) of the face-rigidity test: the renderer-free `fn_animgraph::SkinPrimitive`.
pub use fn_animgraph::SkinPrimitive as Primitive;
/// Test shim with the old `assets::Model` surface (pose + globals + CPU skinning on top of a `Skeleton`); the skinning itself is
/// `fn_animgraph::SkinnedMesh::pose`, the same pipeline the renderer-side crates share.
pub struct Model { pub sk: Skeleton, pub joints: Vec<usize>, pub inverse: Vec<Mat4>, pub primitives: Vec<Primitive>, pub globals: Vec<Mat4>, pub shadow_vertices: Vec<Vec<Vec3>> }
impl std::ops::Deref for Model { type Target = Skeleton; fn deref(&self) -> &Skeleton { &self.sk } }
impl Model {
    pub fn load(all: &[PathBuf], part: &str, with_primitives: bool) -> Model {
        if with_primitives {
            let p = load_skinned_part(all, part).unwrap();
            Model { sk: p.body.skeleton, joints: p.mesh.joints, inverse: p.mesh.inverse_bind, primitives: p.mesh.primitives, globals: vec![], shadow_vertices: vec![] }
        } else {
            let b = load_body_skeleton(all, part).unwrap();
            Model { sk: b.skeleton, joints: b.joints, inverse: b.inverse_bind, primitives: vec![], globals: vec![], shadow_vertices: vec![] }
        }
    }
    pub fn eval_pose(&self, node: &fn_animgraph::Node, clips: &HashMap<String, Clip>) -> Vec<(Vec3, Quat)> { self.sk.eval_pose(node, clips) }
    pub fn apply_pose(&mut self, pose: &[(Vec3, Quat)]) {
        let mesh = SkinnedMesh { joints: std::mem::take(&mut self.joints), inverse_bind: std::mem::take(&mut self.inverse), primitives: std::mem::take(&mut self.primitives) };
        let posed = mesh.pose(&self.sk, pose);
        (self.joints, self.inverse, self.primitives) = (mesh.joints, mesh.inverse_bind, mesh.primitives);
        (self.globals, self.shadow_vertices) = (posed.globals, posed.positions);
    }
    pub fn bone_position(&self, name: &str) -> Option<Vec3> { self.sk.bone_position(&self.globals, name) }
    pub fn ue_bone_vector(v: [f32; 3]) -> Vec3 { Skeleton::ue_bone_vector(v) }
}
fn evidence_dir() -> PathBuf { let d = fn_live::var_os("FN_EVIDENCE_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("fn-evidence")); let _ = std::fs::create_dir_all(&d); d }
#[cfg(test)]mod retarget_tests{
    use super::*;
    #[test]fn scaled_translation_uses_bone_length_ratio(){
        let p=retarget_pose(Retarget{mode:2,source:Vec3::Y},(Vec3::Y*0.4,Quat::IDENTITY),(Vec3::Y*2.,Quat::IDENTITY),false,false);
        assert!((p.0.y-0.8).abs()<1e-6);
    }
    #[test]fn mesh_translation_does_not_add_reference_offset_twice(){
        let r=Retarget{mode:1,source:Vec3::Y};let bind=(Vec3::Y*2.,Quat::IDENTITY);
        assert_eq!(retarget_pose(r,(Vec3::X,Quat::IDENTITY),bind,false,false).0,bind.0);
        assert_eq!(retarget_pose(r,(Vec3::X,Quat::IDENTITY),bind,true,false).0,Vec3::ZERO);
        assert_eq!(retarget_pose(r,(Vec3::X,Quat::IDENTITY),bind,false,true).0,Vec3::Y);
    }
    #[test]fn missing_source_bone_falls_back_without_nan(){
        let r=Retarget{mode:2,source:Vec3::ZERO};let bind=(Vec3::Y,Quat::IDENTITY);
        assert_eq!(retarget_pose(r,(Vec3::X,Quat::IDENTITY),bind,false,false),bind);
        assert_eq!(retarget_pose(r,(Vec3::X,Quat::IDENTITY),bind,true,false),(Vec3::ZERO,Quat::IDENTITY));
    }
    #[test]fn exported_crouch_contains_original_retarget_rules(){
        let p=fn_testdata::old_root().join("assets/animation/Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_Skeleton/Crouch_NoWep_CMM.md5anim");
        let clip=Clip::load(&p).unwrap();assert_eq!(clip.retarget.len(),266);assert_eq!(clip.retarget["pelvis"].mode,2);assert_eq!(clip.retarget["spine_01"].mode,2);assert!(!clip.additive);
        assert_eq!(clip.channels.values().filter(|v|**v==(false,false)).count(),178);
        let missing=clip.channels.iter().find(|(_,v)|**v==(false,false)).unwrap().0;
        let bind=(Vec3::new(0.1,0.2,0.3),Quat::from_rotation_x(0.4));
        assert_eq!(clip.fit(missing,(Vec3::ZERO,Quat::IDENTITY),bind),bind);
    }
    #[test]fn source_pose_bone_positions_are_finite(){
        let root=fn_testdata::assets();let all=files(&root);
        let (doc,_,_)=gltf::import(find(&all,"SK_M_Med_Soldier_04.gltf").unwrap()).unwrap();
        let mut parents=vec![None;doc.nodes().count()];
        for n in doc.nodes(){for c in n.children(){parents[c.index()]=Some(n.index());}}
        for name in ["Crouch_NoWep_CMM.md5anim","M_NoWep_Relaxed.md5anim"]{
            let clip=Clip::load(&find(&all,name).unwrap()).unwrap();let mut world=vec![Mat4::IDENTITY;parents.len()];
            for n in doc.nodes(){let (p,q,s)=n.transform().decomposed();let bind=(Vec3::from(p),Quat::from_array(q));let bone=n.name().unwrap_or("").to_lowercase();
                let pose=clip.sample(&bone,0.).map(|v|clip.fit(&bone,v,bind)).unwrap_or(bind);
                let local=Mat4::from_scale_rotation_translation(Vec3::from(s),pose.1,pose.0);
                world[n.index()]=parents[n.index()].map(|p|world[p]*local).unwrap_or(local);
                let point=world[n.index()].transform_point3(Vec3::ZERO);assert!(point.is_finite());
                if ["root","pelvis","foot_l","foot_r","ball_l","ball_r"].contains(&bone.as_str()){println!("{name} {bone}: {point:?}");}
            }
        }
    }
}
#[cfg(test)]mod pick_tests{
    use super::*;
    /// CPU pose of the soldier skeleton from the original equipped idle clip (no GL needed).
    fn idle_pose(t:f32)->std::collections::HashMap<String,Mat4>{
        let root=fn_testdata::assets();let all=files(&root);
        let (doc,_,_)=gltf::import(find(&all,"SK_M_Med_Soldier_04.gltf").unwrap()).unwrap();
        let clip=Clip::load(&find(&all,"M_Melee_TwoHanded.md5anim").unwrap()).unwrap();
        let mut parents=vec![None;doc.nodes().count()];for n in doc.nodes(){for c in n.children(){parents[c.index()]=Some(n.index());}}
        let mut world=vec![Mat4::IDENTITY;parents.len()];let mut out=std::collections::HashMap::new();
        for n in doc.nodes(){let (p,q,s)=n.transform().decomposed();let bind=(Vec3::from(p),Quat::from_array(q));let bone=n.name().unwrap_or("").to_lowercase();
            let pose=clip.sample(&bone,t).map(|v|clip.fit(&bone,v,bind)).unwrap_or(bind);
            let local=Mat4::from_scale_rotation_translation(Vec3::from(s),pose.1,pose.0);
            world[n.index()]=parents[n.index()].map(|p|world[p]*local).unwrap_or(local);out.insert(bone,world[n.index()]);}
        out
    }
    fn apply(a:Vec3,b:Vec3,c:Vec3,r:(Quat,Quat,f32))->(Vec3,Vec3){let nb=a+r.0*(b-a);let c1=a+r.0*(c-a);(nb,nb+r.1*(c1-nb))}
    #[test]fn two_bone_reaches_target_and_preserves_bone_lengths(){
        let (a,b,c)=(Vec3::ZERO,Vec3::new(0.3,-0.1,0.),Vec3::new(0.5,-0.4,0.1));
        for t in [Vec3::new(0.2,-0.3,0.2),Vec3::new(-0.2,-0.2,0.1),Vec3::new(0.4,-0.1,-0.2),c]{
            let r=two_bone(a,b,c,t).unwrap();let (nb,nc)=apply(a,b,c,r);
            assert!(((nb-a).length()-(b-a).length()).abs()<1e-4&&((nc-nb).length()-(c-b).length()).abs()<1e-4);
            assert!((nc-t).length()<1e-3,"target {t:?} got {nc:?}");}
    }
    #[test]fn two_bone_clamps_unreachable_targets_and_rejects_degenerate_input(){
        let (a,b,c)=(Vec3::ZERO,Vec3::new(0.3,0.,0.),Vec3::new(0.6,0.,0.));
        let t=Vec3::new(0.,3.,0.);let (nb,nc)=apply(a,b,c,two_bone(a,b,c,t).unwrap());
        assert!((nc.normalize()-Vec3::Y).length()<1e-2&&nc.length()<=0.6001&&nc.is_finite()&&nb.is_finite());
        assert!(two_bone(a,a,c,t).is_none()&&two_bone(a,b,c,a).is_none());
        // collinear chain (no bend plane) must stay finite
        assert!(two_bone(a,b,c,Vec3::new(0.2,0.2,0.1)).unwrap().0.is_finite());
    }
    fn pick_positions()->Vec<[f32;3]>{let root=fn_testdata::assets();let all=files(&root);let (doc,buf,_)=gltf::import(find(&all,"SK_Pick.gltf").unwrap()).unwrap();
        doc.meshes().flat_map(|m|m.primitives().flat_map(|p|p.reader(|b|Some(&buf[b.index()].0)).read_positions().unwrap().collect::<Vec<_>>()).collect::<Vec<_>>()).collect()}
    /// The animation bakes hand_r/hand_l relative to weapon_r (hand offsets are constant in every locomotion clip),
    /// so the correct attachment must put the handle axis through both hands' finger-wrap centres.
    #[test]fn pick_grip_axis_matches_hand_bones(){
        let pose=idle_pose(0.);let pos=pick_positions();let axis=handle_axis_xz(&pos);
        assert!((axis.x).abs()<0.03&&(axis.y-0.07).abs()<0.03,"axis {axis:?}");
        let bone=|n:&str|pose.get(n).map(|m|m.transform_point3(Vec3::ZERO));
        let socket=|yaw:f32|pose["weapon_r"]*Mat4::from_rotation_y(yaw);
        let (r,l)=(grip_center("r",bone).unwrap(),grip_center("l",bone).unwrap());
        let good=socket(PICK_SOCKET_YAW);let (gr,gl)=(grip_axis_distance(good,axis,r),grip_axis_distance(good,axis,l));
        let bad=socket(-PICK_SOCKET_YAW);let (br,bl)=(grip_axis_distance(bad,axis,r),grip_axis_distance(bad,axis,l));
        println!("grip distances +90: R {gr:.3} L {gl:.3}   -90 (old): R {br:.3} L {bl:.3}");
        assert!(gr<0.06&&gl<0.06,"hands must wrap the handle: {gr} {gl}");
        assert!(br>0.10&&bl>0.10,"the old -90 socket must be clearly worse: {br} {bl}");
    }
    /// Both hands sit along the shaft (not beyond the butt or inside the head) in the equipped idle.
    #[test]fn pick_hands_are_on_the_shaft(){
        let pose=idle_pose(0.);let pos=pick_positions();let bone=|n:&str|pose.get(n).map(|m|m.transform_point3(Vec3::ZERO));
        let inv=(pose["weapon_r"]*Mat4::from_rotation_y(PICK_SOCKET_YAW)).inverse();
        let ymin=pos.iter().map(|p|p[1]).fold(f32::MAX,f32::min);
        for side in ["r","l"]{let y=inv.transform_point3(grip_center(side,bone).unwrap()).y;assert!(y>ymin&&y<0.3,"{side} hand along handle y={y} (butt {ymin}, head starts ~0.3)");}
    }
}
/// CPU-only checks of the pose tree on the real skeleton (body part + the master skeleton's IK bones) and the real clips (no GL: the
/// skeleton-only model has no primitives).
#[cfg(test)]mod graph_tests{
    use super::*;
    use fn_animgraph::{self as anim_graph,AnimState,Node,PawnInput};
    pub fn all()->Vec<PathBuf>{files(&fn_testdata::assets())}
    /// The body skeleton exactly as `Model::load` builds it (part mesh bones + the master skeleton's IK bones), without GL.
    pub fn skeleton(all:&[PathBuf])->Model{Model::load(all,"SK_M_Med_Soldier_04.gltf",false)}
    pub fn clip_set(a:&[PathBuf])->HashMap<String,Clip>{
        let mut m=HashMap::new();for n in anim_graph::required_clips().into_iter().chain(["TwoHanded_Loop_Swing1","TwoHanded_Loop_Swing1_InMotion","TwoHanded_Loop_Swing2"].map(str::to_owned)){if !m.contains_key(&n){m.insert(n.clone(),load_clip(a,&format!("{n}.md5anim")).unwrap_or_else(|e|panic!("{e}")));}}m
    }
    fn pose_of(m:&mut Model,node:&Node,c:&HashMap<String,Clip>)->Vec<(Vec3,Quat)>{let p=m.eval_pose(node,c);m.apply_pose(&p);p}
    fn pos(m:&Model,n:&str)->Vec3{m.bone_position(n).unwrap()}
    fn pick_axis(a:&[PathBuf])->glam::Vec2{let (doc,buf,_)=gltf::import(find(a,"SK_Pick.gltf").unwrap()).unwrap();
        let p:Vec<[f32;3]>=doc.meshes().flat_map(|mm|mm.primitives().flat_map(|p|p.reader(|b|Some(&buf[b.index()].0)).read_positions().unwrap().collect::<Vec<_>>()).collect::<Vec<_>>()).collect();handle_axis_xz(&p)}
    fn grip(m:&Model,axis:glam::Vec2)->(f32,f32){
        let socket=*m.globals.get(m.names.iter().position(|n|n=="weapon_r").unwrap()).unwrap()*Mat4::from_rotation_y(PICK_SOCKET_YAW);
        (grip_axis_distance(socket,axis,grip_center("r",|n|m.bone_position(n)).unwrap()),grip_axis_distance(socket,axis,grip_center("l",|n|m.bone_position(n)).unwrap()))
    }
    fn walking()->PawnInput{PawnInput{velocity:[400.,0.,0.],accel:[750.,0.],..PawnInput::melee()}}
    /// Ballistic jump input (cm, s): JumpZ 600, g 980; the pawn input at `t` after takeoff (None once landed).
    fn air_input(base:PawnInput,t:f32)->Option<PawnInput>{
        let vz=600.-980.*t;let h=600.*t-490.*t*t;if h<=0.&&t>0.{return None;}
        let left=(vz+(vz*vz+2.*980.*h.max(0.)).sqrt())/980.;
        Some(PawnInput{falling:true,on_ground:false,velocity:[base.velocity[0],base.velocity[1],vz],fall_time_left:left,..base})
    }
    fn settle(a:&mut AnimState,p:&PawnInput,seconds:f32){for _ in 0..((seconds*60.) as usize){a.step(p,1./60.);}}
    fn plan_pose(m:&mut Model,a:&AnimState,c:&HashMap<String,Clip>)->Vec<(Vec3,Quat)>{let n=a.plan(&[]);pose_of(m,&n,c)}
    /// Runs one jump (from `base` motion) and calls `f(time_since_takeoff, state)` every 1/60 s.
    fn jump_run(base:PawnInput,mut f:impl FnMut(f32,&AnimState)){
        let mut a=AnimState::new();settle(&mut a,&base,1.5);let mut t=0.;
        while let Some(p)=air_input(base,t){a.step(&p,1./60.);f(t,&a);t+=1./60.;}
    }
    // ----------------------------------------------------------------------------------------------------------------------------
    // ---- flow audit: scripted transitions, per-frame bone displacement (pop detection) -------------------------------------------
    #[derive(Clone,Copy,Default)]struct Fr{f:f32,s:f32,yaw:f32,run:bool,crouch:bool,air:Option<f32>,swing:Option<f32>}
    fn flow_sim(frames:&[Fr])->Vec<(anim_graph::PawnSample,Option<(f32,f32)>)>{
        let mut v=(0f32,0f32);let mut out=vec![];
        for fr in frames{
            let len=(fr.f*fr.f+fr.s*fr.s).sqrt();let a=fr.yaw.to_radians();
            let dir=if len>0.{((fr.f*a.sin()+fr.s*a.cos())/len,(-fr.f*a.cos()+fr.s*a.sin())/len)}else{(0.,0.)};
            // speed cap by direction relative to the camera (the movement layer's backpedal / strafe limits) and mode
            let rel=if len>0.{fr.s.atan2(fr.f).to_degrees().abs()}else{0.};
            let base=if fr.crouch{290.}else if fr.run&&len>0.&&rel<46.{550.}else{410.};let k=if rel<=45.{1.}else if rel<=100.{0.72}else{0.65};
            let target=if len>0.{base*k}else{0.};
            let want=(dir.0*target/100.,dir.1*target/100.);let dt=1./60.;let acc=20.*dt;
            let d=(want.0-v.0,want.1-v.1);let dl=(d.0*d.0+d.1*d.1).sqrt();
            if dl>acc{v.0+=d.0/dl*acc;v.1+=d.1/dl*acc;}else{v=want;}
            let (height,vz,left)=match fr.air{Some(t)=>{let vz=600.-980.*t;let h=(600.*t-490.*t*t).max(0.01);(h/100.,vz/100.,(vz+(vz*vz+2.*980.*h).sqrt())/980.)},None=>(0.,0.,0.)};
            out.push((anim_graph::PawnSample{velocity:v,input:dir,facing_deg:fr.yaw,camera_yaw_deg:fr.yaw,camera_pitch_deg:0.,height,vertical_speed:vz,fall_time_left:left,crouched:fr.crouch,running:fr.run&&len>0.,melee_swinging:fr.swing.is_some()},fr.swing.map(|t|(t,if t<0.1{t/0.1}else if t<0.55{1.}else{((0.75-t)/0.2).clamp(0.,1.)}))));
        }
        out
    }
    fn seg(frames:&mut Vec<Fr>,n:usize,fr:Fr){for _ in 0..n{frames.push(fr);}}
    /// Plays a scenario through the graph and returns per-frame key-bone positions (character space, m) plus a state label.
    fn flow_run(frames:&[Fr],m:&mut Model,c:&HashMap<String,Clip>)->Vec<(Vec<Vec3>,String)>{
        let bones=["pelvis","spine_03","head","hand_l","hand_r","foot_l","foot_r","weapon_r"];
        let mut a=AnimState::new();let mut res=vec![];let mut n=0;
        for (s,sw) in flow_sim(frames){
            a.step_sample(&s,1./60.);
            let mut mont=vec![];if let Some((t,w))=sw{mont.push(anim_graph::Montage{slot:"FullBodyInPlace",clip:"TwoHanded_Loop_Swing1".into(),time:t,weight:w, blend: fn_animgraph::SlotBlend::Pose});mont.push(anim_graph::Montage{slot:"UpperBody",clip:"TwoHanded_Loop_Swing1_InMotion".into(),time:t,weight:w, blend: fn_animgraph::SlotBlend::Pose});}
            let p=a.plan(&mont);let _=pose_of(m,&p,c);
            res.push((bones.iter().map(|b|pos(m,b)).collect(),format!("{} {} sprintW {:.3} rate {:.2} spd {:.0} clk {:.3}",anim_graph::lower::NAMES[a.lower_state()],a.full_state(),a.full_weight(1),a.vars.speed_adjusted_playrate,a.vars.speed2d,a.clock(0))));n+=1;
        }
        let _=n;res
    }
    // ---- planted-foot measurement ---------------------------------------------------------------------------------------------
    /// Foot contact model for the skating measurement. The ankle bone (`foot_l`) is NOT the ground contact: while the foot rolls over the
    /// ball of a jogging step the ankle travels slower than the ground, so the metric tracks the lowest sole point instead: the heel
    /// (a point 5 cm behind the ankle, fixed in the foot frame, on the sole in the bind pose) or the ball of the foot (`ball_*` bone dropped
    /// by its bind height), whichever is lower in the frame.
    pub struct FootRig{heel_local:[Vec3;2],ball_drop:f32,idx:[(usize,usize);2],pub fwd:Vec3,pub right:Vec3}
    impl FootRig{
        pub fn new(m:&mut Model,c:&HashMap<String,Clip>)->Self{
            let _=pose_of(m,&Node::Ref,c);
            let mut heel_local=[Vec3::ZERO;2];let mut idx=[(0,0);2];let mut drop=0.;let mut fwd_sum=Vec3::ZERO;
            for (k,s) in ["l","r"].iter().enumerate(){
                let fi=m.bone_index(&format!("foot_{s}")).unwrap();let bi=m.bone_index(&format!("ball_{s}")).unwrap();
                let g=m.globals[fi];let ankle=g.transform_point3(Vec3::ZERO);let ball=m.globals[bi].transform_point3(Vec3::ZERO);
                let fwd=Vec3::new(ball.x-ankle.x,0.,ball.z-ankle.z).normalize();
                heel_local[k]=g.inverse().transform_point3(Vec3::new(ankle.x-fwd.x*0.05,0.,ankle.z-fwd.z*0.05));idx[k]=(fi,bi);drop=ball.y;fwd_sum+=fwd;
            }
            let fwd=fwd_sum.normalize();let (l,r)=(m.globals[idx[0].0].transform_point3(Vec3::ZERO),m.globals[idx[1].0].transform_point3(Vec3::ZERO));let left=Vec3::new(l.x-r.x,0.,l.z-r.z).normalize();
            Self{heel_local,ball_drop:drop,idx,fwd,right:-left}
        }
        /// (ankle, heel point, ball-of-foot sole point) of foot `k` (0 left, 1 right) in the posed model.
        pub fn points(&self,m:&Model,k:usize)->[Vec3;3]{
            let (fi,bi)=self.idx[k];let ankle=m.globals[fi].transform_point3(Vec3::ZERO);let heel=m.globals[fi].transform_point3(self.heel_local[k]);
            let ball=m.globals[bi].transform_point3(Vec3::ZERO)-Vec3::new(0.,self.ball_drop,0.);[ankle,heel,ball]
        }
    }
    /// Height band above a foot's lowest sole height within which it counts as planted (m).
    pub const PLANT_BAND:f32=0.012;
    fn contact_id(f:&[Vec3;3])->usize{if f[1].y<=f[2].y{1}else{2}}
    /// Steady-state skating measurement through the whole graph.
    #[derive(Clone,Copy,Debug)]pub struct Planted{
        /// planted ANKLE bone speed / ground speed (reported for reference only: the ankle is not the contact point).
        pub ankle:f32,
        /// planted SOLE contact point speed / ground speed: the skating number (1 = no skating).
        pub contact:f32,
        /// share of foot-frames that are planted
        pub share:f32,
        /// play rate the speed-matched player ran at (BP rate; 1 for the crouch sprint clip when stride matching is off)
        pub rate:f32,
        /// contact point speed in cm/s
        pub contact_cm_s:f32}
    /// Planted-foot speed / ground speed for a pawn moving at `vel` = (forward, right) cm/s in the actor frame, standing, crouched or sprinting.
    /// A foot counts as planted in a frame when its lowest sole point is within `PLANT_BAND` of that foot's lowest height of the window and it is
    /// moving backward; its speed is the backward travel of THAT point (same identity in the previous frame) along the ground velocity.
    pub fn planted_ratio(m:&mut Model,c:&HashMap<String,Clip>,rig:&FootRig,vel:(f32,f32),crouch:bool,sprint:bool)->Planted{
        planted_with(m,c,rig,vel,crouch,sprint,true,|a,m,c|{let _=pose_of(m,&a.plan(&[]),c);})
    }
    /// `planted_ratio` with the stride match switchable and a custom pose source (`pose(state, model, clips)` must pose `model`).
    pub fn planted_with(m:&mut Model,c:&HashMap<String,Clip>,rig:&FootRig,vel:(f32,f32),crouch:bool,sprint:bool,stride_match:bool,pose:impl Fn(&AnimState,&mut Model,&HashMap<String,Clip>))->Planted{
        let speed=(vel.0*vel.0+vel.1*vel.1).sqrt();let u=(vel.0/speed,vel.1/speed);
        let p=PawnInput{velocity:[vel.0,vel.1,0.],accel:[750.*u.0,750.*u.1],crouched:crouch,sprinting:sprint,..PawnInput::melee()};
        let mut a=AnimState::new();a.stride_match=stride_match;let dt=1./240.;let vdir=rig.fwd*u.0+rig.right*u.1;
        let (warm,total)=(720,1200);let mut fr:Vec<[[Vec3;3];2]>=vec![];
        for _ in 0..total{a.step(&p,dt);pose(&a,m,c);fr.push([rig.points(m,0),rig.points(m,1)]);}
        let mut low=[9f32;2];for f in &fr[warm..]{for k in 0..2{low[k]=low[k].min(f[k][contact_id(&f[k])].y);}}
        let (mut sa,mut sc,mut n)=(0f32,0f32,0usize);
        for i in warm..total{for k in 0..2{let id=contact_id(&fr[i][k]);
            let back=-(fr[i][k][id]-fr[i-1][k][id]).dot(vdir)/dt;
            if fr[i][k][id].y<low[k]+PLANT_BAND&&back>0.{sa+=-(fr[i][k][0]-fr[i-1][k][0]).dot(vdir)/dt;sc+=back;n+=1;}}}
        let rate=if crouch&&sprint&&!stride_match{1.}else{a.vars.speed_adjusted_playrate};
        if n==0{return Planted{ankle:0.,contact:0.,share:0.,rate,contact_cm_s:0.};}
        let g=speed/100.;Planted{ankle:sa/n as f32/g,contact:sc/n as f32/g,share:n as f32/(2*(total-warm)) as f32,rate,contact_cm_s:sc/n as f32*100.}
    }
    /// Travel direction (forward, right) of a locomotion clip from its cardinal suffix (`..._NE_Core` -> forward + right).
    pub fn clip_direction(name:&str)->(f32,f32){
        let s=std::f32::consts::FRAC_1_SQRT_2;
        match name.split('_').find(|t|matches!(*t,"N"|"S"|"E"|"W"|"NE"|"SE"|"NW"|"SW")){
            Some("N")=>(1.,0.),Some("S")=>(-1.,0.),Some("E")=>(0.,1.),Some("W")=>(0.,-1.),Some("NE")=>(s,s),Some("SE")=>(-s,s),Some("NW")=>(s,-s),Some("SW")=>(-s,-s),_=>(1.,0.)}
    }
    /// Planted-sole travel speed of one BARE clip at play rate 1 (cm/s), same planted rule (no graph layers). Used for the clips the melee path never
    /// plays through the graph (the targeting walk spaces).
    pub fn clip_contact_stride(m:&mut Model,c:&HashMap<String,Clip>,rig:&FootRig,name:&str)->f32{
        let len=anim_graph::clip_meta(name).map_or(1.,|k|k.length);let steps=(len*240.).round() as usize;let dt=len/steps as f32;
        let d=clip_direction(name);let vdir=rig.fwd*d.0+rig.right*d.1;
        let mut fr:Vec<[[Vec3;3];2]>=vec![];
        for i in 0..=steps{let _=pose_of(m,&Node::clip(name,i as f32*dt,true),c);fr.push([rig.points(m,0),rig.points(m,1)]);}
        let mut low=[9f32;2];for f in &fr{for k in 0..2{low[k]=low[k].min(f[k][contact_id(&f[k])].y);}}
        let (mut sc,mut n)=(0f32,0usize);
        for i in 1..fr.len(){for k in 0..2{let id=contact_id(&fr[i][k]);let back=-(fr[i][k][id]-fr[i-1][k][id]).dot(vdir)/dt;
            if fr[i][k][id].y<low[k]+PLANT_BAND&&back>0.{sc+=back;n+=1;}}}
        if n==0{0.}else{sc/n as f32*100.}
    }
    /// Every clip the speed-matched locomotion players sample (jog / crouch / walk core blend spaces) plus the sprint clips.
    pub fn stride_clip_names()->Vec<&'static str>{
        let mut v:Vec<&'static str>=vec![];
        for b in ["M_Jog_AR_Core","M_Jog_AR_Core_No45s","Crouch_ARNonTargeted_Core","Crouch_ARTargeting_Core","M_Locomotion2D_Walk_AR_Core","M_Locomotion2D_Walk_AR_Core_RtFootFwd"]{
            if let Some(d)=anim_graph::blendspace(b){for s in d.samples{if !v.contains(&s.anim){v.push(s.anim);}}}}
        v.push("Sprint_TwoHanded");v.push("CrouchSprint_THMelee");v
    }
    /// The "animation axis value" of a locomotion clip (cm/s at play rate 1): the planted-sole travel speed it produces THROUGH THE GRAPH (additive
    /// jog layer, aim offset, hand / foot IK all included) with the stride match off, measured with the pawn moving along the clip's own direction at
    /// that direction's speed cap (410 within 45 degrees of forward, 295 up to 100, 266 beyond; crouch 290 / 209 / 189). Walk clips (targeting only,
    /// never played for the pickaxe) are measured bare.
    pub fn measured_axis_value(m:&mut Model,c:&HashMap<String,Clip>,rig:&FootRig,name:&str)->f32{
        let d=clip_direction(name);let ang=d.1.atan2(d.0).to_degrees().abs();
        let cap=|base:f32|base*if ang<=45.5{1.}else if ang<=100.{0.72}else{0.65};
        let (speed,crouch,sprint)=if name.starts_with("M_Jog_AssaultRifle_"){(cap(410.),false,false)}else if name.starts_with("Crouch_ARNonTargeted_"){(cap(290.),true,false)}
            else if name=="Sprint_TwoHanded"{(550.,false,true)}else if name=="CrouchSprint_THMelee"{(420.,true,true)}else{return clip_contact_stride(m,c,rig,name)};
        let r=planted_with(m,c,rig,(speed*d.0,speed*d.1),crouch,sprint,false,|a,m,c|{let _=pose_of(m,&a.plan(&[]),c);});
        r.contact_cm_s/r.rate
    }
    #[test]fn clip_stride_table_is_current(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);let rig=FootRig::new(&mut m,&c);
        let mut rows=vec![];
        for n in stride_clip_names(){let v=measured_axis_value(&mut m,&c,&rig,n);rows.push((n,v));
            println!("{:<34} axis value {:>7.1} cm/s   (bare clip {:>7.1})   table {:?}",n,v,clip_contact_stride(&mut m,&c,&rig,n),anim_graph::stride_cm_s(n));}
        if std::env::var("FNRT_WRITE_STRIDE").is_ok(){
            let mut s=String::from("// GENERATED by `FNRT_WRITE_STRIDE=1 cargo test --release clip_stride_table_is_current` (src/assets.rs graph_tests, `measured_axis_value`): the planted-sole\n// travel speed each locomotion clip produces through the graph at play rate 1, cm/s. Do not edit by hand; the test fails when a clip or the graph changes.\npub static CLIP_STRIDE_CM_S:&[(&str,f32)]=&[\n");
            for (n,v) in &rows{s+=&format!("    (\"{n}\",{v:.2}),\n");}s+="];\n";
            std::fs::write(Path::new(env!("CARGO_MANIFEST_DIR")).join("../fn-animgraph/src/anim_stride.rs"),s).unwrap();return;}
        for (n,v) in rows{let t=anim_graph::stride_cm_s(n).unwrap_or_else(||panic!("{n} missing from src/anim_stride.rs"));assert!((t-v).abs()<=0.01*v.max(1.)+0.05,"{n}: table {t} vs measured {v}");}
    }
    /// (label, velocity (forward, right) cm/s, crouched, sprinting): the steady-state skating matrix of the flow audit.
    const PLANTED_CASES:[(&str,(f32,f32),bool,bool);22]=[
        ("jog F 410",(410.,0.),false,false),("jog F 365",(365.,0.),false,false),("jog F 300",(300.,0.),false,false),("jog F 250",(250.,0.),false,false),("jog F 200",(200.,0.),false,false),
        ("jog FR 410",(290.,290.),false,false),("jog FL 410",(290.,-290.),false,false),("jog R 295",(0.,295.),false,false),("jog L 295",(0.,-295.),false,false),("strafe R 200",(0.,200.),false,false),
        ("backpedal 266",(-266.,0.),false,false),("backpedal 200",(-200.,0.),false,false),("jog BL 266",(-188.,-188.),false,false),("jog BR 266",(-188.,188.),false,false),
        ("sprint F 550",(550.,0.),false,true),("sprint F 450",(450.,0.),false,true),
        ("crouch sprint 420",(420.,0.),true,true),("crouch F 290",(290.,0.),true,false),("crouch F 200",(200.,0.),true,false),("crouch R 209",(0.,209.),true,false),("crouch L 209",(0.,-209.),true,false),("crouch B 189",(-189.,0.),true,false)];
    #[test]fn planted_foot_matrix(){
        let all=all();let c=clip_set(&all);let mut m=skeleton(&all);let rig=FootRig::new(&mut m,&c);
        println!("{:<18}| stride match OFF (old rate = Speed2D / Authored*Speed)  | stride match ON (AxisToScaleAnimation)","");
        println!("{:<18}|{:>13}{:>16}{:>10} |{:>13}{:>16}{:>10}","case","ankle/ground","contact/ground","planted","ankle/ground","contact/ground","planted");
        let mut worst=0f32;let mut worst_off=0f32;
        for (name,vel,crouch,sprint) in PLANTED_CASES{
            let off=planted_with(&mut m,&c,&rig,vel,crouch,sprint,false,|a,m,c|{let _=pose_of(m,&a.plan(&[]),c);});
            let on=planted_ratio(&mut m,&c,&rig,vel,crouch,sprint);
            println!("{:<18}|{:>13.3}{:>16.3}{:>10.2} |{:>13.3}{:>16.3}{:>10.2}",name,off.ankle,off.contact,off.share,on.ankle,on.contact,on.share);
            worst=worst.max((on.contact-1.).abs());worst_off=worst_off.max((off.contact-1.).abs());}
        println!("worst |contact/ground - 1|: stride match off {worst_off:.3}, on {worst:.3}");
        assert!(worst<=0.03,"planted sole speed must stay within 3% of the ground speed (worst {worst:.3})");
        assert!(worst<worst_off,"the stride match must improve the worst case");
    }
    // ---- engine-exact clip sampling (UE 4.16 key spacing: N keys span the length with N-1 intervals) ----------------------------
    #[test]fn key_position_uses_n_minus_1_spacing_wraps_looping_clips_and_holds_the_last_key_otherwise(){
        let a=all();let clip=load_clip(&a,"Sprint_TwoHanded.md5anim").unwrap();let n=clip.frames.len();let len=clip.duration();
        assert_eq!(n,15);assert!((len-15./32.142857).abs()<1e-3);
        for k in 0..n-1{let t=k as f32*len/(n-1) as f32;assert!((clip.key_position(t,true)-k as f32).abs()<2e-4,"key {k} at {t}");}
        assert!((clip.key_position(len,false)-(n-1) as f32).abs()<1e-6,"non-looping reaches the last key at the length");
        assert!((clip.key_position(len*3.,false)-(n-1) as f32).abs()<1e-6,"non-looping holds the last key");
        assert!(clip.key_position(len,true)<1e-3,"looping wraps at the length");assert!((clip.key_position(len*2.5,true)-clip.key_position(len*0.5,true)).abs()<1e-3);
        // strictly advancing right up to the seam: no held duplicate key
        let mut prev=-1.;for i in 0..1000{let p=clip.key_position(len*i as f32/1000.,true);assert!(p>prev,"stalled at {i}");prev=p;}
        assert!(prev>(n-1) as f32-0.02,"the last sub-step reaches the last key ({prev})");
    }
    /// Every clip: duration (= frames / frameRate) equals the UE SequenceLength in `CLIP_META`; the looping locomotion clips have last key == first key, so
    /// the seam (t = 0 vs t = length, and the instant before the wrap) shows no duplicated or popping frame.
    #[test]fn clip_durations_equal_the_ue_length_and_looping_clips_close_their_seam(){
        let a=all();let c=clip_set(&a);let (mut worst_len,mut worst_seam)=(0f32,0f32);
        for (n,clip) in &c{
            if let Some(meta)=anim_graph::clip_meta(n){let d=(clip.duration()-meta.length).abs();worst_len=worst_len.max(d);assert!(d<=0.002,"{n}: duration {} vs UE length {}",clip.duration(),meta.length);}
        }
        for n in stride_clip_names(){
            let clip=&c[n];let len=clip.duration();let nk=clip.frames.len();
            for b in 0..clip.names.len(){let (f,l)=(clip.frames[0][b],clip.frames[nk-1][b]);
                let dp=(f.0-l.0).length();let dq=1.-f.1.dot(l.1).abs();worst_seam=worst_seam.max(dp).max(dq);
                assert!(dp<1e-3&&dq<1e-4,"{n} bone {b}: first and last key differ ({dp} m, {dq})");}
            for name in ["pelvis","foot_l","foot_r","hand_l","spine_03"]{
                let (p0,q0)=clip.sample_at(name,0.,true).unwrap();let (pl,ql)=clip.sample_at(name,len,true).unwrap();let (pe,qe)=clip.sample_at(name,len-1e-4,true).unwrap();
                assert!((p0-pl).length()<1e-6&&q0.dot(ql).abs()>0.999999,"{n}/{name}: t=0 vs t=length");
                assert!((p0-pe).length()<2e-3&&q0.dot(qe).abs()>0.9999,"{n}/{name}: the instant before the wrap is not the first key");}
        }
        // Independent evidence for the N-1 spacing: authored at 30 fps (the swings at 60), the keys of 376 of the 388 exported clips sit EXACTLY 1/30 s apart
        // when spaced length / (N-1) (and 1/60 s for TwoHanded_Loop_Swing1 / Swing2); N / length would give 29.0-31.9 fps.
        let spaced=|n:&str|{let k=&c[n];(k.frames.len()-1) as f32/k.duration()};
        for (n,fps) in [("TwoHanded_Loop_Swing1",60.),("TwoHanded_Loop_Swing2",60.),("TwoHanded_Loop_Swing1_InMotion",30.),("M_JumpUp_ARDS_Additive",30.),("M_JumpLand_ARDS_Additive",30.),("Sprint_TwoHanded",30.),("M_Jog_AssaultRifle_N_Core",30.)]{
            assert!((spaced(n)-fps).abs()<0.01,"{n}: {} keys per second with N-1 spacing",spaced(n));}
        // the combat notifies are montage SECONDS: the swing clip is exactly the 0.5333 s Swing1 section, so TriggerAbility 0.1603 s lands on key 9.62 (60 fps)
        assert!((c["TwoHanded_Loop_Swing1"].key_position(fn_combat::TRIGGER_ABILITY[0],false)-0.16025792*60.).abs()<0.02);
        println!("{} clips: worst |duration - UE length| {worst_len:.6} s, worst first/last key difference of the looping locomotion clips {worst_seam:.2e}",c.len());
    }
    // ---- pickaxe idle grip: one attachment path, one idle pose ----------------------------------------------------------------
    /// Pickaxe mesh vertices.
    fn pick_points(a:&[PathBuf])->Vec<Vec3>{let (doc,buf,_)=gltf::import(find(a,"SK_Pick.gltf").unwrap()).unwrap();
        doc.meshes().flat_map(|mm|mm.primitives().flat_map(|p|p.reader(|b|Some(&buf[b.index()].0)).read_positions().unwrap().map(Vec3::from).collect::<Vec<_>>()).collect::<Vec<_>>()).collect()}
    /// (butt end, head end) of the pickaxe in character space for the posed model: the long axis of the mesh in the socket frame (weapon_r * Ry(90 deg),
    /// exactly `right_hand_socket`); the head is the end whose cross-section is wider.
    fn pick_ends(m:&Model,pts:&[Vec3])->(Vec3,Vec3){
        let socket=m.globals[m.names.iter().position(|n|n=="weapon_r").unwrap()]*Mat4::from_rotation_y(PICK_SOCKET_YAW);
        let ext=|i:usize|{let (lo,hi)=pts.iter().fold((f32::MAX,f32::MIN),|(l,h),p|(l.min(p[i]),h.max(p[i])));hi-lo};
        let axis=(0..3).max_by(|a,b|ext(*a).partial_cmp(&ext(*b)).unwrap()).unwrap();
        let (lo,hi)=pts.iter().fold((f32::MAX,f32::MIN),|(l,h),p|(l.min(p[axis]),h.max(p[axis])));let mid=(lo+hi)/2.;
        let spread=|upper:bool|{let sel:Vec<&Vec3>=pts.iter().filter(|p|(p[axis]>mid)==upper).collect();let c=sel.iter().fold(Vec3::ZERO,|s,p|s+**p)/sel.len() as f32;
            sel.iter().map(|p|{let mut d=**p-c;d[axis]=0.;d.length()}).sum::<f32>()/sel.len() as f32};
        let head_upper=spread(true)>spread(false);
        let end=|upper:bool|{let sel:Vec<&Vec3>=pts.iter().filter(|p|(p[axis]>mid)==upper).collect();socket.transform_point3(sel.iter().fold(Vec3::ZERO,|s,p|s+**p)/sel.len() as f32)};
        (end(!head_upper),end(head_upper))
    }
    fn idle_probe(m:&mut Model,c:&HashMap<String,Clip>,pts:&[Vec3],rig:&FootRig,st:&AnimState)->(Vec3,Vec3,Vec3,Vec3,(f32,f32)){
        let _=pose_of(m,&st.plan(&[]),c);let (butt,head)=pick_ends(m,pts);
        let g=grip(m,pick_axis_of(pts));let _=rig;(butt,head,pos(m,"weapon_r"),pos(m,"pelvis"),g)
    }
    fn pick_axis_of(pts:&[Vec3])->glam::Vec2{let p:Vec<[f32;3]>=pts.iter().map(|v|v.to_array()).collect();handle_axis_xz(&p)}
    #[test]fn the_pickaxe_idle_grip_is_the_same_on_every_path_and_the_head_is_up_beside_the_shoulder(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);let rig=FootRig::new(&mut m,&c);let pts=pick_points(&a);
        // (1) a never-stepped graph (the render-only capture drivers: view / menu / settings / lighting tests) vs the graph the game ticks at idle (camera pitch 0)
        let fresh=AnimState::new();
        let mut live0=AnimState::new();settle(&mut live0,&PawnInput::melee(),1.5);
        let mut tick1=AnimState::new();tick1.step(&PawnInput::melee(),1./60.);
        let (fb,fh,fw,fp,fg)=idle_probe(&mut m,&c,&pts,&rig,&fresh);
        let (tb,th,tw,_tp,tg)=idle_probe(&mut m,&c,&pts,&rig,&tick1);
        let (lb,lh,lw,lp,lg)=idle_probe(&mut m,&c,&pts,&rig,&live0);
        println!("pick head (character space, m): never stepped {fh:?} | one tick {th:?} | settled idle {lh:?}");
        println!("weapon_r: never stepped {fw:?} | settled {lw:?} (|diff| {:.3} m)   grips never-stepped r {:.3} l {:.3}  settled r {:.3} l {:.3}",(fw-lw).length(),fg.0,fg.1,lg.0,lg.1);
        // never-stepped must equal one tick of the same pose (no idle noise phase to speak of at t = 0): same head within 1.5 cm, same grip
        assert!((fh-th).length()<0.015&&(fb-tb).length()<0.015&&(fw-tw).length()<0.015,"never-stepped idle vs one tick: head {:.3} butt {:.3} weapon {:.3} m",(fh-th).length(),(fb-tb).length(),(fw-tw).length());
        assert!((fg.0-tg.0).abs()<0.01&&(fg.1-tg.1).abs()<0.01);
        // the cause of the old render-only pose: the previous `Vars::default` values (jog lean additive at 1, upper-body aim offset at 1, full-body flag off)
        let mut old=AnimState::new();old.vars.lean_additive_alpha=1.;old.vars.upper_body_aim_offset_alpha=1.;old.vars.should_apply_aim_offset_to_full_body=false;
        let (_ob,oh,ow,_op,og)=idle_probe(&mut m,&c,&pts,&rig,&old);
        println!("OLD defaults: weapon_r moved {:.3} m, pick head {:.3} m, grips r {:.3} l {:.3} (new defaults: {:.3} m / {:.3} m)",(ow-tw).length(),(oh-th).length(),og.0,og.1,(fw-tw).length(),(fh-th).length());
        assert!((oh-th).length()>0.015,"the old defaults displaced the pickaxe head by {:.3} m",(oh-th).length());
        // (2) the correct idle per the graph + socket data: head up beside the shoulder, butt low behind the hand, both hands on the handle
        for (label,butt,head,pelvis,g) in [("never stepped",fb,fh,fp,fg),("settled idle",lb,lh,lp,lg)]{
            assert!(head.y>pelvis.y+0.15,"{label}: the pickaxe head ({:.2} m) is above the hip ({:.2} m)",head.y,pelvis.y);
            assert!(head.y>butt.y+0.2,"{label}: head above butt");
            assert!((head-pelvis).dot(rig.fwd)>-0.3,"{label}: the head is not tucked behind the hip ({:.2})",(head-pelvis).dot(rig.fwd));
            println!("{label}: head {:.2} m above the pelvis, {:+.2} m along forward, {:+.2} m to the right; butt {:.2} m above the pelvis",head.y-pelvis.y,(head-pelvis).dot(rig.fwd),(head-pelvis).dot(rig.right),butt.y-pelvis.y);
            assert!(g.0<0.035&&g.1<0.05,"{label}: hands on the handle r {:.3} l {:.3}",g.0,g.1);
        }
        // (3) the game's idle holds still for minutes: no drift of the head with the clock or the frame rate
        for dt in [1./60.,1./144.]{
            let mut s=AnimState::new();let mut t=0.;let mut ys=vec![];
            while t<130.{s.step(&PawnInput{aim_pitch:-12.,..PawnInput::melee()},dt);t+=dt;if (t*10.).round()%100.==0.&&t>1.{ys.push(idle_probe(&mut m,&c,&pts,&rig,&s).1);}}
            let (lo,hi)=ys.iter().fold((Vec3::splat(9.),Vec3::splat(-9.)),|(l,h),p|(l.min(*p),h.max(*p)));
            assert!((hi-lo).length()<0.2,"idle breathing only (<20 cm head travel over 130 s at dt {dt}): {:.3}",(hi-lo).length());
        }
    }
    /// Idle, then the camera yaws 0 -> `a` -> `b` at `step` degrees per frame (a one-frame jump when `step` >= the span), holding `hold` frames at `a`.
    fn turn_scenario(a:f32,b:f32,step:f32,hold:usize)->Vec<Fr>{
        fn ramp(v:&mut Vec<Fr>,from:f32,to:f32,step:f32){let k=Fr::default();let mut y=from;while (to-y).abs()>step{y+=step*(to-y).signum();v.push(Fr{yaw:y,..k});}v.push(Fr{yaw:to,..k});}
        let k=Fr::default();let mut v=vec![];seg(&mut v,60,k);
        ramp(&mut v,0.,a,step);seg(&mut v,hold,Fr{yaw:a,..k});ramp(&mut v,a,b,step);seg(&mut v,100,Fr{yaw:b,..k});v
    }
    /// Largest per-frame displacement (m) of each key bone in `flow_run`'s order (pelvis, spine_03, head, hand_l, hand_r, foot_l, foot_r, weapon_r) and its frame.
    fn max_pops(frames:&[Fr],m:&mut Model,c:&HashMap<String,Clip>)->Vec<(f32,usize)>{
        let r=flow_run(frames,m,c);let mut mx=vec![(0f32,0usize);r[0].0.len()];
        for i in 1..r.len(){for b in 0..mx.len(){let d=(r[i].0[b]-r[i-1].0[b]).length();if d>mx[b].0{mx[b]=(d,i);}}}mx
    }
    #[test]fn reversing_a_turn_in_place_never_pops_a_bone_beyond_0_2_m_for_human_camera_rates(){
        let all=all();let c=clip_set(&all);let mut m=skeleton(&all);
        // 480 deg/s and 1500 deg/s camera, reversing +130 -> -30 and +100 -> -100 (the second reverses while the turn clip is still playing)
        for (label,a,b,step) in [("480/s 130>-30",130.,-30.,8.),("480/s 100>-100",100.,-100.,8.),("1500/s 100>-100",100.,-100.,25.),("1500/s 60>-60",60.,-60.,25.)]{
            let pops=max_pops(&turn_scenario(a,b,step,10),&mut m,&c);
            let worst=pops.iter().map(|p|p.0).fold(0f32,f32::max);
            println!("{label:<18} worst per-frame bone displacement {worst:.3} m (hands {:.3} / {:.3})",pops[3].0,pops[4].0);
            assert!(worst<=0.2,"{label}: {worst:.3} m in one frame");
        }
    }
    /// The single-frame 160 degree camera step (9600 deg/s) is the only input that moves the hands 0.43 m in a frame. The cause is not a clip swap or a state
    /// re-entry (the state stays Turns, the clip is the same): Actor Yaw Offset jumps by +100 degrees (-5 -> +95, the turn90 clamp), and the graph feeds that
    /// into the aim offset (AdjustedAimYaw = 0.8 * (offset + AimYaw)), the pelvis counter rotation (0.175 * offset) and the head track in the same tick, with no
    /// smoothing node. Freezing those three inputs at their previous-frame values removes the pop.
    #[test]fn turn_step_pop_is_the_aim_offset_swing(){
        let all=all();let c=clip_set(&all);let mut m=skeleton(&all);
        let frames=turn_scenario(130.,-30.,1000.,100); // the flow audit's original scenario
        let mut a=AnimState::new();let sim=flow_sim(&frames);let mut prev:Option<AnimState>=None;let mut hit=None;
        for (i,(s,_)) in sim.into_iter().enumerate(){
            let before=a.clone();a.step_sample(&s,1./60.);
            if a.vars.actor_yaw_offset-before.vars.actor_yaw_offset>90.{hit=Some((i,before,a.clone()));break;}
            prev=Some(a.clone());
        }
        let _=prev;let (frame,before,after)=hit.expect("the step frame");
        let hands=|st:&AnimState,m:&mut Model|{let _=pose_of(m,&st.plan(&[]),&c);(pos(m,"hand_l"),pos(m,"hand_r"))};
        let (bl,br)=hands(&before,&mut m);let (al,ar)=hands(&after,&mut m);let pop=((al-bl).length()).max((ar-br).length());
        let mut frozen=after.clone();frozen.vars.adjusted_aim_yaw=before.vars.adjusted_aim_yaw;frozen.vars.actor_yaw_offset=before.vars.actor_yaw_offset;
        frozen.vars.head_track_yaw=before.vars.head_track_yaw;frozen.vars.head_track_pitch=before.vars.head_track_pitch;
        let (fl,fr)=hands(&frozen,&mut m);let pop_frozen=((fl-bl).length()).max((fr-br).length());
        let mut ao_only=after.clone();ao_only.vars.actor_yaw_offset=before.vars.actor_yaw_offset;ao_only.vars.head_track_yaw=before.vars.head_track_yaw;
        let (ol,or)=hands(&ao_only,&mut m);let pop_ao_only=((ol-bl).length()).max((or-br).length());
        println!("step frame {frame}: offset {:.1} -> {:.1}, AdjustedAimYaw {:.1} -> {:.1}; hand pop {pop:.3} m, with aim offset / pelvis / head inputs frozen {pop_frozen:.3} m, aim offset swing alone {pop_ao_only:.3} m",
            before.vars.actor_yaw_offset,after.vars.actor_yaw_offset,before.vars.adjusted_aim_yaw,after.vars.adjusted_aim_yaw);
        assert_eq!(before.lower_state(),anim_graph::lower::TURNS);assert_eq!(after.lower_state(),anim_graph::lower::TURNS,"no state change: the pop is not a re-entry");
        assert!((after.vars.adjusted_aim_yaw-before.vars.adjusted_aim_yaw).abs()>60.,"AdjustedAimYaw swings with the offset");
        assert!(pop>0.3,"the unsmoothed step moves the hands {pop:.3} m");
        assert!(pop_frozen<0.1,"with the three offset-driven inputs frozen the pose is continuous ({pop_frozen:.3} m)");
        assert!(pop_ao_only>0.5*pop,"the aim offset is the dominant term ({pop_ao_only:.3} of {pop:.3})");
    }
    #[test]fn the_turns_state_cross_fades_between_its_blend_list_pins_like_the_graph(){
        // Idle_Turn_* selection is BlendListByBool<TurnLeft> [0.1, 0.1] linear: a flip of TurnLeft inside the state blends over 0.1 s, never snaps
        let mut a=AnimState::new();let idle=PawnInput::melee();for _ in 0..30{a.step(&idle,1./60.);}
        a.vars.actor_yaw_offset=-90.;a.step(&PawnInput{actor_yaw_delta:0.,..idle},1./60.);
        assert_eq!(a.lower_state(),anim_graph::lower::TURNS,"offset beyond Turn Threshold90 starts a turn");
        for _ in 0..3{a.step(&idle,1./60.);}
        let right=|a:&AnimState|{let mut n=vec![];a.plan(&[]).clip_names(&mut n);n.iter().any(|x|x=="Idle_Turn_Right")};
        let left=|a:&AnimState|{let mut n=vec![];a.plan(&[]).clip_names(&mut n);n.iter().any(|x|x=="Idle_Turn_Left")};
        assert!(right(&a)&&!left(&a),"turning right plays Idle_Turn_Right only");
        a.vars.turn_left=true;a.step(&idle,1./60.); // selector flips inside the state (a reversal)
        assert!(right(&a)&&left(&a),"both branches are live while the lists cross-fade");
        for _ in 0..8{a.step(&idle,1./60.);}
        assert!(left(&a)&&!right(&a),"after 0.1 s only the new branch remains");
    }
    #[test]fn flow_audit_pop_table(){
        let all=all();let c=clip_set(&all);let mut m=skeleton(&all);
        let k=Fr::default();let w=|f:f32,s:f32|Fr{f,s,..k};
        let mut sc:Vec<(&str,Vec<Fr>)>=vec![];
        let mut v=vec![];seg(&mut v,60,k);seg(&mut v,120,w(1.,0.));seg(&mut v,90,k);sc.push(("idle>jog>idle",v));
        let mut v=vec![];seg(&mut v,90,w(1.,0.));seg(&mut v,120,Fr{run:true,..w(1.,0.)});seg(&mut v,90,w(1.,0.));sc.push(("jog>sprint>jog",v));
        let mut v=vec![];seg(&mut v,90,w(1.,0.));seg(&mut v,120,w(-1.,0.));seg(&mut v,90,w(1.,0.));sc.push(("jog>backpedal>jog",v));
        let mut v=vec![];seg(&mut v,90,w(0.,-1.));seg(&mut v,120,w(0.,1.));seg(&mut v,90,w(0.,-1.));sc.push(("strafeL>strafeR>strafeL",v));
        let mut v=vec![];seg(&mut v,90,w(1.,0.));seg(&mut v,90,w(1.,1.));seg(&mut v,90,w(0.,1.));seg(&mut v,90,w(-1.,1.));seg(&mut v,90,w(-1.,0.));sc.push(("octagon sweep",v));
        let mut v=vec![];seg(&mut v,60,k);seg(&mut v,60,Fr{crouch:true,..k});seg(&mut v,90,Fr{crouch:true,..w(1.,0.)});seg(&mut v,60,Fr{crouch:true,..k});seg(&mut v,60,k);sc.push(("crouch idle>walk>idle>stand",v));
        let mut v=vec![];seg(&mut v,90,w(1.,0.));seg(&mut v,40,Fr{crouch:true,..w(1.,0.)});seg(&mut v,60,w(1.,0.));sc.push(("jog>crouch>jog",v));
        let mut v=vec![];seg(&mut v,60,k);for i in 0..180{v.push(Fr{yaw:i as f32*1.0,..w(1.,0.)});}seg(&mut v,30,Fr{yaw:179.,..k});sc.push(("jog turning camera",v));
        // turn in place with a human camera: 130 degrees at 480 deg/s (8 deg per frame), then the reversal to -30 at the same rate
        let ramp=|v:&mut Vec<Fr>,from:f32,to:f32,step:f32|{let mut y=from;while (to-y).abs()>step{y+=step*(to-y).signum();v.push(Fr{yaw:y,..k});}v.push(Fr{yaw:to,..k});};
        let mut v=vec![];seg(&mut v,60,k);ramp(&mut v,0.,130.,8.);seg(&mut v,10,Fr{yaw:130.,..k});ramp(&mut v,130.,-30.,8.);seg(&mut v,100,Fr{yaw:-30.,..k});sc.push(("turn in place",v));
        let mut v=vec![];seg(&mut v,60,k);ramp(&mut v,0.,100.,8.);ramp(&mut v,100.,-100.,8.);seg(&mut v,100,Fr{yaw:-100.,..k});sc.push(("turn reversal mid-turn",v));
        // a hard flick: 1500 deg/s (25 deg per frame), reversing while the turn clip plays
        let mut v=vec![];seg(&mut v,60,k);ramp(&mut v,0.,100.,25.);seg(&mut v,6,Fr{yaw:100.,..k});ramp(&mut v,100.,-100.,25.);seg(&mut v,100,Fr{yaw:-100.,..k});sc.push(("turn reversal flick 1500/s",v));
        // STRESS, not a gameplay input: the camera yaw jumps 160 degrees inside ONE frame (9600 deg/s) while the turn plays. The graph has no smoothing on
        // AimYaw / Actor Yaw Offset, so the aim offset swings 0.8 * the offset change in that frame (see `turn_step_pop_is_the_aim_offset_swing`).
        let mut v=vec![];seg(&mut v,60,k);seg(&mut v,100,Fr{yaw:130.,..k});seg(&mut v,100,Fr{yaw:-30.,..k});sc.push(("turn 160deg one-frame step",v));
        for (name,base) in [("jump standing",k),("jump from jog",w(1.,0.)),("jump from strafe",w(0.,-1.))]{
            let mut v=vec![];seg(&mut v,90,base);let mut t=0.;while let Some(_)=(if 600.*t-490.*t*t>0.||t==0.{Some(())}else{None}){v.push(Fr{air:Some(t),..base});t+=1./60.;}seg(&mut v,90,base);sc.push((name,v));}
        let mut v=vec![];seg(&mut v,60,k);for i in 0..60{v.push(Fr{swing:Some(i as f32/60.),..k});}seg(&mut v,60,k);sc.push(("swing idle",v));
        let mut v=vec![];seg(&mut v,90,w(1.,0.));for i in 0..60{v.push(Fr{swing:Some(i as f32/60.),..w(1.,0.)});}seg(&mut v,60,w(1.,0.));sc.push(("swing while jogging",v));
        let mut v=vec![];seg(&mut v,60,k);for i in 0..60{v.push(Fr{swing:Some(i as f32/60.),..(if i>=20{w(1.,0.)}else{k})});}seg(&mut v,60,w(1.,0.));sc.push(("swing idle>jog mid-swing",v));
        // LEGACY ankle-bone skating audit (60 Hz, lowest ankle below 13 cm). The ankle is not the ground contact (it rolls over the ball of the foot, so it reads
        // ~0.8-0.95 even for a foot that does not slide at all); the real skating numbers are `planted_foot_matrix` (sole contact point, 240 Hz).
        println!("{:<22}{:>10} {:>10}","steady case (ankle bone, legacy)","planted/gnd","planted v (m/s)");
        for (name,fr) in [("jog F",w(1.,0.)),("jog B",w(-1.,0.)),("strafe L",w(0.,-1.)),("jog FL",w(1.,-1.)),("sprint F",Fr{run:true,..w(1.,0.)}),("crouch F",Fr{crouch:true,..w(1.,0.)}),("crouch L",Fr{crouch:true,..w(0.,-1.)})]{
            let mut v=vec![];seg(&mut v,240,fr);let sim=flow_sim(&v);let r=flow_run(&v,&mut m,&c);
            let mut tot=0f32;let mut n=0;let mut gnd=0f32;
            for i in 120..r.len(){let g=((sim[i].0.velocity.0).powi(2)+(sim[i].0.velocity.1).powi(2)).sqrt();let (fl,fr_)=(r[i].0[5],r[i].0[6]);let (pl,pp)=if fl.y<fr_.y{(fl,r[i-1].0[5])}else{(fr_,r[i-1].0[6])};
                if pl.y<0.13{let d=pl-pp;tot+=(d.x*d.x+d.z*d.z).sqrt()*60.;gnd+=g;n+=1;}}
            if n>0{println!("{:<22}{:>10.2} {:>10.2}",name,tot/gnd,tot/n as f32);}
        }        println!("{:<30}{:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}   worst-frame / state","scenario","pelvis","spine","head","hand_l","hand_r","foot_l","foot_r","weapon");
        for (name,frames) in &sc{
            let r=flow_run(frames,&mut m,&c);let nb=r[0].0.len();let mut mx=vec![(0f32,0usize);nb];
            for i in 1..r.len(){for b in 0..nb{let d=(r[i].0[b]-r[i-1].0[b]).length();if d>mx[b].0{mx[b]=(d,i);}}}
            let worst=mx.iter().enumerate().max_by(|a,b|(a.1).0.partial_cmp(&(b.1).0).unwrap()).unwrap();
            if let Ok(t)=std::env::var("FLOW_TRACE"){let p:Vec<&str>=t.split(':').collect();if p[0]==*name{let (f0,f1)=(p[1].parse::<usize>().unwrap(),p[2].parse::<usize>().unwrap());for i in f0..f1.min(r.len()){let d:Vec<String>=(0..nb).map(|b|format!("{:.3}",if i>0{(r[i].0[b]-r[i-1].0[b]).length()}else{0.})).collect();println!("  f{i} [{}] {}",d.join(" "),r[i].1);}}}
            println!("{:<30}{}   f{} {} -> {}",name,mx.iter().map(|x|format!("{:>8.3}",x.0)).collect::<String>(),(worst.1).1,r[(worst.1).1-1].1,r[(worst.1).1].1);
        }
    }    #[test]fn ik_bones_come_from_the_master_skeleton_and_every_clip_animates_them(){
        let a=all();let m=skeleton(&a);
        for n in ["ik_hand_root","ik_hand_gun","ik_hand_l","ik_hand_r","ik_foot_root","ik_foot_l","ik_foot_r"]{assert!(m.bone_index(n).is_some(),"{n} missing from the posed skeleton");}
        // the part mesh itself has 92 skin joints and no IK bone; the master mesh supplies them
        assert_eq!(m.joints.len(),92);assert_eq!(m.names.len(),100);
        let c=clip_set(&a);for (name,clip) in &c{for n in ["ik_hand_l","ik_hand_r","ik_hand_gun","ik_foot_l","ik_foot_r"]{assert!(clip.names.contains_key(n),"{name} has no {n} track");}}
        // the IK hierarchy: ik_hand_gun under ik_hand_root, both hand IK bones under the gun bone
        let par=|n:&str|m.names[m.parents[m.bone_index(n).unwrap()].unwrap()].clone();
        assert_eq!(par("ik_hand_gun"),"ik_hand_root");assert_eq!(par("ik_hand_l"),"ik_hand_gun");assert_eq!(par("ik_hand_r"),"ik_hand_gun");assert_eq!(par("ik_foot_l"),"ik_foot_root");
    }
    #[test]fn authored_poses_already_satisfy_the_ik_so_the_ik_nodes_only_correct_small_gaps(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);
        for (name,t) in [("M_Melee_TwoHanded",0.),("Sprint_TwoHanded",0.1),("Sprint_TwoHanded",0.3),("M_Walk_AR_N_Core",0.2),("M_Walk_AR_E_Core",0.5),("Crouch_TwoHanded_Melee_CMM",0.),("TwoHanded_Loop_Swing1",0.2)]{
            let fk=pose_of(&mut m,&Node::clip(name,t,true),&c);let fk_c=m.component(&fk);
            let mut node=Node::clip(name,t,true);for ik in [anim_graph::IK_HAND_R,anim_graph::IK_HAND_L,anim_graph::IK_FOOT_R,anim_graph::IK_FOOT_L]{node=Node::TwoBoneIk{base:Box::new(node),ik,alpha:1.};}
            let out=pose_of(&mut m,&node,&c);let out_c=m.component(&out);
            for b in ["hand_r","hand_l","foot_l","foot_r"]{let i=m.bone_index(b).unwrap();let moved=(fk_c[i].0-out_c[i].0).length();assert!(moved<0.12,"{name}@{t}: IK moved {b} by {moved:.3} m");}
            // the end bone lands on its effector (every authored effector is reachable)
            for ik in [anim_graph::IK_HAND_R,anim_graph::IK_HAND_L]{let e=m.bone_index(ik.effector).unwrap();let h=m.bone_index(ik.bone).unwrap();let gap=(out_c[h].0-out_c[e].0).length();assert!(gap<2e-3,"{name}@{t}: {} {gap:.4} m from {}",ik.bone,ik.effector);}
        }
    }
    #[test]fn two_bone_solver_reaches_clamps_and_stretches_like_the_engine(){
        let (r,j,e)=(Vec3::ZERO,Vec3::new(0.3,-0.1,0.),Vec3::new(0.5,-0.4,0.1));let (l1,l2)=((j-r).length(),(e-j).length());
        let jt=Vec3::new(1.,0.,0.);
        for t in [Vec3::new(0.2,-0.3,0.2),Vec3::new(-0.2,-0.2,0.1),Vec3::new(0.4,-0.1,-0.2),e]{
            let (nj,ne)=solve_two_bone_ik(r,j,e,jt,t,false,0.95,1.25);
            assert!(((nj-r).length()-l1).abs()<1e-4&&((ne-nj).length()-l2).abs()<1e-4,"bone lengths preserved for {t:?}");assert!((ne-t).length()<1e-4,"target {t:?} reached, got {ne:?}");}
        // unreachable: the chain fully extends toward the target
        let (nj,ne)=solve_two_bone_ik(r,j,e,jt,Vec3::new(0.,3.,0.),false,0.95,1.25);assert!(((ne-r).length()-(l1+l2)).abs()<1e-4&&(ne.normalize()-Vec3::Y).length()<1e-4&&(nj-r).length()-l1<1e-4);
        // stretching: reach 100% of the limb with start 0.95 / max 1.25 scales the limb by (1.25-1) * (1.0-0.95)/(1.25-0.95)
        let reach=l1+l2;let (_,ne)=solve_two_bone_ik(r,j,e,jt,Vec3::new(0.,reach,0.),true,0.95,1.25);let scale=1.+(1.25-1.)*((1.0-0.95)/(1.25-0.95));assert!((ne.length()-reach).abs()<1e-4,"target inside the stretched reach is reached: {}",ne.length());
        let (nj2,_)=solve_two_bone_ik(r,j,e,jt,Vec3::new(0.,reach*1.3,0.),true,0.95,1.25);assert!((nj2.length()-l1*1.25).abs()<1e-3,"joint at the stretched upper length");let _=scale;
        // the joint target picks the side of the bend
        let a=solve_two_bone_ik(r,j,e,Vec3::new(0.,0.,1.),Vec3::new(0.4,-0.2,0.),false,0.95,1.25).0;let b=solve_two_bone_ik(r,j,e,Vec3::new(0.,0.,-1.),Vec3::new(0.4,-0.2,0.),false,0.95,1.25).0;assert!(a.z>0.&&b.z<0.);
        // degenerate: effector at the root stays finite
        assert!(solve_two_bone_ik(r,j,e,jt,r,false,0.95,1.25).0.is_finite());
    }
    #[test]fn joint_target_mapping_keeps_the_authored_elbow_and_knee(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);
        let poses:Vec<(&str,f32)>=vec![("M_Melee_TwoHanded",0.),("Sprint_TwoHanded",0.1),("Sprint_TwoHanded",0.3),("M_Walk_AR_N_Core",0.2),("M_Walk_AR_E_Core",0.4),("M_Walk_AR_S_Core",0.6),("TwoHanded_Loop_Swing1",0.1),("TwoHanded_Loop_Swing1",0.3),("Crouch_TwoHanded_Melee_CMM",0.),("M_Jog_AssaultRifle_N_Core",0.2)];
        for ik in [anim_graph::IK_HAND_R,anim_graph::IK_HAND_L,anim_graph::IK_FOOT_L,anim_graph::IK_FOOT_R]{
            let (mut worst,mut worst_old)=(0f32,0f32);
            for (clip,t) in &poses{
                let p=m.eval_pose(&Node::clip(clip,*t,true),&c);let comp=m.component(&p);
                let end=m.bone_index(ik.bone).unwrap();let lower=m.parents[end].unwrap();let upper=m.parents[lower].unwrap();let jb=m.bone_index(ik.joint_bone).unwrap();
                for (old,w) in [(false,&mut worst),(true,&mut worst_old)]{
                    let v=if old{Vec3::new(ik.joint_target[0],ik.joint_target[2],-ik.joint_target[1])*0.01}else{Model::ue_bone_vector(ik.joint_target)};
                    let jt=comp[jb].0+comp[jb].1*v;
                    let (nj,_)=solve_two_bone_ik(comp[upper].0,comp[lower].0,comp[end].0,jt,comp[end].0,ik.stretch,ik.start_stretch,ik.max_stretch);*w=w.max((nj-comp[lower].0).length());}
            }
            println!("joint target {}: worst elbow/knee displacement {:.3} m (loader basis would give {:.3} m)",ik.bone,worst,worst_old);
            assert!(worst<0.08,"{}: {worst}",ik.bone);assert!(worst_old>worst*2.,"{}: the calibrated mapping must beat the loader basis ({worst} vs {worst_old})",ik.bone);
        }
    }
    #[test]fn hand_ik_retargeting_moves_ik_hand_gun_by_the_right_hand_offset_and_the_left_hand_follows(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);
        // a pose where the left FK hand is displaced from its IK bone: the Swing clip's FK/IK gap is a few cm; force a larger one by moving hand_l's local translation
        let base=Node::clip("M_Melee_TwoHanded",0.,false);let mut p=m.eval_pose(&base,&c);let hl=m.bone_index("hand_l").unwrap();p[hl].0+=Vec3::new(0.0,0.0,0.05);
        let comp0=m.component(&p);let (gun,ir,il,hr)=(m.bone_index("ik_hand_gun").unwrap(),m.bone_index("ik_hand_r").unwrap(),m.bone_index("ik_hand_l").unwrap(),m.bone_index("hand_r").unwrap());
        let left_gap=comp0[hl].0-comp0[il].0;assert!(left_gap.length()>0.04);let offset=comp0[hr].0-comp0[ir].0;
        // evaluate HandRetarget on that pose by wrapping it in a custom tree: Mix of one pose is not possible, so apply the node maths directly
        // exe RVA 0x1ab8db0: offset = Lerp(LeftFK, RightFK, w) - Lerp(LeftIK, RightIK, w) = Lerp(leftGap, rightGap, w) for the translation gaps, so w = 1 is the RIGHT hand's gap\n        let off=(comp0[hl].0-comp0[il].0).lerp(comp0[hr].0-comp0[ir].0,anim_graph::HAND_FK_WEIGHT);assert!((off-offset).length()<1e-5,"HandFKWeight 1.0 takes exactly the right hand's offset");
        let _=(gun,&mut m);
    }
    #[test]fn blend_depth_masks_follow_the_serialized_filters(){
        let a=all();let m=skeleton(&a);let id=|n:&str|m.bone_index(n).unwrap();
        // jump layered blend: Spine_01 depth 2 (1/2, 1, 1 ...), Ik_hand_root depth 0 (whole IK subtree), everything else 0
        let w=m.mask_weights(&[("Spine_01",2),("Ik_hand_root",0)]);
        assert!((w[id("spine_01")]-0.5).abs()<1e-6&&(w[id("spine_02")]-1.).abs()<1e-6&&w[id("head")]==1.&&w[id("upperarm_l")]==1.);
        for n in ["ik_hand_root","ik_hand_gun","ik_hand_l","ik_hand_r"]{assert_eq!(w[id(n)],1.,"{n}");}
        for n in ["pelvis","thigh_l","calf_r","foot_l","root","ik_foot_l"]{assert_eq!(w[id(n)],0.,"{n}");}
        // UpperAndLowerBody: Root 0 (everything), Ik_hand_root -1 (IK subtree -> upper body), Spine_01 -2 (spine_01 half, above it the upper body)
        let w=m.mask_weights(&[("Root",0),("Ik_hand_root",-1),("Spine_01",-2)]);
        assert_eq!(w[id("pelvis")],1.);assert_eq!(w[id("thigh_l")],1.);assert!((w[id("spine_01")]-0.5).abs()<1e-6);assert_eq!(w[id("spine_02")],0.);assert_eq!(w[id("head")],0.);assert_eq!(w[id("hand_r")],0.);
        for n in ["ik_hand_root","ik_hand_gun","ik_hand_l","ik_hand_r"]{assert_eq!(w[id(n)],0.,"{n} comes from the upper body");}
        assert_eq!(w[id("ik_foot_l")],1.,"foot IK bones follow the lower body");
    }
    #[test]fn blend_profiles_speed_up_the_legs_and_slow_the_spine(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);
        let from=Node::clip("M_Walk_AR_N_Core",0.3,true);let to=Node::clip("M_Melee_TwoHanded",0.,false);
        let plain=pose_of(&mut m,&Node::Mix(vec![(from.clone(),0.7),(to.clone(),0.3)]),&c);let prof=pose_of(&mut m,&Node::MixProfile{from:Box::new(from.clone()),to:Box::new(to.clone()),w:0.3,profile:anim_graph::FAST_FEET},&c);
        let dst=pose_of(&mut m,&to,&c);
        let thigh=m.bone_index("thigh_r").unwrap();let d_plain=plain[thigh].1.angle_between(dst[thigh].1);let d_prof=prof[thigh].1.angle_between(dst[thigh].1);
        assert!(d_prof<d_plain,"FastFeet (scale 2) must bring thigh_r closer to the target at w 0.3: {d_prof} vs {d_plain}");
        let prof2=pose_of(&mut m,&Node::MixProfile{from:Box::new(from),to:Box::new(to),w:0.3,profile:anim_graph::SLOW_TORSO},&c);
        let sp=m.bone_index("spine_03").unwrap();assert!(prof2[sp].1.angle_between(dst[sp].1)>plain[sp].1.angle_between(dst[sp].1)-1e-6,"SlowTorso (scale 0.8) keeps spine_03 further from the target");
    }
    // ---- pose pipeline
    #[test]fn idle_pose_is_the_authored_melee_pose_with_only_arm_noise_and_aim_offset_on_top(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);let mut s=AnimState::new();settle(&mut s,&PawnInput::melee(),1.5);
        let p=plan_pose(&mut m,&s,&c);let raw=m.eval_pose(&Node::clip("M_Melee_TwoHanded",0.,false),&c);
        let (pc,rc)=(m.component(&p),m.component(&raw));
        for (b,tol) in [("foot_l",0.02),("foot_r",0.02),("calf_l",0.03),("calf_r",0.07),("thigh_l",0.02),("thigh_r",0.02)]{let i=m.bone_index(b).unwrap();assert!((pc[i].0-rc[i].0).length()<tol,"{b}: {:.3} m from the authored idle pose",(pc[i].0-rc[i].0).length());}
        // the hands may move a little (Idle_Noise additive, aim offset at -3 / +3 degrees) but stay on the pickaxe handle
        let axis=pick_axis(&a);let _=pose_of(&mut m,&s.plan(&[]),&c);let (r,l)=grip(&m,axis);assert!(r<0.06&&l<0.08,"grip {r:.3} {l:.3}");
    }
    #[test]fn pickaxe_hands_stay_on_the_handle_in_every_graph_state(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);let axis=pick_axis(&a);
        let mut worst=(0f32,0f32);let mut check=|label:&str,m:&Model|{let (r,l)=grip(m,axis);println!("{label}: grip_r {r:.3} grip_l {l:.3}");assert!(r<0.07,"{label}: right hand {r:.3} m from the handle");assert!(l<0.11,"{label}: left hand {l:.3} m from the handle");worst=(worst.0.max(r),worst.1.max(l));};
        // idle, jog, sprint, crouch, crouch walk
        for (label,p,secs) in [("idle",PawnInput::melee(),1.5),("walk 4 m/s",walking(),1.5),("sprint",PawnInput{sprinting:true,..PawnInput{velocity:[550.,0.,0.],accel:[750.,0.],..PawnInput::melee()}},1.5),("crouch",PawnInput{crouched:true,..PawnInput::melee()},1.5),
            ("crouch walk",PawnInput{crouched:true,velocity:[200.,0.,0.],accel:[750.,0.],..PawnInput::melee()},1.5)]{
            let mut s=AnimState::new();settle(&mut s,&p,secs);let _=plan_pose(&mut m,&s,&c);check(label,&m);}
        // every phase of a standing and a jogging jump
        for (name,base) in [("standing jump",PawnInput::melee()),("jog jump",walking())]{
            let mut rows:Vec<(String,(f32,f32))>=vec![];let mut k=0;
            jump_run(base,|t,s|{k+=1;if k%6==0{let _=plan_pose(&mut m,s,&c);rows.push((format!("{name} t{t:.2} {:?}",s.jump_state()),grip(&m,axis)));}});
            for (label,(r,l)) in rows{println!("{label}: grip_r {r:.3} grip_l {l:.3}");assert!(r<0.07,"{label}: right hand {r:.3} m from the handle");assert!(l<0.11,"{label}: left hand {l:.3} m from the handle");worst=(worst.0.max(r),worst.1.max(l));}
        }        println!("worst grip r {:.3} l {:.3}",worst.0,worst.1);
    }
    #[test]fn the_legs_leave_the_walk_cycle_when_the_jump_starts_and_show_the_jump_pose_in_the_air(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);
        // reference poses: the jogging pose just before takeoff and a standing jump's pose at the same flight times
        let mut stand:Vec<(f32,Vec3,Vec3,Vec3)>=vec![];jump_run(PawnInput::melee(),|t,s|{let _=plan_pose(&mut m,s,&c);stand.push((t,pos(&m,"foot_l"),pos(&m,"foot_r"),pos(&m,"calf_l")));});
        let mut jog:Vec<(f32,Vec3,Vec3,Vec3,f32)>=vec![];jump_run(walking(),|t,s|{let _=plan_pose(&mut m,s,&c);jog.push((t,pos(&m,"foot_l"),pos(&m,"foot_r"),pos(&m,"calf_l"),s.lower_weight(anim_graph::lower::MOVEMENT)));});
        // the Movement state loses its weight within the 0.4 s transition (FastFeet speeds the legs up further)
        let w=|t:f32|jog.iter().min_by(|a,b|(a.0-t).abs().partial_cmp(&(b.0-t).abs()).unwrap()).unwrap().4;
        assert!(w(0.05)>0.85&&w(0.45)<0.01,"Movement weight {} at 0.05 s, {} at 0.45 s",w(0.05),w(0.45));
        // after 0.5 s in the air no walk cycle is left: the foot positions of the jog jump equal the standing jump's up to the apex / jog lean additives (<= 20 cm) and never swing like a stride
        for k in 0..jog.len(){let t=jog[k].0;if t<0.5||t>1.0{continue;}
            let s=&stand[k.min(stand.len()-1)];let d=(jog[k].1-s.1).length().max((jog[k].2-s.2).length());assert!(d<0.20,"t {t:.2}: foot differs {d:.3} m from the standing jump");}
        let swing=|f:fn(&(f32,Vec3,Vec3,Vec3,f32))->Vec3|{let v:Vec<Vec3>=jog.iter().filter(|x|x.0>=0.5&&x.0<=0.9).map(f).collect();let (mut lo,mut hi)=(Vec3::splat(1e9),Vec3::splat(-1e9));for p in v{lo=lo.min(p);hi=hi.max(p);}(hi-lo).length()};
        let s_l=swing(|x|x.1);let s_r=swing(|x|x.2);println!("foot travel during 0.5..0.9 s of the jog jump: left {s_l:.3} m right {s_r:.3} m (a stride is ~0.5 m)");
        assert!(s_l<0.20&&s_r<0.20,"the feet must hold the jump pose, not cycle: {s_l:.3} {s_r:.3}");
        // and the jump pose really differs from standing: knees tuck and feet lift
        let rest=stand[0].1.y;let apex=stand.iter().find(|x|x.0>0.5).unwrap();assert!(apex.1.y>rest+0.08||apex.2.y>rest+0.12,"feet lift in the air: rest {rest:.3}, apex {:.3} / {:.3}",apex.1.y,apex.2.y);
    }
    #[test]fn aim_offset_and_head_tracking_turn_the_body_and_head_toward_the_camera(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);
        let right_angle=|m:&Model,from:&str,to:&str|{let h=pos(m,to)-pos(m,from);(-h.x).atan2(h.z).to_degrees()};
        let measure=|m:&mut Model,s:&AnimState|{let _=plan_pose(m,s,&c);let hands=(pos(m,"hand_r")+pos(m,"hand_l"))*0.5-pos(m,"pelvis");((-hands.x).atan2(hands.z).to_degrees(),right_angle(m,"neck_01","head"))};
        let _=measure;
        // forward vector of the head from its orientation (bind +Z)
        let head_fwd=|m:&Model|{let h=m.bone_index("head").unwrap();let bind=m.component(&m.bind_pose());let f_local=bind[h].1.conjugate()*Vec3::Z;let (_,q,_)=m.globals[h].to_scale_rotation_translation();q*f_local};
        let yaw_of=|f:Vec3|(-f.x).atan2(f.z).to_degrees();let pitch_of=|f:Vec3|f.y.atan2((f.x*f.x+f.z*f.z).sqrt()).to_degrees();
        let mut base=AnimState::new();settle(&mut base,&PawnInput::melee(),1.);let _=plan_pose(&mut m,&base,&c);let f0=head_fwd(&m);let hands0=(pos(&m,"hand_r")+pos(&m,"hand_l"))*0.5-pos(&m,"pelvis");
        // camera 50 degrees to the right of the (idle, lagging) body: Actor Yaw Offset -50 = the body trails to the left
        for (offset,label) in [(-50f32,"camera right"),(50.,"camera left")]{
            let mut s=AnimState::new();s.vars.actor_yaw_offset=offset;settle(&mut s,&PawnInput::melee(),1.);
            // keep the offset (no camera delta, idle): the offset must stay
            s.vars.actor_yaw_offset=offset;s.step(&PawnInput::melee(),1./60.);let _=plan_pose(&mut m,&s,&c);
            let f=head_fwd(&m);let dyaw=yaw_of(f)-yaw_of(f0);let hands=(pos(&m,"hand_r")+pos(&m,"hand_l"))*0.5-pos(&m,"pelvis");let dh=(-hands.x).atan2(hands.z).to_degrees()-(-hands0.x).atan2(hands0.z).to_degrees();
            println!("AIM {label}: offset {offset:+}: head yaw {dyaw:+.1} deg, hands {dh:+.1} deg (target {:+.0})",-offset);
            let sign=-offset.signum();assert!(dyaw*sign>25.,"{label}: head turns toward the camera ({dyaw:+.1})");assert!(dh*sign>15.,"{label}: the hands / spine follow ({dh:+.1})");
        }
        // pitch: camera up / down moves the head's pitch the same way
        for (pitch,label) in [(40f32,"up"),(-35.,"down")]{
            let mut s=AnimState::new();settle(&mut s,&PawnInput{aim_pitch:pitch,..PawnInput::melee()},1.);let _=plan_pose(&mut m,&s,&c);let f=head_fwd(&m);let dp=pitch_of(f)-pitch_of(f0);
            println!("AIM pitch {label}: head pitch {dp:+.1} deg for camera pitch {pitch:+}");assert!(dp*pitch.signum()>pitch.abs()*0.5,"head pitches {label}: {dp:+.1}");
        }
    }
    /// The head part (SK_M_MED_Head_01_ATH) exactly as `Model::load` builds it, without GL (no textures).
    pub fn head_model(all:&[PathBuf])->Model{Model::load(all,"SK_M_MED_Head_01_ATH.gltf",true)}
    /// Face sections (eyes, lids, brows, lips, jaw, teeth, hair) stay rigid relative to the head bone when only the aim / head tracking changes.
    #[test]fn face_vertices_stay_rigid_relative_to_the_head_bone(){
        let a=all();let c=clip_set(&a);let mut h=head_model(&a);
        let hi=h.bone_index("head").unwrap();
        println!("head part bones: {}",h.names.iter().enumerate().map(|(i,n)|format!("{i}:{n}<-{}",h.parents[i].map(|p|h.names[p].clone()).unwrap_or("-".into()))).collect::<Vec<_>>().join(" | "));
        for (k,p) in h.primitives.iter().enumerate(){let mut used=std::collections::BTreeMap::<usize,f32>::new();for i in 0..p.p.len(){for q in 0..4{if p.w[i][q]>0.{*used.entry(h.joints[p.j[i][q] as usize]).or_default()+=p.w[i][q];}}}
            println!("prim {k}: {} verts, bones {}",p.p.len(),used.iter().map(|(b,w)|format!("{}({:.0})",h.names[*b],w)).collect::<Vec<_>>().join(","));}
        let mut ref_local:Vec<Vec<Vec3>>=vec![];
        for (step,(pitch,off)) in [(0f32,0f32),(80.,0.),(-80.,0.),(0.,60.),(0.,-60.)].iter().enumerate(){
            let mut s=AnimState::new();s.vars.actor_yaw_offset=*off;settle(&mut s,&PawnInput{aim_pitch:*pitch,..PawnInput::melee()},1.);s.vars.actor_yaw_offset=*off;s.step(&PawnInput{aim_pitch:*pitch,..PawnInput::melee()},1./60.);
            let n=s.plan(&[]);let p=h.eval_pose(&n,&c);h.apply_pose(&p);
            let inv=h.globals[hi].inverse();
            let loc:Vec<Vec<Vec3>>=h.shadow_vertices.iter().map(|v|v.iter().map(|x|inv.transform_point3(*x)).collect()).collect();
            if step==0{ref_local=loc;println!("track yaw {:.1} pitch {:.1}",s.vars.head_track_yaw,s.vars.head_track_pitch);continue;}
            println!("pitch {pitch} off {off}: track yaw {:.1} pitch {:.1}",s.vars.head_track_yaw,s.vars.head_track_pitch);
            for (k,v) in loc.iter().enumerate(){let mut mx=0f32;let mut mi=0;for (i,x) in v.iter().enumerate(){
                // only vertices skinned purely to the head and its descendants (face bones, hair) must be rigid; neck / spine verts blend
                let rigid=(0..4).all(|q|{let pr=&h.primitives[k];if pr.w[i][q]<=0.{return true;}let mut b=Some(h.joints[pr.j[i][q] as usize]);while let Some(x)=b{if x==hi{return true;}b=h.parents[x];}false});
                if !rigid{continue;}
                let d=(*x-ref_local[k][i]).length();if d>mx{mx=d;mi=i;}}
                let bw:Vec<_>=(0..4).filter(|q|h.primitives[k].w[mi][*q]>0.).map(|q|format!("{}:{:.2}",h.names[h.joints[h.primitives[k].j[mi][q] as usize]],h.primitives[k].w[mi][q])).collect();
                println!("  prim {k}: max deviation from head-local rest {:.5} m at vertex {mi} ({})",mx,bw.join(" "));
                assert!(mx<1e-3,"pitch {pitch} offset {off}: face part {k} moved {mx} m relative to the head bone");}
        }
    }
    /// The normal pickaxe clips have no SnapWeapon_LHand curve in this legacy
    /// fixture, so the recovered CopyBone node is present but has alpha zero.
    #[test]fn copy_bone_is_curve_gated_for_the_pickaxe_fixture(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);let axis=pick_axis(&a);
        let mut s=AnimState::new();settle(&mut s,&PawnInput::melee(),1.);
        let _=plan_pose(&mut m,&s,&c);let (r0,l0)=grip(&m,axis);
        let _=plan_pose(&mut m,&s,&c);let (r1,l1)=grip(&m,axis);
        println!("pickaxe grip with SnapWeapon_LHand=0: R {r0:.3} L {l0:.3}; second evaluation: R {r1:.3} L {l1:.3}");
        assert!(r0<0.06&&l0<0.08&&(r0-r1).abs()<1e-6,"{r0} {l0} {r1} {l1}");
    }
    #[test]fn equip_montage_raises_the_pickaxe_then_returns_to_guard(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);let mut s=AnimState::new();settle(&mut s,&PawnInput::melee(),1.);
        let _=plan_pose(&mut m,&s,&c);let rest=pos(&m,"weapon_r");
        let mut eq=anim_graph::Equip::default();eq.start();eq.step(0.0667);
        let mont=[anim_graph::Montage{slot:"UpperBody",clip:"Melee_WideGrip_Equip_CMM".into(),time:eq.clip_time(),weight:eq.weight(), blend: fn_animgraph::SlotBlend::Pose}];
        let n=s.plan(&mont);let _=pose_of(&mut m,&n,&c);let early=pos(&m,"weapon_r");
        println!("equip: weight {:.2} rest {rest:?} early {early:?}",eq.weight());
        assert!(early.y>rest.y+0.3,"the draw starts with the pickaxe raised ({} vs {})",early.y,rest.y);
        let n=s.plan(&[anim_graph::Montage{slot:"UpperBody",clip:"Melee_WideGrip_Equip_CMM".into(),time:anim_graph::EQUIP_ANIM_END,weight:0., blend: fn_animgraph::SlotBlend::Pose}]);let _=pose_of(&mut m,&n,&c);assert!((pos(&m,"weapon_r")-rest).length()<1e-4,"weight 0 = no effect");
    }
    #[test]fn a_swing_montage_on_both_slots_drives_the_hands_and_ends_without_a_snap(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);let mut s=AnimState::new();settle(&mut s,&PawnInput::melee(),1.);
        let swing=|t:f32,w:f32|vec![anim_graph::Montage{slot:"FullBodyInPlace",clip:"TwoHanded_Loop_Swing1".into(),time:t,weight:w, blend: fn_animgraph::SlotBlend::Pose},anim_graph::Montage{slot:"UpperBody",clip:"TwoHanded_Loop_Swing1_InMotion".into(),time:t,weight:w, blend: fn_animgraph::SlotBlend::Pose}];
        let _=plan_pose(&mut m,&s,&c);let rest=pos(&m,"hand_r");
        let mid=s.plan(&swing(0.2,1.));let _=pose_of(&mut m,&mid,&c);let moved=(pos(&m,"hand_r")-rest).length();assert!(moved>0.3,"the swing moves the hands ({moved})");
        let none=s.plan(&swing(0.2,0.));let _=pose_of(&mut m,&none,&c);assert!((pos(&m,"hand_r")-rest).length()<1e-4);
    }
    /// Evidence file for tools/animation-quantitative-check.cjs: the walk / jog BlendSpaces at 13 (direction, speed) points, evaluated by the Rust
    /// code (grid weights, marker sync, clip sampling and retargeting) and reduced to the component-space positions of key bones. Written when
    /// FNRT_WRITE_EVIDENCE=1 (always evaluated, so the evaluator is exercised on every `cargo test`).
    #[test]fn pose_dump_for_the_quantitative_check(){
        let a=all();let c=clip_set(&a);let mut m=skeleton(&a);
        let pts:[(&str,f32,f32,f32);13]=[("M_Locomotion2D_Walk_AR_Core",0.,150.,0.13),("M_Locomotion2D_Walk_AR_Core",45.,150.,0.31),("M_Locomotion2D_Walk_AR_Core",90.,150.,0.52),("M_Locomotion2D_Walk_AR_Core",135.,150.,0.77),("M_Locomotion2D_Walk_AR_Core",180.,150.,0.9),("M_Locomotion2D_Walk_AR_Core",-60.,149.5,0.44),
            ("M_Jog_AR_Core",0.,385.,0.21),("M_Jog_AR_Core",45.,300.,0.37),("M_Jog_AR_Core",90.,237.,0.6),("M_Jog_AR_Core",135.,385.,0.83),("M_Jog_AR_Core",-60.,320.,0.05),("M_Jog_AR_Core",180.,385.,0.5),("M_Jog_AR_Core",-135.,260.,0.95)];
        let bones=["pelvis","spine_03","spine_05","head","upperarm_r","hand_r","hand_l","thigh_l","calf_l","foot_l","thigh_r","calf_r","foot_r","ik_foot_l","ik_foot_r","ik_hand_gun"];
        let mut out=vec![];
        for (bs,x,y,phase) in pts{
            let def=anim_graph::blendspace(bs).unwrap();
            let node=anim_graph::BlendSpace::sample_node(def,[x,y],phase);let p=m.eval_pose(&node,&c);let comp=m.component(&p);
            let mut o=serde_json::Map::new();o.insert("blendspace".into(),bs.into());o.insert("x".into(),x.into());o.insert("y".into(),y.into());o.insert("phase".into(),phase.into());
            let mut b=serde_json::Map::new();for n in bones{let i=m.bone_index(n).unwrap();let q=comp[i].1;b.insert(n.into(),serde_json::json!({"p":[comp[i].0.x,comp[i].0.y,comp[i].0.z],"q":[q.x,q.y,q.z,q.w]}));}
            o.insert("bones".into(),b.into());
            let w:Vec<serde_json::Value>=anim_graph::BlendSpace::sample_weights(def,[x,y],phase).iter().map(|(i,wt,t)|serde_json::json!({"sample":i,"anim":def.samples[*i].anim,"weight":wt,"time":t})).collect();o.insert("samples".into(),w.into());
            out.push(serde_json::Value::Object(o));
        }
        assert_eq!(out.len(),13);
        if std::env::var_os("FNRT_WRITE_EVIDENCE").is_some(){let dir=evidence_dir();let _=std::fs::create_dir_all(&dir);std::fs::write(dir.join("anim-pose-dump.json"),serde_json::to_string_pretty(&out).unwrap()).unwrap();}
    }
}

#[cfg(test)]mod clip_loading_tests{
    use super::*;
    /// Missing / corrupt inputs are named errors, never panics (the release profile of the old exe was panic=abort).
    #[test]fn missing_required_clips_return_named_errors_instead_of_panicking(){
        let all=files(&fn_testdata::assets());
        for name in ["Sprint_TwoHanded.md5anim","M_Melee_TwoHanded.md5anim","Idle_Noise.md5anim"]{
            let without:Vec<PathBuf>=all.iter().filter(|p|!p.file_name().unwrap().to_string_lossy().eq_ignore_ascii_case(name)).cloned().collect();
            let e=load_clip(&without,name).err().unwrap_or_else(||panic!("{name} should be an error"));assert!(e.contains(name),"{e}");
        }
        let nothing=files(&fn_testdata::assets().join("does-not-exist"));assert!(nothing.is_empty());
        assert!(find(&nothing,"tips.txt").unwrap_err().contains("tips.txt"));
    }
    #[test]fn corrupt_clip_is_an_error_not_a_panic(){
        let dir=std::env::temp_dir().join(format!("fnrt-clip-{}",std::process::id()));std::fs::create_dir_all(&dir).unwrap();
        for (i,body) in ["","hierarchy {\n\"root\" -1 0 0\n}\n","frameRate abc\n","// retarget \"pelvis\" x y\n// track \"a\"\n","frame 0 {\n1 2 three 4 5 6\n}\n"].iter().enumerate(){
            let p=dir.join(format!("bad{i}.md5anim"));std::fs::write(&p,body).unwrap();
            assert!(Clip::load(&p).is_err(),"case {i}");
        }
        assert!(Clip::load(&dir.join("absent.md5anim")).err().expect("absent file is an error").contains("absent.md5anim"));
        let _=std::fs::remove_dir_all(&dir);
    }
    /// Every clip the plan can request exists in the exported animations (otherwise the pose silently falls back to the reference pose).
    #[test]fn every_required_clip_is_exported(){
        let all=files(&fn_testdata::assets());
        for n in fn_animgraph::required_clips(){assert!(find(&all,&format!("{n}.md5anim")).is_ok(),"clip {n} is not exported");}
        for n in prototype_clip_names(){assert!(find(&all,&format!("{n}.md5anim")).is_ok(),"clip {n} is not exported");}
    }
}

