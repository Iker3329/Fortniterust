//! Firing a recovered gun: aim source, spread cone, per-pellet trace, damage falloff, zone multipliers, props damaged through their health.
//!
//! The model is the decoded native code of `AFortWeaponRanged` (see `spread.rs` for the spread / bloom / cone / falloff maths and the RVA list); this file is the fire
//! function (RVA 0x933630), the aim source selection (0x942c80) and the trace (0x945cc0, 0x920270) around it, applied to the props of `fn_combat::World`.
//!
//! EXACT (decoded): spread = `mult * heat` degrees (full angle) with the heat / cool curves of the stat row, `FRandomStream` seeded from `rand()` once per fire call,
//! `VRandCone` with half angle `spread * 0.5` rad, first-pellet perfect aim, the trace distance `max(RngMax, RngLong, 100 cm)`, the sweep radius `SweepRadius`, the
//! camera / muzzle origin choice, damage falloff `(0,PB) (RngPB,PB) (RngMid,Mid) (RngLong,Long) (RngMax,Max)`.
//! NOT 1:1 (labelled where used): the sweep against convex-hull props is a line (the sphere radius only inflates placeholder cylinders), the physical-surface -> damage zone
//! (head / body) decision and the gameplay-effect application are not part of the prop world here, `sin` / `cos` / `acos` are the CRT functions of Rust's `std`.
use crate::data::GunHit;
use crate::spread::{safe_normal, to_ue, from_ue, vrand_cone, DamageKind, RandomStream, SpreadState};
use fn_combat::{harvest_fx::FxEvent, Harvestable, Resource, World};
use glam::Vec3;

/// Kept so existing callers still compile: the spread / falloff stand-ins it used to switch on are replaced by the decoded model, so it has no effect any more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy { pub assume_unrecovered: bool }

/// `DamageZone_*` columns: the zone a hit lands in (Critical = head).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone { Light, Normal, Critical, Vulnerability }

/// EPhysicalSurface indices the weapon actor tables are keyed by (sounds: 1 WOOD, 2 STONE, 3 MTL, 4 player; effects: 0 default, 1 wood, 2 stone).
pub const SURFACE_DEFAULT: u32 = 0;
pub const SURFACE_WOOD: u32 = 1;
pub const SURFACE_STONE: u32 = 2;
pub const SURFACE_METAL: u32 = 3;
pub fn surface_of(r: Resource) -> u32 { match r { Resource::Wood => SURFACE_WOOD, Resource::Stone => SURFACE_STONE, Resource::Metal => SURFACE_METAL } }

impl crate::data::StatRow {
    pub fn zone_multiplier(&self, z: Zone) -> f32 { match z { Zone::Light => self.zone_light, Zone::Normal => self.zone_normal, Zone::Critical => self.zone_critical, Zone::Vulnerability => self.zone_vulnerability } }
    /// Trace length in metres: `max(RngMax, RngLong)` floored at 100 cm (RVA 0x933761 `maxss [100.0]`).
    pub fn trace_range_m(&self) -> f32 { self.trace_distance_cm().max(100.) / 100. }
    /// `SweepRadius` (stat+0x238) in metres.
    pub fn sweep_radius_m(&self) -> f32 { self.sweep_radius / 100. }
}

/// The aim of one trigger pull: camera eye + forward (unit), plus the muzzle when the caller knows it (needed for the close-range muzzle correction).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot { pub eye: Vec3, pub forward: Vec3 }

/// What the fire function reads from the pawn / weapon besides the spread state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireContext {
    /// `bIsTargeting` (`[weapon + 0x965]`).
    pub targeting: bool,
    /// The owner's velocity is exactly (0, 0, 0) (`GetVelocity()` compared with `ucomiss`, RVA 0x93b14f..0x93b168).
    pub velocity_zero: bool,
    /// The weapon muzzle in world metres (`[owner + 0x7E0]` aim-source transform with type 4, RVA 0x942dcf); `None` keeps every bullet on the camera ray.
    pub muzzle: Option<Vec3>,
    /// Weapon level (`vtable + 0x878`).
    pub level: i32,
    /// `World->TimeSeconds`.
    pub now: f32,
}
impl Default for FireContext { fn default() -> Self { FireContext { targeting: false, velocity_zero: false, muzzle: None, level: 1, now: 0. } } }

/// One bullet's landing, everything the audio / particle / HUD side needs (hit events).
#[derive(Clone, Debug, PartialEq)]
pub struct Impact {
    pub pos: Vec3, pub normal: Vec3, pub surface: u32, pub distance_m: f32,
    /// Index into `World::props` (None = ground).
    pub prop: Option<usize>, pub object: u64,
    pub damage: f32, pub destroyed: bool, pub zone: Zone,
    /// `ImpactPhysicalSurfaceEffects[surface]` / `ImpactPhysicalSurfaceSounds[surface]` of the weapon actor (package paths).
    pub effect: Option<String>, pub sound: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShotReport { pub impacts: Vec<Impact>, pub pellets: u32, pub misses: u32, /// where each pellet's tracer ends (hit point or max range)
    pub tracer_ends: Vec<Vec3>,
    /// the direction each pellet left along (after the cone), the origin the bullets started from and whether it was the muzzle, the spread (degrees, full angle) that was used
    pub dirs: Vec<Vec3>, pub start: Vec3, pub from_muzzle: bool, pub spread_deg: f32 }

/// A world hit: distance from the trace start (m), surface normal, the prop (None = ground plane).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldHit { pub t: f32, pub normal: Vec3, pub prop: Option<usize> }

/// Nearest prop along the ray: (index, distance, normal). `radius` = sweep sphere radius (m): inflates the placeholder cylinders; the real convex hulls are traced as a line.
pub fn trace_props_swept(props: &[Harvestable], origin: Vec3, dir: Vec3, max_m: f32, radius: f32) -> Option<(usize, f32, Vec3)> {
    let mut best: Option<(usize, f32, Vec3)> = None;
    for (i, p) in props.iter().enumerate() {
        if !p.alive() { continue; }
        if let Some(h) = &p.hull {
            if h.segment_clearance(origin, origin + dir * max_m) > 0. { continue; }
            for piece in &h.hulls {
                if let Some((t, n)) = piece.ray(origin, dir, max_m) { if best.map_or(true, |b| t < b.1) { best = Some((i, t, n)); } }
            }
            continue;
        }
        // placeholder vertical cylinder (exact ray / cylinder side + cap test), radius inflated by the sweep radius
        let r = p.radius + radius;
        let (ox, oz) = (origin.x - p.pos.x, origin.z - p.pos.z);
        let (a, b, c) = (dir.x * dir.x + dir.z * dir.z, 2. * (ox * dir.x + oz * dir.z), ox * ox + oz * oz - r * r);
        let (y0, y1) = (p.pos.y, p.pos.y + p.height);
        let mut hit: Option<(f32, Vec3)> = None;
        if a > 1e-9 {
            let disc = b * b - 4. * a * c;
            if disc >= 0. { let t = (-b - disc.sqrt()) / (2. * a); let y = origin.y + dir.y * t;
                if t >= 0. && t <= max_m && y >= y0 && y <= y1 { let q = origin + dir * t; hit = Some((t, Vec3::new(q.x - p.pos.x, 0., q.z - p.pos.z).normalize_or_zero())); } }
        }
        if dir.y.abs() > 1e-9 { for (yy, n) in [(y1, Vec3::Y), (y0, -Vec3::Y)] { let t = (yy - origin.y) / dir.y; if t >= 0. && t <= max_m && hit.map_or(true, |h| t < h.0) { let q = origin + dir * t; if (q.x - p.pos.x).powi(2) + (q.z - p.pos.z).powi(2) <= r * r { hit = Some((t, n)); } } } }
        if let Some((t, n)) = hit { if best.map_or(true, |bb| t < bb.1) { best = Some((i, t, n)); } }
    }
    best
}
/// Line trace against the props (`trace_props_swept` with radius 0).
pub fn trace_props(props: &[Harvestable], origin: Vec3, dir: Vec3, max_m: f32) -> Option<(usize, f32, Vec3)> { trace_props_swept(props, origin, dir, max_m, 0.) }

/// `WeaponTrace` (RVA 0x945cc0): nearest blocking hit of the props and the ground plane (y = 0) within `max_m`.
pub fn trace_world(props: &[Harvestable], origin: Vec3, dir: Vec3, max_m: f32, sweep_m: f32) -> Option<WorldHit> {
    let prop = trace_props_swept(props, origin, dir, max_m, sweep_m);
    let ground = if dir.y < -1e-6 && origin.y > 0. { let t = -origin.y / dir.y; (t <= max_m).then_some(t) } else { None };
    match (prop, ground) {
        (Some((i, t, n)), g) if g.map_or(true, |gt| t <= gt) => Some(WorldHit { t, normal: n, prop: Some(i) }),
        (_, Some(t)) => Some(WorldHit { t, normal: Vec3::Y, prop: None }),
        _ => None,
    }
}

/// Where the bullets of this trigger pull start and which way they point before the spread: RVA 0x942c80 (`vtable + 0x858`) stores them in `CurrentDamageStartLocation`
/// (+0x93C) / `CurrentAdjustedAimDirection` (+0x948), the fire function reads them through `vtable + 0x668` (RVA 0x9245e0).
///
/// 1. `hit1` = trace from the camera along its forward over `D = max(RngMax, RngLong)`; `target` = its hit location or the far end.
/// 2. `adj = SafeNormal(target - muzzle)`; `hit2` = trace from the muzzle along `adj` for 650 cm.
/// 3. The CAMERA ray is the default. The MUZZLE ray (`start = muzzle`, `dir = adj`) is used when the muzzle trace is blocked, `bShouldAimFromMuzzleAtCloseRange` is set
///    and the weapon is not a scoped weapon that is targeting; the other branch of the exe (a hit actor of an enemy team on both traces) needs pawns, which the prop world has not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimSelection { pub start: Vec3, pub dir: Vec3, pub from_muzzle: bool, pub muzzle_blocked_at: Option<Vec3> }
pub fn select_aim(gun: &GunHit, props: &[Harvestable], eye: Vec3, forward: Vec3, muzzle: Option<Vec3>, targeting: bool) -> AimSelection {
    let camera = AimSelection { start: eye, dir: forward, from_muzzle: false, muzzle_blocked_at: None };
    let Some(m) = muzzle else { return camera };
    let sweep = gun.stats.sweep_radius_m();
    let d_m = gun.stats.trace_distance_cm() / 100.;
    let hit1 = trace_world(props, eye, forward, d_m, sweep);
    let target = hit1.map_or(eye + forward * d_m, |h| eye + forward * h.t);
    let adj = safe_normal(target - m);
    let hit2 = trace_world(props, m, adj, 6.5, sweep);
    let close_range_ok = gun.pres.aim_from_muzzle_at_close_range && (!targeting || !gun.pres.use_scope_targeting);
    let Some(h2) = hit2 else { return camera };
    if !close_range_ok { return camera; }
    let loc2 = m + adj * h2.t;
    let loc1 = hit1.map_or(Vec3::ZERO, |h| eye + forward * h.t);
    // 100 cm^2 = (10 cm)^2 -> 0.01 m^2
    let blocked = ((loc2 - loc1).length_squared() > 0.01).then_some(loc2);
    AimSelection { start: m, dir: adj, from_muzzle: true, muzzle_blocked_at: blocked }
}

/// `FRotator(Pitch + offset, Yaw, 0).Vector()` of a direction (the projectile branch of the fire function adds `ProjectilePitchOffset`, RVA 0x933e09..0x933e49).
pub fn add_pitch_offset(dir: Vec3, offset_deg: f32) -> Vec3 {
    if offset_deg == 0. { return dir; }
    let d = to_ue(dir);
    let yaw = d.y.atan2(d.x); let pitch = d.z.atan2((d.x * d.x + d.y * d.y).sqrt()) + offset_deg.to_radians();
    let (sy, cy) = yaw.sin_cos(); let (sp, cp) = pitch.sin_cos();
    from_ue(Vec3::new(cp * cy, cp * sy, sp))
}

/// Half cone angle (radians) of a pellet: `spread * 0.5 * (PI/180)` as compiled (0x933954..0x933959: `*0.5`, `*0.017453292`).
pub fn half_angle_rad(spread_deg: f32) -> f32 { spread_deg * 0.5 * 0.017453292 }

/// Direction of pellet `index` of a cartridge of `bullets`: perfect aim (no stream draw) or `VRandCone` around `aim`.
fn pellet_dir(gun: &GunHit, spread: &SpreadState, ctx: &FireContext, aim: Vec3, index: u32, rs: &mut RandomStream) -> Vec3 {
    let bullets = gun.stats.bullets_per_cartridge.max(1);
    if spread.perfect_aim(index, bullets, gun.perfect_aim_when_targeting_min_spread, ctx.velocity_zero, ctx.targeting) { return aim; }
    let half = half_angle_rad(spread.spread_deg(0.));
    if half > 0. { vrand_cone(aim, half, half, rs) } else { safe_normal(aim) }
}

/// Applies one cartridge (`BulletsPerCartridge` pellets) to the world, RVA 0x933630: `seed` is the CRT `rand()` value that seeds the `FRandomStream` (one per call), every
/// pellet leaves the chosen aim ray inside the spread cone (the first pellet of a multi-pellet cartridge exactly on it), the nearest prop / the ground takes the hit, props lose
/// `EnvDmg` at that range (through `Harvestable::health`; a destroyed prop emits `FxEvent::Broke`), `LastFireTime` is stamped per pellet and the heat is added once at the end.
pub fn fire(gun: &GunHit, world: &mut World, shot: &Shot, spread: &mut SpreadState, ctx: &FireContext, seed: i32) -> ShotReport {
    let s = &gun.stats;
    let bullets = s.bullets_per_cartridge.max(1);
    let max_m = s.trace_range_m();
    let sweep = s.sweep_radius_m();
    let aim = select_aim(gun, &world.props, shot.eye, shot.forward, ctx.muzzle, ctx.targeting);
    let mut rs = RandomStream::new(seed);
    let mut rep = ShotReport { pellets: bullets, start: aim.start, from_muzzle: aim.from_muzzle, spread_deg: spread.spread_deg(0.), ..Default::default() };
    for index in 0..bullets {
        let dir = pellet_dir(gun, spread, ctx, aim.dir, index, &mut rs);
        spread.mark_fired(ctx.now);
        rep.dirs.push(dir);
        if dir == Vec3::ZERO { rep.misses += 1; continue; }
        match trace_world(&world.props, aim.start, dir, max_m, sweep) {
            Some(WorldHit { t, normal, prop: Some(i) }) => {
                let dmg = s.damage_at(DamageKind::Environmental, t * 100., ctx.level) * s.zone_multiplier(Zone::Normal);
                let pos = aim.start + dir * t;
                let p: &mut Harvestable = &mut world.props[i];
                p.health -= dmg;
                let destroyed = p.health <= 0.;
                if destroyed { p.gone_for = 0.; }
                let object = fn_combat::harvest_fx::object_id(i, p);
                let (surface, kind, res, proto, ppos) = (surface_of(p.res), p.kind, p.res, p.proto, p.pos);
                if destroyed { world.fx.events.push(FxEvent::Broke { object, pos: ppos, kind, resource: res, proto }); }
                world.hits += 1;
                rep.tracer_ends.push(pos);
                rep.impacts.push(Impact { pos, normal, surface, distance_m: t, prop: Some(i), object, damage: dmg, destroyed, zone: Zone::Normal,
                    effect: gun.pres.impact_effect(surface).map(str::to_owned), sound: gun.pres.impact_sound(surface).map(str::to_owned) });
            }
            Some(WorldHit { t, prop: None, .. }) => {
                let pos = aim.start + dir * t;
                rep.tracer_ends.push(pos);
                rep.impacts.push(Impact { pos, normal: Vec3::Y, surface: SURFACE_DEFAULT, distance_m: t, prop: None, object: 0, damage: 0., destroyed: false, zone: Zone::Normal,
                    effect: gun.pres.impact_effect(SURFACE_DEFAULT).map(str::to_owned), sound: gun.pres.impact_sound(SURFACE_DEFAULT).map(str::to_owned) });
            }
            None => { rep.misses += 1; rep.tracer_ends.push(aim.start + dir * max_m); }
        }
    }
    spread.add_heat(s, ctx.level); // RVA 0x917e40, after the pellet loop
    rep
}

/// A fired projectile (bolt-action sniper: `ProjectileTemplate`): launched along the (spread) aim at the data's `InitialSpeed`, falling under `gravity_m_s2`
/// (engine `ProjectileMovementComponent` default gravity scale; the world value is the movement crate's). `MaxSpeed` equals `InitialSpeed` in the data, so speed is constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projectile { pub pos: Vec3, pub vel: Vec3, pub age: f32, pub travelled: f32 }
impl Projectile {
    /// The projectile branch of the fire function (RVA 0x933630): same aim selection and cone as a bullet, pitch raised by `ProjectilePitchOffset`, then `LastFireTime` + heat.
    pub fn launch(gun: &GunHit, world: &World, shot: &Shot, spread: &mut SpreadState, ctx: &FireContext, seed: i32) -> Option<Projectile> {
        let pr = gun.projectile.as_ref()?;
        let aim = select_aim(gun, &world.props, shot.eye, shot.forward, ctx.muzzle, ctx.targeting);
        let mut rs = RandomStream::new(seed);
        let bullets = gun.stats.bullets_per_cartridge.max(1);
        let mut dir = Vec3::ZERO;
        for index in 0..bullets { dir = pellet_dir(gun, spread, ctx, aim.dir, index, &mut rs); spread.mark_fired(ctx.now); }
        spread.add_heat(&gun.stats, ctx.level);
        let dir = add_pitch_offset(dir, gun.pres.projectile_pitch_offset);
        Some(Projectile { pos: aim.start, vel: dir * (pr.initial_speed / 100.).min(pr.max_speed / 100.), age: 0., travelled: 0. })
    }
    /// Advances `dt`; returns the impact (prop damage applied like `fire`) when the step's segment hits a prop or the ground. `max_flight_m`: despawn distance (native, caller's).
    pub fn step(&mut self, gun: &GunHit, world: &mut World, dt: f32, gravity_m_s2: f32, max_flight_m: f32, level: i32) -> Option<Impact> {
        let s = &gun.stats;
        self.vel.y -= gravity_m_s2 * dt;
        let seg = self.vel * dt; let len = seg.length();
        if len <= 0. { return None; }
        let dir = seg / len;
        let hit = trace_world(&world.props, self.pos, dir, len, 0.);
        let travelled = self.travelled;
        self.age += dt; self.travelled += len;
        let at = |t: f32| self.pos + dir * t;
        let result = match hit {
            Some(WorldHit { t, normal, prop: Some(i) }) => {
                let range = travelled + t;
                let dmg = s.damage_at(DamageKind::Environmental, range * 100., level);
                let p = &mut world.props[i];
                p.health -= dmg; let destroyed = p.health <= 0.; if destroyed { p.gone_for = 0.; }
                let object = fn_combat::harvest_fx::object_id(i, p);
                let (surface, kind, res, proto, ppos) = (surface_of(p.res), p.kind, p.res, p.proto, p.pos);
                if destroyed { world.fx.events.push(FxEvent::Broke { object, pos: ppos, kind, resource: res, proto }); }
                world.hits += 1;
                Some(Impact { pos: at(t), normal, surface, distance_m: range, prop: Some(i), object, damage: dmg, destroyed, zone: Zone::Normal,
                    effect: gun.pres.impact_effect(surface).map(str::to_owned), sound: gun.pres.impact_sound(surface).map(str::to_owned) })
            }
            Some(WorldHit { t, prop: None, .. }) => Some(Impact { pos: at(t), normal: Vec3::Y, surface: SURFACE_DEFAULT, distance_m: travelled + t, prop: None, object: 0, damage: 0., destroyed: false, zone: Zone::Normal,
                effect: gun.pres.impact_effect(SURFACE_DEFAULT).map(str::to_owned), sound: gun.pres.impact_sound(SURFACE_DEFAULT).map(str::to_owned) }),
            None => None,
        };
        self.pos += seg;
        if result.is_none() && self.travelled > max_flight_m { self.vel = Vec3::ZERO; }
        result
    }
    pub fn dead(&self) -> bool { self.vel == Vec3::ZERO }
}

/// World gravity for projectiles in m/s^2: `DefaultGravityZ` (-2800 cm/s^2, PROVENANCE: decrypted config, not independently verified; exe default -980; via fn-movement) x the engine `GravityScale`/`ProjectileGravityScale` default 1.
pub fn projectile_gravity_m_s2() -> f32 { -fn_movement::params::WORLD_GRAVITY_Z / 100. }

