//! FOOT FX (fn-game): when the 1.8 BR player pawn kicks up dust. Renderer-free gate logic of `PlayerPawn_Athena_Generic`; fn-app `foot_fx.rs` turns the
//! [`Cmd`]s into the real Cascade systems (`P_Walk_DustKickup_01`, `P_Run_DustKickup_01`, `P_Player_Land_Effect`).
//!
//! RECOVERED (cooked `/Game/Athena/PlayerPawn_Athena_Generic`, `ExecuteUbergraph_PlayerPawn_Athena_Generic`; offsets are the IN-MEMORY offsets of the
//! ubergraph, `fn-uasset` example `kismet_mem`, i.e. the jump / entry-point space; the file offset follows in brackets):
//! * the dust is Blueprint-only. The footstep anim notifies (`AnimNotify_FootStep_Left/Right` on the jog / sprint sequences, e.g. `Sprint_AR`) call
//!   `AFortPawn::PlayFootStepSound` (exe vfunc 305 0x1407f9f80 -> 306: `AFortPlayerPawnAthena` 0x14041c440 plays the footstep / sprint footstep sound and
//!   broadcasts `OnFootstepEvent`, nothing else); the pawn's own `FootStepLeft` / `FootStepRight` events enter the ubergraph at 34394 / 34395, which is a bare
//!   `popflow`. No physical surface is read anywhere on this path: the land dust is the same on grass, dirt, road, wood, stone and metal. The only swap is
//!   `IsInWater` -> the `Effect_Player_*_Water` systems (the port has no water volumes, so they are not ported).
//! * components (SCS, `CharacterMesh0` children): `Effect_Player_Walk_Land` (template `P_Walk_DustKickup_01`) and `Effect_Player_Run_Land` (`P_Run_DustKickup_01`),
//!   `RelativeLocation (0, 0, -9)`, `bAutoActivate false`; `UserConstructionScript` attaches both to the mesh socket `FX_Feet` (Root bone, (0, 0, 15)); the mesh sits
//!   at `(0, 0, -75)` = the capsule bottom, so they ride 6 cm above the feet.
//! * CDO: `Walk_Dust_Activate_Speed` 320, `Walk_Particle_Reset_Speed` 310, `Run_Particle_Activate_Speed` 480 (cm/s); `Effect_Player_LandedDust` = `P_Player_Land_Effect`.
//! * `ReceiveTick` (entry 34085): `PlayerSpeed = |Velocity.xy|` (34309); 23526: if `IsNinjaInShadowStance || !CharacterMovement.IsMovingOnGround()` the five
//!   dust components are `Deactivate()`d and the tick ends (the DoOnce gates below are NOT reset). Not in water (25179): the water ones are deactivated and
//!   `RunWalkParticleRotation = MakeRotFromX((Velocity.x, Velocity.y, 0))`, then a Sequence (3137):
//!   0. (3152) speed > 320: DoOnce (reset while speed <= 320): `if CanSpawnWalkKickupFX` { CanSpawnWalkKickupFX = false; Walk_Land: SetWorldRotation(RunWalkParticleRotation),
//!      SetWorldScale3D(1.4), CustomTimeDilation 1, Activate(true), SetEmitterEnable(Puffs, true); Delay 0.5 -> CanSpawnWalkKickupFX = true }. The DoOnce closes even when
//!      CanSpawnWalkKickupFX is false.
//!   1. (3640) speed < 310: DoOnce (reset while speed >= 310; open at spawn): Walk_Land: SetWorldRotation((-PreviousVelocityVector).Rotation()), SetWorldScale3D(1),
//!      CustomTimeDilation 1.45, Activate(true), SetEmitterEnable(Puffs, false); CanSpawnWalkKickupFX = false; Delay 0.25 -> CanSpawnWalkKickupFX = true.
//!   2. (3625) only if CanSpawnRunKickupFX: speed > 480: DoOnce (reset while speed <= 480): { CanSpawnRunKickupFX = false; Run_Land: SetWorldRotation(RunWalkParticleRotation),
//!      Activate(false); Delay 0.5 -> CanSpawnRunKickupFX = true }, and the "off" DoOnce is reset; speed <= 480: DoOnce (open at spawn, reset by the "on" branch): Run_Land.Deactivate().
//!   3. (2980) `PreviousVelocityVector = (Velocity.x, Velocity.y, 0)`.
//! * `OnLanded` (entry 32055): unless `IsNinjaInShadowStance || IsInWater`: `OnLand_CE` (water level only), then (not a dedicated server) `if CanSpawnDustLandFX`
//!   { SpawnEmitterAtLocation(P_Player_Land_Effect, ActorLocation + (0.84, -6.21, -71.75), ComposeRotators(ActorRotation, MakeRotator(0, 0, RandomFloatInRange(-45, 45)))
//!   = actor yaw + U(-45, 45), bAutoDestroy); the wind manager's RTT particle (not ported, no wind RTT); CanSpawnDustLandFX = false; Delay 0.4 -> true (entry 16) }.
//!   Every landing plays it (no height or speed threshold); the offset is world space (not rotated by the actor).
//! * `UKismetSystemLibrary::Delay` does nothing while the same Delay node is still pending (UE4.16 KismetSystemLibrary.cpp `FindExistingAction<FDelayAction>`).
//!
//! With the 1.8 speeds (run 410, sprint 550 cm/s, fn-movement params) this means: one walk puff when the pawn gets going, one skid puff when it stops, and the
//! continuous run stream (`P_Run_DustKickup_01`'s SpawnPerUnit emitters) only while sprinting.
//! ASSUMED: `Random` is a port LCG (the original `FMath::FRandRange` stream is not reproducible); latent Delays count down at the start of the next update.
use glam::Vec3;

/// `Walk_Dust_Activate_Speed` (CDO), cm/s.
pub const WALK_DUST_ACTIVATE_SPEED: f32 = 320.;
/// `Walk_Particle_Reset_Speed` (CDO), cm/s.
pub const WALK_PARTICLE_RESET_SPEED: f32 = 310.;
/// `Run_Particle_Activate_Speed` (CDO), cm/s.
pub const RUN_PARTICLE_ACTIVATE_SPEED: f32 = 480.;
/// `ActorLocation +` this (UE cm, world space) is where `P_Player_Land_Effect` spawns.
pub const LAND_OFFSET_UE: [f32; 3] = [0.84, -6.21, -71.75];
/// FX_Feet socket (0, 0, 15) + the components' RelativeLocation (0, 0, -9): height of the walk / run dust above the mesh origin (= capsule bottom), cm.
pub const FEET_FX_HEIGHT_CM: f32 = 6.;
pub const P_WALK: &str = "/Game/Effects/Fort_Effects/Gameplay/SmokeDustExplosions/Particles/P_Walk_DustKickup_01";
pub const P_RUN: &str = "/Game/Effects/Fort_Effects/Gameplay/SmokeDustExplosions/Particles/P_Run_DustKickup_01";
pub const P_LAND: &str = "/Game/Effects/Fort_Effects/Gameplay/SmokeDustExplosions/Particles/P_Player_Land_Effect";

/// What the Blueprint did to its dust components this update (in order).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    /// `Effect_Player_Walk_Land`: SetWorldRotation(yaw), SetWorldScale3D(scale), CustomTimeDilation, `Activate(true)`, SetEmitterEnable(Puffs, puffs).
    WalkActivate { yaw_deg: f32, scale: f32, time_dilation: f32, puffs: bool },
    /// `Effect_Player_Run_Land`: SetWorldRotation(yaw), `Activate(false)`.
    RunActivate { yaw_deg: f32 },
    WalkDeactivate,
    RunDeactivate,
    /// `SpawnEmitterAtLocation(P_Player_Land_Effect, loc, (0, yaw, 0))`.
    Land { loc_ue: Vec3, yaw_deg: f32 },
}

/// One update's view of the pawn.
#[derive(Clone, Copy, Debug)]
pub struct Input {
    /// `CharacterMovement.IsMovingOnGround()`.
    pub on_ground: bool,
    /// `GetVelocity()`, UE cm/s.
    pub velocity: Vec3,
    /// The pawn landed since the last update (`OnLanded`).
    pub landed: bool,
    /// `K2_GetActorLocation()` (capsule centre), UE cm.
    pub actor_loc: Vec3,
    /// Actor yaw, UE degrees.
    pub actor_yaw_deg: f32,
    pub dt: f32,
}

/// The Blueprint state of the gates (DoOnce nodes, CanSpawn flags, pending Delays).
#[derive(Clone, Debug)]
pub struct FootFx {
    pub can_walk: bool,
    pub can_run: bool,
    pub can_land: bool,
    walk_start_closed: bool,
    stop_closed: bool,
    run_on_closed: bool,
    run_off_closed: bool,
    prev_vel: Vec3,
    /// Pending Delays: walk start 0.5 / stop 0.25 (-> can_walk), run 0.5 (-> can_run), land 0.4 (-> can_land).
    delays: [Option<f32>; 4],
    rng: u64,
}
impl Default for FootFx {
    fn default() -> Self { FootFx { can_walk: true, can_run: true, can_land: true, walk_start_closed: false, stop_closed: false, run_on_closed: false, run_off_closed: false, prev_vel: Vec3::ZERO, delays: [None; 4], rng: 0x2545_f491_4f6c_dd1d } }
}
const D_WALK_START: usize = 0; const D_STOP: usize = 1; const D_RUN: usize = 2; const D_LAND: usize = 3;
const DELAY_S: [f32; 4] = [0.5, 0.25, 0.5, 0.4];

/// `FVector::Rotation().Yaw` of a horizontal vector (0 for the zero vector).
fn yaw_of(v: Vec3) -> f32 { if v.x == 0. && v.y == 0. { 0. } else { v.y.atan2(v.x).to_degrees() } }

impl FootFx {
    fn delay(&mut self, k: usize) { if self.delays[k].is_none() { self.delays[k] = Some(DELAY_S[k]); } }
    fn frand(&mut self, lo: f32, hi: f32) -> f32 {
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        lo + (hi - lo) * ((self.rng >> 40) as f32 / (1u64 << 24) as f32)
    }
    /// One Blueprint tick (plus `OnLanded` when `i.landed`); returns the component commands in execution order.
    pub fn update(&mut self, i: &Input) -> Vec<Cmd> {
        let mut out = vec![];
        // latent Delays
        for k in 0..4 {
            if let Some(t) = self.delays[k].as_mut() {
                *t -= i.dt;
                if *t <= 0. { self.delays[k] = None; match k { D_WALK_START | D_STOP => self.can_walk = true, D_RUN => self.can_run = true, _ => self.can_land = true } }
            }
        }
        // OnLanded
        if i.landed && self.can_land {
            let r = self.frand(-45., 45.);
            out.push(Cmd::Land { loc_ue: i.actor_loc + Vec3::from(LAND_OFFSET_UE), yaw_deg: i.actor_yaw_deg + r });
            self.can_land = false; self.delay(D_LAND);
        }
        // ReceiveTick
        if !i.on_ground { out.push(Cmd::RunDeactivate); out.push(Cmd::WalkDeactivate); return out; }
        let speed = Vec3::new(i.velocity.x, i.velocity.y, 0.).length();
        let run_walk_yaw = yaw_of(i.velocity);
        // 0: walk start puff
        if speed > WALK_DUST_ACTIVATE_SPEED {
            if !self.walk_start_closed {
                self.walk_start_closed = true;
                if self.can_walk {
                    self.can_walk = false;
                    out.push(Cmd::WalkActivate { yaw_deg: run_walk_yaw, scale: 1.4, time_dilation: 1., puffs: true });
                    self.delay(D_WALK_START);
                }
            }
        } else { self.walk_start_closed = false; }
        // 1: stop (skid) puff
        if speed < WALK_PARTICLE_RESET_SPEED {
            if !self.stop_closed {
                self.stop_closed = true;
                out.push(Cmd::WalkActivate { yaw_deg: yaw_of(-self.prev_vel), scale: 1., time_dilation: 1.45, puffs: false });
                self.can_walk = false; self.delay(D_STOP);
            }
        } else { self.stop_closed = false; }
        // 2: run stream
        if self.can_run {
            if speed > RUN_PARTICLE_ACTIVATE_SPEED {
                if !self.run_on_closed { self.run_on_closed = true; self.can_run = false; out.push(Cmd::RunActivate { yaw_deg: run_walk_yaw }); self.delay(D_RUN); }
                self.run_off_closed = false;
            } else {
                self.run_on_closed = false;
                if !self.run_off_closed { self.run_off_closed = true; out.push(Cmd::RunDeactivate); }
            }
        }
        // 3
        self.prev_vel = Vec3::new(i.velocity.x, i.velocity.y, 0.);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inp(speed: f32, on_ground: bool) -> Input { Input { on_ground, velocity: Vec3::new(speed, 0., 0.), landed: false, actor_loc: Vec3::new(0., 0., 75.), actor_yaw_deg: 0., dt: 1. / 60. } }
    fn run(f: &mut FootFx, speed: f32, on_ground: bool, secs: f32) -> Vec<Cmd> { let mut all = vec![]; for _ in 0..(secs * 60.).round() as usize { all.extend(f.update(&inp(speed, on_ground))); } all }
    fn walks(c: &[Cmd]) -> Vec<(f32, f32, bool)> { c.iter().filter_map(|c| if let Cmd::WalkActivate { scale, time_dilation, puffs, .. } = c { Some((*scale, *time_dilation, *puffs)) } else { None }).collect() }
    fn runs(c: &[Cmd]) -> usize { c.iter().filter(|c| matches!(c, Cmd::RunActivate { .. })).count() }

    #[test] fn spawn_plays_the_stop_puff_once_then_jog_plays_one_start_puff_and_no_run_stream() {
        let mut f = FootFx::default();
        let c = run(&mut f, 0., true, 1.);
        assert_eq!(walks(&c), vec![(1., 1.45, false)], "the stop DoOnce is open at spawn");
        let c = run(&mut f, 410., true, 2.);
        assert_eq!(walks(&c), vec![(1.4, 1., true)]);
        assert_eq!(runs(&c), 0, "jog speed 410 < 480");
    }
    #[test] fn sprint_starts_the_run_stream_once_and_stopping_deactivates_it_and_skids() {
        let mut f = FootFx::default();
        run(&mut f, 0., true, 1.);
        let c = run(&mut f, 550., true, 2.);
        assert_eq!(runs(&c), 1); assert_eq!(walks(&c).len(), 1);
        let c = run(&mut f, 0., true, 1.);
        assert!(c.contains(&Cmd::RunDeactivate)); assert_eq!(walks(&c), vec![(1., 1.45, false)]);
    }
    #[test] fn the_skid_puff_faces_back_along_the_previous_velocity() {
        let mut f = FootFx::default();
        run(&mut f, 0., true, 1.);
        let mut v = inp(0., true); v.velocity = Vec3::new(0., 550., 0.);
        for _ in 0..60 { f.update(&v); }
        v.velocity = Vec3::ZERO;
        let c = f.update(&v);
        let yaw = c.iter().find_map(|c| if let Cmd::WalkActivate { yaw_deg, .. } = c { Some(*yaw_deg) } else { None }).unwrap();
        assert!((yaw + 90.).abs() < 1e-3, "{yaw}");
    }
    #[test] fn a_jump_while_sprinting_stops_the_stream_and_landing_at_sprint_speed_does_not_restart_it() {
        let mut f = FootFx::default();
        run(&mut f, 0., true, 1.);
        assert_eq!(runs(&run(&mut f, 550., true, 1.)), 1);
        let c = run(&mut f, 550., false, 0.6);
        assert!(c.contains(&Cmd::RunDeactivate));
        let mut l = inp(550., true); l.landed = true;
        let c = f.update(&l);
        assert!(matches!(c[0], Cmd::Land { .. }));
        let c = run(&mut f, 550., true, 2.);
        assert_eq!(runs(&c), 0, "the DoOnce stays closed until the speed drops to 480");
        run(&mut f, 400., true, 0.5);
        assert_eq!(runs(&run(&mut f, 550., true, 1.)), 1);
    }
    #[test] fn landing_dust_has_a_0_4s_cooldown_and_sits_at_the_actor_offset() {
        let mut f = FootFx::default();
        let mut l = inp(0., true); l.landed = true; l.actor_loc = Vec3::new(100., 200., 375.); l.actor_yaw_deg = 30.;
        let c = f.update(&l);
        let Cmd::Land { loc_ue, yaw_deg } = c[0] else { panic!("{c:?}") };
        assert!((loc_ue - Vec3::new(100.84, 193.79, 303.25)).length() < 1e-3);
        assert!((-15. ..=75.).contains(&yaw_deg));
        assert!(!f.update(&l).iter().any(|c| matches!(c, Cmd::Land { .. })), "CanSpawnDustLandFX is false for 0.4 s");
        run(&mut f, 0., true, 0.45);
        assert!(f.update(&l).iter().any(|c| matches!(c, Cmd::Land { .. })));
    }
}
