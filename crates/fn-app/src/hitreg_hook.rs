//! HITREG HOOK (fn-app): glue between the arms (shots fired) and `fn-hitreg` (trace / spread / damage / prop health). The LOGIC is `fn_hitreg`; this file loads the three
//! firearms' hit data from the install on a worker thread, turns every shot the arms report into a cartridge fired from the camera ray at the props of `sim.world`, flies
//! sniper projectiles, applies the recovered recoil (`fn_weapons::recovered_recoil`, RECOIL HOOK) to the look delta and queues the impacts (`take_impacts`) for audio / particles.
use crate::arms::Arms;
use fn_game::{FrameInput, GameSim};
use fn_hitreg::{fire, load, AimState, GunHit, Impact, Policy, Projectile, Shot};
use fn_weapons::recovered_recoil::{CrtRand, RecoilState, RecoilStats}; // RECOIL HOOK: recovered native recoil (fn-weapons/RECOIL.md)
use std::sync::Mutex;

struct State { guns: Vec<GunHit>, rng: fn_combat::harvest_fx::Rng, recoil: [RecoilState; 3], crt: CrtRand, now: f32, last: [u32; 3], flying: Vec<(usize, Projectile)> }
static GUNS: Mutex<Option<Vec<GunHit>>> = Mutex::new(None);
static STARTED: Mutex<bool> = Mutex::new(false);
static STATE: Mutex<Option<State>> = Mutex::new(None);
static IMPACTS: Mutex<Vec<(usize, Impact)>> = Mutex::new(Vec::new());

/// Impacts since the last call: (gun index 0..3, impact). `Impact::effect` / `sound` are the weapon actor's `ImpactPhysicalSurfaceEffects` / `Sounds` package paths.
pub fn take_impacts() -> Vec<(usize, Impact)> { IMPACTS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default() }

fn start(install: std::path::PathBuf, wids: Vec<String>) {
    let Ok(mut s) = STARTED.lock() else { return };
    if *s { return; }
    *s = true;
    std::thread::spawn(move || {
        let paks = match fn_pak::PakSet::open_install(&install) { Ok(p) => p, Err(e) => { eprintln!("[hitreg] unavailable: {e}"); return } };
        let src = fn_pose::ClipSourceDir::Pak(&paks);
        let mut guns = vec![];
        for w in &wids { match load(&src, w) { Ok(g) => guns.push(g), Err(e) => { eprintln!("[hitreg] {w}: {e}"); return } } }
        for g in &guns { eprintln!("[hitreg] {} row {}: trace {} m, spread {} deg x multipliers, EnvDmg {} / DmgPB {}, headshot x{}, {} pellet(s){}", g.wid.rsplit('/').next().unwrap_or(""), g.row, g.stats.trace_range_m(), g.stats.spread, g.stats.env_dmg[0], g.stats.dmg[0], g.stats.zone_critical, g.stats.bullets_per_cartridge, g.projectile.as_ref().map_or(String::new(), |p| format!(", projectile {} cm/s", p.initial_speed))); }
        if let Ok(mut slot) = GUNS.lock() { *slot = Some(guns); }
    });
}

/// Once per frame after `arms.update` (the first call starts the load): `aiming` = the ADS driver's state.
static INSTALL: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);
/// The game install (called once when the loadout resources arrive).
pub fn set_install(dir: Option<std::path::PathBuf>) { if let Ok(mut i) = INSTALL.lock() { *i = dir; } }
/// RECOIL HOOK: `inp` = this frame's look input (the `DeltaRot` of `AFortPlayerController::UpdateRotation`), `gamepad` = `IsUsingGamepad`.
pub fn on_arms(sim: &mut GameSim, arms: &Arms, aiming: bool, dt: f32, inp: &mut FrameInput, gamepad: bool) {
    let install = INSTALL.lock().ok().and_then(|i| i.clone());
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if st.is_none() {
        if let Some(dir) = install { start(dir, arms.data.guns.iter().map(|g| g.def.item.clone()).collect()); }
        let Some(guns) = GUNS.lock().ok().and_then(|mut g| g.take()) else { return };
        *st = Some(State { guns, rng: fn_combat::harvest_fx::Rng(0x5eed_f00d), recoil: [RecoilState::default(); 3], crt: CrtRand::new(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos())), now: 0., last: arms.shots, flying: vec![] });
    }
    let s = st.as_mut().expect("loaded");
    let policy = Policy { assume_unrecovered: arms.policy.assume_unrecovered };
    let c = &sim.controller;
    let state = AimState { ads: aiming, crouched: c.crouched, airborne: c.airborne(), sprinting: c.running, speed_cm_s: c.velocity.length() * 100. };
    let shot = Shot { eye: sim.cam_view.eye, forward: sim.cam_view.forward };
    s.now += dt; // World->TimeSeconds
    for g in 0..3 {
        let n = arms.shots[g].saturating_sub(s.last[g]);
        s.last[g] = arms.shots[g];
        for _ in 0..n {
            let gun = &s.guns[g];
            // RECOIL HOOK: AddRecoil (RVA 0x917930) once per shot; LastFireTime = TimeSeconds
            let crt = &mut s.crt; s.recoil[g].on_fire(s.now); s.recoil[g].add_recoil(&recoil_stats(&gun.stats), aiming, gamepad, ATHENA_RECOIL, &mut || crt.next());
            if let Some(p) = Projectile::launch(gun, &shot, &state, policy, &mut s.rng) { s.flying.push((g, p)); continue; }
            let rep = fire(gun, &mut sim.world, &shot, &state, policy, &mut s.rng);
            eprintln!("[hitreg f{:05}] {} fired {} pellet(s): {} impact(s) {} miss(es){}", sim.frames, gun.row, rep.pellets, rep.impacts.len(), rep.misses,
                rep.impacts.first().map_or(String::new(), |i| format!("; first {:.1} m prop {:?} surface {} damage {} destroyed {}", i.distance_m, i.prop, i.surface, i.damage, i.destroyed)));
            if let Ok(mut q) = IMPACTS.lock() { q.extend(rep.impacts.into_iter().map(|i| (g, i))); }
        }
    }
    // sniper projectiles (fixed 1/120 s sub-steps)
    let (grav, mut left) = (fn_hitreg::shoot::projectile_gravity_m_s2(), dt);
    while left > 1e-6 && !s.flying.is_empty() {
        let h = left.min(1. / 120.); left -= h;
        let mut done = vec![];
        for (i, (g, p)) in s.flying.iter_mut().enumerate() {
            let gun = &s.guns[*g];
            if let Some(imp) = p.step(gun, &mut sim.world, h, grav, 1000., policy) {
                eprintln!("[hitreg f{:05}] {} projectile hit {:.1} m prop {:?} surface {} damage {} destroyed {}", sim.frames, gun.row, imp.distance_m, imp.prop, imp.surface, imp.damage, imp.destroyed);
                if let Ok(mut q) = IMPACTS.lock() { q.push((*g, imp)); }
                done.push(i);
            } else if p.dead() { done.push(i); }
        }
        for i in done.into_iter().rev() { s.flying.swap_remove(i); }
    }
    // RECOIL HOOK: UpdateRecoil (vtable +0x760, RVA 0x936200) of the HELD firearm adds its step to the look delta; sim.begin_frame applies the delta to the view
    if let Some(g) = arms.gun_index() {
        let mut d = [inp.pitch_delta, inp.yaw_delta, 0.];
        s.recoil[g].update(&recoil_stats(&s.guns[g].stats), dt, &mut d, s.now, 1., ATHENA_RECOIL);
        inp.pitch_delta = d[0]; inp.yaw_delta = d[1];
    }
}

/// RECOIL HOOK: `Weapon.UseAthenaRecoil`. The exe default is 0 (`fn_weapons::recovered_recoil::USE_ATHENA_RECOIL_EXE_DEFAULT`, legacy columns); the Battle Royale build uses the
/// Athena path (user decision 2026-10-04: the `AthenaRecoil*` columns are authored per BR weapon row and the cvar is named for Athena; the encrypted
/// engine config that would set it is not read, see fn-weapons/RECOIL.md).
const ATHENA_RECOIL: bool = true;
fn recoil_stats(s: &fn_hitreg::data::StatRow) -> RecoilStats {
    RecoilStats { vert: s.recoil_vert, vert_scale: s.recoil_vert_scale, vert_scale_gamepad: s.recoil_vert_scale_gamepad, vert_down_chance: s.vert_recoil_down_chance, horiz: s.recoil_horiz, horiz_scale: s.recoil_horiz_scale, horiz_scale_gamepad: s.recoil_horiz_scale_gamepad,
        interp_speed: s.recoil_interp_speed, recovery_interp_speed: s.recoil_recovery_interp_speed, recovery_delay: s.recoil_recovery_delay, recovery_fraction: s.recoil_recovery_fraction, downsights_mult: s.recoil_downsights_mult,
        athena_mag_min: s.athena_recoil_magnitude.0, athena_mag_max: s.athena_recoil_magnitude.1, athena_mag_scale: s.athena_recoil_magnitude_scale, athena_angle_min: s.athena_recoil_angle.0, athena_angle_max: s.athena_recoil_angle.1,
        athena_roll_min: s.athena_recoil_roll.0, athena_roll_max: s.athena_recoil_roll.1, athena_interp_speed: s.athena_recoil_interp_speed, athena_recovery_interp_speed: s.athena_recoil_recovery_interp_speed, athena_downsights_mult: s.athena_recoil_downsights_mult }
}
