//! Firing a recovered gun: spread, recoil, range, damage falloff, zone multipliers, the trace against props, damage through the existing prop health.
//!
//! EXACT (the numbers and their names come from the 1.8 stat row / Blueprints, see `data.rs`): which multiplier belongs to which state, the damage + range points,
//! the zone multipliers, the trace length column, the surface -> effect / sound tables, EnvDmg* for props.
//! NATIVE-ONLY (C++ in `FortWeaponRanged`, not in any package; `Policy::assume_unrecovered` switches the labelled stand-ins on for development only, default OFF = left out):
//!  * how the spread multipliers combine and the unit of `Spread` (stand-in: product of the active multipliers, degrees, half angle, uniform in the cone),
//!  * the spread HEAT / COOL curve model (`HeatX/Y`, `CoolX/Y`): not implemented at all,
//!  * the damage interpolation between the (range, damage) points (stand-in: piecewise linear; default: `DmgPB`/`EnvDmgPB` at every range),
//!  * the recoil integrator (`AthenaRecoil*`: stand-in below) and its camera application; the cvars `Weapon.UseAthenaRecoil` / `Weapon.UseAthenaFalloff` default to 0 in the exe
//!    image (read by fn-exe-defaults) and the configs that could flip them are encrypted,
//!  * the close-range muzzle aim distance of `bShouldAimFromMuzzleAtCloseRange` (not serialized) -> bullets leave the camera ray unless the caller asks for a muzzle.
use crate::data::{GunHit, StatRow};
use fn_combat::{harvest_fx::{FxEvent, Rng}, Harvestable, Resource, World};
use glam::Vec3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy { pub assume_unrecovered: bool }

/// Pawn state the stat-row multipliers key on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AimState { pub ads: bool, pub crouched: bool, pub airborne: bool, pub sprinting: bool, /// horizontal speed, cm/s
    pub speed_cm_s: f32 }

/// `DamageZone_*` columns: the zone a hit lands in (Critical = head).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone { Light, Normal, Critical, Vulnerability }

/// EPhysicalSurface indices the weapon actor tables are keyed by (sounds: 1 WOOD, 2 STONE, 3 MTL, 4 player; effects: 0 default, 1 wood, 2 stone).
pub const SURFACE_DEFAULT: u32 = 0;
pub const SURFACE_WOOD: u32 = 1;
pub const SURFACE_STONE: u32 = 2;
pub const SURFACE_METAL: u32 = 3;
pub fn surface_of(r: Resource) -> u32 { match r { Resource::Wood => SURFACE_WOOD, Resource::Stone => SURFACE_STONE, Resource::Metal => SURFACE_METAL } }

impl StatRow {
    pub fn zone_multiplier(&self, z: Zone) -> f32 { match z { Zone::Light => self.zone_light, Zone::Normal => self.zone_normal, Zone::Critical => self.zone_critical, Zone::Vulnerability => self.zone_vulnerability } }
    /// Trace length in metres: the `RngMax` column (cm).
    pub fn trace_range_m(&self) -> f32 { self.rng[3] / 100. }
    /// Spread multiplier for a state. Which columns apply to which state is the data; combining them is a stand-in (product). Default policy: 1 (no spread logic).
    /// "Standing still" applies below `MinSpeedForSpreadMultiplier` and blends to 1 at `MaxSpeedForSpreadMultiplier` (the column names; the blend shape is a stand-in).
    pub fn spread_multiplier(&self, s: &AimState, policy: Policy) -> f32 {
        if !policy.assume_unrecovered { return 1.; }
        let mut m = 1.;
        if s.ads { m *= self.spread_downsights; }
        if s.crouched { m *= self.crouching_spread_mult; }
        if s.airborne { m *= self.jumping_falling_spread_mult; }
        if s.sprinting { m *= self.sprinting_spread_mult; }
        let (lo, hi) = (self.min_speed_for_spread_mult, self.max_speed_for_spread_mult);
        let still = if s.speed_cm_s <= lo { 1. } else if s.speed_cm_s >= hi || hi <= lo { 0. } else { 1. - (s.speed_cm_s - lo) / (hi - lo) };
        m * (1. + (self.standing_still_spread_mult - 1.) * still)
    }
    /// Half angle of the spread cone in degrees (stand-in unit), 0 without the assume policy.
    pub fn spread_deg(&self, s: &AimState, policy: Policy) -> f32 { if !policy.assume_unrecovered { 0. } else { self.spread * self.spread_multiplier(s, policy) } }
    /// Damage at `range_cm` from one of the four point tables (`dmg`, `env_dmg`, `impact_dmg`) against the ranges `rng`. The point-blank value holds up to `RngPB` (data);
    /// beyond it the interpolation is native: default keeps the point-blank value, the stand-in is piecewise linear through the points.
    pub fn damage_at(&self, table: &[f32; 4], range_cm: f32, policy: Policy) -> f32 {
        let r = self.rng;
        if range_cm <= r[0] || !policy.assume_unrecovered { return table[0]; }
        for i in 0..3 {
            if range_cm <= r[i + 1] { let span = (r[i + 1] - r[i]).max(1e-6); return table[i] + (table[i + 1] - table[i]) * ((range_cm - r[i]) / span); }
        }
        table[3]
    }
}

/// Uniform direction inside a cone of half angle `deg` around `dir`.
pub fn spread_direction(dir: Vec3, deg: f32, rng: &mut Rng) -> Vec3 {
    let d = dir.normalize_or_zero();
    if deg <= 0. || d == Vec3::ZERO { return d; }
    let up = if d.y.abs() < 0.99 { Vec3::Y } else { Vec3::X };
    let right = d.cross(up).normalize(); let up = right.cross(d);
    let (theta, phi) = (deg.to_radians() * rng.unit().sqrt(), rng.unit() * std::f32::consts::TAU);
    (d * theta.cos() + (right * phi.cos() + up * phi.sin()) * theta.sin()).normalize()
}

/// Athena recoil (stand-in, `Policy::assume_unrecovered` only): every shot adds a kick of `AthenaRecoilMagnitude` degrees at an angle from vertical drawn between
/// `AthenaRecoilAngleMin/Max`, scaled by `AthenaRecoilDownsightsMultiplier` while aiming; the shown offset follows the target at `AthenaRecoilInterpSpeed`, the target
/// returns to 0 at `AthenaRecoilRecoveryInterpSpeed` (`FInterpTo`). Offsets are (pitch up, yaw right) in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Recoil { pub target: (f32, f32), pub current: (f32, f32) }
fn interp_to(cur: f32, tgt: f32, dt: f32, speed: f32) -> f32 { if speed <= 0. { return tgt; } let d = tgt - cur; if d * d < 1e-8 { tgt } else { cur + d * (dt * speed).clamp(0., 1.) } }
impl Recoil {
    pub fn kick(&mut self, s: &StatRow, ads: bool, rng: &mut Rng, policy: Policy) {
        if !policy.assume_unrecovered { return; }
        let mag = rng.range(s.athena_recoil_magnitude.0, s.athena_recoil_magnitude.1) * s.athena_recoil_magnitude_scale * if ads { s.athena_recoil_downsights_mult } else { 1. };
        let a = rng.range(s.athena_recoil_angle.0, s.athena_recoil_angle.1).to_radians();
        self.target.0 += mag * a.cos(); self.target.1 += mag * a.sin();
    }
    pub fn update(&mut self, s: &StatRow, dt: f32, firing: bool, policy: Policy) {
        if !policy.assume_unrecovered { return; }
        if !firing { self.target = (interp_to(self.target.0, 0., dt, s.athena_recoil_recovery_interp_speed), interp_to(self.target.1, 0., dt, s.athena_recoil_recovery_interp_speed)); }
        self.current = (interp_to(self.current.0, self.target.0, dt, s.athena_recoil_interp_speed), interp_to(self.current.1, self.target.1, dt, s.athena_recoil_interp_speed));
    }
}

/// The aim of one trigger pull: camera eye + forward (unit); `muzzle` only when the caller wants the close-range muzzle correction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot { pub eye: Vec3, pub forward: Vec3 }

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
    pub tracer_ends: Vec<Vec3> }

/// Nearest prop along the ray: (index, distance, normal).
pub fn trace_props(props: &[Harvestable], origin: Vec3, dir: Vec3, max_m: f32) -> Option<(usize, f32, Vec3)> {
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
        // placeholder vertical cylinder (exact ray / cylinder side + cap test)
        let (ox, oz) = (origin.x - p.pos.x, origin.z - p.pos.z);
        let (a, b, c) = (dir.x * dir.x + dir.z * dir.z, 2. * (ox * dir.x + oz * dir.z), ox * ox + oz * oz - p.radius * p.radius);
        let (y0, y1) = (p.pos.y, p.pos.y + p.height);
        let mut hit: Option<(f32, Vec3)> = None;
        if a > 1e-9 {
            let disc = b * b - 4. * a * c;
            if disc >= 0. { let t = (-b - disc.sqrt()) / (2. * a); let y = origin.y + dir.y * t;
                if t >= 0. && t <= max_m && y >= y0 && y <= y1 { let q = origin + dir * t; hit = Some((t, Vec3::new(q.x - p.pos.x, 0., q.z - p.pos.z).normalize_or_zero())); } }
        }
        if dir.y.abs() > 1e-9 { for (yy, n) in [(y1, Vec3::Y), (y0, -Vec3::Y)] { let t = (yy - origin.y) / dir.y; if t >= 0. && t <= max_m && hit.map_or(true, |h| t < h.0) { let q = origin + dir * t; if (q.x - p.pos.x).powi(2) + (q.z - p.pos.z).powi(2) <= p.radius * p.radius { hit = Some((t, n)); } } } }
        if let Some((t, n)) = hit { if best.map_or(true, |bb| t < bb.1) { best = Some((i, t, n)); } }
    }
    best
}

/// Applies one cartridge (`BulletsPerCartridge` pellets) to the world: pellets leave the camera ray inside the spread cone, the nearest prop / the ground takes the hit,
/// props lose `EnvDmg*` at that range (through `Harvestable::health`, a destroyed prop emits `FxEvent::Broke` for the existing break feedback), impacts come back for audio / particles.
pub fn fire(gun: &GunHit, world: &mut World, shot: &Shot, state: &AimState, policy: Policy, rng: &mut Rng) -> ShotReport {
    let s = &gun.stats;
    let mut rep = ShotReport { pellets: s.bullets_per_cartridge.max(1), ..Default::default() };
    let max_m = s.trace_range_m();
    let deg = s.spread_deg(state, policy);
    for _ in 0..rep.pellets {
        let dir = spread_direction(shot.forward, deg, rng);
        if dir == Vec3::ZERO { rep.misses += 1; continue; }
        let prop = trace_props(&world.props, shot.eye, dir, max_m);
        let ground = if dir.y < -1e-6 && shot.eye.y > 0. { let t = -shot.eye.y / dir.y; (t <= max_m).then_some(t) } else { None };
        match (prop, ground) {
            (Some((i, t, n)), g) if g.map_or(true, |gt| t <= gt) => {
                let dmg = s.damage_at(&s.env_dmg, t * 100., policy) * s.zone_multiplier(Zone::Normal);
                let pos = shot.eye + dir * t;
                let p: &mut Harvestable = &mut world.props[i];
                p.health -= dmg;
                let destroyed = p.health <= 0.;
                if destroyed { p.gone_for = 0.; }
                let object = fn_combat::harvest_fx::object_id(i, p);
                let (surface, kind, res, proto, ppos) = (surface_of(p.res), p.kind, p.res, p.proto, p.pos);
                if destroyed { world.fx.events.push(FxEvent::Broke { object, pos: ppos, kind, resource: res, proto }); }
                world.hits += 1;
                rep.tracer_ends.push(pos);
                rep.impacts.push(Impact { pos, normal: n, surface, distance_m: t, prop: Some(i), object, damage: dmg, destroyed, zone: Zone::Normal,
                    effect: gun.pres.impact_effect(surface).map(str::to_owned), sound: gun.pres.impact_sound(surface).map(str::to_owned) });
            }
            (_, Some(t)) => {
                let pos = shot.eye + dir * t;
                rep.tracer_ends.push(pos);
                rep.impacts.push(Impact { pos, normal: Vec3::Y, surface: SURFACE_DEFAULT, distance_m: t, prop: None, object: 0, damage: 0., destroyed: false, zone: Zone::Normal,
                    effect: gun.pres.impact_effect(SURFACE_DEFAULT).map(str::to_owned), sound: gun.pres.impact_sound(SURFACE_DEFAULT).map(str::to_owned) });
            }
            _ => { rep.misses += 1; rep.tracer_ends.push(shot.eye + dir * max_m); }
        }
    }
    rep
}


/// A fired projectile (bolt-action sniper: `ProjectileTemplate`): launched along the (spread) aim at the data's `InitialSpeed`, falling under `gravity_m_s2`
/// (engine `ProjectileMovementComponent` default gravity scale; the world value is the movement crate's). `MaxSpeed` equals `InitialSpeed` in the data, so speed is constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projectile { pub pos: Vec3, pub vel: Vec3, pub age: f32, pub travelled: f32 }
impl Projectile {
    pub fn launch(gun: &GunHit, shot: &Shot, state: &AimState, policy: Policy, rng: &mut Rng) -> Option<Projectile> {
        let pr = gun.projectile.as_ref()?;
        let dir = spread_direction(shot.forward, gun.stats.spread_deg(state, policy), rng);
        Some(Projectile { pos: shot.eye, vel: dir * (pr.initial_speed / 100.).min(pr.max_speed / 100.), age: 0., travelled: 0. })
    }
    /// Advances `dt`; returns the impact (prop damage applied like `fire`) when the step's segment hits a prop or the ground. `max_flight_m`: despawn distance (native, caller's).
    pub fn step(&mut self, gun: &GunHit, world: &mut World, dt: f32, gravity_m_s2: f32, max_flight_m: f32, policy: Policy) -> Option<Impact> {
        let s = &gun.stats;
        self.vel.y -= gravity_m_s2 * dt;
        let seg = self.vel * dt; let len = seg.length();
        if len <= 0. { return None; }
        let dir = seg / len;
        let prop = trace_props(&world.props, self.pos, dir, len);
        let ground = if dir.y < -1e-6 && self.pos.y > 0. { let t = -self.pos.y / dir.y; (t <= len).then_some(t) } else { None };
        let travelled = self.travelled;
        self.age += dt; self.travelled += len;
        let at = |t: f32| self.pos + dir * t;
        let result = match (prop, ground) {
            (Some((i, t, n)), g) if g.map_or(true, |gt| t <= gt) => {
                let range = travelled + t;
                let dmg = s.damage_at(&s.env_dmg, range * 100., policy);
                let p = &mut world.props[i];
                p.health -= dmg; let destroyed = p.health <= 0.; if destroyed { p.gone_for = 0.; }
                let object = fn_combat::harvest_fx::object_id(i, p);
                let (surface, kind, res, proto, ppos) = (surface_of(p.res), p.kind, p.res, p.proto, p.pos);
                if destroyed { world.fx.events.push(FxEvent::Broke { object, pos: ppos, kind, resource: res, proto }); }
                world.hits += 1;
                Some(Impact { pos: at(t), normal: n, surface, distance_m: range, prop: Some(i), object, damage: dmg, destroyed, zone: Zone::Normal,
                    effect: gun.pres.impact_effect(surface).map(str::to_owned), sound: gun.pres.impact_sound(surface).map(str::to_owned) })
            }
            (_, Some(t)) => Some(Impact { pos: at(t), normal: Vec3::Y, surface: SURFACE_DEFAULT, distance_m: travelled + t, prop: None, object: 0, damage: 0., destroyed: false, zone: Zone::Normal,
                effect: gun.pres.impact_effect(SURFACE_DEFAULT).map(str::to_owned), sound: gun.pres.impact_sound(SURFACE_DEFAULT).map(str::to_owned) }),
            _ => None,
        };
        self.pos += seg;
        if result.is_none() && self.travelled > max_flight_m { self.vel = Vec3::ZERO; }
        result
    }
    pub fn dead(&self) -> bool { self.vel == Vec3::ZERO }
}


/// World gravity for projectiles in m/s^2: `DefaultGravityZ` (-2800 cm/s^2, PROVENANCE: decrypted config, not independently verified; exe default -980; via fn-movement) x the engine `GravityScale`/`ProjectileGravityScale` default 1.
pub fn projectile_gravity_m_s2() -> f32 { -fn_movement::params::WORLD_GRAVITY_Z / 100. }
