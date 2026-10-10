//! Damage to player builds, piece health per tier, destruction and the structural collapse driver (owner: structural / damage agent).
//!
//! SOURCES (1.8 cooked data read with fn-inspect / fn-uasset, exe = `C:\FortniteRef\decomp\named`):
//! * HEALTH - every `PBWA_{W,S,M}{1,2,3}_*` CDO has `AttributeInitKey { PlayerBuilding{Wood,Stone,Metal}{1,2,3}, Size9 }` for the wall (`Solid`) and `Size7` for
//!   `Floor`, `StairW`, `RoofC`, `AttributeInitLevelSource = PlayerBuildingSkill`. `ABuildingActor` vfunc 226 @ 14048c9c0 initialises the attribute sets
//!   with the level of vfunc 225 @ 14046bc40, which for `PlayerBuildingSkill` (1) reads the game state's `PlayerBuildingSkillLevel` (`AFortGameStateZone`
//!   +0x85C, ctor 0, only updated from the game state's `FortConstructionSet.BuildingSkill` attribute in vfunc 271 @ 1405e0760; no Athena table or effect sets
//!   it, the only rows are the Save-the-World heroes' in `AttributesHeroNonScaling`, all 0). Fortnite's own initter (`FFortAttributeSetInitter`, preload
//!   @ 140448960) keeps the curve and evaluates it AT the level (not UE's discrete `Level - 1` table), so `MaxHealth = Eval(0)` of
//!   `/Game/Balance/DataTables/AttributesBuildingSection` row `<Category>.<Size>.FortHealthSet.MaxHealth`: see [`MAX_HEALTH_L0`].
//! * DAMAGE - guns: `EnvDmg{PB,Mid,Long,MaxRange}` of the weapon's stat row with the range falloff (`fn_hitreg::data::StatRow::damage_at(Environmental, ..)`, RVA
//!   0x9223e0) times the surface ratio of `UFortDamageFormulaExecutionCalculation::Execute` @14065f1c0 (the hit's `GetFortPhysicalSurface` -> the damage source
//!   interface @14092a6f0 -> FUN_14092a5e0: the weapon stats' `SurfaceRatioRowName` row of `SurfaceDamageCategory` names a `SurfaceDamageTeams` row per
//!   surface, `FFortSurfaceDamageRatioStats::GetSurfaceRatioMultiplier` @140557d70 picks its Friendly / Neutral / Hostile column by `GetTeamAffiliation`).
//!   Every Athena gun row is `AllWithFriendlyFire` (1 everywhere): guns deal their EnvDmg. The pickaxe (`EnvDmgPB` 50 of `Harvest_Pickaxe_Athena_C_T01`) is
//!   `MeleeHarvest`: Wood / Stone / Metal -> `HarvestBiasHigh` (Friendly 0.5, Neutral / Hostile 1), WeakSpot_Wood / _Stone / _Metal -> `WeakSpotHigh`
//!   (Friendly 1, Neutral / Hostile 2). The port's pieces are all the local player's (team affiliation Friendly): 25 per swing, 50 on a weak spot. (The props
//!   are Neutral: 50 / 100, `fn_combat::harvest_fx`.)
//! * WEAK SPOTS on player builds - `ABuildingSMActor` ctor @ 14045b960 sets `bAllowWeakSpots` / `bUseComplexForWeakSpots` (+0x810) to 1 and no PBWA CDO overrides
//!   them; `OnDamageServer` (vfunc 238 @ 140478e70) calls `ServerSetupWeakSpotsOnBuildingActor` whenever the damage tags hold `Gameplay.Effect.GenerateWeakSpots`
//!   (gate vfunc 414 @ 140491c80, no player-built exclusion). So the pickaxe makes weak spots on player builds exactly as on trees and rocks.
//! * RESOURCES - none. The harvest gate (vfunc 415 @ 140491b40: `bAllowResourceDrop` + tag `Gameplay.Effect.HarvestResources` + `GameMode` vfunc 296, true in Athena)
//!   passes, but the amount is `MaxResourcesToSpawn` (`ABuildingSMActor::DetermineMaxResourcesToSpawn` @ 1404687d0): with no `BuildingResourceAmountOverride`
//!   row (PBWA CDOs have none; trees name e.g. `ResourceRates.ResourceWoodHigh`) it is `GetBuildingCost * GetBuildingDeconCostMult(.., bIsPlayerBuildable)`,
//!   and `DefaultGameData.PlayerBuildingDeconCostMult_{Wood,Stone,Metal}` is 0 at every level. Hitting a player build yields 0 materials.
//! * DESTRUCTION - health <= 0 kills the piece; a structurally unsupported piece dies through `InstantDeath.StructuralSupport` (see [`crate::structural`]).
//!   Either death schedules the neighbours' structural checks. [`DestroyedPiece`] is the event the visuals / audio consume (`DeathParticles`,
//!   `DeathSound` of the class CDO; `bPlayDestructionEffects` ctor default 1).
use glam::Vec3;

use crate::pieces::{Material, PieceKind};
use crate::structural::{Collapse, Structural, StructuralEnv};
use crate::world::{BuildingWorld, DamageOutcome, PieceId, PlacedPiece};

/// `EnvDmgPB` of `AthenaMeleeWeapons.Harvest_Pickaxe_Athena_C_T01`.
pub const PICKAXE_ENV_DAMAGE: f32 = 50.;
/// `SurfaceDamageTeams.HarvestBiasHigh.Friendly` (the pickaxe's `MeleeHarvest` row for the Wood / Stone / Metal surfaces), see the module docs.
pub const PICKAXE_FRIENDLY_BUILDING_RATIO: f32 = 0.5;
/// `SurfaceDamageTeams.WeakSpotHigh.Friendly` (`MeleeHarvest` for WeakSpot_Wood / _Stone / _Metal).
pub const PICKAXE_FRIENDLY_WEAK_SPOT_RATIO: f32 = 1.0;
/// Pickaxe damage to one of the player's own pieces.
pub const PICKAXE_DAMAGE: f32 = PICKAXE_ENV_DAMAGE * PICKAXE_FRIENDLY_BUILDING_RATIO;
/// Pickaxe damage on a weak spot of one of the player's own pieces.
pub const PICKAXE_WEAK_SPOT_DAMAGE: f32 = PICKAXE_ENV_DAMAGE * PICKAXE_FRIENDLY_WEAK_SPOT_RATIO;
/// Materials gained per hit on a player build (`PlayerBuildingDeconCostMult_*` = 0, see the module docs).
pub const PLAYER_BUILD_RESOURCES_PER_HIT: u32 = 0;
/// `PlayerBuildingSkillLevel` in Athena (see the module docs): the curve time the health rows are evaluated at.
pub const ATHENA_BUILDING_LEVEL: f32 = 0.;

/// Building upgrade tier of the class (`PBWA_W1` .. `PBWA_W3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier { L1, L2, L3 }

/// `AttributesBuildingSection` `<PlayerBuilding{Mat}{Tier}>.<Size>.FortHealthSet.MaxHealth` at t = 0, indexed [material][tier][Size9 wall = 0, Size7 others = 1].
pub const MAX_HEALTH_L0: [[[f32; 2]; 3]; 3] = [
    // Wood1/2/3
    [[200., 190.], [450., 430.], [700., 665.]],
    // Stone1/2/3
    [[300., 280.], [675., 630.], [1050., 980.]],
    // Metal1/2/3
    [[400., 370.], [900., 835.], [1400., 1295.]],
];

/// `AttributeInitKey` of a piece class: (category, sub category). Wall `Size9`, floor / stairs / roof `Size7` (PBWA CDOs).
pub fn attribute_key(material: Material, kind: PieceKind, tier: Tier) -> (String, &'static str) {
    let m = match material { Material::Wood => "Wood", Material::Stone => "Stone", Material::Metal => "Metal" };
    let t = match tier { Tier::L1 => 1, Tier::L2 => 2, Tier::L3 => 3 };
    (format!("PlayerBuilding{m}{t}"), if kind == PieceKind::Wall { "Size9" } else { "Size7" })
}

/// Max health of a piece in Athena (level 0).
pub fn max_health(material: Material, kind: PieceKind, tier: Tier) -> f32 {
    let m = match material { Material::Wood => 0, Material::Stone => 1, Material::Metal => 2 };
    let t = match tier { Tier::L1 => 0, Tier::L2 => 1, Tier::L3 => 2 };
    MAX_HEALTH_L0[m][t][if kind == PieceKind::Wall { 0 } else { 1 }]
}

/// Tier from a PBWA class path (`.../PBWA_W2_Solid.PBWA_W2_Solid_C` -> L2). Unknown -> L1.
pub fn tier_of_class(class_path: &str) -> Tier {
    let name = class_path.rsplit(['/', '.']).next().unwrap_or(class_path);
    let b = name.as_bytes();
    match b.iter().position(|&c| c == b'_').and_then(|i| b.get(i + 2)) { Some(b'2') => Tier::L2, Some(b'3') => Tier::L3, _ => Tier::L1 }
}

/// What hit the piece.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    /// Pickaxe swing; `weak_spot` = the camera line hit the piece's weak spot (critical).
    Pickaxe { weak_spot: bool },
    /// A bullet / pellet / projectile: `env_damage` = the weapon's `EnvDmg` at the hit range (caller evaluates the falloff).
    Weapon { env_damage: f32 },
}

impl Hit {
    pub fn damage(&self) -> f32 {
        match *self { Hit::Pickaxe { weak_spot: true } => PICKAXE_WEAK_SPOT_DAMAGE, Hit::Pickaxe { weak_spot: false } => PICKAXE_DAMAGE, Hit::Weapon { env_damage } => env_damage.max(0.) }
    }
}

/// Why a piece was destroyed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DestroyCause {
    /// Health reached 0 from a hit.
    Damage(Hit),
    /// `Gameplay.Effect.InstantDeath.StructuralSupport` (no longer connected to the ground).
    StructuralCollapse,
}

/// A destroyed piece: the event the destroy FX / audio consume (visuals agent). Everything needed to place the effect, since the piece is gone from the world.
#[derive(Clone, Debug, PartialEq)]
pub struct DestroyedPiece {
    pub id: PieceId,
    pub kind: PieceKind,
    pub material: Material,
    /// The class in effect when it died (the edited class if edited): its CDO holds `DeathParticles` / `DeathSound` / `DeathParticleSocketName`.
    pub class_path: String,
    pub location: Vec3,
    pub yaw_deg: f32,
    pub cause: DestroyCause,
    /// World time of the death (`BuildingWorld::time`).
    pub time: f64,
}

impl DestroyedPiece {
    fn of(p: &PlacedPiece, cause: DestroyCause, time: f64) -> Self {
        DestroyedPiece { id: p.id, kind: p.kind, material: p.material, class_path: p.effective_class().to_owned(), location: p.location, yaw_deg: p.yaw_deg, cause, time }
    }
}

/// Result of [`BuildingDamage::apply_hit`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitResult {
    pub damage: f32,
    pub health: f32,
    pub destroyed: bool,
    /// Materials the hitter gains (always 0 for player builds).
    pub resources: u32,
}

/// Damage + structural driver: apply hits, run the structural timers, emit [`DestroyedPiece`] events.
#[derive(Default)]
pub struct BuildingDamage {
    pub structural: Structural,
    destroyed: Vec<DestroyedPiece>,
    /// Every destroyed piece ever (bounded), with a cursor API like `BuildingWorld::events_since`.
    log: Vec<DestroyedPiece>,
    log_base: u64,
}

const LOG_CAP: usize = 4096;

impl BuildingDamage {
    pub fn new() -> Self { Self::default() }

    /// Applies one hit. Kills the piece at 0 health (removing it from `world`) and schedules its neighbours' structural checks.
    /// STRUCTURAL FIX (2026-10-05): `env` (world collision, class cubes, grid origin) replaces the old `world_supported` closure; see
    /// `crate::structural`.
    pub fn apply_hit(&mut self, world: &mut BuildingWorld, id: PieceId, hit: Hit, env: &StructuralEnv) -> Option<HitResult> {
        // register this frame's placements before the world changes (their registration sweeps / groups)
        self.structural.sync(world, env);
        let before = world.get(id)?.clone();
        let damage = hit.damage();
        match world.apply_damage(id, damage) {
            DamageOutcome::NoSuchPiece => None,
            DamageOutcome::Damaged { health } => Some(HitResult { damage, health, destroyed: false, resources: PLAYER_BUILD_RESOURCES_PER_HIT }),
            DamageOutcome::Destroyed => {
                self.died(world, &before, DestroyCause::Damage(hit), env);
                Some(HitResult { damage, health: 0., destroyed: true, resources: PLAYER_BUILD_RESOURCES_PER_HIT })
            }
        }
    }

    /// A piece that left the world through a death (already removed): event + `ABuildingSMActor` vfunc 240 (schedule the cube-connected
    /// neighbours' checks, leave the cell map, dissolve the group).
    pub fn died(&mut self, world: &BuildingWorld, dead: &PlacedPiece, cause: DestroyCause, env: &StructuralEnv) {
        let now = world.time();
        self.structural.on_piece_died(world, dead, now, env);
        let ev = DestroyedPiece::of(dead, cause, now);
        self.destroyed.push(ev.clone());
        if self.log.len() >= LOG_CAP { let drop = LOG_CAP / 2; self.log.drain(..drop); self.log_base += drop as u64; }
        self.log.push(ev);
    }

    /// Runs the structural checks due by `world.time()` in due order, one timer at a time as the timer manager does: an unsupported piece
    /// dies at once (`InstantDeath.StructuralSupport`, its death regroups and schedules its own neighbours) before the next due check runs.
    /// Call after `world.tick`.
    pub fn tick(&mut self, world: &mut BuildingWorld, env: &StructuralEnv) -> Vec<Collapse> {
        let now = world.time();
        let mut out = vec![];
        while let Some(r) = self.structural.run_next_due(world, now, env) {
            let Some(c) = r else { continue };
            if let Some(p) = world.remove(c.id) { self.died(world, &p, DestroyCause::StructuralCollapse, env); }
            out.push(c);
        }
        out
    }

    /// Destroyed pieces since the last call (single consumer convenience).
    pub fn take_destroyed(&mut self) -> Vec<DestroyedPiece> { std::mem::take(&mut self.destroyed) }

    /// Destroyed pieces with sequence number >= `cursor`, and the next cursor (multi consumer, like `BuildingWorld::events_since`).
    pub fn destroyed_since(&self, cursor: u64) -> (&[DestroyedPiece], u64) {
        let start = cursor.max(self.log_base);
        let off = ((start - self.log_base) as usize).min(self.log.len());
        (&self.log[off..], self.log_base + self.log.len() as u64)
    }
}

// ---- hit geometry (traces) ---------------------------------------------------------------------------------------------------
// The pieces' trace shapes in their local frame (see `crate::structural`): the cooked meshes' bounds, not their collision meshes (not read):
//  wall = box ExtendedBounds of PBW_W1_Solid (extent 256.2 x 24.0 x 194.0 about (-0.2, 1.2, 194.0)); floor = PBW_W1_Floor (258.6 x 261.1 x 14.0 about
//  (-1.5, 254.0, 3.7)); ramp = a slab from the low edge (y 512, z 0) to the high edge (y 0, z 384), half thickness 14 (the floor's); roof = the 4 faces of
//  the pyramid from the base square to the apex (0, 256, 179.5) = top of PBW_W1_RoofC's bounds (84.9 + 94.6).
const WALL_BOX: (Vec3, Vec3) = (Vec3::new(-0.165, 1.249, 194.027), Vec3::new(256.165, 24.019, 194.031));
const FLOOR_BOX: (Vec3, Vec3) = (Vec3::new(-1.515, 254.050, 3.679), Vec3::new(258.645, 261.108, 14.047));
const RAMP_HALF_THICKNESS: f32 = 14.047;
const ROOF_APEX: Vec3 = Vec3::new(0., 256., 179.49);

fn to_local(p: &PlacedPiece, v: Vec3, is_dir: bool) -> Vec3 {
    let (s, c) = p.yaw_deg.to_radians().sin_cos();
    let d = if is_dir { v } else { v - p.location };
    Vec3::new(d.x * c + d.y * s, -d.x * s + d.y * c, d.z)
}
fn dir_to_world(p: &PlacedPiece, v: Vec3) -> Vec3 { let (s, c) = p.yaw_deg.to_radians().sin_cos(); Vec3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z) }

/// Ray vs box (slab test) in a frame; returns (t, local normal).
fn ray_box(o: Vec3, d: Vec3, centre: Vec3, half: Vec3) -> Option<(f32, Vec3)> {
    let (mut tmin, mut tmax, mut n) = (f32::NEG_INFINITY, f32::INFINITY, Vec3::ZERO);
    for a in 0..3 {
        let (oa, da, lo, hi) = (o[a], d[a], centre[a] - half[a], centre[a] + half[a]);
        if da.abs() < 1e-9 { if oa < lo || oa > hi { return None; } continue; }
        let (mut t0, mut t1) = ((lo - oa) / da, (hi - oa) / da);
        let mut sign = -1.;
        if t0 > t1 { std::mem::swap(&mut t0, &mut t1); sign = 1.; }
        if t0 > tmin { tmin = t0; n = Vec3::ZERO; n[a] = sign; }
        tmax = tmax.min(t1);
        if tmin > tmax { return None; }
    }
    if tmax < 0. { return None; }
    Some((tmin.max(0.), n))
}

fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, Vec3)> {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2); let det = e1.dot(p);
    if det.abs() < 1e-9 { return None; }
    let inv = 1. / det; let s = o - a; let u = s.dot(p) * inv;
    if !(0. ..=1.).contains(&u) { return None; }
    let q = s.cross(e1); let v = d.dot(q) * inv;
    if v < 0. || u + v > 1. { return None; }
    let t = e2.dot(q) * inv;
    if t < 0. { return None; }
    let mut n = e1.cross(e2).normalize_or_zero(); if n.dot(d) > 0. { n = -n; }
    Some((t, n))
}

/// Ray (UE cm, `dir` unit) against one piece, `inflate` = sweep radius (cm). Returns (distance cm, world normal).
pub fn trace_piece(p: &PlacedPiece, origin: Vec3, dir: Vec3, inflate: f32) -> Option<(f32, Vec3)> {
    let (o, d) = (to_local(p, origin, false), to_local(p, dir, true));
    let hit = match p.kind {
        PieceKind::Wall => ray_box(o, d, WALL_BOX.0, WALL_BOX.1 + Vec3::splat(inflate)),
        PieceKind::Floor => ray_box(o, d, FLOOR_BOX.0, FLOOR_BOX.1 + Vec3::splat(inflate)),
        PieceKind::Stairs => {
            // slab frame: x across, u up the slope (from the low edge to the high edge), n its normal
            let (low, high) = (Vec3::new(0., 512., 0.), Vec3::new(0., 0., 384.));
            let len = (high - low).length(); let u = (high - low) / len; let n = Vec3::X.cross(u);
            let rel = o - (low + high) * 0.5;
            let (lo, ld) = (Vec3::new(rel.x, rel.dot(u), rel.dot(n)), Vec3::new(d.x, d.dot(u), d.dot(n)));
            ray_box(lo, ld, Vec3::ZERO, Vec3::new(256. + inflate, len * 0.5 + inflate, RAMP_HALF_THICKNESS + inflate)).map(|(t, ln)| (t, Vec3::X * ln.x + u * ln.y + n * ln.z))
        }
        PieceKind::Roof => {
            let c = [Vec3::new(-256., 0., 0.), Vec3::new(256., 0., 0.), Vec3::new(256., 512., 0.), Vec3::new(-256., 512., 0.)];
            (0..4).filter_map(|i| ray_tri(o, d, c[i], c[(i + 1) % 4], ROOF_APEX)).min_by(|a, b| a.0.total_cmp(&b.0))
        }
    };
    hit.map(|(t, n)| (t, dir_to_world(p, n).normalize_or_zero()))
}

/// Nearest piece along a ray within `max_cm`. Returns (piece, distance cm, world normal).
pub fn trace(world: &BuildingWorld, origin: Vec3, dir: Vec3, max_cm: f32, inflate: f32) -> Option<(PieceId, f32, Vec3)> {
    let mut best: Option<(PieceId, f32, Vec3)> = None;
    for p in world.iter() {
        if let Some((t, n)) = trace_piece(p, origin, dir, inflate) {
            if t <= max_cm && best.map_or(true, |b| t < b.1) { best = Some((p.id, t, n)); }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{GridKey, PieceSpawn};
    use crate::structural::tests::place;
    use crate::structural::FlatGround;
    use glam::IVec3;
    use std::collections::HashMap;
    const GROUND: FlatGround = FlatGround(Some(0.));

    fn put(w: &mut BuildingWorld, kind: PieceKind, material: Material, loc: Vec3, yaw: f32, tag: i32) -> PieceId {
        let hp = max_health(material, kind, Tier::L1);
        w.spawn(PieceSpawn { kind, material, class_path: "/Game/Building/ActorBlueprints/Player/Wood/L1/PBWA_W1_Solid.PBWA_W1_Solid_C".into(),
            key: GridKey { kind, cell: IVec3::new(tag, 0, 0), rot: 0 }, location: loc, yaw_deg: yaw, max_health: hp, start_health: hp, build_time: 0. })
    }

    #[test]
    fn health_table_and_keys() {
        assert_eq!(max_health(Material::Wood, PieceKind::Wall, Tier::L1), 200.);
        assert_eq!(max_health(Material::Wood, PieceKind::Floor, Tier::L1), 190.);
        assert_eq!(max_health(Material::Stone, PieceKind::Stairs, Tier::L1), 280.);
        assert_eq!(max_health(Material::Metal, PieceKind::Roof, Tier::L3), 1295.);
        assert_eq!(attribute_key(Material::Stone, PieceKind::Wall, Tier::L2), ("PlayerBuildingStone2".into(), "Size9"));
        assert_eq!(tier_of_class("/Game/Building/ActorBlueprints/Player/Metal/L3/PBWA_M3_RoofC.PBWA_M3_RoofC_C"), Tier::L3);
        assert_eq!(tier_of_class("/Game/Building/ActorBlueprints/Player/Wood/L1/PBWA_W1_Solid.PBWA_W1_Solid_C"), Tier::L1);
    }

    /// Data check: the table equals `AttributesBuildingSection` at t = 0 and `PlayerBuildingDeconCostMult_*` is 0 (FN_TEST_ROOT = the 1.8 FortniteGame dir).
    #[test]
    fn health_table_matches_the_paks() {
        let Ok(root) = fn_live::var("FN_TEST_ROOT") else { return };
        let set = fn_pak::PakSet::open_install(std::path::Path::new(&root)).expect("paks");
        let curves = |pkg_path: &str| -> std::collections::HashMap<String, f64> {
            let pd = fn_uasset::PackageData::load(&set, pkg_path).expect("pkg");
            let pkg = pd.package().expect("parse");
            let i = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i) == "CurveTable").expect("curve table");
            let t = fn_uasset::classes::read_table(&pkg, i).expect("table");
            t.rows.iter().filter_map(|(n, p)| { let j = p.to_json(&pkg); Some((n.clone(), j.get("Keys")?.as_array()?.first()?.get("Value")?.as_f64()?)) }).collect()
        };
        let hs = curves("/Game/Balance/DataTables/AttributesBuildingSection");
        for m in Material::ALL { for t in [Tier::L1, Tier::L2, Tier::L3] { for k in PieceKind::ALL {
            let (cat, size) = attribute_key(m, k, t);
            let row = format!("{cat}.{size}.FortHealthSet.MaxHealth");
            assert_eq!(hs[&row] as f32, max_health(m, k, t), "{row}");
        } } }
        let gd = curves("/Game/Balance/DataTables/DefaultGameData");
        for m in ["Wood", "Stone", "Metal"] { assert_eq!(gd[&format!("PlayerBuildingDeconCostMult_{m}")], 0.); }
    }

    #[test]
    fn eight_pickaxe_hits_break_a_wood_wall_and_its_floor_follows() {
        // STRUCTURAL FIX (2026-10-05): grid-snapped pieces and the exact structural env (flat ground at z 0, base cubes)
        let mut w = BuildingWorld::new();
        let hp = max_health(Material::Wood, PieceKind::Wall, Tier::L1);
        let wall = place(&mut w, PieceKind::Wall, IVec3::ZERO, 0, hp); // +X edge of cell 0
        let floor = place(&mut w, PieceKind::Floor, IVec3::Z, 0, max_health(Material::Wood, PieceKind::Floor, Tier::L1));
        let mut dmg = BuildingDamage::new();
        let e = StructuralEnv::with_collision(&GROUND);
        for _ in 0..7 { assert!(!dmg.apply_hit(&mut w, wall, Hit::Pickaxe { weak_spot: false }, &e).unwrap().destroyed); } // 200 HP / 25
        let r = dmg.apply_hit(&mut w, wall, Hit::Pickaxe { weak_spot: false }, &e).unwrap();
        assert!(r.destroyed && r.resources == 0);
        assert_eq!(dmg.take_destroyed().len(), 1);
        // STRUCTURAL FIX 2: the frame's timer tick (as the hook runs it right after the hits) pushes the pending timer at t = 0
        assert!(dmg.tick(&mut w, &e).is_empty());
        w.tick(0.3);
        assert!(dmg.tick(&mut w, &e).is_empty(), "0.35 s: the floor is looked up across the wall, one cell from the wall's cell");
        w.tick(0.06);
        assert_eq!(dmg.tick(&mut w, &e).len(), 1);
        assert!(w.get(floor).is_none());
        let ev = dmg.take_destroyed();
        assert_eq!(ev[0].cause, DestroyCause::StructuralCollapse);
        let (all, _) = dmg.destroyed_since(0);
        assert_eq!(all.len(), 2);
    }

    /// Side view (X/Z) of a chain collapse with each piece's death time; written to `verification/building/structural/` when FN_WRITE_VERIFY is set.
    #[test]
    fn chain_collapse_timeline_image() {
        // STRUCTURAL FIX (2026-10-05): grid-snapped pieces and the exact structural env
        let mut w = BuildingWorld::new();
        let mut ids = vec![];
        // a two-wall-high pillar on the +X edge of cell 0 holding a 5-floor bridge at level 2 over cells 0..4, walls on the floors' -Y edges
        let hp = max_health(Material::Wood, PieceKind::Wall, Tier::L1);
        ids.push(place(&mut w, PieceKind::Wall, IVec3::ZERO, 0, hp));
        ids.push(place(&mut w, PieceKind::Wall, IVec3::Z, 0, hp));
        for k in 0..5 { ids.push(place(&mut w, PieceKind::Floor, IVec3::new(k, 0, 2), 0, hp)); }
        for k in 0..5 { ids.push(place(&mut w, PieceKind::Wall, IVec3::new(k, 0, 2), 3, hp)); }
        let e = StructuralEnv::with_collision(&GROUND);
        let mut dmg = BuildingDamage::new();
        let mut deaths: HashMap<PieceId, f64> = HashMap::new();
        let before: HashMap<PieceId, PlacedPiece> = ids.iter().map(|&i| (i, w.get(i).cloned().unwrap())).collect();
        for _ in 0..8 { dmg.apply_hit(&mut w, ids[0], Hit::Pickaxe { weak_spot: false }, &e); }
        for e in dmg.take_destroyed() { deaths.insert(e.id, e.time); }
        for _ in 0..400 { w.tick(0.01); dmg.tick(&mut w, &e); for e in dmg.take_destroyed() { deaths.insert(e.id, e.time); } }
        assert_eq!(deaths.len(), ids.len(), "everything above the broken base falls");
        let mut order: Vec<(f64, PieceId)> = deaths.iter().map(|(k, v)| (*v, *k)).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert!(order.windows(2).all(|p| p[1].0 >= p[0].0));
        let Ok(dir) = fn_live::var("FN_WRITE_VERIFY") else { return };
        use std::fmt::Write;
        let mut svg = String::new();
        let (sx, sz, ox, oz) = (0.25f32, 0.25f32, 80f32, 320f32);
        let _ = write!(svg, r##"<svg xmlns="http://www.w3.org/2000/svg" width="900" height="380" font-family="monospace" font-size="11"><rect width="900" height="380" fill="#fff"/><text x="10" y="18">1.8 structural collapse: the base wall is broken at t=0; label = death time (s). Delay per hop = 0.25 + 0.1 x cell distance (UFortGameData ctor)</text><line x1="0" y1="{}" x2="900" y2="{}" stroke="#7a5"/>"##, oz, oz);
        let piece_of = before;
        for &i in &ids {
            let p = &piece_of[&i];
            let t = deaths[&i];
            let shade = (255. - (t as f32 / 2.0).min(1.) * 200.) as u8;
            let (x0, x1, z0, z1) = match p.kind {
                PieceKind::Floor => { let a = crate::structural::to_world(p, Vec3::new(0., 0., 0.)); let b = crate::structural::to_world(p, Vec3::new(0., 512., 0.)); (a.x.min(b.x), a.x.max(b.x), p.location.z - 8., p.location.z + 8.) }
                _ => (p.location.x - 10., p.location.x + 10., p.location.z, p.location.z + 384.),
            };
            let _ = write!(svg, r##"<rect x="{}" y="{}" width="{}" height="{}" fill="rgb(255,{shade},{shade})" stroke="#333"/><text x="{}" y="{}">{:.2}</text>"##,
                ox + x0 * sx, oz - z1 * sz, (x1 - x0) * sx, (z1 - z0) * sz, ox + (x0 + x1) * 0.5 * sx - 12., oz - z1 * sz - 4., t);
        }
        svg.push_str("</svg>");
        let path = std::path::Path::new(&dir).join("chain_collapse_timeline.svg");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, svg).unwrap();
        let mut log = String::from("piece death times (s) in order:\n");
        for (t, i) in order { let p = &piece_of[&i]; let _ = writeln!(log, "{t:6.2}  {:?} at {:?}", p.kind, p.location); }
        std::fs::write(std::path::Path::new(&dir).join("chain_collapse_timeline.txt"), log).unwrap();
    }

    #[test]
    fn traces_hit_each_shape() {
        let mut w = BuildingWorld::new();
        let wall = put(&mut w, PieceKind::Wall, Material::Wood, Vec3::ZERO, 90., 0); // panel in x = 0, spanning y -256..256
        let (id, t, n) = trace(&w, Vec3::new(-500., 0., 150.), Vec3::X, 10_000., 0.).unwrap();
        assert_eq!(id, wall);
        assert!((t - (500. - 24.019 - 1.249)).abs() < 1., "{t}");
        assert!(n.x < -0.9);
        let ramp = put(&mut w, PieceKind::Stairs, Material::Wood, Vec3::new(2000., 0., 0.), 0., 1);
        let (id, t, _) = trace(&w, Vec3::new(2000., 256., 1000.), -Vec3::Z, 10_000., 0.).unwrap();
        assert_eq!(id, ramp);
        assert!((t - (1000. - 192. - 14.047 * 640. / 512.)).abs() < 2., "{t}");
        let roof = put(&mut w, PieceKind::Roof, Material::Wood, Vec3::new(-3000., 0., 0.), 0., 2);
        let (id, t, _) = trace(&w, Vec3::new(-3000., 256., 1000.), -Vec3::Z, 10_000., 0.).unwrap();
        assert_eq!(id, roof);
        assert!((t - (1000. - 179.49)).abs() < 0.5);
    }
}
