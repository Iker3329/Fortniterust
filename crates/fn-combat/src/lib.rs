//! Pickaxe montage state, TriggerAbility notify timing, blocked-hit bounce branch and a hit trace
//! against placeholder harvestables. Durations/notify times come from reports/melee-montage.json.
//! Port note: the old `World::draw` (OpenGL immediate mode) is gone; a renderer draws `prop_mesh(kind)` itself and positions vertices with
//! `prop_vertex` (shake sway, see `prop_wobble`) and normals with `prop_normal`.
use glam::Vec3;
pub use fn_hull::{Hull,HullSet,TriMesh,trace_sphere_hulls};
/// Montage times of the TriggerAbility notifies, relative to the start of Swing1 / Swing2.
/// Serialized absolute montage times 0.1603 (Swing1) and 0.7348 (Swing2, section begins 0.5333).
pub const TRIGGER_ABILITY:[f32;2]=[0.16025792,0.73478013-0.5333];
/// DecisionWindow notify-state (next-swing input window): Swing1 0.1285..0.5332, Swing2 0.6805..1.1665 (absolute).
pub const DECISION_WINDOW:[(f32,f32);2]=[(0.12850618,0.5332),(0.68047649-0.5333,1.16653335-0.5333)];
/// Bounce1/Bounce2 section lengths (AnimEndTime of TwoHanded_Loop_Swing{1,2}_Bounce).
pub const BOUNCE_LENGTH:[f32;2]=[0.4,0.46666667];
/// Notifies emitted while stepping the montage.
#[derive(Debug,Clone,Copy,PartialEq)]pub enum Notify{TriggerAbility{combo:u8}}
/// The pawn's `FFortActiveMontageDecisionWindow` of the playing Swing section (`AFortPawn` +0xc48 array, 1.8 SDK struct): pushed by
/// `UFortAnimNotifyState_AbilityDecisionWindow::NotifyBegin` (0x1402a57e0 -> `AFortPawn::vfunc_333` 0x1407fc280), popped and decided by `NotifyEnd`
/// (0x1402a5920 -> `AFortPawn::vfunc_334` 0x1407faeb0).
#[derive(Default,Clone,Copy,Debug,PartialEq)]struct ActiveWindow{/** bReceivedPrimaryInput (+0x10) */received:bool}
#[derive(Default)]pub struct Swing{pub stage:u8,pub time:f32,held:bool,entering:bool,/** the DecisionWindow of this section is open */window:Option<ActiveWindow>,/** NotifyEnd chose PrimaryInput.NextSection (the other Swing section); else DefaultNextSection End1 / End2 */go:bool}
// Provisional graph gate: preserve lower-body crouch/jump/locomotion during attacks.
pub fn upper_body_attack(speed:f32,height:f32,crouched:bool)->bool{speed>0.1||height>0.||crouched}
impl Swing{
 pub fn duration(&self)->f32{match self.stage{1=>0.5333,2=>0.6333333,3=>0.5333333,4=>0.3,5=>BOUNCE_LENGTH[0],6=>BOUNCE_LENGTH[1],_=>0.}}
 /// Advances the montage. Returns the `TriggerAbility` notify when its serialized time is crossed
 /// during Swing1/Swing2 (reports/melee-montage.json notify times), so the caller can run the hit trace.
 pub fn step(&mut self,down:bool,dt:f32)->Option<Notify>{
  let press=down&&!self.held;
  if press&&self.stage==0{self.stage=1;self.time=0.;self.entering=true;self.window=None;self.go=false;}
  // AFortPawn::PawnStartFire (0x1407f8ac0) sets +0xc58 (fire held) and first offers the press to the open decision windows (vfunc_343 0x1407fc010, primary branch):
  // a window with a PrimaryInput NextSection records bReceivedPrimaryInput (+0x10) once and swallows the press. PawnStopFire (0x1407f8b80) only clears +0xc58.
  else if press{if let Some(w)=&mut self.window{w.received=true;}}
  self.held=down;
  let mut fired=None;
  if self.stage>0{let before=self.time;self.time+=dt.clamp(0.,0.1);
   if (1..=2).contains(&self.stage){let (a,b)=DECISION_WINDOW[self.stage as usize-1];
    if before<=a&&self.time>a&&self.window.is_none(){self.window=Some(ActiveWindow::default());}
    // NotifyEnd -> vfunc_334: PrimaryInput.InputType is not serialized in the pickaxe montage, so it is the ctor default 0 = WindowClickOrHold
    // (UFortAnimNotifyState_AbilityDecisionWindow ctor 0x1402823d0; 1.8 SDK EFortMontageInputType). The window takes PrimaryInput.NextSection (Swing2 / Swing1)
    // when the press was received inside it (ClickOrHold) OR the fire button is held at the window's end (+0xc58); otherwise DefaultNextSection (End1 / End2).
    // (The secondary-input branch needs the weapon's secondary ability handle +0x9fc and +0xc59; the port has no melee secondary input.)
    if before<=b&&self.time>b{if let Some(w)=self.window.take(){self.go=w.received||down;}}}
   if (1..=2).contains(&self.stage){let at=TRIGGER_ABILITY[self.stage as usize-1];if before<at&&self.time>=at{fired=Some(Notify::TriggerAbility{combo:self.stage});}}
   let duration=self.duration();if self.time>=duration{let from=self.stage;let go=self.go;self.go=false;self.window=None;self.time-=duration;self.entering=false;self.stage=match from{1=>if go{2}else{3},2=>if go{1}else{4},5|6=>if down{1}else{0},_=>0};if from>=5&&down{self.entering=true;self.time=0.;}}}
  fired
 }
 /// Blocked hit: MontageSectionIfBlocked jumps Swing1 -> Bounce1 and Swing2 -> Bounce2
 /// (GAB_Melee_ImpactCombo tags Gameplay.Melee.Hit1/Hit2). Returns true if a bounce started.
 pub fn blocked(&mut self)->bool{self.window=None;self.go=false;match self.stage{1=>{self.stage=5;self.time=0.;self.entering=false;true},2=>{self.stage=6;self.time=0.;self.entering=false;true},_=>false}}
 pub fn clip(&self,moving:bool)->Option<&'static str>{match (self.stage,moving){
  (1,false)=>Some("TwoHanded_Loop_Swing1"),(2,false)=>Some("TwoHanded_Loop_Swing2"),(3,false)=>Some("TwoHanded_Loop_Swing1_2Idle"),(4,false)=>Some("TwoHanded_Loop_Swing2_2Idle"),
  (1,true)=>Some("TwoHanded_Loop_Swing1_InMotion"),(2,true)=>Some("TwoHanded_Loop_Swing2_InMotion"),(3,true)=>Some("TwoHanded_Loop_Swing1_2Idle_InMotion"),(4,true)=>Some("TwoHanded_Loop_Swing2_2Idle_InMotion"),
  // Only one Bounce clip per combo exists in the montage (the moving variant reuses the same UpperBody clip).
  (5,_)=>Some("TwoHanded_Loop_Swing1_Bounce"),(6,_)=>Some("TwoHanded_Loop_Swing2_Bounce"),_=>None}}
 /// `Gameplay.Action.Player.InteractBuild` is blocked by this ability (GAB_Melee_GenericCombo AbilityTags `Gameplay.Action.Player.Light`; DefaultGameData
 /// `AbilityTagRelationships`: Light BLOCKS Sprint / Reload / InteractSearch / InteractBuild, merged by `UFortAbilitySystemComponent::ApplyAbilityBlockAndCancelTags`
 /// @14028aa60). UE 4.16 turns the block on at activation (GameplayAbility.cpp:687) and off at EndAbility (:629); the combo ends only through `K2_EndAbility` on the
 /// montage task's Completed = the non-interrupted auto blend-out (task slot 80 @1402d3e80), which starts when <= BlendOut 0.15 s is left of a section without a next
 /// section (AnimMontage.cpp:1894-1905, BlendOutTriggerTime not cooked = -1). So: blocked for the whole held Swing1 <-> Swing2 loop and through End1 / End2 until
 /// their blend-out starts (End1 0.533 - 0.15, End2 0.3 - 0.15 s in), the same edge `weight` fades on. Bounce1 / Bounce2 (5 / 6): ASSUMED the same 0.15 s rule
 /// (their next section is not traced; the port ends them at stage 0 when the button is up).
 pub fn blocks_interact_build(&self)->bool{match self.stage{0=>false,1|2=>true,_=>self.duration()-self.time>0.15}}
 pub fn weight(&self)->f32{if self.stage==0{0.}else if self.stage>=3{((self.duration()-self.time)/0.15).clamp(0.,1.)}else if self.entering{(self.time/0.08).clamp(0.,1.)}else{1.}}
}
/// Melee trace, 1.8 DATA (tools/verify-1.8.cjs re-reads every number below from the paks): sphere sweep along the facing direction.
///  * reach (m) = `AthenaMeleeWeapons` row `Harvest_Pickaxe_Athena_C_T01` (WID_Harvest_Pickaxe_Athena_C_T01.WeaponStatHandle) `RangeVSBuildings2D` 288 cm
///    (`RangeVSBuildingsZ` 384, `RangeVSEnemies` 296, `RangeVSWeakSpots` 400); the ability's `Damageable` TargetSelection sphere (GAB_Melee_ImpactCombo_Athena
///    EffectContainers[0].TargetSelection[2], Swept, MaxTargets 1, Range 300 from the camera) is the native trace this approximates.
///  * radius (m) = that sphere's `HalfExtents` 20 cm (ASSUMED to be the sphere radius: the Sphere shape's native use of HalfExtents / ConeMinRadius 50 is not serialized).
/// The trace ORIGIN (chest height `CHEST_HEIGHT`, along the pawn's facing instead of from the camera) stays an approximation.
pub const MELEE_REACH:f32=2.88;
pub const MELEE_RADIUS:f32=0.2;
pub const CHEST_HEIGHT:f32=1.1;
/// Height of the fallback sweep (ASSUMED: knee height, so shrubs and small rocks below the chest line can be hit; the real trace starts at the camera and follows the aim).
pub const LOW_TRACE_HEIGHT:f32=0.35;
pub const DAMAGEABLE_RANGE:f32=3.0;
pub const PAWN_CENTER_HEIGHT:f32=0.75;
pub fn melee_sweeps(eye_ground:Vec3,facing:Vec3,aim:Option<harvest_fx::Aim>)->Vec<(Vec3,Vec3,f32)>{
 match aim{
  Some(a) if a.dir.length_squared()>1e-6=>{let d=a.dir.normalize();let p=eye_ground+Vec3::Y*PAWN_CENTER_HEIGHT;vec![(a.eye+d*d.dot(p-a.eye),d,DAMAGEABLE_RANGE)]}
  _=>{let f=facing.normalize_or_zero();vec![(eye_ground+Vec3::Y*CHEST_HEIGHT,f,MELEE_REACH),(eye_ground+Vec3::Y*LOW_TRACE_HEIGHT,f,MELEE_REACH)]}
 }
}
/// Damage one swing deals to a prop: `EnvDmgPB` 50 of the same stat row (`DmgPB` 15 is the damage to players, `ImpactDmg*` 0). Weak-spot / crit multipliers
/// (`DamageZone_*`, `DiceCrit*`) are applied natively and are not modelled. The RESOURCE AMOUNT a hit awards is still PROVISIONAL (= the damage dealt): the
/// 1.8 `ResourceRates` / `AthenaResourceRates` curves (e.g. ResourceWoodHigh 40 / 48 at t=0, 0.3 at t=1) are evaluated by native FortBuildingActor code.
pub const HARVEST_DAMAGE:f32=50.;
/// Prop hit points, `AttributesBuildingProps` row `<AttributeInitCategory>.<AttributeInitSubCategory>.FortHealthSet.MaxHealth` at curve time 0:
/// the sandbox tree stands for `Athena_Tree_Large_1` (Parent_Tree category ResourceWood, sub-category overridden to 4 -> 600), the rock for
/// `Athena_Prop_Rocks_StoneLarge01` (ResourceStone, sub-category 3 -> 450).
pub const TREE_HEALTH:f32=600.;
pub const ROCK_HEALTH:f32=450.;
#[derive(Clone,Copy,PartialEq,Debug)]pub enum Kind{Tree,Rock}
/// `Harvestable::proto` of a procedural placeholder (no real 1.8 prop behind it).
pub const NO_PROTO:u32=u32::MAX;
/// POI (2026-10-07): `Harvestable::proto` of a POI actor (fn-app poi.rs) = this bit + the actor index. It has no prop-library kind: the POI's own render path draws it
/// (every static-mesh component, full transform), so the props draw paths skip it.
pub const POI_PROTO:u32=0x4000_0000;
pub fn is_poi_proto(p:u32)->bool{p!=NO_PROTO&&p&POI_PROTO!=0}
/// Harvestable: by default a procedurally generated low-poly placeholder (vertical-cylinder collider); a REAL 1.8 prop (`proto` / `hull` set) when the app has the exported assets.
#[derive(Clone,Debug)]pub struct Harvestable{pub kind:Kind,pub pos:Vec3,pub radius:f32,pub height:f32,pub health:f32,pub shake:f32,pub gone_for:f32,/** respawn counter: part of the HUD popup's `ResourceHoldingObject` identity */pub life:u32,
 /// Hit points when whole (`max_health(kind)` for placeholders, the catalog value of the real prop kind otherwise); the regrow timer restores this.
 pub max_hp:f32,
 /// What a harvest hit yields (placeholders: wood for trees, stone for rocks).
 pub res:Resource,
 /// Placeholders come back 8 s after being destroyed (the old runtime's sandbox rule); real 1.8 props do not (ASSUMED: the real game never regrows a destroyed tree / rock).
 pub regrow:bool,
 /// Index of the real prop kind (the app's prop library) this stands for, `NO_PROTO` for a placeholder. The renderer picks the mesh by it.
 pub proto:u32,
 /// Placement yaw (radians about +Y) and uniform scale of a real prop (placeholders: 0 / 1).
 pub yaw:f32,pub scale:f32,
 /// MIRRORFIX: the placement's scale has a negative determinant (a mirrored prop): the mesh is mirrored in its local X before the yaw (`fn_propdata::PropInstance::mirror_x`).
 pub mirror_x:bool,
 /// The REAL collision: the convex hulls of the prop in world space (metres, Y up), baked with position / yaw / scale. `None` = the placeholder cylinder.
 pub hull:Option<std::sync::Arc<HullSet>>,
 /// COMPLEX COLLISION (2026-10-07): the placed render triangles the WEAPON trace hits (`FCollisionQueryParams("WeaponTrace", .., bTraceComplex = 1)`, AFortWeapon
 /// vfunc_269 @140945cc0; see `fn_hull::TriMesh`): bullets pass the opening of an arch, the gap under a table or a truck. `None` = bullets hit `hull` (the
 /// `CTF_UseSimpleAsComplex` bodies, the placeholders). The pickaxe, the camera and the pawn never read it (bTraceComplex 0 / simple shapes).
 pub complex:Option<std::sync::Arc<TriMesh>>,
 /// BUILDING: the actor's `ABuildingActor` +0x564 bits the placement sweep reads (bit0 `bDoNotBlockBuildings`, bit1 `bDestroyOnPlayerBuildingPlacement`,
 /// bit2 `bUseCentroidForBlockBuildingsCheck`; 0 = the native `ABuildingProp` default: blocks, no centroid test) and `GetCentroid` (world, metres).
 pub build_flags:u8,pub centroid:Vec3,
 /// POI (2026-10-07): the actor takes no damage at all (a `FortStaticMeshActor` landscape piece, a class without a `FortHealthSet.MaxHealth` row, `bIsInvulnerable`,
 /// the chests / ammo boxes while loot is on hold): hits land (impact, no number, no weak spot) and yield nothing.
 pub invulnerable:bool,
 /// POI: a damageable actor whose class has no `ResourceType` (mushrooms, some furniture): it breaks but gives no materials.
 pub no_yield:bool}
impl Harvestable{
 pub fn new(kind:Kind,pos:Vec3)->Self{let (radius,height)=match kind{Kind::Tree=>(0.4,4.5),Kind::Rock=>(1.0,1.4)};Self{kind,pos,radius,height,health:Self::max_health(kind),shake:0.,gone_for:0.,life:0,max_hp:Self::max_health(kind),res:match kind{Kind::Tree=>Resource::Wood,Kind::Rock=>Resource::Stone},regrow:true,proto:NO_PROTO,yaw:0.,scale:1.,mirror_x:false,hull:None,complex:None,build_flags:0,centroid:pos+Vec3::Y*(height*0.5),invulnerable:false,no_yield:false}}
 /// A real 1.8 prop: `hull` is its collision in world space (already placed), `max_hp` its catalog health. `kind` selects the audio / HUD family (rock vs tree).
 pub fn real(kind:Kind,res:Resource,pos:Vec3,yaw:f32,scale:f32,max_hp:f32,proto:u32,hull:Option<std::sync::Arc<HullSet>>)->Self{
  let (mut radius,mut height)=(0.4f32,1.0f32);
  if let Some(h)=&hull{radius=0.;height=0.;for piece in &h.hulls{height=height.max(piece.max.y-pos.y);
   // footprint of the pieces that reach the ground (the trunk / the rock body), horizontal distance of their vertices from the origin
   if piece.min.y-pos.y<1.{for v in &piece.verts{radius=radius.max(Vec3::new(v.x-pos.x,0.,v.z-pos.z).length());}}}
   radius=radius.clamp(0.2,4.);height=height.max(0.5);}
  Self{kind,pos,radius,height,health:max_hp,shake:0.,gone_for:0.,life:0,max_hp,res,regrow:false,proto,yaw,scale,mirror_x:false,hull,complex:None,build_flags:0,centroid:pos,invulnerable:false,no_yield:false}}
 /// 1.8 hit points of the prop this placeholder stands for (`TREE_HEALTH` / `ROCK_HEALTH`).
 pub fn max_health(kind:Kind)->f32{match kind{Kind::Tree=>TREE_HEALTH,Kind::Rock=>ROCK_HEALTH}}
 pub fn alive(&self)->bool{self.health>0.}
}
#[derive(Debug,Clone,Copy,PartialEq)]pub struct TraceHit{pub index:usize,pub fraction:f32}
/// Sphere sweep of `radius` from `origin` along `dir` (unit) for `reach`, against vertical cylinders.
pub fn trace_cylinders(props:&[Harvestable],origin:Vec3,dir:Vec3,reach:f32,radius:f32)->Option<TraceHit>{trace_cylinders_iter(props.iter().enumerate(),origin,dir,reach,radius)}
pub fn trace_cylinders_ids(props:&[Harvestable],ids:&[u32],origin:Vec3,dir:Vec3,reach:f32,radius:f32)->Option<TraceHit>{trace_cylinders_iter(ids.iter().map(|&i|(i as usize,&props[i as usize])),origin,dir,reach,radius)}
fn trace_cylinders_iter<'a>(props:impl Iterator<Item=(usize,&'a Harvestable)>,origin:Vec3,dir:Vec3,reach:f32,radius:f32)->Option<TraceHit>{
 let d=dir.normalize_or_zero();if d==Vec3::ZERO{return None;}
 let mut best:Option<TraceHit>=None;
 for (index,p) in props{if !p.alive(){continue;}
  // REAL collision: sphere sweep against the convex hulls of the 1.8 prop (the placeholder cylinder below is the fallback).
  if let Some(h)=&p.hull{if let Some(f)=h.trace_sphere(origin,origin+d*reach,radius){if best.map_or(true,|b|f<b.fraction){best=Some(TraceHit{index,fraction:f});}}continue;}
  // Sample the swept segment densely; the cylinder test is exact per sample (step 2 cm << radius).
  let steps=((reach/0.02).ceil() as usize).max(1);
  for s in 0..=steps{let f=s as f32/steps as f32;let q=origin+d*(reach*f);
   let dx=q.x-p.pos.x;let dz=q.z-p.pos.z;
   if (dx*dx+dz*dz).sqrt()<=p.radius+radius&&q.y>=p.pos.y-radius&&q.y<=p.pos.y+p.height+radius{
    if best.map_or(true,|b|f<b.fraction){best=Some(TraceHit{index,fraction:f});}break;}}}
 best
}
/// Placeholder harvestable scene plus resource counters.
/// Resource a harvest hit yields (the HUD popup icon / pack).
#[derive(Clone,Copy,PartialEq,Eq,Debug)]pub enum Resource{Wood,Stone,Metal}
/// One resource hit as `FortHUDContext.OnDamagedResourceBuilding(DamagedResourceObject, PotentialResource, bDestroyed, bWeakpointDamage)` would report it
/// (drained by `reticle::tick`). `weakpoint` is never set by this runtime: the placeholder props have no weak spots (the real ones are FortWeakspot components).
#[derive(Clone,Copy,PartialEq,Debug)]pub struct HarvestEvent{pub object:u64,pub resource:Resource,pub amount:u32,pub destroyed:bool,pub weakpoint:bool}
/// BUILDING HOOK: `(world, eye_ground, facing) -> true` when a player build took the strike (the props are then left alone). Installed by fn-app.
pub type BuildingStrike=fn(&World,Vec3,Vec3)->bool;
pub static BUILDING_STRIKE:std::sync::Mutex<Option<BuildingStrike>>=std::sync::Mutex::new(None);
#[derive(Default)]pub struct World{pub props:Vec<Harvestable>,pub wood:u32,pub stone:u32,/** metal harvested (no 1.8 pickaxe prop of this build yields it; kept so `Resource::Metal` is never dropped) */pub metal:u32,pub hits:u32,pub misses:u32,/** harvest hits since the HUD last drained them */pub events:Vec<HarvestEvent>,/** HARVEST HOOK: weak points, damage numbers, crack / break feedback (harvest_fx.rs) */pub fx:harvest_fx::Fx}
impl World{
 /// Default sandbox props near the spawn (player spawns at z=12 looking toward -Z).
 pub fn sandbox()->Self{Self{props:vec![Harvestable::new(Kind::Tree,Vec3::new(-3.2,0.,6.)),Harvestable::new(Kind::Rock,Vec3::new(3.0,0.,5.)),Harvestable::new(Kind::Tree,Vec3::new(5.5,0.,2.))],..Default::default()}}
 /// Run the TriggerAbility trace; on a hit damages the prop, awards resources and returns true
 /// (caller then plays the blocked-hit Bounce section).
 pub fn strike(&mut self,eye_ground:Vec3,facing:Vec3)->bool{
  // HARVEST HOOK: the strike (target selection incl. the camera weak-spot line, damage 50 / 100, events, weak spot, break) lives in harvest_fx.rs; the original body below is
  // the ordinary-hit path it reproduces and stays as `strike_plain` for reference / comparison.
  // BUILDING HOOK (fn-app building_damage_hook.rs): a player build that wins the ability's target selection over the props takes the swing (fn_building::damage)
  if let Some(hook)=BUILDING_STRIKE.lock().ok().and_then(|g|*g){if hook(self,eye_ground,facing){self.fx.aim=None;self.hits+=1;return true;}}
  harvest_fx::strike(self,eye_ground,facing)
 }
 /// The ordinary pickaxe strike without weak points (the pre-HARVEST-HOOK `strike`).
 pub fn strike_plain(&mut self,eye_ground:Vec3,facing:Vec3)->bool{
  let origin=eye_ground+Vec3::Y*CHEST_HEIGHT;
  // A second, knee-height sweep only when the chest-height one found nothing: real 1.8 props can be lower than the chest (shrubs 0.6 m, small rocks); the placeholder
  // cylinders span the whole height, so for them the second sweep can never succeed where the first failed.
  let hit=trace_cylinders(&self.props,origin,facing,MELEE_REACH,MELEE_RADIUS).or_else(||trace_cylinders(&self.props,eye_ground+Vec3::Y*LOW_TRACE_HEIGHT,facing,MELEE_REACH,MELEE_RADIUS));
  match hit{
   Some(h)=>{if self.props[h.index].invulnerable{self.hits+=1;return true;}let p=&mut self.props[h.index];p.health-=HARVEST_DAMAGE;p.shake=0.3;self.hits+=1;if p.health<=0.{p.gone_for=0.;}
    let resource=p.res;let amount=if p.no_yield{0}else{HARVEST_DAMAGE as u32};self.add_mats(resource,amount);/* INVENTORY API (mats.rs) */let p=&self.props[h.index];
    self.events.push(HarvestEvent{object:h.index as u64|((p.life as u64)<<32),resource,amount,destroyed:p.health<=0.,weakpoint:false});true}
   None=>{self.misses+=1;false}}
 }
 pub fn update(&mut self,dt:f32){for p in &mut self.props{p.shake=(p.shake-dt).max(0.);if !p.alive(){p.gone_for+=dt;if p.regrow&&p.gone_for>8.{p.health=p.max_hp;p.life+=1;}}}self.fx.update(&self.props,dt);/* HARVEST HOOK */}
}
/// SINGLE SOURCE OF TRUTH for the placeholder prop shapes: `prop_mesh` (rendering) and `propcol` (camera + player
/// collision) are both built from these numbers, and a propcol test proves every rendered vertex lies inside the collider.
/// Tree trunk: (base radius, top radius, height) in metres.
pub const TREE_TRUNK:(f32,f32,f32)=(0.4,0.3,3.0);
/// Tree canopy cones: (base radius, base height, apex height, colour).
pub const TREE_CANOPIES:[(f32,f32,f32,[f32;3]);2]=[(1.9,2.4,5.0,[0.16,0.42,0.14]),(1.4,4.2,6.6,[0.2,0.5,0.17])];
/// Rock rings (radius, height) from the base up, then the apex height and its x offset; each ring vertex k is scaled by
/// `1 + ROCK_JITTER * rock_jitter_unit(k)`, with the unit in [-1,1].
pub const ROCK_RINGS:[(f32,f32);3]=[(1.05,0.),(0.95,0.7),(0.5,1.25)];
pub const ROCK_APEX:(f32,f32)=(0.05,1.4);
pub const ROCK_JITTER:f32=0.12;
pub fn rock_jitter_unit(k:usize)->f32{((k*7%5) as f32-2.)/2.}
/// Procedural low-poly geometry in prop-local metres (origin on the ground).
/// Horizontal sway factor of a shaking prop (old World::draw).
pub fn prop_wobble(p:&Harvestable)->f32{if p.shake>0.{(p.shake*60.).sin()*p.shake*0.15}else{0.}}
/// World-space position of a prop-local mesh vertex including the shake sway (old World::draw).
pub fn prop_vertex(p:&Harvestable,v:Vec3)->Vec3{let sway=v.y/p.height.max(0.1)*prop_wobble(p);Vec3::new(p.pos.x+v.x+sway,p.pos.y+v.y,p.pos.z+v.z)}
/// Outward unit normal of a prop-mesh triangle (old World::draw: oriented away from the prop's core).
pub fn prop_normal(p:&Harvestable,tri:&[Vec3;3])->Vec3{let mut n=(tri[1]-tri[0]).cross(tri[2]-tri[0]).normalize_or_zero();if n.dot((tri[0]+tri[1]+tri[2])/3.-Vec3::new(0.,p.height*0.4,0.))<0.{n=-n;}n}
pub fn prop_mesh(kind:Kind)->Vec<([Vec3;3],[f32;3])>{
 let mut out=vec![];let tau=std::f32::consts::TAU;let ring=|r:f32,y:f32,n:usize,i:usize|{let a=tau*i as f32/n as f32;Vec3::new(r*a.cos(),y,r*a.sin())};
 match kind{
  Kind::Tree=>{let n=8;let (tr0,tr1,th)=TREE_TRUNK;
   for i in 0..n{let (a,b)=(i,(i+1)%n);let (b0,b1,t0,t1)=(ring(tr0,0.,n,a),ring(tr0,0.,n,b),ring(tr1,th,n,a),ring(tr1,th,n,b));out.push(([b0,t0,b1],[0.36,0.24,0.13]));out.push(([b1,t0,t1],[0.36,0.24,0.13]));}
   for (r,y0,y1,c) in TREE_CANOPIES{for i in 0..n{let (a,b)=(i,(i+1)%n);let apex=Vec3::new(0.,y1,0.);out.push(([ring(r,y0,n,a),apex,ring(r,y0,n,b)],c));out.push(([ring(r,y0,n,b),Vec3::new(0.,y0,0.),ring(r,y0,n,a)],c));}}}
  Kind::Rock=>{let n=9;
   for i in 0..n{let (a,b)=(i,(i+1)%n);let w=|k:usize,r:f32,y:f32|ring(r*(1.0+ROCK_JITTER*rock_jitter_unit(k)),y,n,k);
    let (b0,b1)=(w(a,ROCK_RINGS[0].0,ROCK_RINGS[0].1),w(b,ROCK_RINGS[0].0,ROCK_RINGS[0].1));let (m0,m1)=(w(a,ROCK_RINGS[1].0,ROCK_RINGS[1].1),w(b,ROCK_RINGS[1].0,ROCK_RINGS[1].1));let (t0,t1)=(w(a,ROCK_RINGS[2].0,ROCK_RINGS[2].1),w(b,ROCK_RINGS[2].0,ROCK_RINGS[2].1));let apex=Vec3::new(ROCK_APEX.0,ROCK_APEX.1,0.);
    let c=[0.46,0.46,0.48];out.push(([b0,m0,b1],c));out.push(([b1,m0,m1],c));out.push(([m0,t0,m1],c));out.push(([m1,t0,t1],c));out.push(([t0,apex,t1],[0.55,0.55,0.57]));}}
 }
 out
}
#[cfg(test)]mod tests{use super::*;
 fn montage()->serde_json::Value{serde_json::from_str(&fn_testdata::report_text("melee-montage.json")).unwrap()}
 fn prop_f32(m:&serde_json::Value,name:&str)->Vec<f64>{m["properties"].as_array().unwrap().iter().filter(|p|p["name"]==name).map(|p|p["value"].as_f64().unwrap()).collect()}
 #[test]fn notify_constants_match_recovered_montage(){let m=montage();let t=prop_f32(&m,"TriggerTime");assert_eq!(t.len(),6);
  // NotifyIndex 5/12 (TriggerAbility) at absolute 0.1603 / 0.7348; 4/11 (DecisionWindow) start/end.
  assert!((t[1]-TRIGGER_ABILITY[0] as f64).abs()<1e-5);assert!((t[4]-0.5333-TRIGGER_ABILITY[1] as f64).abs()<2e-4);
  assert!((t[0]-DECISION_WINDOW[0].0 as f64).abs()<1e-5);assert!((t[2]-DECISION_WINDOW[0].1 as f64).abs()<2e-4);assert!((t[3]-0.5333-DECISION_WINDOW[1].0 as f64).abs()<2e-4);
  let names:Vec<String>=m["properties"].as_array().unwrap().iter().filter(|p|p["name"]=="NotifyName").map(|p|p["value"]["name"].as_str().unwrap().to_owned()).collect();
  // Zero-based notify array index -> name proves indices 5 and 12 are TriggerAbility, 4 and 11 DecisionWindow.
  assert_eq!(names[5],"TriggerAbility");assert_eq!(names[12],"TriggerAbility");assert_eq!(names[4],"DecisionWindow");assert_eq!(names[11],"DecisionWindow");}
 #[test]fn blocked_tags_map_to_bounce_sections(){let m=montage();let s=m["properties"].to_string();assert!(s.contains("Gameplay.Melee.Hit1")&&s.contains("Bounce1")&&s.contains("Gameplay.Melee.Hit2")&&s.contains("Bounce2"));
  let len=prop_f32(&m,"AnimEndTime");assert!(len.iter().any(|v|(*v-BOUNCE_LENGTH[0] as f64).abs()<1e-5));assert!(len.iter().any(|v|(*v-BOUNCE_LENGTH[1] as f64).abs()<1e-5));}
 fn run(s:&mut Swing,down:bool,secs:f32)->Vec<(f32,Notify)>{let mut out=vec![];let mut t=0.;while t<secs{if let Some(n)=s.step(down,1./120.){out.push((t,n));}t+=1./120.;}out}
 #[test]fn trigger_ability_fires_once_per_swing_at_serialized_time(){let mut s=Swing::default();let ev=run(&mut s,true,0.5);assert_eq!(ev.len(),1);assert_eq!(ev[0].1,Notify::TriggerAbility{combo:1});assert!((ev[0].0-TRIGGER_ABILITY[0]).abs()<0.02,"{:?}",ev);
  // continue into Swing2 while held: fires combo 2 at 0.2015 into that section
  let ev=run(&mut s,true,0.7);assert!(ev.iter().any(|e|e.1==Notify::TriggerAbility{combo:2}));assert_eq!(ev.iter().filter(|e|e.1==Notify::TriggerAbility{combo:2}).count(),1);}
 #[test]fn recovery_sections_never_trigger_ability(){let mut s=Swing::default();s.step(true,0.01);let ev=run(&mut s,false,2.);assert_eq!(ev.len(),1);assert_eq!(s.stage,0);}
 #[test]fn blocked_hit_branches_to_matching_bounce_then_ends(){let mut s=Swing::default();let _=run(&mut s,false,0.0);s.step(true,0.01);run(&mut s,true,0.2);assert_eq!(s.stage,1);assert!(s.blocked());assert_eq!(s.stage,5);assert_eq!(s.clip(false),Some("TwoHanded_Loop_Swing1_Bounce"));assert_eq!(s.clip(true),Some("TwoHanded_Loop_Swing1_Bounce"));assert!((s.duration()-0.4).abs()<1e-6);
  assert_eq!(s.weight(),1.);run(&mut s,false,0.5);assert_eq!(s.stage,0);
  let mut s=Swing::default();s.step(true,0.01);run(&mut s,true,0.7);assert_eq!(s.stage,2);assert!(s.blocked());assert_eq!(s.stage,6);assert_eq!(s.clip(false),Some("TwoHanded_Loop_Swing2_Bounce"));assert!(!{s.stage=3;s.blocked()});}
 #[test]fn held_input_restarts_swing_after_bounce(){let mut s=Swing::default();s.step(true,0.01);run(&mut s,true,0.2);s.blocked();run(&mut s,true,0.45);assert_eq!(s.stage,1);}
 fn tree()->Harvestable{Harvestable::new(Kind::Tree,Vec3::new(0.,0.,-3.))}
 #[test]fn trace_hits_prop_in_reach_and_misses_otherwise(){let p=[tree()];let o=Vec3::new(0.,CHEST_HEIGHT,-1.);let f=Vec3::new(0.,0.,-1.);
  assert!(trace_cylinders(&p,o,f,MELEE_REACH,MELEE_RADIUS).is_some()); // surface at z=-2.6, within 1.6+0.3
  assert!(trace_cylinders(&p,Vec3::new(0.,CHEST_HEIGHT,2.),f,MELEE_REACH,MELEE_RADIUS).is_none()); // too far
  assert!(trace_cylinders(&p,o,Vec3::new(0.,0.,1.),MELEE_REACH,MELEE_RADIUS).is_none()); // behind
  assert!(trace_cylinders(&p,Vec3::new(1.5,CHEST_HEIGHT,-1.),f,MELEE_REACH,MELEE_RADIUS).is_none()); // lateral miss
  assert!(trace_cylinders(&p,Vec3::new(0.,6.5,-1.),f,MELEE_REACH,MELEE_RADIUS).is_none()); // above the trunk collider
  assert!(trace_cylinders(&p,o,Vec3::ZERO,MELEE_REACH,MELEE_RADIUS).is_none());}
 #[test]fn trace_returns_nearest_prop(){let p=[Harvestable::new(Kind::Rock,Vec3::new(0.,0.,-4.)),Harvestable::new(Kind::Rock,Vec3::new(0.,0.,-2.5))];let h=trace_cylinders(&p,Vec3::new(0.,CHEST_HEIGHT-0.5,0.),Vec3::NEG_Z,MELEE_REACH,MELEE_RADIUS).unwrap();assert_eq!(h.index,1);}
 #[test]fn strike_damages_awards_and_destroys_then_respawns(){let mut w=World{props:vec![tree()],..Default::default()};let eye=Vec3::new(0.,0.,-1.);
  let n=(TREE_HEALTH/HARVEST_DAMAGE).ceil() as u32;assert_eq!(n,12,"600 HP / 50 per swing");
  for i in 0..n{assert!(w.props[0].alive(),"alive before hit {i}");assert!(w.strike(eye,Vec3::NEG_Z),"hit {i}");}assert_eq!(w.hits,n);assert_eq!(w.wood,n*HARVEST_DAMAGE as u32);assert!(!w.props[0].alive());
  assert!(!w.strike(eye,Vec3::NEG_Z));assert_eq!(w.misses,1);w.update(9.);assert!(w.props[0].alive());assert_eq!(w.props[0].health,TREE_HEALTH);}
 /// The pickaxe numbers are the 1.8 recovered values (tools/verify-1.8.cjs re-reads them from the paks; reports/verify-1.8.json rows group "combat").
 #[test]fn pickaxe_numbers_are_the_recovered_1_8_values(){assert_eq!(HARVEST_DAMAGE,50.);assert!((MELEE_REACH-2.88).abs()<1e-6);assert!((MELEE_RADIUS-0.2).abs()<1e-6);assert_eq!((TREE_HEALTH,ROCK_HEALTH),(600.,450.));
  assert_eq!(Harvestable::new(Kind::Rock,Vec3::ZERO).health,450.);}
 #[test]fn prop_meshes_are_finite_and_within_their_colliders_height(){for k in [Kind::Tree,Kind::Rock]{let m=prop_mesh(k);assert!(m.len()>=16);let top=m.iter().flat_map(|t|t.0).map(|v|{assert!(v.is_finite());v.y}).fold(0.,f32::max);let h=Harvestable::new(k,Vec3::ZERO).height;assert!(top>=h*0.9&&top<=h*1.6,"{k:?} top {top}");}}
 fn hulled(kind:Kind,res:Resource,hp:f32,pos:Vec3,h:f32)->Harvestable{let mut pts=vec![];for x in [-0.5,0.5]{for z in [-0.5,0.5]{for y in [0.,h]{pts.push(pos+Vec3::new(x,y,z));}}}
  Harvestable::real(kind,res,pos,0.,1.,hp,7,Some(std::sync::Arc::new(HullSet::new(vec![Hull::from_points(&pts).unwrap()]))))}
 #[test]fn a_real_prop_takes_its_own_hit_points_yields_its_own_resource_and_never_regrows(){
  let eye=Vec3::new(0.,0.,-1.);
  let mut w=World{props:vec![hulled(Kind::Rock,Resource::Stone,180.,Vec3::new(0.,0.,-3.),2.)],..Default::default()};
  for i in 0..4{assert!(w.props[0].alive(),"alive before hit {i}");assert!(w.strike(eye,Vec3::NEG_Z),"hit {i}");}
  assert!(!w.props[0].alive(),"180 hp / 50 per swing = 4 swings");assert_eq!((w.stone,w.wood,w.hits),(200,0,4));
  assert_eq!(w.events.last().map(|e|(e.resource,e.destroyed,e.object>>32)),Some((Resource::Stone,true,0)));
  assert!(!w.strike(eye,Vec3::NEG_Z),"nothing left to hit");w.update(60.);assert!(!w.props[0].alive(),"real props do not come back");
  // a placeholder still regrows after 8 s
  let mut p=World{props:vec![Harvestable::new(Kind::Tree,Vec3::new(0.,0.,-3.))],..Default::default()};p.props[0].health=0.;p.update(9.);assert!(p.props[0].alive()&&p.props[0].health==TREE_HEALTH);}
 #[test]fn a_low_real_prop_is_hit_by_the_knee_sweep_and_a_tall_one_by_the_chest_sweep(){
  let eye=Vec3::new(0.,0.,-1.);
  let mut shrub=World{props:vec![hulled(Kind::Tree,Resource::Wood,30.,Vec3::new(0.,0.,-3.),0.6)],..Default::default()};
  assert!(trace_cylinders(&shrub.props,eye+Vec3::Y*CHEST_HEIGHT,Vec3::NEG_Z,MELEE_REACH,MELEE_RADIUS).is_none(),"a 0.6 m shrub is below the chest line");
  assert!(shrub.strike(eye,Vec3::NEG_Z)&&!shrub.props[0].alive(),"30 hp: one swing, found by the knee sweep");
  let mut tall=World{props:vec![hulled(Kind::Tree,Resource::Wood,600.,Vec3::new(0.,0.,-3.),9.)],..Default::default()};
  assert!(tall.strike(eye,Vec3::NEG_Z));assert_eq!(tall.props[0].health,550.);}
 #[test]fn the_hit_trace_follows_the_hull_not_the_cylinder(){
  // the hull is 1 m wide around x = 2: a ray 0.3 m off the origin axis hits it, one 0.6 m off the edge does not (the 0.4 m cylinder would decide differently)
  let mut p=hulled(Kind::Tree,Resource::Wood,600.,Vec3::new(2.,0.,-3.),3.);p.radius=0.01;
  let o=Vec3::new(2.4,CHEST_HEIGHT,-1.);assert!(trace_cylinders(&[p.clone()],o,Vec3::NEG_Z,MELEE_REACH,0.).is_some(),"inside the hull width but outside the 0.4 m cylinder around the origin");
  assert!(trace_cylinders(&[p],Vec3::new(3.1,CHEST_HEIGHT,-1.),Vec3::NEG_Z,MELEE_REACH,0.).is_none(),"beyond the hull edge is a miss");}
 #[test]fn stationary_crouch_keeps_lower_body_pose(){assert!(upper_body_attack(0.,0.,true));assert!(upper_body_attack(1.,0.,false));assert!(upper_body_attack(0.,1.,false));assert!(!upper_body_attack(0.,0.,false));}
 #[test]fn held_input_alternates_source_sections(){let mut s=Swing::default();for _ in 0..33{s.step(true,1./60.);}assert_eq!(s.stage,2);for _ in 0..38{s.step(true,1./60.);}assert_eq!(s.stage,1);}
 /// AFortPawn::vfunc_334 (window end): ClickOrHold takes Swing2 for a press received inside the window or a button still held at the window's end, never for a
 /// hold that was released inside the window without a new press.
 #[test]fn decision_window_takes_a_press_inside_it_or_a_hold_at_its_end(){
  let dt=1./120.;let steps=|s:&mut Swing,down:bool,secs:f32|{for _ in 0..(secs/dt).round() as u32{s.step(down,dt);}};
  // held from the start, released at 0.3 s (inside 0.1285..0.5332): no press in the window, not held at its end -> End1
  let mut s=Swing::default();steps(&mut s,true,0.3);steps(&mut s,false,0.3);assert_eq!(s.stage,3);
  // tapped inside the window (released again before its end) -> Swing2
  let mut s=Swing::default();steps(&mut s,true,0.05);steps(&mut s,false,0.15);steps(&mut s,true,0.05);steps(&mut s,false,0.35);assert_eq!(s.stage,2);
  // tapped before the window opens -> the press is not the window's -> End1
  let mut s=Swing::default();steps(&mut s,true,0.03);steps(&mut s,false,0.03);steps(&mut s,true,0.03);steps(&mut s,false,0.6);assert_eq!(s.stage,3);
  // pressed late and still held when the window ends -> Swing2
  let mut s=Swing::default();steps(&mut s,true,0.02);steps(&mut s,false,0.3);steps(&mut s,true,0.3);assert_eq!(s.stage,2);}
 /// Edit-start gate (`AFortPlayerController::vfunc_669` @14083e780 -> InteractBuild blocked by the combo's Light tag): blocked through the held loop, unblocked
 /// when the End section's blend-out starts (End1 0.3833 s in, End2 0.15 s in), never blocked at idle.
 #[test]fn interact_build_is_blocked_while_the_combo_ability_is_active(){
  let dt=1./240.;let mut s=Swing::default();assert!(!s.blocks_interact_build());
  for _ in 0..(2.5/dt) as u32{s.step(true,dt);assert!(s.stage==1||s.stage==2);assert!(s.blocks_interact_build(),"held loop at stage {} t {}",s.stage,s.time);}
  // released before Swing1's window opens -> End1: blocked until 0.5333 - 0.15 into it
  let mut s=Swing::default();for _ in 0..(0.05/dt) as u32{s.step(true,dt);}while s.stage!=3{s.step(false,dt);assert!(s.blocks_interact_build());}
  let mut t=s.time;while s.blocks_interact_build(){s.step(false,dt);t+=dt;}assert!((t-0.38333).abs()<0.01,"End1 unblocks at {t}");assert_eq!(s.stage,3);
  // held into Swing2, released -> End2: unblocked 0.15 s in
  let mut s=Swing::default();for _ in 0..(0.6/dt) as u32{s.step(true,dt);}assert_eq!(s.stage,2);while s.stage!=4{s.step(false,dt);assert!(s.blocks_interact_build());}
  let mut t=s.time;while s.blocks_interact_build(){s.step(false,dt);t+=dt;}assert!((t-0.15).abs()<0.01,"End2 unblocks at {t}");
  while s.stage!=0{s.step(false,dt);assert!(!s.blocks_interact_build());}}
 #[test]fn released_input_recovers_to_idle(){let mut s=Swing::default();s.step(true,0.01);for _ in 0..110{s.step(false,0.01);}assert_eq!(s.stage,0);}
 #[test]fn second_swing_uses_its_own_recovery(){let mut s=Swing::default();for _ in 0..33{s.step(true,1./60.);}for _ in 0..38{s.step(false,1./60.);}assert_eq!(s.stage,4);assert_eq!(s.clip(true),Some("TwoHanded_Loop_Swing2_2Idle_InMotion"));}
}

pub mod reticle;
pub mod harvest_fx; // HARVEST HOOK: weak points / crit / damage numbers / Target Info focus / break (see harvest_fx.rs)
pub mod mats; // INVENTORY API: World::mats / add_mats / spend_mats (see crates/fn-weapons/INVENTORY_API.md)
