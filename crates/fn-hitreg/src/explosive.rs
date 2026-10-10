use crate::data::{GunHit, APPLICATION_EXPLODE};
use crate::shoot::{building_damage, building_trace, hit_pawn, pellet_dir, select_aim, trace_pawns, trace_world, FireContext, Impact, PawnFire, PawnHit, PawnTarget, PawnTraceHit, Shot, WorldHit, Zone, SURFACE_DEFAULT};
use crate::spread::{safe_normal, DamageKind, RandomStream, SpreadState};
use fn_combat::{harvest_fx::FxEvent, World};
use fn_pose::{loader, ClipSourceDir};
use fn_ueio::props::{read_props, Ctx, PropList, PropValue};
use fn_ueio::{Package, Reader};
use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Rocket, Grenade }

#[derive(Clone, Debug, PartialEq)]
pub struct ExplosiveDef {
    pub kind: Kind,
    pub template: String,
    pub mesh: Option<String>,
    pub mesh_scale: Vec3,
    pub capsule_radius_cm: f32,
    pub capsule_half_height_cm: f32,
    pub trail: Option<String>,
    pub ribbon: Option<String>,
    pub trail_attached_ribbon: bool,
    pub trail_lifetime: f32,
    pub explosion_fx: Option<String>,
    pub explosion_water_fx: Option<String>,
    pub explosion_sound: Option<String>,
    pub explosion_water_sound: Option<String>,
    pub in_air_sound: Option<String>,
    pub in_air_fade_out: f32,
    pub bounce_fx: Option<String>,
    pub bounce_water_fx: Option<String>,
    pub bounce_sound: Option<String>,
    pub fuse_s: f32,
    pub explode_on_pawn_bounce: bool,
    pub impact_offset_cm: f32,
    pub explode_at_actor: bool,
    pub camera_shake: Option<(String, f32, f32)>,
    pub force_feedback_radii: (f32, f32),
    pub post_process_radius_cm: Option<f32>,
    pub spin_rate_deg: f32,
}

struct Recipe {
    template: &'static str,
    kind: Kind,
    mesh_scale_var: Option<&'static str>,
    mesh_scale_lit: [f32; 3],
    fuse_var: Option<&'static str>,
    fuse_lit: f32,
    trail: Option<&'static str>,
    ribbon: &'static str,
    explosion: &'static str,
    explosion_water: &'static str,
    explosion_sound: &'static str,
    explosion_water_sound: &'static str,
    bounce: Option<(&'static str, &'static str, &'static str)>,
    explode_on_pawn_bounce: bool,
    impact_offset_var: Option<&'static str>,
    explode_at_actor: bool,
    shake: Option<(&'static str, f32, f32)>,
    feedback: (f32, f32),
    in_air_fade_out: f32,
}

const RECIPES: &[Recipe] = &[
    Recipe { template: "B_Prj_Pumpkin_RPG_Athena", kind: Kind::Rocket, mesh_scale_var: Some("RocketScale"), mesh_scale_lit: [1., 1., 1.], fuse_var: None, fuse_lit: 15.,
        trail: Some("Rocket Trail Template"), ribbon: "Ribbon_Trail_PS", explosion: "Explosion_Generic_PS", explosion_water: "Water_Explosion_PS",
        explosion_sound: "Rocket_Projectile_Explosion_Sound", explosion_water_sound: "Rocket_Projectile_Explosion_Water_Sound", bounce: None, explode_on_pawn_bounce: false,
        impact_offset_var: Some("Explosion Impact Offset"), explode_at_actor: false, shake: Some(("B_RocketLauncherCameraShake", 400., 850.)), feedback: (400., 850.), in_air_fade_out: 1. },
    Recipe { template: "B_Prj_Ranged_GrenadeLauncher_Athena", kind: Kind::Grenade, mesh_scale_var: None, mesh_scale_lit: [2., 2., 2.], fuse_var: Some("FuseTimer@Spawn"), fuse_lit: 0.,
        trail: None, ribbon: "RibbonTrail_PS", explosion: "Explosion_PS", explosion_water: "Explosion_Water_PS",
        explosion_sound: "Grenade_Explosion_Sound", explosion_water_sound: "Grenade_Explosion_Water_Sound", bounce: Some(("Bounce_Generic_PS", "Bounce_WaterSplash_PS", "Grenade_Projectile_Bounce_Cue")), explode_on_pawn_bounce: true,
        impact_offset_var: None, explode_at_actor: true, shake: None, feedback: (600., 1200.), in_air_fade_out: 0. },
];

fn props_of(pkg: &Package, i: usize) -> Option<PropList> {
    let (s, e) = pkg.export_range(i);
    read_props(&mut Reader::at(&pkg.data[..e], s), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).ok()
}
fn obj_path(pkg: &Package, v: Option<&PropValue>) -> Option<String> {
    match v? { PropValue::Object(i) if *i < 0 => Some(pkg.import_path(*i)), PropValue::Str(s) if s.starts_with("/Game/") => Some(s.clone()), _ => None }
}
fn vec3_of(v: Option<&PropValue>) -> Option<Vec3> {
    match v? {
        PropValue::Vec3(a) => Some(Vec3::from_array(*a)),
        PropValue::Struct { fields, .. } => Some(Vec3::new(fields.float("X")? as f32, fields.float("Y")? as f32, fields.float("Z")? as f32)),
        _ => None,
    }
}

pub fn load_def(src: &ClipSourceDir, template: &str) -> Option<ExplosiveDef> {
    let pkg_path = template.split('.').next().unwrap_or(template);
    let name = pkg_path.rsplit('/').next().unwrap_or(pkg_path);
    let r = RECIPES.iter().find(|r| r.template == name)?;
    let pkg = loader::package(src, pkg_path).ok()?;
    let cdo_i = pkg.exports.iter().position(|e| e.object_name.starts_with("Default__"))?;
    let cdo = props_of(&pkg, cdo_i)?;
    let class_props = |class: &str| (0..pkg.exports.len()).filter(|&i| pkg.export_class(i) == class).filter_map(|i| props_of(&pkg, i)).collect::<Vec<_>>();
    let capsule = class_props("CapsuleComponent").into_iter().next();
    let mesh = class_props("StaticMeshComponent").iter().find_map(|p| obj_path(&pkg, p.get("StaticMesh")));
    let in_air = class_props("AudioComponent").iter().find_map(|p| obj_path(&pkg, p.get("Sound")));
    let path = |k: &str| obj_path(&pkg, cdo.get(k));
    let import_named = |n: &str| (0..pkg.imports.len()).find(|&i| pkg.imports[i].object_name == n).map(|i| pkg.import_path(-(i as i32) - 1));
    let mesh_scale = r.mesh_scale_var.and_then(|k| vec3_of(cdo.get(k))).unwrap_or(Vec3::from_array(r.mesh_scale_lit));
    let fuse_s = r.fuse_var.and_then(|k| cdo.float(k)).map_or(r.fuse_lit, |v| v as f32);
    let shake = r.shake.map(|(n, a, b)| (import_named(&format!("{n}_C")).or_else(|| import_named(n)).unwrap_or_else(|| n.to_owned()), a, b));
    let pp = (0..pkg.exports.len()).filter(|&i| pkg.export_class(i) == "PostProcessComponent").filter_map(|i| props_of(&pkg, i)).find_map(|p| p.float("BlendRadius")).map(|v| v as f32);
    Some(ExplosiveDef {
        kind: r.kind, template: pkg_path.to_owned(), mesh, mesh_scale,
        capsule_radius_cm: capsule.as_ref().and_then(|c| c.float("CapsuleRadius")).map_or(22., |v| v as f32),
        capsule_half_height_cm: capsule.as_ref().and_then(|c| c.float("CapsuleHalfHeight")).map_or(44., |v| v as f32),
        trail: r.trail.and_then(path), ribbon: path(r.ribbon), trail_attached_ribbon: r.kind == Kind::Grenade, trail_lifetime: cdo.float("TrailLifetime").map_or(0., |v| v as f32),
        explosion_fx: path(r.explosion), explosion_water_fx: path(r.explosion_water), explosion_sound: path(r.explosion_sound), explosion_water_sound: path(r.explosion_water_sound),
        in_air_sound: in_air, in_air_fade_out: r.in_air_fade_out,
        bounce_fx: r.bounce.and_then(|b| path(b.0)), bounce_water_fx: r.bounce.and_then(|b| path(b.1)), bounce_sound: r.bounce.and_then(|b| import_named(b.2)),
        fuse_s, explode_on_pawn_bounce: r.explode_on_pawn_bounce,
        impact_offset_cm: r.impact_offset_var.and_then(|k| cdo.float(k)).map_or(0., |v| v as f32), explode_at_actor: r.explode_at_actor,
        camera_shake: shake, force_feedback_radii: r.feedback, post_process_radius_cm: pp, spin_rate_deg: cdo.float("RocketSpinRate").map_or(0., |v| v as f32),
    })
}

pub fn def_of(gun: &GunHit) -> Option<&ExplosiveDef> { gun.projectile.as_ref()?.explosive.as_ref() }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Launch { pub pos: Vec3, pub vel: Vec3 }

pub fn launch(gun: &GunHit, world: &World, shot: &Shot, spread: &mut SpreadState, ctx: &FireContext, seed: i32) -> Option<Launch> {
    let pr = gun.projectile.as_ref()?;
    pr.explosive.as_ref()?;
    let aim = select_aim(gun, &world.props, shot.eye, shot.forward, ctx.muzzle, ctx.targeting);
    let mut rs = RandomStream::new(seed);
    let bullets = gun.stats.bullets_per_cartridge.max(1);
    let mut dir = aim.dir;
    for index in 0..bullets { dir = pellet_dir(gun, spread, ctx, aim.dir, index, &mut rs); spread.mark_fired(ctx.now); }
    spread.add_heat(&gun.stats, ctx.level);
    let range = gun.stats.trace_range_m();
    let target = trace_world(&world.props, aim.start, dir, range, 0.).map_or(aim.start + dir * range, |h| aim.start + dir * h.t);
    let start = ctx.muzzle.unwrap_or(aim.start);
    let toward = safe_normal(target - start);
    let toward = if toward == Vec3::ZERO { dir } else { toward };
    let toward = crate::shoot::add_pitch_offset(toward, gun.pres.projectile_pitch_offset);
    let speed = (pr.initial_speed.min(pr.max_speed)) / 100.;
    Some(Launch { pos: start, vel: toward * speed })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cause { Impact, Fuse, PawnBounce }

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Bounce { pos: Vec3, normal: Vec3, surface: u32, pawn: Option<u64> },
    Stopped { pos: Vec3 },
    Explode { pos: Vec3, normal: Vec3, surface: u32, cause: Cause, direct_object: u64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Flying {
    pub id: u64,
    pub instigator: u64,
    pub team: Option<u32>,
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    pub bounces: u32,
    pub stopped: bool,
    pub exploded: bool,
    pub travelled: f32,
}

pub const BOUNCE_STOP_CM_S: f32 = 5.;
pub const MAX_SIM_STEP: f32 = 0.05;

impl Flying {
    pub fn new(id: u64, instigator: u64, team: Option<u32>, l: Launch) -> Flying { Flying { id, instigator, team, pos: l.pos, vel: l.vel, age: 0., bounces: 0, stopped: false, exploded: false, travelled: 0. } }

    fn limit(v: Vec3, max_speed_m: f32) -> Vec3 { if max_speed_m.is_finite() && max_speed_m > 0. && v.length() > max_speed_m { v.normalize() * max_speed_m } else { v } }

    pub fn step(&mut self, gun: &GunHit, world: &World, pawns: &[PawnTarget], dt: f32, gravity_m_s2: f32) -> Vec<Event> {
        let mut out = vec![];
        if self.exploded { return out; }
        let (Some(pr), Some(def)) = (gun.projectile.as_ref(), def_of(gun)) else { return out };
        let mut left = dt;
        while left > 1e-6 && !self.exploded {
            let h = left.min(MAX_SIM_STEP); left -= h;
            self.age += h;
            if !self.stopped { self.move_step(gun, pr, def, world, pawns, h, gravity_m_s2, &mut out); }
            if !self.exploded && self.age >= def.fuse_s && def.fuse_s > 0. {
                self.exploded = true;
                out.push(Event::Explode { pos: self.pos, normal: Vec3::Y, surface: SURFACE_DEFAULT, cause: Cause::Fuse, direct_object: 0 });
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn move_step(&mut self, gun: &GunHit, pr: &crate::data::ProjectileData, def: &ExplosiveDef, world: &World, pawns: &[PawnTarget], h: f32, gravity_m_s2: f32, out: &mut Vec<Event>) {
        let max_speed = pr.max_speed / 100.;
        let g = Vec3::new(0., -gravity_m_s2 * crate::shoot::projectile_gravity_scale(gun), 0.);
        let v1 = Self::limit(self.vel + g * h, max_speed);
        let delta = (self.vel + v1) * 0.5 * h;
        let len = delta.length();
        if len <= 1e-7 { self.vel = v1; return; }
        let dir = delta / len;
        let radius = def.capsule_radius_cm / 100.;
        let centre = trace_world(&world.props, self.pos, dir, len + radius, 0.).map(|hh| ((hh.t - radius).max(0.), hh));
        let bottom = trace_world(&world.props, self.pos - Vec3::Y * (radius - 0.005).max(0.), dir, len, 0.).filter(|hh| hh.normal.y > 0.).map(|hh| (hh.t, hh));
        let world_hit = match (centre, bottom) { (Some(a), Some(b)) => Some(if b.0 < a.0 { b } else { a }), (a, b) => a.or(b) };
        let building = building_trace(self.pos, dir, world_hit.map_or(len, |w| w.0), radius);
        let opts = PawnFire { shooter: self.instigator, shooter_team: self.team, ..Default::default() };
        let limit = building.as_ref().map(|b| b.t).or(world_hit.map(|w| w.0)).unwrap_or(len);
        let pawn = trace_pawns(pawns, &opts, self.pos, dir, limit, radius).into_iter().next();
        let hit: Option<(f32, Vec3, u32, u64, bool)> = if let Some(p) = pawn { Some((p.t, p.normal, crate::shoot::SURFACE_PLAYER, pawns[p.pawn].id, true)) }
            else if let Some(b) = building { Some((b.t, b.normal, crate::shoot::surface_of(b.resource), b.object, false)) }
            else { world_hit.map(|(t, w): (f32, WorldHit)| (t, w.normal, w.prop.map_or(SURFACE_DEFAULT, |i| crate::surface::of_prop(&world.props[i])), w.prop.map_or(0, |i| fn_combat::harvest_fx::object_id(i, &world.props[i])), false)) };
        let Some((t_hit, normal, surface, object, is_pawn)) = hit.filter(|x| x.0 <= len) else {
            self.pos += delta; self.travelled += len; self.vel = v1; return;
        };
        let t = (t_hit - 0.001).max(0.);
        let frac = (t / len).clamp(0., 1.);
        self.pos += dir * t; self.travelled += t;
        let normal = if normal.length_squared() > 1e-8 { normal.normalize() } else { -dir };
        let impact = self.pos - normal * radius;
        if !pr.should_bounce {
            self.exploded = true; self.vel = Vec3::ZERO;
            let at = impact + normal * (def.impact_offset_cm / 100.);
            out.push(Event::Explode { pos: if def.explode_at_actor { self.pos } else { at }, normal, surface, cause: Cause::Impact, direct_object: object });
            return;
        }
        let vel = if frac > 1e-4 { Self::limit(self.vel + g * (h * frac), max_speed) } else { self.vel };
        let vdn = vel.dot(normal);
        let mut nv = vel;
        if vdn < 0. {
            let proj = normal * -vdn;
            nv += proj;
            nv *= (1. - pr.friction).clamp(0., 1.);
            nv += proj * pr.bounciness.max(0.);
            nv = Self::limit(nv, max_speed);
        }
        self.vel = nv;
        self.bounces += 1;
        let pawn_id = if is_pawn { Some(object) } else { None };
        out.push(Event::Bounce { pos: impact, normal, surface, pawn: pawn_id });
        if pawn_id.is_some() && def.explode_on_pawn_bounce {
            self.exploded = true;
            out.push(Event::Explode { pos: self.pos, normal, surface, cause: Cause::PawnBounce, direct_object: object });
            return;
        }
        if self.vel.length() * 100. < BOUNCE_STOP_CM_S { self.vel = Vec3::ZERO; self.stopped = true; out.push(Event::Stopped { pos: self.pos }); }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildingCandidate { pub object: u64, pub point: Vec3 }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Blast {
    pub pos: Vec3,
    pub radius_m: f32,
    pub max_targets: i32,
    pub pawn_hits: Vec<PawnHit>,
    pub impacts: Vec<Impact>,
    pub considered: usize,
    pub obstructed: usize,
}

enum Target { Pawn(usize, Vec3), Prop(usize, Vec3), Building(u64, Vec3) }

fn obstructed(world: &World, from: Vec3, to: Vec3, own_prop: Option<usize>, own_building: Option<u64>) -> bool {
    let d = (to - from).length();
    if d < 1e-3 { return false; }
    let dir = (to - from) / d;
    let lim = (d - 0.02).max(0.);
    if let Some(w) = trace_world(&world.props, from, dir, lim, 0.) { if w.prop.is_none() || w.prop != own_prop { return true; } }
    if let Some(b) = building_trace(from, dir, lim, 0.) { if Some(b.object) != own_building { return true; } }
    false
}

fn prop_point(p: &fn_combat::Harvestable, c: Vec3) -> (Vec3, f32) {
    if let Some(h) = &p.hull {
        let mut best: Option<(Vec3, f32)> = None;
        for piece in &h.hulls {
            let d = piece.sdf(c).max(0.);
            let q = if d <= 0. { c } else { let n = piece.closest(c, Vec3::ZERO); n.point };
            if best.map_or(true, |b| d < b.1) { best = Some((q, d)); }
        }
        if let Some(b) = best { return b; }
    }
    let y = c.y.clamp(p.pos.y, p.pos.y + p.height);
    let flat = Vec3::new(c.x - p.pos.x, 0., c.z - p.pos.z);
    let r = flat.length();
    let q = if r <= p.radius { Vec3::new(c.x, y, c.z) } else { Vec3::new(p.pos.x, y, p.pos.z) + flat / r * p.radius };
    (q, (q - c).length())
}

pub fn explode(gun: &GunHit, world: &mut World, pawns: &mut [PawnTarget], instigator: u64, team: Option<u32>, buildings: &[BuildingCandidate], pos: Vec3, level: i32) -> Blast {
    let Some(sel) = gun.ability.selection(APPLICATION_EXPLODE).cloned() else { return Blast { pos, ..Default::default() } };
    let radius = sel.range / 100.;
    let mut blast = Blast { pos, radius_m: radius, max_targets: sel.max_targets, ..Default::default() };
    if sel.effect_classes.is_empty() { return blast; }
    let mut cands: Vec<(f32, Target)> = vec![];
    for (i, p) in pawns.iter().enumerate() {
        if !p.vitals.alive() { continue; }
        if sel.exclude_instigator && p.id == instigator { continue; }
        if sel.exclude_pawn_friends && team.is_some() && team == Some(p.team) && p.id != instigator { continue; }
        let (a, b, r) = p.capsule_segment();
        let ab = b - a; let k = ((pos - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0., 1.); let q = a + ab * k;
        let d = (q - pos).length();
        if d - r > radius { continue; }
        let surf = if d > r { q + (pos - q) / d * r } else { pos };
        cands.push(((d - r).max(0.), Target::Pawn(i, surf)));
    }
    for (i, p) in world.props.iter().enumerate() {
        if !p.alive() { continue; }
        if (Vec3::new(p.pos.x, pos.y, p.pos.z) - pos).length() > radius + p.radius.max(0.) + 30. { continue; }
        let (q, d) = prop_point(p, pos);
        if d <= radius { cands.push((d, Target::Prop(i, q))); }
    }
    for b in buildings {
        let d = (b.point - pos).length();
        if d <= radius { cands.push((d, Target::Building(b.object, b.point))); }
    }
    blast.considered = cands.len();
    if sel.exclude_obstructed_by_world {
        let before = cands.len();
        cands.retain(|(_, t)| match t {
            Target::Pawn(_, q) => !obstructed(world, pos, *q, None, None),
            Target::Prop(i, q) => !obstructed(world, pos, *q, Some(*i), None),
            Target::Building(o, q) => !obstructed(world, pos, *q, None, Some(*o)),
        });
        blast.obstructed = before - cands.len();
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    if sel.max_targets > 0 { cands.truncate(sel.max_targets as usize); }
    let s = gun.stats.clone();
    for (d, t) in cands {
        match t {
            Target::Pawn(i, q) => {
                let normal = safe_normal(q - pos);
                let ph = PawnTraceHit { pawn: i, t: d, normal, zone: Zone::Normal, bone: None };
                let (imp, hit) = hit_pawn(gun, pawns, ph, q, d, 0, level);
                blast.impacts.push(imp);
                blast.pawn_hits.push(hit);
            }
            Target::Prop(i, q) => {
                let dmg = s.damage_at(DamageKind::Environmental, d * 100., level);
                let p = &mut world.props[i];
                let dmg = if p.invulnerable { 0. } else { dmg };
                p.health -= dmg; let destroyed = p.health <= 0.; if destroyed { p.gone_for = 0.; }
                let object = fn_combat::harvest_fx::object_id(i, p);
                let (surface, kind, res, proto, ppos) = (crate::surface::of_prop(p), p.kind, p.res, p.proto, p.pos);
                if destroyed { world.fx.events.push(FxEvent::Broke { object, pos: ppos, kind, resource: res, proto }); }
                world.hits += 1;
                blast.impacts.push(Impact { pos: q, normal: safe_normal(pos - q), surface, distance_m: d, prop: Some(i), object, damage: dmg, destroyed, zone: Zone::Normal, effect: None, sound: None });
            }
            Target::Building(o, q) => {
                let dmg = s.damage_at(DamageKind::Environmental, d * 100., level);
                let destroyed = building_damage(o, dmg, q);
                blast.impacts.push(Impact { pos: q, normal: safe_normal(pos - q), surface: SURFACE_DEFAULT, distance_m: d, prop: None, object: o, damage: dmg, destroyed, zone: Zone::Normal, effect: None, sound: None });
            }
        }
    }
    blast
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{ProjectileData, StatRow, TargetSelection, FireAbility, Presentation};

    fn def(kind: Kind) -> ExplosiveDef {
        ExplosiveDef { kind, template: String::new(), mesh: None, mesh_scale: Vec3::ONE, capsule_radius_cm: 6., capsule_half_height_cm: 6., trail: None, ribbon: None, trail_attached_ribbon: false, trail_lifetime: 0.,
            explosion_fx: None, explosion_water_fx: None, explosion_sound: None, explosion_water_sound: None, in_air_sound: None, in_air_fade_out: 0., bounce_fx: None, bounce_water_fx: None, bounce_sound: None,
            fuse_s: if kind == Kind::Rocket { 15. } else { 2.75 }, explode_on_pawn_bounce: kind == Kind::Grenade, impact_offset_cm: if kind == Kind::Rocket { 10. } else { 0. }, explode_at_actor: kind == Kind::Grenade,
            camera_shake: None, force_feedback_radii: (0., 0.), post_process_radius_cm: None, spin_rate_deg: 0. }
    }
    fn gun(kind: Kind) -> GunHit {
        let rocket = kind == Kind::Rocket;
        let stats = StatRow { dmg: [110.; 4], env_dmg: [375.; 4], rng: [1., 1., 1., 25000.], base_level: 1, zone_normal: 1., zone_critical: 1., zone_light: 1., bullets_per_cartridge: 1, ..Default::default() };
        let pr = ProjectileData { template: String::new(), table: String::new(), initial_speed_row: String::new(), max_speed_row: String::new(), initial_speed: if rocket { 2000. } else { 2750. }, max_speed: if rocket { 2250. } else { 2750. },
            gravity_scale: Some(if rocket { 0. } else { 0.3 }), life_span: None, should_bounce: !rocket, bounciness: 0.4, friction: if rocket { 0.6 } else { 0.45 }, explosive: Some(def(kind)) };
        let sel = TargetSelection { shape: "EFortTargetSelectionShape::Sphere".into(), test_type: String::new(), primary_source: String::new(), range: 512., cone_yaw: 0., cone_pitch: 0., cone_min_radius: 1.,
            actor_type_filter: String::new(), exclude_pawn_friends: false, exclude_obstructed_by_world: true, max_targets: 12, application_tag: APPLICATION_EXPLODE.into(), exclude_instigator: false, effect_classes: vec!["GE".into()] };
        GunHit { stats, pres: Presentation::default(), ability: FireAbility { selections: vec![sel], ..Default::default() }, projectile: Some(pr), ..Default::default() }
    }

    #[test]
    fn launchers_load_their_explosive_data_from_the_install() {
        let Some(root) = fn_live::var_os("FN_TEST_ROOT") else { return };
        let paks = fn_pak::PakSet::open_install(&root).unwrap();
        let src = ClipSourceDir::Pak(&paks);
        let r = crate::data::load(&src, "/Game/Athena/Items/Weapons/WID_Launcher_Rocket_Athena_R_Ore_T03").unwrap();
        let d = def_of(&r).unwrap();
        assert_eq!((d.kind, d.fuse_s, d.impact_offset_cm, d.capsule_radius_cm, d.mesh_scale), (Kind::Rocket, 15., 10., 6., Vec3::splat(2.)));
        assert!(d.mesh.as_deref().unwrap().ends_with("S_PumpkinLauncher_Missile"), "{:?}", d.mesh);
        assert!(d.trail.as_deref().unwrap().contains("P_RocketTrail_Pumpkin_02") && d.explosion_fx.as_deref().unwrap().contains("P_PumpkinRPG_ExplodeGoop_01"));
        assert!(d.in_air_sound.as_deref().unwrap().contains("Pumpkin_InAirLoop_Athena_Cue") && d.explosion_sound.as_deref().unwrap().contains("Pumpkin_Proj_Impact_Athena_Cue"));
        let pr = r.projectile.as_ref().unwrap();
        assert_eq!((pr.initial_speed, pr.max_speed, pr.gravity_scale, pr.should_bounce), (2000., 2250., Some(0.), false));
        let ex = r.ability.selection(APPLICATION_EXPLODE).unwrap();
        assert_eq!((ex.range, ex.max_targets, ex.exclude_instigator, ex.exclude_pawn_friends, ex.exclude_obstructed_by_world), (512., 12, false, false, true));
        assert!(ex.effect_classes.iter().any(|e| e.contains("GE_Ranged_GenericDamage")));
        assert!(r.ability.selection(crate::data::APPLICATION_HIT).is_some() && r.ability.effect_classes.is_empty());
        let g = crate::data::load(&src, "/Game/Athena/Items/Weapons/WID_Launcher_Grenade_Athena_R_Ore_T03").unwrap();
        let d = def_of(&g).unwrap();
        assert_eq!((d.kind, d.fuse_s, d.explode_on_pawn_bounce), (Kind::Grenade, 2.75, true));
        assert!(d.bounce_sound.as_deref().unwrap().contains("Grenade_Projectile_Bounce_Cue") && d.explosion_fx.as_deref().unwrap().contains("P_GrenadeLauncher_Explosion"));
        let pr = g.projectile.as_ref().unwrap();
        assert_eq!((pr.initial_speed, pr.gravity_scale, pr.should_bounce, pr.bounciness, pr.friction), (2750., Some(0.3), true, 0.4, 0.45));
    }

    #[test]
    fn rocket_flies_straight_at_2000_and_explodes_on_the_ground_offset_by_the_impact_offset() {
        let g = gun(Kind::Rocket);
        let w = World::default();
        let mut f = Flying::new(1, 7, None, Launch { pos: Vec3::new(0., 1.5, 0.), vel: Vec3::new(0., -0.5, -20.).normalize() * 20. });
        let mut ev = vec![];
        for _ in 0..600 { ev.extend(f.step(&g, &w, &[], 1. / 120., 28.)); if f.exploded { break; } }
        let Some(Event::Explode { pos, cause, .. }) = ev.last().cloned() else { panic!("{ev:?}") };
        assert_eq!(cause, Cause::Impact);
        assert!((pos.y - 0.1).abs() < 0.01, "{pos}");
        assert!((f.vel.length()).abs() < 1e-6);
    }

    #[test]
    fn rocket_fuse_is_15_seconds() {
        let g = gun(Kind::Rocket);
        let w = World::default();
        let mut f = Flying::new(1, 7, None, Launch { pos: Vec3::new(0., 100., 0.), vel: Vec3::new(0., 0., -20.) });
        let mut t = 0f32;
        while !f.exploded && t < 20. { f.step(&g, &w, &[], 1. / 60., 28.); t += 1. / 60.; }
        assert!((t - 15.).abs() < 0.05, "{t}");
        assert!((f.travelled - 300.).abs() < 1., "{}", f.travelled);
    }

    #[test]
    fn grenade_bounces_loses_energy_and_explodes_on_the_fuse() {
        let g = gun(Kind::Grenade);
        let w = World::default();
        let mut f = Flying::new(1, 7, None, Launch { pos: Vec3::new(0., 1.5, 0.), vel: Vec3::new(0., 0., -27.5) });
        let mut ev = vec![];
        let mut t = 0f32;
        while !f.exploded && t < 5. { ev.extend(f.step(&g, &w, &[], 1. / 120., 28.)); t += 1. / 120.; }
        assert!(ev.iter().any(|e| matches!(e, Event::Bounce { .. })), "{ev:?}");
        assert!((t - 2.75).abs() < 0.02, "{t}");
        assert!(f.pos.y >= 0.05);
    }

    #[test]
    fn explosion_hits_the_instigator_flat_110_inside_512_and_nothing_outside() {
        let g = gun(Kind::Rocket);
        let mut w = World::default();
        let mut pawns = vec![PawnTarget::new(7, 0, Vec3::new(0., 0., 0.), crate::shoot::Vitals::new(100., 50.)), PawnTarget::new(8, 1, Vec3::new(0., 0., -7.), crate::shoot::Vitals::new(100., 0.))];
        let b = explode(&g, &mut w, &mut pawns, 7, Some(0), &[], Vec3::new(0., 0.1, -1.), 1);
        assert_eq!(b.pawn_hits.len(), 1);
        assert_eq!(b.pawn_hits[0].id, 7);
        assert!((b.pawn_hits[0].damage - 110.).abs() < 1e-3);
        assert_eq!(pawns[0].vitals.shield, 0.);
        assert!((pawns[0].vitals.health - 40.).abs() < 1e-3);
        assert_eq!(pawns[1].vitals.health, 100.);
    }
}
