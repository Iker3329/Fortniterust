//! BUILDING DAMAGE HOOK (fn-app, owner: structural / damage agent): routes pickaxe swings and bullets into the player builds and runs the structural collapse.
//! The LOGIC is `fn_building::damage` / `fn_building::structural`; this file is glue:
//! * [`install`] (idempotent) registers the hit targets with fn-hitreg (`fn_hitreg::shoot::BUILDING_TARGETS`: hitscan pellets and projectiles stop on a piece and
//!   deal their `EnvDmg`) and fn-combat (`fn_combat::BUILDING_STRIKE`: the pickaxe's target selection includes the pieces).
//! * [`tick`] - call once per frame with the app's `BuildingWorld`, AFTER `BuildingWorld::tick`: applies the queued hits, runs the structural timers (kills the
//!   unsupported pieces), refreshes the snapshot the hit tests read.
//! * [`destroyed_since`] - the destroyed-piece events for the destroy FX / audio (visuals agent), [`weak_spots`] - the pieces' weak spots for drawing.
//!
//! Space: the pieces are UE cm (Z up, `fn_movement::scene`); the hit tests come in sim metres (Y up). [`sim_to_cm`] / [`cm_to_sim`] follow the scene mapping
//! (`fn_movement::scene::Scene::from_blockers`: scene X/Y = controller X/Y = sim X / sim Z - `Z_OFFSET`, scene Z = sim Y; x100).
//!
//! Pickaxe target selection (fn_combat::harvest_fx, `GAB_Melee_ImpactCombo_Athena`): the WeakSpots camera line (400 cm from the pawn, nearest spot, a building's or a
//! prop's) first, then the melee sweep (`fn_combat::melee_sweeps`: the camera aim ray from the pawn's projection on it, `DAMAGEABLE_RANGE` / `MELEE_RADIUS`) where the nearest of piece / prop wins.
//! Weak spots on pieces: exactly like props (see `fn_building::damage` docs): the first ordinary hit generates one, a weak-spot hit deals 100 and moves it.
//! RECONSTRUCTED (native `ServerSetupWeakSpotsOnBuildingActor` not recovered, same as harvest_fx): WHERE the spot goes - a random point of the piece seen from the
//! camera, at reachable height on walls.
use std::collections::HashMap;
use std::sync::Mutex;

use fn_building::damage::{self as bd, BuildingDamage, DestroyedPiece, Hit};
use fn_building::pieces::{Material, PieceKind};
use fn_building::structural;
use fn_building::world::{BuildingWorld, PieceId, PlacedPiece};
use fn_combat::harvest_fx::{line_hits_sphere, object_id, Rng, WEAKSPOT_LINE_RANGE, WEAKSPOT_LINE_SWEEP, WEAKSPOT_RADIUS, PLACE_MAX_HEIGHT, PLACE_MIN_HEIGHT, RELOCATE_MIN_DISTANCE};
use fn_combat::{trace_cylinders, Resource, World, CHEST_HEIGHT, MELEE_RADIUS};
use fn_game::sim::Z_OFFSET;
use glam::Vec3;

/// `object` ids of building impacts (fn-hitreg `Impact::object`): this bit set + the piece id (prop ids never use bit 62).
pub const BUILDING_OBJECT: u64 = 1 << 62;

pub fn sim_to_cm(v: Vec3) -> Vec3 { Vec3::new(v.x * 100., (v.z - Z_OFFSET) * 100., v.y * 100.) }
pub fn cm_to_sim(v: Vec3) -> Vec3 { Vec3::new(v.x / 100., v.z / 100., v.y / 100. + Z_OFFSET) }
fn dir_to_cm(d: Vec3) -> Vec3 { Vec3::new(d.x, d.z, d.y) }
fn dir_to_sim(d: Vec3) -> Vec3 { Vec3::new(d.x, d.z, d.y) }

/// A piece's weak spot (UE cm).
#[derive(Clone, Copy, Debug)]
pub struct PieceWeakSpot { pub pos: Vec3, pub normal: Vec3 }

#[derive(Default)]
struct State {
    dmg: BuildingDamage,
    /// Copy of the world the hit tests read (health already reduced by the hits queued this frame).
    pieces: Vec<PlacedPiece>,
    queued: Vec<(PieceId, Hit)>,
    weak: HashMap<PieceId, PieceWeakSpot>,
    rng: Option<Rng>,
    /// Height (UE z, cm) of the world under the pieces: the sandbox baseplate is the plane Z = 0 (`fn_movement::scene`).
    ground_z: f32,
    /// TERRAIN: the landscape in world cm (`PlacementEnv::terrain`, last env seen); `ground_z` stands in where it is `None` / under its holes.
    terrain: Option<fn_terrain::Local>,
    /// The structural grid origin (= `PlacementEnv::grid_origin`, the level origin in world space).
    grid_origin: Vec3,
    /// BVISFIX (visuals): origin (sim m) of the last building trace, the shot whose `damage` follows (the attacker side of the hit bounce).
    last_trace_origin: Option<Vec3>,
    /// BVISFIX (visuals): pickaxe hits to hand to `building_render::report_hit` once the lock is released.
    hit_reports: Vec<(PieceId, Vec3, Vec3)>,
    /// STRUCTURAL FIX 2: the structural timers seen last frame (run-log lines for newly set ones).
    timers_seen: Vec<(PieceId, f64)>,
}
static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| State { rng: Some(Rng(0x5eed_b11d)), ..Default::default() }))
}

fn resource(m: Material) -> Resource { match m { Material::Wood => Resource::Wood, Material::Stone => Resource::Stone, Material::Metal => Resource::Metal } }

/// STRUCTURAL FIX (2026-10-05): `bSupportedDirectly` is no longer guessed from the piece bottom here: `fn_building::structural` runs the
/// exe's "WorldOverlapCheck" sweep at registration against [`structural::WorldCollision`]; the port's world is the flat baseplate
/// ([`structural::Ground`]: TERRAIN, the landscape when one is installed, else the flat plane at `ground_z`).
/// STRUCTURAL FIX 2: on this plate a ground-level floor snaps 149 cm below it (the grid is the level's, anchored on the level origin) and
/// gets no world support; that is the exe rule on a plate of the wrong height, not patched (see `structural::FlatGround`).
fn world_collision(s: &State) -> structural::Ground { structural::Ground { terrain: s.terrain.clone(), flat_z: Some(s.ground_z) } }

/// The world height the structural check treats as "the ground" (default 0 = the baseplate).
pub fn set_ground_z(z_cm: f32) { with(|s| s.ground_z = z_cm); }

// ---- hit tests on the snapshot -------------------------------------------------------------------------------------------
fn trace_snapshot(s: &State, origin_sim: Vec3, dir_sim: Vec3, max_m: f32, sweep_m: f32) -> Option<(usize, f32, Vec3)> {
    let (o, d) = (sim_to_cm(origin_sim), dir_to_cm(dir_sim).normalize_or_zero());
    let mut best: Option<(usize, f32, Vec3)> = None;
    for (i, p) in s.pieces.iter().enumerate() {
        if p.health <= 0. { continue; }
        if let Some((t, n)) = bd::trace_piece(p, o, d, sweep_m * 100.) {
            let t = t / 100.;
            if t <= max_m && best.map_or(true, |b| t < b.1) { best = Some((i, t, dir_to_sim(n))); }
        }
    }
    best
}

/// BUILDING HUD: the first live piece on a ray (sim metres) within `max_m`: (piece, distance m).
pub fn trace_pieces(origin: Vec3, dir: Vec3, max_m: f32) -> Option<(PieceId, f32)> { with(|s| trace_snapshot(s, origin, dir, max_m, 0.).map(|(i, t, _)| (s.pieces[i].id, t))) }

fn queue_hit(s: &mut State, i: usize, hit: Hit) -> bool {
    let p = &mut s.pieces[i];
    p.health -= hit.damage();
    s.queued.push((p.id, hit));
    p.health <= 0.
}

struct Targets;
impl fn_hitreg::shoot::BuildingTargets for Targets {
    fn trace(&self, origin: Vec3, dir: Vec3, max_m: f32, sweep_m: f32) -> Option<fn_hitreg::shoot::BuildingHit> {
        with(|s| { s.last_trace_origin = Some(origin); trace_snapshot(s, origin, dir, max_m, sweep_m).map(|(i, t, n)| { let p = &s.pieces[i]; fn_hitreg::shoot::BuildingHit { t, normal: n, object: BUILDING_OBJECT | p.id as u64, resource: resource(p.material) } }) })
    }
    fn damage(&mut self, object: u64, env_damage: f32, pos: Vec3) -> bool {
        let id = (object & !BUILDING_OBJECT) as PieceId;
        let (r, origin) = with(|s| (match s.pieces.iter().position(|p| p.id == id) { Some(i) => Some(queue_hit(s, i, Hit::Weapon { env_damage })), None => None }, s.last_trace_origin));
        // BVISFIX (visuals): the hit bounce of the piece (`ABuildingSMActor::vfunc_276` @140486ae0: ImpactPoint, direction towards the causer = the weapon,
        // i.e. the trace origin of this shot)
        if r.is_some() { crate::building_render::report_hit(id, sim_to_cm(pos), dir_to_cm(origin.unwrap_or(pos) - pos)); }
        r.unwrap_or(false)
    }
}

/// Places (or moves) the weak spot of piece `i` (RECONSTRUCTED placement, see the module docs).
fn place_weak_spot(s: &mut State, i: usize, eye_sim: Vec3, avoid: Option<Vec3>) {
    let p = s.pieces[i].clone();
    let eye = sim_to_cm(eye_sim);
    let mut rng = s.rng.take().unwrap_or(Rng(1));
    let mut placed = None;
    for _ in 0..12 {
        let local = match p.kind {
            PieceKind::Wall => Vec3::new(rng.range(-200., 200.), 0., rng.range(PLACE_MIN_HEIGHT * 100., PLACE_MAX_HEIGHT * 100.)),
            _ => Vec3::new(rng.range(-200., 200.), rng.range(56., 456.), 0.),
        };
        let mut target = structural::to_world(&p, local);
        if p.kind == PieceKind::Stairs { target.z += 384. * (1. - local.y / 512.); }
        if p.kind == PieceKind::Roof { target.z += 90.; }
        let d = (target - eye).normalize_or_zero();
        let Some((t, n)) = bd::trace_piece(&p, eye, d, 0.) else { continue };
        let pos = eye + d * t;
        if avoid.is_some_and(|a| (a - pos).length() < RELOCATE_MIN_DISTANCE * 100.) { continue; }
        placed = Some(PieceWeakSpot { pos, normal: n });
        break;
    }
    s.rng = Some(rng);
    if let Some(w) = placed { s.weak.insert(p.id, w); }
}

/// `fn_combat::BUILDING_STRIKE`: the pickaxe swing. True = a piece took it.
/// BVISFIX (visuals): [`strike_inner`], then the hit bounce of the struck piece (`ABuildingSMActor::vfunc_276` @140486ae0: ImpactPoint, direction towards
/// the causer = the pickaxe at the pawn) handed to the renderer outside the hook lock.
fn strike(world: &World, eye_ground: Vec3, facing: Vec3) -> bool {
    let r = strike_inner(world, eye_ground, facing);
    for (id, p, d) in with(|s| std::mem::take(&mut s.hit_reports)) { crate::building_render::report_hit(id, p, d); }
    r
}
fn strike_inner(world: &World, eye_ground: Vec3, facing: Vec3) -> bool {
    let aim = world.fx.aim;
    with(|s| {
        if s.pieces.is_empty() { return false; }
        let pawn = eye_ground + Vec3::Y * CHEST_HEIGHT;
        // 1. the WeakSpots camera line: nearest spot of a piece or a prop
        if let Some(a) = aim {
            let mut best_piece: Option<(usize, f32)> = None;
            for (i, p) in s.pieces.iter().enumerate() {
                let Some(w) = s.weak.get(&p.id) else { continue };
                let c = cm_to_sim(w.pos);
                if (c - pawn).length() > WEAKSPOT_LINE_RANGE || p.health <= 0. { continue; }
                if let Some(t) = line_hits_sphere(a.eye, a.dir, fn_combat::reticle::AIM_TRACE, WEAKSPOT_LINE_SWEEP, c, WEAKSPOT_RADIUS) { if best_piece.map_or(true, |b| t < b.1) { best_piece = Some((i, t)); } }
            }
            let best_prop = world.props.iter().enumerate().filter(|(_, p)| p.alive()).filter_map(|(i, p)| world.fx.weak_spot(i, p).filter(|w| (w.pos - pawn).length() <= WEAKSPOT_LINE_RANGE)
                .and_then(|w| line_hits_sphere(a.eye, a.dir, fn_combat::reticle::AIM_TRACE, WEAKSPOT_LINE_SWEEP, w.pos, WEAKSPOT_RADIUS))).fold(None, |m: Option<f32>, t| Some(m.map_or(t, |m| m.min(t))));
            match (best_piece, best_prop) {
                (Some((i, t)), p) if p.map_or(true, |pt| t < pt) => {
                    let old = s.weak.remove(&s.pieces[i].id).map(|w| w.pos);
                    if let Some(w) = old { s.hit_reports.push((s.pieces[i].id, w, dir_to_cm(pawn - cm_to_sim(w)).normalize_or_zero())); }
                    if !queue_hit(s, i, Hit::Pickaxe { weak_spot: true }) { place_weak_spot(s, i, a.eye, old); }
                    return true;
                }
                (_, Some(_)) => return false,
                _ => {}
            }
        }
        // 2. the melee sweep: chest height, then knee height; the nearer of piece / prop wins
        for (o, f, reach) in fn_combat::melee_sweeps(eye_ground, facing, aim) {
            let piece = trace_snapshot(s, o, f, reach, MELEE_RADIUS);
            let prop = trace_cylinders(&world.props, o, f, reach, MELEE_RADIUS).map(|t| t.fraction * reach);
            match (piece, prop) {
                (Some((i, t, _)), p) if p.map_or(true, |pt| t < pt) => {
                    let had = s.weak.contains_key(&s.pieces[i].id);
                    s.hit_reports.push((s.pieces[i].id, sim_to_cm(o + f * t), dir_to_cm(-f)));
                    // GE_Melee_GenericDamageMakeWeakspot (Gameplay.Effect.GenerateWeakSpots): an ordinary hit generates the spot
                    if !queue_hit(s, i, Hit::Pickaxe { weak_spot: false }) && !had { place_weak_spot(s, i, aim.map_or(o, |a| a.eye), None); }
                    return true;
                }
                (_, Some(_)) => return false,
                _ => {}
            }
        }
        false
    })
}

/// Registers the hit targets with fn-hitreg / fn-combat (idempotent).
pub fn install() {
    if let Ok(mut g) = fn_hitreg::shoot::BUILDING_TARGETS.lock() { if g.is_none() { *g = Some(Box::new(Targets)); } }
    if let Ok(mut g) = fn_combat::BUILDING_STRIKE.lock() { *g = Some(strike); }
}

/// Once per frame after `BuildingWorld::tick`: queued hits, structural timers, snapshot. Returns the pieces destroyed this frame.
/// Without a catalog the pieces use the base connectivity cubes (`structural::base_cube`) and the last ground / grid origin.
pub fn tick(world: &mut BuildingWorld) -> Vec<DestroyedPiece> { tick_inner(world, None, None) }

/// [`tick`] with the class cubes of the catalog and the placement env (ground height, grid origin) - STRUCTURAL FIX (2026-10-05).
pub fn tick_with(world: &mut BuildingWorld, cat: Option<&fn_building::pieces::PieceCatalog>, env: &fn_building::placement::PlacementEnv) -> Vec<DestroyedPiece> {
    tick_inner(world, cat, Some(env))
}

fn tick_inner(world: &mut BuildingWorld, cat: Option<&fn_building::pieces::PieceCatalog>, env: Option<&fn_building::placement::PlacementEnv>) -> Vec<DestroyedPiece> {
    install();
    with(|s| {
        if let Some(e) = env { if let Some(gz) = e.ground_z { s.ground_z = gz; } s.terrain = e.terrain.clone(); s.grid_origin = e.grid_origin; }
        let flat = world_collision(s);
        // STRUCTURAL FIX 2: with the placement env, the registration sweeps use the same WorldCollision as the placement query
        // (`placement::PlacementCollision`: the ground plane + the static mesh actors; props are skipped by allow = 1)
        let placement_coll = env.map(|e| fn_building::placement::PlacementCollision { env: e });
        let ground: &dyn structural::WorldCollision = match &placement_coll { Some(c) => c, None => &flat };
        let cubes = |class: &str| cat.and_then(|c| c.class(class)).map(|c| c.cube);
        let anchored = |v: &structural::Voxel| env.is_some_and(|e| fn_building::placement::anchored(e, v));
        let senv = structural::StructuralEnv { collision: ground, cubes: &cubes, grid_origin: s.grid_origin, anchored: &anchored };
        for (id, hit) in std::mem::take(&mut s.queued) {
            if let Some(r) = s.dmg.apply_hit(world, id, hit, &senv) { if r.destroyed { s.weak.remove(&id); } }
        }
        s.dmg.tick(world, &senv);
        // STRUCTURAL FIX 2: one run-log line per newly set structural timer (placement / edit / death: FUN_1404f4be0), for the verification runs
        let timers: Vec<(PieceId, f64)> = s.dmg.structural.pending().collect();
        for t in &timers { if !s.timers_seen.contains(t) { eprintln!("[structural] timer on #{} due t={:.4} s (set at t={:.4} s)", t.0, t.1, world.time()); } }
        s.timers_seen = timers;
        s.weak.retain(|id, _| world.get(*id).is_some());
        s.pieces = world.iter().cloned().collect();
        let gone = s.dmg.take_destroyed();
        // STRUCTURAL FIX (2026-10-05): one run-log line per destroyed piece (cause and world time), for the collapse verification runs
        for d in &gone { eprintln!("[structural] #{} {:?} {:?} destroyed at t={:.3} s ({:?}) at {:?}", d.id, d.kind, d.material, d.time, d.cause, d.location); }
        gone
    })
}

/// Destroyed pieces with sequence number >= `cursor` and the next cursor (each consumer keeps its own; start at 0).
pub fn destroyed_since(cursor: u64) -> (Vec<DestroyedPiece>, u64) { with(|s| { let (e, c) = s.dmg.destroyed_since(cursor); (e.to_vec(), c) }) }

/// EDITING (additive, editing agent): what blocks editing a piece in `ABuildingSMActor::vfunc_330` @140494860 -> `FUN_1404f1310`.
/// STRUCTURAL FIX 2 (2026-10-05): `FUN_1404f1310` is NOT the structural timer: it computes the actor's slot key (`FUN_1404dd690`) and looks it
/// up in the system's +0x218 map (`FUN_1404eaef0`), the `ABuildingLayoutRequirement` map (written by `FUN_1404d96e0` "Layout Requirement
/// (%s) added where extant building...", removed by `FUN_1404fd050`, both called from `ABuildingLayoutRequirement`). Athena has no layout
/// requirements, so it is always false. (Treating a set structural timer as the blocker would refuse edits for ~0.35 s after every
/// neighbouring placement, which now sets timers: `InitializeBuildingActor(1)` -> `FUN_1404f4be0`.)
pub fn structural_pending(_id: PieceId) -> bool { false }

/// Current weak spots of the pieces (UE cm), for drawing.
pub fn weak_spots() -> Vec<(PieceId, PieceWeakSpot)> { with(|s| s.weak.iter().map(|(k, v)| (*k, *v)).collect()) }

/// Prop object ids from `fn_combat::harvest_fx::object_id` never set [`BUILDING_OBJECT`].
#[allow(dead_code)]
fn _object_ids_disjoint(i: usize, p: &fn_combat::Harvestable) -> bool { object_id(i, p) & BUILDING_OBJECT == 0 }

#[cfg(test)]
mod tests {
    use super::*;
    use fn_building::world::{GridKey, PieceSpawn};
    use glam::IVec3;

    #[test]
    fn mapping_round_trips() {
        let v = Vec3::new(1.5, 2.25, -3.);
        assert!((cm_to_sim(sim_to_cm(v)) - v).length() < 1e-4);
    }

    #[test]
    fn a_swing_at_a_wall_hits_it_and_the_floor_on_top_collapses() {
        let mut bw = BuildingWorld::new();
        let hp = bd::max_health(Material::Wood, PieceKind::Wall, bd::Tier::L1);
        // wall 2 m in front of the pawn (pawn at sim (0, 0, Z_OFFSET) facing -Z = scene -Y)
        let spawn = |kind, loc: Vec3, t: i32| PieceSpawn { kind, material: Material::Wood, class_path: "C".into(), key: GridKey { kind, cell: IVec3::new(t, 0, 0), rot: 0 }, location: loc, yaw_deg: 0., max_health: hp, start_health: hp, build_time: 0. };
        // the wall's pivot (0, -200) lies in cell (0, 0) on its -Y side; the floor (pivot -> cell (0, 0, 1)) rests on the wall's top edge
        // (exact connectivity test, structural fix 2026-10-05); the floor is looked up from the wall's own cell: 0 cells, 0.25 s
        let wall = bw.spawn(spawn(PieceKind::Wall, Vec3::new(0., -200., 0.), 0));
        let floor = bw.spawn(spawn(PieceKind::Floor, Vec3::new(0., -200., 384.), 1));
        tick(&mut bw);
        let w = World::default();
        let pawn = Vec3::new(0., 0., Z_OFFSET);
        assert!(strike(&w, pawn, Vec3::NEG_Z));
        assert_eq!(weak_spots().len(), 1, "the first ordinary hit generates a weak spot");
        for _ in 0..7 { assert!(strike(&w, pawn, Vec3::NEG_Z)); } // 200 HP at 25 per swing (MeleeHarvest x HarvestBiasHigh.Friendly)
        let gone = tick(&mut bw);
        assert_eq!(gone.len(), 1);
        assert!(bw.get(wall).is_none() && bw.get(floor).is_some());
        bw.tick(0.36);
        let gone = tick(&mut bw);
        assert_eq!(gone.iter().map(|d| d.id).collect::<Vec<_>>(), vec![floor]);
    }
}
