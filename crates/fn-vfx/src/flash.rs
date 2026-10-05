//! The `Muzzle(Empty)` particle component of `B_Ranged_Generic` (every BR firearm Blueprint derives from it; recovered from the Kismet bytecode of
//! `/Game/Weapons/Blueprints/B_Ranged_Generic`, `fn-inspect disasm`):
//!
//! ```text
//! UserConstructionScript:  Muzzle(Empty).K2_AttachToComponent(WeaponMesh, MuzzleSocketName, KeepRelative x3, weld); Muzzle(Empty).SetTemplate(MuzzleParticleSystem)
//! OnPlayWeaponFireFX(bPersistentFire, bSecondaryFire):        (native event, called once per shot)
//!     Muzzle Flash FX(bPersistentFire)
//!     if bPersistentFireFX: SetTimer("StopWeaponFireFX", LocalMagazineAmmoCount * TimeToNextFire * 1.1)
//!     PlayForceFeedback(...); if !IsUsingScopeOrFirstPersonCamera(): ActivateShellsFX; ActivateOrDeactivateWindParticle(true)
//! Muzzle Flash FX(bPersistentFire):
//!     if !IsUsingScope():
//!         if !bPersistentFire: AddRandomScale()
//!         Muzzle(Empty).Activate(true); LastPlayFXTime = now
//! AddRandomScale: r = rand(0,1); s = r <= 0.8 ? rand(0.7, 1.0) : rand(0.5, 0.6); Muzzle(Empty).SetRelativeScale3D(s, s, s)
//! OnStopWeaponFireFX -> DeactivateMuzzleFX: Muzzle(Empty).Deactivate()
//! ```
//! `bAutoActivate` of the component is false. `Activate(true)` on a running component re-initialises every emitter and keeps the live particles (`Instance::reactivate`).
use fn_particles::sim::{EmitterDraw, Env, Instance, ParticleLight, Transform};
use fn_particles::{ParticleSystem, Rng};
use glam::Vec3;
use std::sync::Arc;

/// What the host knows when the shot's FX event runs.
#[derive(Clone, Copy, Debug, Default)]
pub struct FireContext {
    /// `AFortWeaponRanged::IsUsingScope()` (native): true while looking through a scope; the Blueprint then skips the muzzle flash entirely.
    pub using_scope: bool,
    /// Camera -> muzzle distance in cm (LOD selection of `ActivateSystem`).
    pub camera_distance_cm: f32,
    /// `GetGameTimeInSeconds`.
    pub now: f32,
    /// `GetLocalMagazineAmmoCount()` (only used for the stop timer of persistent-fire weapons).
    pub magazine_ammo: i32,
    /// `GetTimeToNextFire()` seconds (same).
    pub time_to_next_fire: f32,
}

pub struct MuzzleFlash {
    system: Arc<ParticleSystem>,
    inst: Option<Instance>,
    /// `Muzzle(Empty)` relative scale (AddRandomScale), uniform.
    pub rel_scale: f32,
    rng: Rng,
    serial: u64,
    seed: u64,
    /// Socket transform of the weapon mesh (UE space) the component is attached to, refreshed every frame by the host.
    socket: Transform,
    part_sys_velocity: Vec3,
    /// `LastPlayFXTime`.
    pub last_play_fx_time: f32,
    stop_timer: Option<f32>,
    /// Counters (diagnostics / tests): shots that played a flash, shots suppressed by the scope rule.
    pub played: u32,
    pub suppressed: u32,
}

impl MuzzleFlash {
    /// `system` = the decoded `MuzzleParticleSystem`; `seed` makes the random scale and the particle streams reproducible.
    pub fn new(system: Arc<ParticleSystem>, seed: u64) -> MuzzleFlash {
        MuzzleFlash { system, inst: None, rel_scale: 1.0, rng: Rng::from_pair(seed, 0x4D5A), serial: 0, seed, socket: Transform::identity(), part_sys_velocity: Vec3::ZERO, last_play_fx_time: 0.0, stop_timer: None, played: 0, suppressed: 0 }
    }
    pub fn system(&self) -> &Arc<ParticleSystem> { &self.system }

    /// `AddRandomScale`.
    fn add_random_scale(&mut self) {
        let r = self.rng.range(0.0, 1.0);
        self.rel_scale = if r <= 0.8 { self.rng.range(0.7, 1.0) } else { self.rng.range(0.5, 0.6) };
    }
    fn component_transform(&self) -> Transform { Transform { pos: self.socket.pos, rot: self.socket.rot, scale: self.socket.scale * self.rel_scale } }

    /// `OnPlayWeaponFireFX(bPersistentFire)` -> `Muzzle Flash FX` (+ the stop timer of `bPersistentFireFX` weapons). Returns true when the flash was (re)activated.
    pub fn on_play_weapon_fire_fx(&mut self, persistent_fire: bool, persistent_fire_fx: bool, ctx: &FireContext, socket: Transform) -> bool {
        self.socket = socket;
        if ctx.using_scope { self.suppressed += 1; return false; }
        if !persistent_fire { self.add_random_scale(); }
        let xf = self.component_transform();
        let lod = self.system.lod_for_distance(ctx.camera_distance_cm);
        match self.inst.as_mut() {
            Some(i) => { i.xf = xf; i.lod = lod; for e in &mut i.emitters { e.lod = lod; } i.reactivate(); }
            None => { self.serial += 1; let mut i = Instance::new(self.system.clone(), xf, self.seed ^ self.serial.wrapping_mul(0x9E37_79B9_7F4A_7C15), ctx.camera_distance_cm); i.id = self.serial; self.inst = Some(i); }
        }
        self.last_play_fx_time = ctx.now;
        self.stop_timer = if persistent_fire_fx { Some((ctx.magazine_ammo.max(0) as f32) * ctx.time_to_next_fire * 1.1) } else { None };
        self.played += 1;
        true
    }

    /// `OnStopWeaponFireFX` -> `DeactivateMuzzleFX` (`Muzzle(Empty).Deactivate()`).
    pub fn on_stop_weapon_fire_fx(&mut self) { if let Some(i) = self.inst.as_mut() { i.deactivate(); } self.stop_timer = None; }

    /// The component follows the weapon: call every frame before [`tick`](Self::tick). `part_sys_velocity` = world velocity of the component (UE cm/s).
    pub fn set_socket(&mut self, socket: Transform, part_sys_velocity: Vec3) { self.socket = socket; self.part_sys_velocity = part_sys_velocity; }

    pub fn tick(&mut self, dt: f32, env: &Env) {
        if let Some(t) = self.stop_timer.as_mut() { *t -= dt; if *t <= 0.0 { self.stop_timer = None; if let Some(i) = self.inst.as_mut() { i.deactivate(); } } }
        let xf = self.component_transform();
        if let Some(i) = self.inst.as_mut() { i.xf = xf; i.part_sys_velocity = self.part_sys_velocity; i.tick(dt, env); }
    }

    pub fn draw(&self, camera_game: [f32; 3], out: &mut Vec<EmitterDraw>) { if let Some(i) = &self.inst { i.draw(camera_game, out); } }
    /// Dynamic particle lights of the live flash (`ParticleModuleLight`).
    pub fn lights(&self) -> Vec<ParticleLight> { self.inst.as_ref().map(|i| i.lights()).unwrap_or_default() }
    pub fn instance(&self) -> Option<&Instance> { self.inst.as_ref() }
    pub fn live_particles(&self) -> usize { self.inst.as_ref().map_or(0, |i| i.live_count()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_particles::dist::Dist;
    use fn_particles::model::*;

    fn flash_system() -> Arc<ParticleSystem> {
        let modules: Vec<Module> = [ModuleKind::Lifetime { lifetime: Dist::constant(0.1) }, ModuleKind::Size { start: Dist::constant3([10.0; 3]) }].into_iter().map(|kind| Module { kind, enabled: true, lod_validity: 15, seed: None, name: String::new() }).collect();
        let lod = Lod { level: 0, enabled: true, required: Required { emitter_duration: 0.1, emitter_loops: 1, use_local_space: true, ..Default::default() },
            spawn: Spawn { rate: Dist::constant(0.0), rate_scale: Dist::constant(1.0), burst_scale: Dist::constant(1.0), bursts: vec![Burst { count: 1, count_low: -1, time: 0.0 }] }, type_data: TypeData::Sprite, modules, peak_active: 0 };
        Arc::new(ParticleSystem { name: "P_T".into(), emitters: vec![Emitter { name: "Flash".into(), lods: vec![lod] }], lod_distances: vec![0.0], ..Default::default() })
    }
    fn ctx(scope: bool, now: f32) -> FireContext { FireContext { using_scope: scope, camera_distance_cm: 100.0, now, magazine_ammo: 30, time_to_next_fire: 0.1 } }

    #[test] fn every_shot_restarts_the_flash_and_the_scope_suppresses_it() {
        let mut m = MuzzleFlash::new(flash_system(), 7);
        let env = Env::none();
        assert!(m.on_play_weapon_fire_fx(false, false, &ctx(false, 0.0), Transform::identity()));
        m.tick(1.0 / 60.0, &env); assert_eq!(m.live_particles(), 1);
        for _ in 0..12 { m.tick(1.0 / 60.0, &env); } assert_eq!(m.live_particles(), 0, "the 0.1 s flash is gone");
        // fire again: the same component activates again and the burst fires again
        assert!(m.on_play_weapon_fire_fx(false, false, &ctx(false, 0.2), Transform::identity())); m.tick(1.0 / 60.0, &env); assert_eq!(m.live_particles(), 1);
        // rapid fire: Activate(true) before the previous particle died keeps it and adds the new burst
        assert!(m.on_play_weapon_fire_fx(false, false, &ctx(false, 0.21), Transform::identity())); m.tick(1.0 / 60.0, &env); assert_eq!(m.live_particles(), 2);
        // scoped: nothing plays and nothing changes
        assert!(!m.on_play_weapon_fire_fx(false, false, &ctx(true, 0.5), Transform::identity())); assert_eq!(m.suppressed, 1); assert_eq!(m.played, 3);
    }
    #[test] fn random_scale_follows_add_random_scale() {
        let mut m = MuzzleFlash::new(flash_system(), 99);
        let (mut small, mut big) = (0, 0);
        for i in 0..2000 {
            m.on_play_weapon_fire_fx(false, false, &ctx(false, i as f32), Transform::identity());
            let s = m.rel_scale;
            if (0.7..=1.0).contains(&s) { big += 1 } else if (0.5..=0.6).contains(&s) { small += 1 } else { panic!("scale {s}") }
        }
        let p = small as f32 / 2000.0; assert!((0.15..0.25).contains(&p), "20 % of the shots get the small scale: {p}");
        assert!(big > 1500);
        // persistent fire does not re-roll the scale
        let before = m.rel_scale; m.on_play_weapon_fire_fx(true, true, &ctx(false, 5000.0), Transform::identity()); assert_eq!(m.rel_scale, before);
    }
    #[test] fn persistent_fire_stops_after_ammo_times_fire_interval_and_deactivate_halts_spawning() {
        let mut sys = (*flash_system()).clone(); sys.emitters[0].lods[0].required.emitter_loops = 0; // looping flash
        let mut m = MuzzleFlash::new(Arc::new(sys), 3);
        m.on_play_weapon_fire_fx(true, true, &FireContext { using_scope: false, camera_distance_cm: 0.0, now: 0.0, magazine_ammo: 3, time_to_next_fire: 0.5 }, Transform::identity());
        let env = Env::none();
        for _ in 0..60 { m.tick(1.0 / 60.0, &env); } let n1 = m.instance().unwrap().spawned_total(); assert!(n1 >= 8, "looping: {n1}");
        for _ in 0..60 { m.tick(1.0 / 60.0, &env); } // 1.65 s timer expired at ~1.65 s: still spawning until then
        for _ in 0..60 { m.tick(1.0 / 60.0, &env); } let n2 = m.instance().unwrap().spawned_total(); for _ in 0..60 { m.tick(1.0 / 60.0, &env); } assert_eq!(m.instance().unwrap().spawned_total(), n2, "StopWeaponFireFX deactivated the component");
        assert!(m.instance().unwrap().finished());
    }
}
