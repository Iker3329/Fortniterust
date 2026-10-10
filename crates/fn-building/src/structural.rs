//! Structural support of player builds (owner: structural / damage agent).
//!
//! What 1.8 does, recovered from the exe (`C:\FortniteRef\decomp\named`, addresses are image VAs). Spec: `docs/spec/building_structural_gaps.md`.
//!
//! * SUPPORT RULE - `UBuildingStructuralSupportSystem::IsBuildingActorStructurallySupported` @ 1404f3dc0: an actor is supported when
//!   `IsSupportedByWorld()` (vtable +0x930 = slot 294: `bSupportedDirectly || bForciblyStructurallySupported`, flag word +0xE90 mask 0x18),
//!   otherwise when its group record (0xA8 bytes at system +0x78, valid flag +0xA0) has at least one world-supported member (`TSet` at
//!   record +0x50, `Num - NumFree > 0`). [`Structural::is_supported`].
//! * WORLD SUPPORT - `bSupportedDirectly` (+0xE90 |= 8) is written only by vtable +0x928 (slot 293), called by the three register functions
//!   (`FUN_1404d9250` floors, `FUN_1404d9d80` walls, `FUN_1404d90f0` centre cells) when their slot check's "WorldOverlapCheck" box sweep
//!   (`FUN_1404df7e0`) touched the world. See [`overlap_box`], [`world_overlap_check`] and the [`WorldCollision`] trait.
//! * GROUPS - `FUN_1404db8e0` (register): every gathered neighbour ([`gather`], `FUN_1404ee8c0`) that is cube-connected ([`connected`],
//!   `FUN_1404daa90`) contributes its group; none -> a new group (`FUN_1404e1840`, only if some connected neighbour has no group), one -> join
//!   (`FUN_1404d8dc0`), several -> merge into the largest (`FUN_1404e0b20`). Ungrouped connected neighbours join too. Members with
//!   +0x930 true also go to the record's world set. On death (`ABuildingSMActor` vfunc 240 @ 14047aa10): `ClearAllTimersForObject`
//!   (`FUN_1424d6510`), schedule (`FUN_1404f4be0`), unregister from the cell maps (`FUN_1404fc7a0`), dissolve (`FUN_1404fc9a0`, see
//!   [`Structural::unregister`]: every former member's group = -1 and each re-registers; the old record stays valid, the largest of the
//!   resulting groups is moved into it and takes the old id, its own record is freed).
//! * EDIT - `ServerEditBuildingActor` -> `ReplaceBuildingActor` (`ABuildingSMActor` vfunc 329 @ 14048afb0): spawn the new actor, its
//!   `InitializeBuildingActor(2, .., old)` (vfunc 247 @ 14047bed0: reason 2 does NOT call `FUN_1404f4be0`), then the OLD actor's vfunc 243
//!   (+0x798 @ 140468b50) -> vfunc 240 (+0x780, the death teardown above: its cube-connected neighbours get collapse timers, it leaves the cell
//!   maps, its group dissolves), then the NEW actor's vfunc 248 (+0x7c0 -> `FUN_140461950`) registers it. See [`Structural::sync`].
//! * PLACEMENT - `ServerCreateBuildingActor_Implementation` @ 140845d70: `InitializeBuildingActor(1, ..)` then vfunc 248 (register).
//!   `InitializeBuildingActor` with reason 1 calls `FUN_1404f4be0(system, newActor)` (14047bed0, `if (param_2 == 1)` branch; both the
//!   animated and the instant build path reach it): the new piece's cube-connected neighbours get a structural timer (which replaces a
//!   pending one), BEFORE the new piece is in the cell maps.
//! * TIMERS - UE4.16 `FTimerManager` (`TimerManager.cpp`; exe: SetTimer `FUN_1424d6af0` -> `FUN_1424d69d0`, Tick `FUN_1424e3070`, HeapPush
//!   `FUN_1424d24c0`): a binary min-heap (`TArray::HeapPush` / `HeapPop` / `HeapRemoveAt`, predicate `ExpireTime <` only, so equal times keep
//!   the heap's structural order), a timer fires when `InternalTime > ExpireTime` (strict); SetTimer before the manager ticked this frame,
//!   or from inside a firing timer, goes to `PendingTimerList` with ExpireTime = delay and is pushed (in list order, ExpireTime += InternalTime)
//!   after the frame's fire loop. Re-setting a handle first clears it (`FindTimer`: executing, heap -> `HeapRemoveAt`, pending -> `RemoveAtSwap`).
//!   [`TimerHeap`]. Only the structural timers are modelled: the real heap also holds every other timer of the world, which can change
//!   the order of EQUAL due times (unknowable here).
//! * WHO IS CHECKED, WHEN - `FUN_1404f4be0` gathers the dead actor's neighbours and calls `FUN_1404cecc0` (wall and floor lists) /
//!   `FUN_1404ce6b0` (centre list): each candidate that passes `FUN_1404daa90` against the dead actor gets `FTimerManager::SetTimer` on its
//!   handle at actor +0xFB0 (a new timer REPLACES a pending one) of `Rand(lo, hi) + Rand(lo2, hi2) * |deadCell - lookupCell|` (Manhattan;
//!   `lookupCell` = the cell the gather looked the candidate up with, NOT the candidate's own cell), with `lo/hi = clamp(D -/+ V*D, 0.1, FLT_MAX)`
//!   and `lo2/hi2 = clamp(A -/+ VA*A, 0, FLT_MAX)` (0x143fe36f0 = 0.1, 0x144008e5c = FLT_MAX, `Rand` = `rand() * (1/32767)` 0x143fe36ec).
//!   D, V, A, VA = `UFortGameData` +0x4B0..+0x4BC (`BuildingStructuralCollapseDelay` ...): not serialised by `/Game/Balance/DefaultGameData`,
//!   so the ctor values (@ 1405751d0: 0.25 / 0 / 0.1 / 0) apply: 0.25 s + 0.1 s per cell, no randomness.
//! * THE TIMER - `ABuildingSMActor` vfunc 366 @ 1404665c0: if NOT `IsStructurallySupported` (@ 14047fde0) and `LastStructuralCheck` (+0xCB0) !=
//!   Unstable: `OnStructurallyUnstable` (InstantDeath `Gameplay.Effect.InstantDeath.StructuralSupport`) and `= Unstable`; if supported and != Stable:
//!   `= Stable`. The collapsed piece's death runs vfunc 240 again, so the collapse walks outward one hop at a time.
//! * GRID - cells of 512 x 512 x 384 cm (`FUN_1404ea060`: `ROUND(2u + 0.5) >> 1` = `floor(u + 0.5)`, origin system +0x3C4 = the level origin,
//!   here `StructuralEnv::grid_origin`). The location of a piece is vfunc +0x8C8 = slot 281 @ 140477f30: `location - rot(BaseLocToPivotOffset)`
//!   ((0,-256,0) from the Wall @14049fe20 / Floor @140459d10 / Roof @14045b660 ctors, stairs via the floor ctor), the plain location when
//!   +0x554 bit 0x10 is set (wall ctor) = [`crate::grid::grid_location`].
//!
//! NOT PORTED / ASSUMED:
//! * `FUN_14048d780` (the per-actor index written next to the group id) is not needed by any support decision and is not kept.
//! * The connectivity result cache (system +0x28, `FUN_1404e7b40` / `FUN_1404dd030`) only memoises a pure function; not kept.
//! * The group records' member sets are kept ordered by piece id; the exe's `TSet` iterates in sparse-array (insertion / free-slot) order,
//!   which decides the re-registration order in a dissolve and the first-of-equal-size group in a merge or rename.
//! * The port keeps the edited piece's `PieceId` for the new actor (the exe spawns a new actor); the old actor's timers are cleared first
//!   (`ClearAllTimersForObject`), then the new one gets its own structural timer of `BuildingStructuralCollapseDelay` (0.25 s;
//!   `InitializeBuildingActor(2)` @14047bed0 -> `FUN_1424d6af0` on the +0xFB0 handle, set in the exe before the old actor's teardown, so it
//!   precedes the neighbours' timers) and `LastStructuralCheck` = Max_None (taken as the new actor's ctor value, not traced).
//! * `bRegisterWithStructuralGrid` (+0xE90 bit 0x20) is taken as set on every player piece (without it `IsStructurallySupported` is always false).
use std::collections::{BTreeSet, HashMap, HashSet};

use glam::{IVec3, Vec3};

use crate::grid::{self, cell_of, on_level, wall_side, Slot};
use crate::pieces::{ConnectivityCube, PieceKind};
pub use crate::placement::{connected, voxel, Voxel};
use crate::world::{BuildingWorld, PieceId, PlacedPiece, WorldEvent};

/// Structural grid cell size, cm (`FUN_1404dd690`: X/Y `<< 9`, Z `* 0x180`).
pub const CELL_XY: f32 = 512.;
pub const CELL_Z: f32 = 384.;
/// `UFortGameData::BuildingStructuralCollapseDelay` (ctor @ 1405751d0, not overridden by DefaultGameData).
pub const COLLAPSE_DELAY: f32 = 0.25;
/// `...CollapseDelayVariance` (ctor default 0).
pub const COLLAPSE_DELAY_VARIANCE: f32 = 0.;
/// `...CollapseCellDistAdditiveDelay` (ctor default 0.1).
pub const COLLAPSE_CELL_DIST_DELAY: f32 = 0.1;
/// `...CollapseCellDistAdditiveDelayVariance` (ctor default 0).
pub const COLLAPSE_CELL_DIST_DELAY_VARIANCE: f32 = 0.;
/// Lower clamp of the base delay range (`DAT_143fe36f0`, `FUN_1404f4be0`).
pub const COLLAPSE_DELAY_MIN: f32 = 0.1;
/// `rand()` scale (`DAT_143fe36ec` = 1 / 32767).
const RAND_SCALE: f32 = 1. / 32767.;

/// `ConnectivityCubeData` of the four BR build-slot patterns (`EMP_Wall_Solid`, `EMP_Floor_Floor`, `EMP_Stair_StairW`, `EMP_Roof_RoofC`, 1.8 paks,
/// checked by `tests::base_cubes_match_the_paks`). Used when a piece's class cube is not known (no catalog, e.g. unit tests).
pub fn base_cube(kind: PieceKind) -> ConnectivityCube {
    match kind {
        PieceKind::Wall => [0x000F_C63F, 0x0008_4210, 0, 0x0000_8421, 0x01F0_0000, 0x01F0_0000],
        PieceKind::Floor => [0x000F_8000, 0x000F_8000, 0x000F_8000, 0x000F_8000, 0, 0x01F8_D63F],
        PieceKind::Stairs => [0x000F_8000, 0x0008_0001, 0x0000_001F, 0x0000_8010, 0x0000_001F, 0x01F0_0000],
        PieceKind::Roof => [0x000F_8000, 0x000F_8000, 0x000F_8000, 0x000F_8000, 0x0000_1000, 0x01F8_C63F],
    }
}

/// `EStructuralSupportCheck` (SDK): Stable 0, Unstable 1, Max_None 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportCheck { Stable, Unstable, None }

/// Structural cell of a location relative to the grid origin (UE cm): `floor(loc / size + 0.5)` per axis (`FUN_1404ea060`).
pub fn structural_cell(loc: Vec3) -> IVec3 { cell_of(loc) }

/// Manhattan distance between two cells (`FUN_1404cecc0`: `|dx| + |dy| + |dz|`).
pub fn cell_distance(a: IVec3, b: IVec3) -> i32 { (a - b).abs().element_sum() }

/// The collapse timer delay for a neighbour `cells` away (`FUN_1404cecc0`); `r1`, `r2` are two `rand()` draws (0..=32767).
pub fn collapse_delay(cells: i32, r1: i32, r2: i32) -> f32 {
    let (d, v) = (COLLAPSE_DELAY, COLLAPSE_DELAY_VARIANCE);
    let lo = (d - v * d).max(COLLAPSE_DELAY_MIN);
    let hi = (d + v * d).max(COLLAPSE_DELAY_MIN);
    let (a, va) = (COLLAPSE_CELL_DIST_DELAY, COLLAPSE_CELL_DIST_DELAY_VARIANCE);
    let lo2 = (a - va * a).max(0.);
    let hi2 = (a + va * a).max(0.);
    r1 as f32 * RAND_SCALE * (hi - lo) + lo + (r2 as f32 * RAND_SCALE * (hi2 - lo2) + lo2) * cells as f32
}

/// MSVC CRT `rand()` (the exe calls the CRT): `state = state * 214013 + 2531011`, result `(state >> 16) & 0x7fff`, default seed 1.
#[derive(Clone, Debug)]
pub struct CrtRand(pub u32);
impl Default for CrtRand { fn default() -> Self { CrtRand(1) } }
impl CrtRand {
    pub fn next(&mut self) -> i32 { self.0 = self.0.wrapping_mul(214013).wrapping_add(2531011); ((self.0 >> 16) & 0x7fff) as i32 }
}

/// Local -> world transform of a placed piece (yaw about Z, UE degrees). Used by the damage hook's weak-spot placement.
pub fn to_world(p: &PlacedPiece, local: Vec3) -> Vec3 {
    let (s, c) = p.yaw_deg.to_radians().sin_cos();
    p.location + Vec3::new(local.x * c - local.y * s, local.x * s + local.y * c, local.z)
}

/// Lowest point (UE z, cm) of a piece: every 1.8 player piece's mesh starts at local Z = 0.
pub fn bottom_z(p: &PlacedPiece) -> f32 { p.location.z }

// =================================================================================================================================
// World support: the "WorldOverlapCheck" sweep (FUN_1404df7e0)
// =================================================================================================================================

/// What a "WorldOverlapCheck" sweep hit (`FUN_1404df7e0` sorts hits into these cases).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorldHitKind {
    /// An `ABuildingActor` whose hit component BLOCKS channel 0x18 (`GetCollisionResponseToChannel(0x18) == 2`, vtable +0x488).
    /// Flags from its +0x564: bit 0 `bDoNotBlockBuildings`, bit 1 `bDestroyOnPlayerBuildingPlacement`, bit 2 `bUseCentroidForBlockBuildingsCheck`;
    /// `centroid` = vtable +0x7F0. (Placed player pieces have `bDoNotBlockBuildings` and are never reported here; a non-blocking building
    /// actor is [`WorldHitKind::Other`].)
    BuildingActor { do_not_block_buildings: bool, destroy_on_player_building_placement: bool, use_centroid: bool, centroid: Vec3 },
    /// An `AStaticMeshActor` hit through a `UStaticMeshComponent`.
    StaticMeshActor,
    /// A `ULandscapeHeightfieldCollisionComponent` (the terrain).
    Landscape,
    /// Anything else (ignored by the sweep).
    Other,
}

/// One hit of the sweep. `location` = `FHitResult::Location` (+0x10; the sweep reads its z at +0x14).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldHit { pub kind: WorldHitKind, pub location: Vec3, /** the hit actor (`PlacementEnv::props` id) so placement can destroy it (`bDestroyOnPlayerBuildingPlacement`); None for the ground / meshes */ pub actor: Option<u64> }

/// The world's collision as the structural code queries it: `FUN_14209d330`, a multi-hit box sweep (object query params `FUN_140297ab0`
/// / `DAT_145198f30`; probably `SweepMultiByObjectType`, UNCONFIRMED) of an axis-aligned box of `half_extent` from `start` to `end`.
/// [`Ground`] is the landscape (TERRAIN) with the flat plane under its holes; [`FlatGround`] is the sandbox baseplate.
pub trait WorldCollision {
    fn sweep_box(&self, start: Vec3, end: Vec3, half_extent: Vec3) -> Vec<WorldHit>;
}

/// The port's current ground: the flat baseplate at height `z` (world cm), standing in for the landscape (it always supports).
///
/// BASEPLATE LIMITATION (STRUCTURAL FIX 2, 2026-10-05): the build grid is anchored on the Athena LEVEL origin (`PlacementEnv::grid_origin`),
/// as the exe's is (system +0x3C4), so the floor planes are the level's (multiples of 384 cm from the level origin), not the baseplate's.
/// On the sandbox plate a ground-level floor snaps 149 cm below the plate, outside the floor sweep (+15 / -18 cm), and is not world
/// supported; walls / ramps / roofs (+375 / -18) still reach it. That is the exe's rule applied to a flat plate at a height the real
/// terrain does not have, so it is NOT patched here: every ground-support decision (registration, the placement query) goes through
/// [`WorldCollision`], and the real landscape / static mesh collision replaces this type once the port has it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlatGround(pub Option<f32>);
impl WorldCollision for FlatGround {
    fn sweep_box(&self, start: Vec3, end: Vec3, half: Vec3) -> Vec<WorldHit> {
        let Some(z) = self.0 else { return vec![] };
        // the box's lowest face goes from start.z - half.z down to end.z - half.z (and the top face likewise): it touches the plane when
        // the plane lies within the swept span (an initially overlapping box is a hit too)
        let (lo, hi) = (start.z.min(end.z) - half.z, start.z.max(end.z) + half.z);
        if z < lo || z > hi { return vec![]; }
        let t = if (start.z - end.z).abs() > 1e-6 { ((start.z - z) / (start.z - end.z)).clamp(0., 1.) } else { 0. };
        vec![WorldHit { kind: WorldHitKind::Landscape, location: start + (end - start) * t, actor: None }]
    }
}

/// TERRAIN: the port's ground as the structural sweep sees it: the 1.8 landscape (`ULandscapeHeightfieldCollisionComponent` -> [`WorldHitKind::Landscape`]) in
/// world cm (`fn_terrain::Local` with the movement scene's z offset), and the flat plane `flat_z` where no landscape is installed or a hole of it is under the box.
///
/// The swept box is the exe's zero-height rectangle (`half.z` 0 for every slot box), moved vertically from `start` to `end`: it touches the landscape exactly when
/// the landscape's height range over the rectangle (`fn_terrain::Heightfield::height_range`, exact) meets the swept z span, i.e. the surface crosses the swept
/// slab somewhere under the box. ASSUMED: a box entirely under the surface is no hit (the PhysX 3.4 heightfield sweep reports surface contacts; 1.8's
/// placement trace never puts a preview there because `BuildingPlaceTrace` ignores the landscape and the level retry walks up / down from the view ray).
/// `location.z` = the highest surface point under the box, clamped to the span (the first contact of a downward sweep; the structural code does not read it).
#[derive(Clone, Debug, Default)]
pub struct Ground { pub terrain: Option<fn_terrain::Local>, pub flat_z: Option<f32> }
impl WorldCollision for Ground {
    fn sweep_box(&self, start: Vec3, end: Vec3, half: Vec3) -> Vec<WorldHit> {
        let (lo, hi) = (start.truncate().min(end.truncate()) - half.truncate(), start.truncate().max(end.truncate()) + half.truncate());
        let Some(t) = &self.terrain else { return FlatGround(self.flat_z).sweep_box(start, end, half) };
        let (zlo, zhi) = (start.z.min(end.z) - half.z, start.z.max(end.z) + half.z);
        let mut hits = vec![];
        if let Some((hmin, hmax)) = t.height_range(lo, hi) {
            if hmax >= zlo && hmin <= zhi {
                let z = hmax.clamp(zlo, zhi);
                let s = if (start.z - end.z).abs() > 1e-6 { ((start.z - half.z - z) / (start.z - end.z)).clamp(0., 1.) } else { 0. };
                hits.push(WorldHit { kind: WorldHitKind::Landscape, location: start + (end - start) * s, actor: None });
            }
        }
        // a hole of the landscape under part of the box: the plane stands in there (CHOSEN)
        if hits.is_empty() && !t.covers(lo, hi) { hits = FlatGround(self.flat_z).sweep_box(start, end, half); }
        hits
    }
}

/// The sweep of one piece: start / end (box centre, world cm), half extent, and `threshZ`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlapBox { pub start: Vec3, pub end: Vec3, pub half_extent: Vec3, pub thresh_z: f32 }

/// The sweep the slot checks run around the piece pivot `p` (world cm, [`crate::grid::grid_location`]):
/// * floor (`FUN_1404de610`): half (248, 248, 0), from z+15 down to z-18, threshZ z+13;
/// * wall (`FUN_1404de8c0`): half (6, 248, 0) for sides 0/1 (plane normal to X) or (248, 6, 0) for sides 2/3, from z+375 to z-18, threshZ z+10;
/// * centre cell (`FUN_1404de3e0`): half (248, 248, 0), from z+375 to z-18, threshZ z+10.
/// Constants: 0x1440c5e38 = 248, 0x144078d7c = 6, 0x1440c5e30 = 18, 0x144040c98 = 15, 0x1440c5e40 = 375, 0x1440c5e2c = 13, 0x143fe3714 = 10.
pub fn overlap_box(kind: PieceKind, p: Vec3, wall_side: u8) -> OverlapBox {
    let at = |z: f32| Vec3::new(p.x, p.y, z);
    match kind {
        PieceKind::Floor => OverlapBox { start: at(p.z + 15.), end: at(p.z - 18.), half_extent: Vec3::new(248., 248., 0.), thresh_z: p.z + 13. },
        PieceKind::Wall => OverlapBox { start: at(p.z + 375.), end: at(p.z - 18.), half_extent: if wall_side < 2 { Vec3::new(6., 248., 0.) } else { Vec3::new(248., 6., 0.) }, thresh_z: p.z + 10. },
        PieceKind::Stairs | PieceKind::Roof => OverlapBox { start: at(p.z + 375.), end: at(p.z - 18.), half_extent: Vec3::new(248., 248., 0.), thresh_z: p.z + 10. },
    }
}

/// Result of [`world_overlap_check`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlapResult {
    /// The sweep's return value: false = blocked (the slot check returns 2, Obstructed).
    pub ok: bool,
    /// The out-param `touchedWorld`: landscape, or a static mesh actor allowed by the threshold.
    pub touched_world: bool,
    /// Indices (into the sweep's hits) of the blocking building actors appended to the list (to be destroyed by placement when they
    /// have `bDestroyOnPlayerBuildingPlacement`).
    pub building_hits: Vec<usize>,
}

/// `FUN_1404df7e0` over the hits of `coll.sweep_box(b.start, b.end, b.half_extent)`. `allow` = the last argument (the register functions pass 1,
/// the placement query 0). Per hit:
/// * blocking building actor: ignored if `bDoNotBlockBuildings` or `allow`; with `bUseCentroidForBlockBuildingsCheck` it counts only if its
///   centroid lies strictly inside the box spanned by both ends of the sweep and centroid.z >= threshZ; then it is listed, and without
///   `bDestroyOnPlayerBuildingPlacement` the sweep fails;
/// * static mesh actor: hit z (> threshZ and not `allow`) fails the sweep, otherwise `touchedWorld = 1`;
/// * landscape: `touchedWorld = 1`; anything else: ignored.
/// The loop never stops early, so a failing sweep can still report `touchedWorld`.
pub fn world_overlap_check(coll: &dyn WorldCollision, b: &OverlapBox, allow: bool) -> OverlapResult {
    let mut r = OverlapResult { ok: true, ..Default::default() };
    for (i, h) in coll.sweep_box(b.start, b.end, b.half_extent).iter().enumerate() {
        match h.kind {
            WorldHitKind::BuildingActor { do_not_block_buildings, destroy_on_player_building_placement, use_centroid, centroid } => {
                if do_not_block_buildings || allow { continue; }
                if use_centroid {
                    let lo = b.start.min(b.end) - b.half_extent;
                    let hi = b.start.max(b.end) + b.half_extent;
                    let inside = centroid.x > lo.x && centroid.x < hi.x && centroid.y > lo.y && centroid.y < hi.y && centroid.z > lo.z && centroid.z < hi.z;
                    if !inside || centroid.z < b.thresh_z { continue; }
                }
                if !r.building_hits.contains(&i) { r.building_hits.push(i); }
                if !destroy_on_player_building_placement { r.ok = false; }
            }
            WorldHitKind::StaticMeshActor => {
                if h.location.z > b.thresh_z && !allow { r.ok = false; continue; }
                r.touched_world = true;
            }
            WorldHitKind::Landscape => r.touched_world = true,
            WorldHitKind::Other => {}
        }
    }
    r
}

// =================================================================================================================================
// Cell maps and neighbour gathering (FUN_1404ee8c0)
// =================================================================================================================================

/// The system's cell maps: walls by edge key (sys+0x128 for sides 0/1, +0x178 for sides 2/3, keys `FUN_1404f0160`), floors by plane
/// (sys+0x1C8, `FUN_1404de610`: plane = cell.z, +1 when the floor is not on its cell's level) and centre-cell actors by cell (sys+0xD8).
#[derive(Clone, Debug, Default)]
pub struct CellIndex { walls: HashMap<(u8, IVec3), PieceId>, floors: HashMap<IVec3, PieceId>, centres: HashMap<IVec3, PieceId> }

impl CellIndex {
    pub fn insert(&mut self, slot: Slot, id: PieceId) {
        match slot { Slot::Wall { axis, idx } => { self.walls.insert((axis, idx), id); } Slot::Floor(c) => { self.floors.insert(c, id); } Slot::Center(c) => { self.centres.insert(c, id); } }
    }
    pub fn remove(&mut self, slot: Slot, id: PieceId) {
        let same = |v: Option<&PieceId>| v == Some(&id);
        match slot {
            Slot::Wall { axis, idx } => if same(self.walls.get(&(axis, idx))) { self.walls.remove(&(axis, idx)); },
            Slot::Floor(c) => if same(self.floors.get(&c)) { self.floors.remove(&c); },
            Slot::Center(c) => if same(self.centres.get(&c)) { self.centres.remove(&c); },
        }
    }
    /// The maps of every piece of `world` (the grid lattice anchored at `origin`).
    pub fn from_world(world: &BuildingWorld, origin: Vec3) -> CellIndex {
        let mut m = CellIndex::default();
        for p in world.iter() { m.insert(crate::placement::slot_of(p, origin), p.id); }
        m
    }
    /// `FUN_1404f0040`: the wall on `side` of `cell` (keys `FUN_1404f0160`: side 0 -> x, 1 -> x + 1 (sides 0/1 map), 2 -> y + 1, 3 -> y).
    pub fn wall(&self, cell: IVec3, side: u8) -> Option<PieceId> {
        let key = match side { 0 => (0, cell), 1 => (0, cell + IVec3::X), 2 => (1, cell + IVec3::Y), _ => (1, cell) };
        self.walls.get(&key).copied()
    }
    /// `FUN_1404e96c0`: `which` 0 = the floor plane above the cell (z + 1), 1 = the cell's own plane.
    pub fn floor(&self, cell: IVec3, which: u8) -> Option<PieceId> {
        self.floors.get(&(cell + if which == 0 { IVec3::Z } else { IVec3::ZERO })).copied()
    }
    /// `FUN_1404e8800`: the centre-cell actor (stairs / roof) of the cell.
    pub fn centre(&self, cell: IVec3) -> Option<PieceId> { self.centres.get(&cell).copied() }
}

/// The three candidate lists of `FUN_1404ee8c0` (`out` +0x00 walls, +0x10 floors, +0x20 centre cells), each entry = (actor, the cell it was
/// looked up with), in the exe's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Neighbours { pub walls: Vec<(PieceId, IVec3)>, pub floors: Vec<(PieceId, IVec3)>, pub centres: Vec<(PieceId, IVec3)> }
impl Neighbours {
    /// The lists flattened in order (`FUN_1404e6840`).
    pub fn all(&self) -> impl Iterator<Item = (PieceId, IVec3)> + '_ { self.walls.iter().chain(&self.floors).chain(&self.centres).copied() }
}

/// `FUN_1404ee8c0(sys, type, loc, &cell, out)`: the piece's own cell (`FUN_1404ea060`, unadjusted) and its neighbour candidates.
/// `p` = the piece's grid location relative to the grid origin. Dispatch by the type sets of the ctor @ 1404d3af0 (walls {0}, floors {1},
/// centre cells {6, 5, 7, 11}).
pub fn gather(idx: &CellIndex, kind: PieceKind, p: Vec3) -> (IVec3, Neighbours) {
    let c = cell_of(p);
    let mut n = Neighbours::default();
    let v = |x: i32, y: i32, z: i32| c + IVec3::new(x, y, z);
    macro_rules! w { ($cell:expr, $side:expr) => { if let Some(id) = idx.wall($cell, $side) { n.walls.push((id, $cell)); } }; }
    macro_rules! f { ($cell:expr, $which:expr) => { if let Some(id) = idx.floor($cell, $which) { n.floors.push((id, $cell)); } }; }
    macro_rules! ce { ($cell:expr) => { if let Some(id) = idx.centre($cell) { n.centres.push((id, $cell)); } }; }
    match kind {
        // FUN_1404edcf0
        PieceKind::Wall => {
            let s = wall_side(p, c);
            let along_y = s < 2; // the wall plane is normal to X: it runs along Y
            let (pa, pb) = if along_y { (3, 2) } else { (0, 1) }; // the perpendicular sides
            w!(c, pa); w!(c, pb); f!(c, 0); f!(c, 1); ce!(c);
            // the cell across the wall
            let b = c + match s { 0 => IVec3::NEG_X, 1 => IVec3::X, 2 => IVec3::Y, _ => IVec3::NEG_Y };
            w!(b, pa); w!(b, pb); f!(b, 0); f!(b, 1); ce!(b);
            // the same side one level up / down, with those cells' centre actors
            let up = v(0, 0, 1); w!(up, s); ce!(up);
            let dn = v(0, 0, -1); w!(dn, s); ce!(dn);
            // the in-line walls to either side
            let (l, r) = if along_y { (v(0, 1, 0), v(0, -1, 0)) } else { (v(-1, 0, 0), v(1, 0, 0)) };
            w!(l, s); w!(r, s);
            // the centre actors across, one level up / down
            ce!(b + IVec3::Z); ce!(b - IVec3::Z);
        }
        // FUN_1404ecf40
        PieceKind::Floor => {
            let on = on_level(p, c);
            let which = on as u8; // FUN_1404e1de0: 1 = the floor sits on its cell's level
            let zz = if on { -1 } else { 1 };
            for s in [2, 3, 0, 1] { w!(c, s); }
            ce!(c);
            let cz = v(0, 0, zz);
            for s in [2, 3, 0, 1] { w!(cz, s); }
            ce!(cz);
            for (dx, dy) in [(0, 1), (0, -1), (-1, 0), (1, 0)] {
                let h = v(dx, dy, 0); f!(h, which); ce!(h);
                ce!(v(dx, dy, zz));
            }
        }
        // FUN_1404ebeb0
        PieceKind::Stairs | PieceKind::Roof => {
            for s in [2, 3, 0, 1] { w!(c, s); }
            f!(c, 0); f!(c, 1);
            for dz in [1, -1] { let u = v(0, 0, dz); for s in [2, 3, 0, 1] { w!(u, s); } ce!(u); }
            for (dx, dy) in [(0, 1), (0, -1), (-1, 0), (1, 0)] {
                let h = v(dx, dy, 0); f!(h, 0); f!(h, 1); ce!(h);
                ce!(v(dx, dy, 1)); ce!(v(dx, dy, -1));
            }
        }
    }
    (c, n)
}

/// `IsBuildingActorStructurallySupported` evaluated directly on a world without the incremental group records: the piece's connected component
/// (edges = gathered neighbours that pass [`connected`]) holds a world-supported piece. The group records of [`Structural`] are exactly these
/// components (registration merges every connected group, death rebuilds them); used by the placement query's neighbour check (`FUN_1405047a0`).
pub fn component_supported(world: &BuildingWorld, idx: &CellIndex, origin: Vec3, start: PieceId, voxel_of: &dyn Fn(&PlacedPiece) -> Option<Voxel>, world_supported: &dyn Fn(&PlacedPiece) -> bool) -> bool {
    let mut seen: HashSet<PieceId> = HashSet::from([start]);
    let mut stack = vec![start];
    while let Some(cur) = stack.pop() {
        let Some(p) = world.get(cur) else { continue };
        if world_supported(p) { return true; }
        let Some(vp) = voxel_of(p) else { continue };
        let (_, nb) = gather(idx, p.kind, grid::grid_location(p.kind, p.location - origin, p.yaw_deg));
        for (n, _) in nb.all() {
            if seen.contains(&n) { continue; }
            let Some(q) = world.get(n) else { continue };
            let Some(vq) = voxel_of(q) else { continue };
            if connected(&vp, &vq) { seen.insert(n); stack.push(n); }
        }
    }
    false
}

// =================================================================================================================================
// The system
// =================================================================================================================================

/// What the structural system needs from outside the building world.
#[derive(Clone, Copy)]
pub struct StructuralEnv<'a> {
    /// The world's collision for the "WorldOverlapCheck" sweep.
    pub collision: &'a dyn WorldCollision,
    /// `EditModePatternData` cube of a building class (object path of the generated class); `None` -> [`base_cube`] of the piece kind
    /// until it becomes known (the piece is then registered again).
    pub cubes: &'a dyn Fn(&str) -> Option<ConnectivityCube>,
    /// The structural grid origin (system +0x3C4) in world space = the level origin (`crate::placement::PlacementEnv::grid_origin`).
    pub grid_origin: Vec3,
    pub anchored: &'a dyn Fn(&Voxel) -> bool,
}
fn no_cubes(_: &str) -> Option<ConnectivityCube> { None }
pub fn no_anchor(_: &Voxel) -> bool { false }
impl<'a> StructuralEnv<'a> {
    /// Base cubes, grid origin 0, the given collision.
    pub fn with_collision(collision: &'a dyn WorldCollision) -> Self { StructuralEnv { collision, cubes: &no_cubes, grid_origin: Vec3::ZERO, anchored: &no_anchor } }
}

/// One registered actor (its cell-map slot, oriented cube, `bSupportedDirectly`, group id `UndermineGroup`).
#[derive(Clone, Debug)]
struct Reg {
    kind: PieceKind, slot: Slot, voxel: Voxel, supported_directly: bool, group: i32, cube_known: bool,
    /// Grid location relative to the origin (vfunc +0x8C8 = slot 281) the registration was made with.
    pivot: Vec3,
}

/// A group record (0xA8 bytes at system +0x78): members (+0x10) and world-supported members (+0x50).
#[derive(Clone, Debug, Default)]
struct Group { members: BTreeSet<PieceId>, world: BTreeSet<PieceId> }

/// One structural timer (`FTimerData`: the actor's handle at +0xFB0 and `ExpireTime`, a double). `set_at` = the world time of the
/// `SetTimer` call (reporting only).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Timer { id: PieceId, expire: f64, set_at: f64 }

/// UE4.16 `FTimerManager` restricted to the structural timers (`TimerManager.cpp`, `Array.h` heap functions; exe `FUN_1424d69d0` SetTimer,
/// `FUN_1424e3070` Tick, `FUN_1424d24c0` HeapPush). STRUCTURAL FIX 2 (2026-10-05): replaces the port's "earliest due, ties by piece id".
#[derive(Clone, Debug, Default)]
pub struct TimerHeap {
    /// `ActiveTimerHeap`: binary min-heap on `ExpireTime` (`FTimerData::operator<`), children 2i+1 / 2i+2.
    heap: Vec<Timer>,
    /// `PendingTimerList`: `expire` holds the delay until the push after the fire loop.
    pending: Vec<Timer>,
    /// `CurrentlyExecutingTimer`.
    executing: Option<PieceId>,
    /// `LastTickedFrame`, as the world time of that frame.
    last_tick: Option<f64>,
    /// Inside `Tick`'s fire loop at this `InternalTime`.
    ticking: Option<f64>,
}

impl TimerHeap {
    /// `TArray::SiftDown`: the smaller child (`Pred(Left, Right) ? Left : Right`, so the RIGHT one on equal keys) moves up while it is
    /// strictly less.
    fn sift_down(&mut self, mut i: usize) {
        let n = self.heap.len();
        loop {
            let l = 2 * i + 1;
            if l >= n { break; }
            let r = l + 1;
            let m = if r < n && !(self.heap[l].expire < self.heap[r].expire) { r } else { l };
            if !(self.heap[m].expire < self.heap[i].expire) { break; }
            self.heap.swap(i, m);
            i = m;
        }
    }
    /// `TArray::SiftUp(RootIndex, NodeIndex)`: swap with the parent ((i - 1) / 2) while strictly less.
    fn sift_up(&mut self, root: usize, mut i: usize) {
        while i > root {
            let p = (i - 1) / 2;
            if !(self.heap[i].expire < self.heap[p].expire) { break; }
            self.heap.swap(i, p);
            i = p;
        }
    }
    /// `HeapPush`: add at the end, sift up.
    fn push(&mut self, t: Timer) { self.heap.push(t); let n = self.heap.len() - 1; self.sift_up(0, n); }
    /// `HeapPop`: take [0], `RemoveAtSwap(0)` (the last element moves to 0), sift down.
    fn pop(&mut self) -> Timer { let t = self.heap.swap_remove(0); self.sift_down(0); t }
    /// `HeapRemoveAt(Index)`: `RemoveAtSwap`, `SiftDown(Index)`, `SiftUp(0, Min(Index, Num - 1))`.
    fn remove_at(&mut self, i: usize) {
        self.heap.swap_remove(i);
        self.sift_down(i);
        let n = self.heap.len();
        if n > 0 { self.sift_up(0, i.min(n - 1)); }
    }

    /// `ClearTimer` of a handle (`FindTimer` order: the executing timer, the heap, the pending list).
    pub fn clear(&mut self, id: PieceId) {
        if self.executing == Some(id) { self.executing = None; return; }
        if let Some(i) = self.heap.iter().position(|t| t.id == id) { self.remove_at(i); return; }
        if let Some(i) = self.pending.iter().position(|t| t.id == id) { self.pending.swap_remove(i); }
    }

    /// `SetTimer(handle, delay)` at world time `now` (re-setting clears the handle first). Before the manager ticked this frame, or from
    /// inside the fire loop, it goes to the pending list. Returns the due time it gets when the manager ticks at `now`.
    pub fn set(&mut self, id: PieceId, delay: f32, now: f64) -> f64 {
        self.clear(id);
        if self.ticking.is_none() && self.last_tick == Some(now) {
            self.push(Timer { id, expire: now + delay as f64, set_at: now });
        } else {
            self.pending.push(Timer { id, expire: delay as f64, set_at: now });
        }
        now + delay as f64
    }

    /// The end of `Tick`: `LastTickedFrame` = this frame, the pending timers pushed in list order with ExpireTime += InternalTime.
    fn end_tick(&mut self, now: f64) {
        self.ticking = None;
        self.executing = None;
        self.last_tick = Some(now);
        for mut t in std::mem::take(&mut self.pending) { t.expire += now; self.push(t); }
    }

    /// `FTimerManager::Tick` at `InternalTime` = `now`, one timer per call: the heap top when `now > ExpireTime` (it becomes the executing
    /// timer; returns (piece, ExpireTime)), else the loop ends (pending pushed) and None. A frame that already ticked returns None at once.
    pub fn next_expired(&mut self, now: f64) -> Option<(PieceId, f64)> {
        if let Some(t) = self.ticking { if t != now { self.end_tick(t); } }
        if self.ticking.is_none() {
            if self.last_tick == Some(now) { return None; }
            self.ticking = Some(now);
        }
        self.executing = None;
        if self.heap.first().is_some_and(|t| now > t.expire) {
            let t = self.pop();
            self.executing = Some(t.id);
            return Some((t.id, t.expire));
        }
        self.end_tick(now);
        None
    }

    /// Every set timer: (piece, due) - heap entries with their ExpireTime, pending ones with `set time + delay`.
    pub fn iter(&self) -> impl Iterator<Item = (PieceId, f64)> + '_ {
        self.heap.iter().map(|t| (t.id, t.expire)).chain(self.pending.iter().map(|t| (t.id, t.set_at + t.expire)))
    }
    /// The heap order (index 0 = top) of the active timers, for tests.
    pub fn heap_ids(&self) -> Vec<PieceId> { self.heap.iter().map(|t| t.id).collect() }
}

/// The structural support system (state the exe keeps on `UBuildingStructuralSupportSystem` and on each `ABuildingSMActor`).
#[derive(Default)]
pub struct Structural {
    /// STRUCTURAL FIX 2: the timer manager's structural timers.
    pub timers: TimerHeap,
    last_check: HashMap<PieceId, SupportCheck>,
    /// `bForciblyStructurallySupported` pieces.
    pub forced: HashSet<PieceId>,
    pub rand: CrtRand,
    regs: HashMap<PieceId, Reg>,
    index: CellIndex,
    groups: Vec<Option<Group>>,
    event_cursor: u64,
}

/// One collapse decision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Collapse { pub id: PieceId, pub check_time: f64 }

impl Structural {
    pub fn new() -> Self { Self::default() }

    /// vtable +0x930 (slot 294): `(+0xE90 & 0x18) != 0` = `bSupportedDirectly || bForciblyStructurallySupported`.
    pub fn is_supported_by_world(&self, id: PieceId) -> bool { self.forced.contains(&id) || self.regs.get(&id).is_some_and(|r| r.supported_directly) }

    /// `IsBuildingActorStructurallySupported` @ 1404f3dc0 (after [`sync`](Self::sync)).
    pub fn is_supported(&self, id: PieceId) -> bool {
        if self.is_supported_by_world(id) { return true; }
        let Some(r) = self.regs.get(&id) else { return false };
        self.group(r.group).is_some_and(|g| !g.world.is_empty())
    }

    /// Group id of a piece (-1 = none).
    pub fn group_of(&self, id: PieceId) -> i32 { self.regs.get(&id).map_or(-1, |r| r.group) }
    /// Members of a group.
    pub fn group_members(&self, g: i32) -> Vec<PieceId> { self.group(g).map(|g| g.members.iter().copied().collect()).unwrap_or_default() }
    fn group(&self, g: i32) -> Option<&Group> { if g < 0 { None } else { self.groups.get(g as usize).and_then(|x| x.as_ref()) } }

    /// The oriented cube of a registered piece.
    pub fn voxel_of(&self, id: PieceId) -> Option<Voxel> { self.regs.get(&id).map(|r| r.voxel) }

    /// Brings the registrations up to date with the world's events, at world time `world.time()`:
    /// * `Placed` = `InitializeBuildingActor(1)` (`FUN_1404f4be0` on the new piece: timers on its cube-connected neighbours, see the module
    ///   docs) then the registration (`FUN_1404d9250` / `d9d80` / `d90f0` + `FUN_1404db8e0`);
    /// * `Edited` (an edit, `ReplaceBuildingActor`) = the old actor's death teardown (vfunc 243 -> vfunc 240: timers cleared, its neighbours
    ///   scheduled, unregister + dissolve) then the new actor's registration (vfunc 248). The `Edited` event `placement::place` emits to set
    ///   `bMirrored` on a new piece (class unchanged, `edit.class_path` None) is not an edit: the exe sets the flag (vfunc +0x918) before
    ///   `InitializeBuildingActor`, and the `Placed` registration already used it;
    /// * `Removed` without a reported death unregisters (+ dissolve, no timers).
    /// Pieces whose class cube was not known are registered again once it is.
    pub fn sync(&mut self, world: &BuildingWorld, env: &StructuralEnv) { self.sync_except(world, env, None) }

    /// [`sync`](Self::sync), leaving the `Removed` event of `dying` to [`on_piece_died`](Self::on_piece_died).
    fn sync_except(&mut self, world: &BuildingWorld, env: &StructuralEnv, dying: Option<PieceId>) {
        let (events, cur) = world.events_since(self.event_cursor);
        let events: Vec<WorldEvent> = events.to_vec();
        self.event_cursor = cur;
        let now = world.time();
        for e in events {
            match e {
                WorldEvent::Placed(id) => if world.get(id).is_some() && !self.regs.contains_key(&id) {
                    // STRUCTURAL FIX 2: InitializeBuildingActor @14047bed0, reason 1 -> FUN_1404f4be0(system, newActor), before vfunc 248
                    // registers it (ServerCreateBuildingActor_Implementation @140845d70: +0x7b8(1) then +0x7c0(1))
                    let p = world.get(id).unwrap();
                    let gl = grid::grid_location(p.kind, p.location - env.grid_origin, p.yaw_deg);
                    let (v, _) = Self::piece_voxel(p, env);
                    self.schedule_neighbours(p.kind, gl, &v, now);
                    self.register(world, id, env);
                },
                WorldEvent::Edited(id) => {
                    if world.get(id).is_some_and(|p| p.edit.class_path.is_none()) && self.regs.contains_key(&id) { continue; }
                    // STRUCTURAL FIX 2: ABuildingSMActor vfunc 329 @14048afb0 -> old actor +0x798 (vfunc 243 @140468b50) -> +0x780 (vfunc 240
                    // @14047aa10): ClearAllTimersForObject (FUN_1424d6510), FUN_1404f4be0 (with the OLD actor's cube and cell), FUN_1404fc7a0,
                    // FUN_1404fc9a0; then the new actor's +0x7c0 (vfunc 248 -> FUN_140461950) registers it.
                    // The port reuses the PieceId for the new actor, so the old actor's clear-all comes first here; in the exe it clears a
                    // different object, after the new actor's own timer was set.
                    self.timers.clear(id);
                    self.last_check.remove(&id);
                    self.forced.remove(&id);
                    // VERIFIER FIX: InitializeBuildingActor(2) @14047bed0 (both build paths fall through to it for reason 2) sets the NEW
                    // actor's structural timer: FUN_1424d6af0(TimerManager, +0xFB0 handle, delegate LAB_14046041c (= FUN_1404cecc0's),
                    // FUN_1405861d0 = UFortGameData +0x4B0 BuildingStructuralCollapseDelay, 0.25 s, no variance), BEFORE vfunc 243 runs.
                    if world.get(id).is_some() { self.timers.set(id, COLLAPSE_DELAY, now); }
                    if let Some(old) = self.regs.get(&id).cloned() { self.schedule_neighbours(old.kind, old.pivot, &old.voxel, now); }
                    self.unregister(id);
                    if world.get(id).is_some() { self.register(world, id, env); }
                }
                WorldEvent::Removed(id) if Some(id) == dying => {}
                WorldEvent::Removed(id) => { self.unregister(id); self.forget(id); }
                _ => {}
            }
        }
        // reconcile (an overrun event log, or a world swapped under us)
        let live = |id: &PieceId| world.get(*id).is_some() || Some(*id) == dying;
        if self.regs.len() != world.len() + dying.map_or(0, |d| self.regs.contains_key(&d) as usize) || self.regs.keys().any(|id| !live(id)) {
            let gone: Vec<PieceId> = self.regs.keys().copied().filter(|id| !live(id)).collect();
            for id in gone { self.unregister(id); self.forget(id); }
            let mut new: Vec<PieceId> = world.iter().map(|p| p.id).filter(|id| !self.regs.contains_key(id)).collect();
            new.sort_unstable();
            for id in new { self.register(world, id, env); }
        }
        // cubes that became known
        let stale: Vec<PieceId> = self.regs.iter().filter(|(id, r)| !r.cube_known && world.get(**id).is_some_and(|p| (env.cubes)(p.effective_class()).is_some())).map(|(id, _)| *id).collect();
        for id in stale { self.unregister(id); self.register(world, id, env); }
    }

    fn piece_voxel(p: &PlacedPiece, env: &StructuralEnv) -> (Voxel, bool) {
        let (cube, known) = match (env.cubes)(p.effective_class()) { Some(c) => (c, true), None => (base_cube(p.kind), false) };
        (voxel(p.kind, cube, p.location - env.grid_origin, p.yaw_deg, p.edit.mirrored), known)
    }

    /// Registration of a placed piece: the slot check's sweep (allow = 1) sets `bSupportedDirectly` (slot 293) on a world contact, the piece
    /// enters its cell map, then joins / creates / merges groups (`FUN_1404db8e0`).
    fn register(&mut self, world: &BuildingWorld, id: PieceId, env: &StructuralEnv) {
        let Some(p) = world.get(id) else { return };
        let rel = p.location - env.grid_origin;
        let slot = grid::key_for(p.kind, rel, p.yaw_deg).1;
        let gl = grid::grid_location(p.kind, rel, p.yaw_deg);
        let side = if p.kind == PieceKind::Wall { wall_side(gl, cell_of(gl)) } else { 0 };
        let b = overlap_box(p.kind, gl + env.grid_origin, side);
        let (v, known) = Self::piece_voxel(p, env);
        let touched = world_overlap_check(env.collision, &b, true).touched_world || (env.anchored)(&v);
        self.regs.insert(id, Reg { kind: p.kind, slot, voxel: v, supported_directly: touched, group: -1, cube_known: known, pivot: gl });
        self.index.insert(slot, id);
        self.join(id);
    }

    /// `FUN_1404e1840`: a record holding the actor (the first free record is reused).
    fn create_group(&mut self, id: PieceId) -> i32 {
        let g = match self.groups.iter().position(|x| x.is_none()) { Some(i) => i, None => { self.groups.push(None); self.groups.len() - 1 } };
        self.groups[g] = Some(Group::default());
        self.add_member(g as i32, id);
        g as i32
    }
    /// `FUN_1404d8dc0` (+ the group id setter, vtable +0xB68): member, and world member when vtable +0x930 is true.
    fn add_member(&mut self, g: i32, id: PieceId) {
        let w = self.is_supported_by_world(id);
        if let Some(Some(rec)) = self.groups.get_mut(g as usize) { rec.members.insert(id); if w { rec.world.insert(id); } }
        if let Some(r) = self.regs.get_mut(&id) { r.group = g; }
    }
    /// `FUN_1404e0b20`: the group with the most members (first on ties) takes the others' members.
    fn merge(&mut self, groups: &[i32]) -> i32 {
        let size = |s: &Self, g: i32| s.group(g).map_or(0, |r| r.members.len());
        let mut best = groups[0];
        for &g in groups { if size(self, g) > size(self, best) { best = g; } }
        for &g in groups {
            if g == best { continue; }
            let Some(rec) = self.groups.get_mut(g as usize).and_then(|x| x.take()) else { continue };
            for m in rec.members { self.add_member(best, m); }
        }
        best
    }

    /// `FUN_1404fc7a0` (leave the cell map) + `FUN_1404fc9a0` (dissolve the group). STRUCTURAL FIX 2 (2026-10-05), `FUN_1404fc9a0`:
    /// 1. `g` = the actor's group (vfunc +0xA38), its group := -1 (+0xB68), `FUN_1404fc370` takes it out of record `g`;
    /// 2. every remaining member's group := -1, then each re-registers (`FUN_1404db8e0`) in the record's order, collecting the group ids
    ///    they end up in (a set, first-seen order; -1 skipped). Record `g` stays VALID meanwhile, so no new group can take its slot;
    /// 3. record `g` is emptied (`FUN_1404f30b0`) but kept valid (+0xA0 = 1);
    /// 4. the collected group with the most members (`Num - NumFree`, the first on ties, at least one) is renamed to `g`: each of its members
    ///    joins record `g` (and its world set when vtable +0x930 is true) and gets group id `g`; that group's record is emptied and freed
    ///    (`FUN_1404f30b0`: +0xA0 = 0). With no such group, record `g` is freed (it is empty);
    /// 5. the system's group-changed delegate (+0x3F0, `FUN_1415471e0`) is broadcast for the renamed group and `g` (nothing in the port listens).
    fn unregister(&mut self, id: PieceId) {
        let Some(reg) = self.regs.remove(&id) else { return };
        self.index.remove(reg.slot, id);
        let g = reg.group;
        if g < 0 { return; }
        let members: Vec<PieceId> = match self.groups.get_mut(g as usize).and_then(|x| x.as_mut()) {
            Some(rec) => { rec.members.remove(&id); rec.world.remove(&id); rec.members.iter().copied().collect() }
            None => return,
        };
        for m in &members { if let Some(r) = self.regs.get_mut(m) { r.group = -1; } }
        let mut new_groups: Vec<i32> = vec![];
        for m in members {
            self.join(m);
            let ng = self.group_of(m);
            if ng != -1 && !new_groups.contains(&ng) { new_groups.push(ng); }
        }
        self.groups[g as usize] = Some(Group::default());
        let size = |s: &Self, x: i32| s.group(x).map_or(0, |r| r.members.len());
        let mut best: Option<(i32, usize)> = None;
        for &x in &new_groups { let n = size(self, x); if n > best.map_or(0, |b| b.1) { best = Some((x, n)); } }
        match best {
            Some((b, _)) => {
                let rec = self.groups[b as usize].take().unwrap_or_default();
                for m in rec.members { self.add_member(g, m); }
            }
            None => self.groups[g as usize] = None,
        }
    }

    /// `FUN_1404f4be0(system, actor)`: gathers the actor's neighbours (`FUN_1404ee8c0` from its cell, vfunc +0x8C8 location) and gives every
    /// one that is cube-connected to it (`FUN_1404cecc0` walls / floors, `FUN_1404ce6b0` centre cells, test `FUN_1404daa90`) a structural timer
    /// of [`collapse_delay`] (two `rand()` draws per connected candidate). `pivot` = the actor's grid location relative to the origin.
    fn schedule_neighbours(&mut self, kind: PieceKind, pivot: Vec3, voxel: &Voxel, now: f64) -> Vec<(PieceId, f64)> {
        let (from, nb) = gather(&self.index, kind, pivot);
        let mut out = vec![];
        for (n, lookup) in nb.all() {
            let Some(r) = self.regs.get(&n) else { continue };
            if !connected(voxel, &r.voxel) { continue; }
            let (r1, r2) = (self.rand.next(), self.rand.next());
            let due = self.timers.set(n, collapse_delay(cell_distance(from, lookup), r1, r2), now);
            out.push((n, due));
        }
        out
    }

    /// `FUN_1404db8e0`: join / create / merge the groups of the cube-connected gathered neighbours (also run for every former member of a
    /// dissolved group).
    fn join(&mut self, id: PieceId) {
        let Some(me) = self.regs.get(&id).cloned() else { return };
        let (_, nb) = gather(&self.index, me.kind, me.pivot);
        let mut groups: Vec<i32> = vec![];
        let mut ungrouped: Vec<PieceId> = vec![];
        for (n, _) in nb.all() {
            let Some(r) = self.regs.get(&n) else { continue };
            if !connected(&me.voxel, &r.voxel) { continue; }
            if r.group == -1 { if !ungrouped.contains(&n) { ungrouped.push(n); } } else if !groups.contains(&r.group) { groups.push(r.group); }
        }
        let g = match groups.len() {
            0 if ungrouped.is_empty() => -1,
            0 => self.create_group(id),
            1 => { self.add_member(groups[0], id); groups[0] }
            _ => { let g = self.merge(&groups); self.add_member(g, id); g }
        };
        if let Some(r) = self.regs.get_mut(&id) { r.group = g; }
        if g != -1 { for n in ungrouped { self.add_member(g, n); } }
    }

    /// The death of `dead` (already removed from `world`): `FUN_1404f4be0` schedules a structural check on every gathered neighbour that is
    /// cube-connected to it, then the dead actor leaves its cell map and its group is dissolved. `now` = world time.
    /// Returns the scheduled (piece, due time) pairs in the exe's order.
    pub fn on_piece_died(&mut self, world: &BuildingWorld, dead: &PlacedPiece, now: f64, env: &StructuralEnv) -> Vec<(PieceId, f64)> {
        // registers this frame's placements (the dead piece's own `Removed` event is handled here, in the exe's order)
        self.sync_except(world, env, Some(dead.id));
        let reg = self.regs.get(&dead.id).cloned();
        let dead_voxel = reg.as_ref().map(|r| r.voxel).unwrap_or_else(|| Self::piece_voxel(dead, env).0);
        let gl = reg.as_ref().map(|r| r.pivot).unwrap_or_else(|| grid::grid_location(dead.kind, dead.location - env.grid_origin, dead.yaw_deg));
        // STRUCTURAL FIX 2: vfunc 240 @14047aa10 order: ClearAllTimersForObject (FUN_1424d6510), FUN_1404f4be0, FUN_1404fc7a0 + FUN_1404fc9a0
        self.timers.clear(dead.id);
        let out = self.schedule_neighbours(dead.kind, gl, &dead_voxel, now);
        self.unregister(dead.id);
        self.forget(dead.id);
        out
    }

    /// Forget a piece's timer / check state (`ClearAllTimersForObject` + the actor's own fields).
    pub fn forget(&mut self, id: PieceId) { self.timers.clear(id); self.last_check.remove(&id); self.forced.remove(&id); }

    /// Pieces with a structural timer set: (piece, due time).
    pub fn pending(&self) -> impl Iterator<Item = (PieceId, f64)> + '_ { self.timers.iter() }
    pub fn last_check(&self, id: PieceId) -> SupportCheck { self.last_check.get(&id).copied().unwrap_or(SupportCheck::None) }

    /// One step of the timer manager's fire loop at world time `now` ([`TimerHeap::next_expired`]: the heap top when `now > ExpireTime`),
    /// running its check (vfunc 366). Returns `Some(Some(collapse))` when the piece became unstable (the caller kills it with
    /// `InstantDeath.StructuralSupport` and reports the death through [`on_piece_died`](Self::on_piece_died) before the next call; the timers
    /// that death sets are pending until this frame's loop ends), `Some(None)` when a check ran without a collapse, `None` when the loop
    /// ended (call once per frame until `None`; a second loop in the same frame does nothing, as `HasBeenTickedThisFrame`).
    pub fn run_next_due(&mut self, world: &BuildingWorld, now: f64, env: &StructuralEnv) -> Option<Option<Collapse>> {
        self.sync(world, env);
        let (id, due) = self.timers.next_expired(now)?;
        if world.get(id).is_none() { self.last_check.remove(&id); return Some(None); }
        let supported = self.is_supported(id);
        let last = self.last_check(id);
        if !supported {
            if last != SupportCheck::Unstable { self.last_check.insert(id, SupportCheck::Unstable); return Some(Some(Collapse { id, check_time: due })); }
        } else if last != SupportCheck::Stable {
            self.last_check.insert(id, SupportCheck::Stable);
        }
        Some(None)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::grid::{key_for, snap_transform, Rot};
    use crate::pieces::Material;
    use crate::world::PieceSpawn;

    /// Places a piece the way the build preview snaps it: `cell` = the target cell (x, y, level), `quarter` = view yaw / 90
    /// (walls: 0 = the +X edge, 1 = +Y, 2 = -X, 3 = -Y of the cell).
    pub(crate) fn place(w: &mut BuildingWorld, kind: PieceKind, cell: IVec3, quarter: i32, hp: f32) -> PieceId {
        let target = Vec3::new(cell.x as f32 * 512., cell.y as f32 * 512., cell.z as f32 * 384. + 100.);
        let (loc, rot) = snap_transform(kind, target, Rot::yaw(quarter as f32 * 90.), 0);
        let (key, _) = key_for(kind, loc, rot.yaw);
        w.spawn(PieceSpawn { kind, material: Material::Wood, class_path: "C".into(), key, location: loc, yaw_deg: rot.yaw, max_health: hp, start_health: hp, build_time: 0. })
    }
    fn vox(w: &BuildingWorld, id: PieceId) -> Voxel { let p = w.get(id).unwrap(); voxel(p.kind, base_cube(p.kind), p.location, p.yaw_deg, p.edit.mirrored) }
    const GROUND: FlatGround = FlatGround(Some(0.));
    fn env() -> StructuralEnv<'static> { StructuralEnv::with_collision(&GROUND) }

    #[test]
    fn delay_is_the_exe_formula() {
        assert!((collapse_delay(0, 0, 0) - 0.25).abs() < 1e-6);
        assert!((collapse_delay(3, 32767, 32767) - (0.25 + 0.3)).abs() < 1e-6, "variance 0: the rand draws do not matter");
        assert_eq!(structural_cell(Vec3::new(256., -256., 192.)), IVec3::new(1, 0, 1), "ties go up (ROUND(2u + 0.5) >> 1)");
        let mut r = CrtRand::default();
        assert_eq!((r.next(), r.next(), r.next()), (41, 18467, 6334), "MSVC rand() with seed 1");
    }

    /// The four known pairs of the spec: wall on floor, ramp on floor, wall-to-wall (L corner vs corner point), cone on wall.
    #[test]
    fn connectivity_of_known_pairs() {
        let mut w = BuildingWorld::new();
        let o = IVec3::ZERO;
        let floor = place(&mut w, PieceKind::Floor, o, 0, 1.);
        let wall_px = place(&mut w, PieceKind::Wall, o, 0, 1.);
        let wall_py = place(&mut w, PieceKind::Wall, o, 1, 1.);
        let wall_py_up = place(&mut w, PieceKind::Wall, o + IVec3::Z, 1, 1.);
        let ramp = place(&mut w, PieceKind::Stairs, o, 0, 1.);
        let cone = place(&mut w, PieceKind::Roof, o + IVec3::Z, 0, 1.);
        let far_floor = place(&mut w, PieceKind::Floor, IVec3::new(2, 0, 0), 0, 1.);
        let v = |id| vox(&w, id);
        assert!(connected(&v(wall_px), &v(floor)), "wall on floor");
        assert!(connected(&v(floor), &v(wall_px)), "symmetric");
        for q in 0..4 {
            // a ramp in any of the 4 orientations rests its low edge on the floor of its cell
            let mut w2 = BuildingWorld::new();
            let f = place(&mut w2, PieceKind::Floor, o, 0, 1.);
            let r = place(&mut w2, PieceKind::Stairs, o, q, 1.);
            assert!(connected(&vox(&w2, r), &vox(&w2, f)), "ramp on floor, view quarter {q}");
        }
        assert!(connected(&v(ramp), &v(floor)));
        assert!(connected(&v(wall_px), &v(wall_py)), "L corner: a shared vertical edge");
        assert!(!connected(&v(wall_px), &v(wall_py_up)), "only the corner point (x+, y+, level 1) is shared");
        assert!(connected(&v(cone), &v(wall_px)), "cone on the wall's top edge");
        assert!(!connected(&v(cone), &v(floor)), "the cone is one level above the floor");
        assert!(!connected(&v(far_floor), &v(floor)), "two cells apart");
        // the gather finds the pairs that connect
        let idx = CellIndex::from_world(&w, Vec3::ZERO);
        let p = w.get(wall_px).unwrap();
        let (_, nb) = gather(&idx, PieceKind::Wall, grid::grid_location(p.kind, p.location, p.yaw_deg));
        let ids: Vec<PieceId> = nb.all().map(|x| x.0).collect();
        for want in [floor, wall_py, ramp, cone] { assert!(ids.contains(&want), "{want} in {ids:?}"); }
        assert!(!ids.contains(&far_floor));
    }

    #[test]
    fn world_overlap_check_rules() {
        let g = FlatGround(Some(0.));
        // a wall at the ground touches it (z+375 .. z-18), one storey up does not; a floor 15 cm above the ground still does, 20 cm does not
        assert!(world_overlap_check(&g, &overlap_box(PieceKind::Wall, Vec3::ZERO, 0), true).touched_world);
        assert!(!world_overlap_check(&g, &overlap_box(PieceKind::Wall, Vec3::new(0., 0., 384.), 0), true).touched_world);
        assert!(world_overlap_check(&g, &overlap_box(PieceKind::Floor, Vec3::new(0., 0., 17.), 0), true).touched_world);
        assert!(!world_overlap_check(&g, &overlap_box(PieceKind::Floor, Vec3::new(0., 0., 20.), 0), true).touched_world);
        assert!(world_overlap_check(&g, &overlap_box(PieceKind::Roof, Vec3::new(0., 0., -300.), 0), true).touched_world, "a buried cone still touches");
        assert_eq!(overlap_box(PieceKind::Wall, Vec3::ZERO, 1).half_extent, Vec3::new(6., 248., 0.));
        assert_eq!(overlap_box(PieceKind::Wall, Vec3::ZERO, 3).half_extent, Vec3::new(248., 6., 0.));
        // static meshes: above threshZ they block the query (allow = 0) but count as world when registering (allow = 1); placed buildings never do
        struct Hits(Vec<WorldHit>);
        impl WorldCollision for Hits { fn sweep_box(&self, _: Vec3, _: Vec3, _: Vec3) -> Vec<WorldHit> { self.0.clone() } }
        let b = overlap_box(PieceKind::Floor, Vec3::ZERO, 0);
        let mesh = |z: f32| Hits(vec![WorldHit { kind: WorldHitKind::StaticMeshActor, location: Vec3::new(0., 0., z), actor: None }]);
        assert_eq!(world_overlap_check(&mesh(13.), &b, false), OverlapResult { ok: true, touched_world: true, building_hits: vec![] });
        assert_eq!(world_overlap_check(&mesh(14.), &b, false), OverlapResult { ok: false, touched_world: false, building_hits: vec![] });
        assert!(world_overlap_check(&mesh(14.), &b, true).touched_world);
        let house = Hits(vec![WorldHit { kind: WorldHitKind::BuildingActor { do_not_block_buildings: false, destroy_on_player_building_placement: false, use_centroid: false, centroid: Vec3::ZERO }, location: Vec3::ZERO, actor: Some(7) }]);
        let r = world_overlap_check(&house, &b, false);
        assert!(!r.ok && !r.touched_world && r.building_hits == vec![0]);
        assert_eq!(world_overlap_check(&house, &b, true), OverlapResult { ok: true, touched_world: false, building_hits: vec![] });
    }

    #[test]
    fn groups_register_merge_and_dissolve() {
        let mut w = BuildingWorld::new();
        let a = place(&mut w, PieceKind::Wall, IVec3::ZERO, 0, 1.); // grounded
        let b = place(&mut w, PieceKind::Wall, IVec3::new(3, 0, 0), 2, 1.); // grounded, on the -X edge of cell 3
        let mut s = Structural::new();
        s.sync(&w, &env());
        assert!(s.is_supported_by_world(a) && s.is_supported_by_world(b));
        assert_eq!((s.group_of(a), s.group_of(b)), (-1, -1), "a piece with no connected neighbour has no group");
        // a floor bridge at level 1 over cells 1..2 between the two walls: joins both walls into one group
        let fa = place(&mut w, PieceKind::Floor, IVec3::new(1, 0, 1), 0, 1.);
        s.sync(&w, &env());
        assert!(s.group_of(fa) >= 0);
        assert_eq!(s.group_of(fa), s.group_of(a));
        let fb = place(&mut w, PieceKind::Floor, IVec3::new(2, 0, 1), 0, 1.);
        s.sync(&w, &env());
        assert_eq!(s.group_of(fb), s.group_of(fa), "joins fa's group (and brings the ungrouped wall b)");
        assert_eq!(s.group_of(b), s.group_of(fa));
        assert_eq!(s.group_members(s.group_of(fa)).len(), 4);
        assert!(s.is_supported(fb) && !s.is_supported_by_world(fb));
        // fb dies: the group is dissolved and rebuilt (fa + a)
        let g_old = s.group_of(fb);
        let dead = w.remove(fb).unwrap();
        s.on_piece_died(&w, &dead, 0., &env());
        assert_eq!(s.group_of(fa), s.group_of(a));
        assert_eq!(s.group_of(fa), g_old, "FUN_1404fc9a0: the regrouped record takes the old id");
        assert_eq!(s.group_of(b), -1);
        assert!(s.is_supported(fa));
        // two separate groups merge when a piece connects both
        let fc = place(&mut w, PieceKind::Floor, IVec3::new(3, 0, 1), 0, 1.);
        let wc = place(&mut w, PieceKind::Wall, IVec3::new(3, 0, 0), 0, 1.);
        s.sync(&w, &env());
        assert_ne!(s.group_of(fc), s.group_of(fa));
        let mid = place(&mut w, PieceKind::Floor, IVec3::new(2, 0, 1), 0, 1.);
        s.sync(&w, &env());
        let g = s.group_of(mid);
        assert!(g >= 0 && [a, b, fa, fc, wc].iter().all(|&x| s.group_of(x) == g), "merge");
        assert_eq!(s.group_members(g).len(), 6);
    }

    /// The exe's timings on a pillar: the base wall dies, the wall above is looked up at the cell above (1 cell: 0.35 s), the floor the upper
    /// wall holds is looked up across the wall (1 cell: 0.35 s).
    #[test]
    fn chain_collapse_with_the_exact_test() {
        let mut w = BuildingWorld::new();
        let w0 = place(&mut w, PieceKind::Wall, IVec3::ZERO, 0, 1.);
        let w1 = place(&mut w, PieceKind::Wall, IVec3::Z, 0, 1.);
        let f = place(&mut w, PieceKind::Floor, IVec3::new(0, 0, 2), 0, 1.);
        let mut s = Structural::new();
        s.sync(&w, &env());
        assert!(s.is_supported(f) && s.is_supported_by_world(w0) && !s.is_supported_by_world(w1));
        let dead = w.remove(w0).unwrap();
        let sched = s.on_piece_died(&w, &dead, 10., &env());
        assert_eq!(sched.len(), 1);
        assert_eq!(sched[0].0, w1);
        assert!((sched[0].1 - 10.35).abs() < 1e-5, "{}", sched[0].1);
        assert!(!s.is_supported(w1) && !s.is_supported(f), "the group lost its only grounded member");
        // the frame's timer tick (the death happened before it: the timer was pending and is pushed with InternalTime 10)
        assert_eq!(s.run_next_due(&w, 10., &env()), None);
        assert_eq!(s.run_next_due(&w, 10.3, &env()), None);
        assert_eq!(s.run_next_due(&w, 10.35, &env()), Some(Some(Collapse { id: w1, check_time: sched[0].1 })));
        let dead = w.remove(w1).unwrap();
        let sched = s.on_piece_died(&w, &dead, 10.35, &env());
        assert_eq!(sched.iter().map(|x| x.0).collect::<Vec<_>>(), vec![f]);
        assert!((sched[0].1 - 10.70).abs() < 1e-5, "{}", sched[0].1);
        assert_eq!(s.run_next_due(&w, 10.71, &env()).flatten().map(|c| c.id), Some(f));
    }

    /// A tower (4 walls, a floor, 4 walls, a cone) stands until the last of its 4 base walls breaks, then everything above falls,
    /// the cone last.
    #[test]
    fn tower_collapses_only_when_the_last_base_wall_breaks() {
        let mut w = BuildingWorld::new();
        let base: Vec<PieceId> = (0..4).map(|q| place(&mut w, PieceKind::Wall, IVec3::ZERO, q, 1.)).collect();
        let floor = place(&mut w, PieceKind::Floor, IVec3::Z, 0, 1.);
        let upper: Vec<PieceId> = (0..4).map(|q| place(&mut w, PieceKind::Wall, IVec3::Z, q, 1.)).collect();
        let cone = place(&mut w, PieceKind::Roof, IVec3::new(0, 0, 2), 0, 1.);
        let mut d = crate::damage::BuildingDamage::new();
        let e = env();
        let hit = crate::damage::Hit::Weapon { env_damage: 10. };
        let mut deaths: Vec<(f64, PieceId)> = vec![];
        for (i, &b) in base.iter().enumerate() {
            d.apply_hit(&mut w, b, hit, &e);
            for _ in 0..300 { w.tick(0.01); d.tick(&mut w, &e); }
            for ev in d.take_destroyed() { if ev.id != b { deaths.push((ev.time, ev.id)); } }
            if i < 3 { assert!(deaths.is_empty(), "base wall {i}: the floor still stands on the others: {deaths:?}"); }
        }
        let mut all: Vec<PieceId> = deaths.iter().map(|x| x.1).collect();
        all.sort_unstable();
        let mut want = vec![floor, cone]; want.extend(&upper); want.sort_unstable();
        assert_eq!(all, want, "everything above the base fell");
        assert!(deaths.windows(2).all(|p| p[1].0 >= p[0].0));
        assert_eq!(deaths.last().unwrap().1, cone);
        assert!(w.is_empty());
    }

    /// STRUCTURAL FIX 2: one edit of `wall` to `class` at the app's frame order (edit intents, `BuildingWorld::tick`, the damage hook), then
    /// frames of `dt` for 2 s. Returns (the frame time of the edit, `floor`'s death time if it fell).
    fn edit_and_run(w: &mut BuildingWorld, e: &StructuralEnv, wall: PieceId, floor: PieceId, class: &str) -> (f64, Option<f64>) {
        let mut d = crate::damage::BuildingDamage::new();
        let dt = 1. / 60.;
        for _ in 0..60 { w.tick(dt); d.tick(w, e); }
        assert!(d.structural.is_supported(floor) && d.structural.is_supported_by_world(wall));
        assert!(d.take_destroyed().is_empty(), "the placement timers find everything supported");
        w.set_edit(wall, crate::world::EditState { class_path: Some(class.into()), rotation_iterations: 0, mirrored: false });
        w.tick(dt);
        let t_edit = w.time();
        d.tick(w, e);
        let due: Vec<(PieceId, f64)> = d.structural.pending().collect();
        assert!(due.iter().any(|&(id, t)| id == floor && (t - (t_edit + collapse_delay(1, 0, 0) as f64)).abs() < 1e-9),
            "the old wall's teardown (vfunc 240 -> FUN_1404f4be0) sets the floor's timer, looked up across the wall (1 cell): {due:?}");
        assert!(due.iter().any(|&(id, t)| id == wall && (t - (t_edit + COLLAPSE_DELAY as f64)).abs() < 1e-9),
            "InitializeBuildingActor(2) sets the new actor's own 0.25 s timer: {due:?}");
        for _ in 0..120 { w.tick(dt); d.tick(w, e); }
        let died = d.take_destroyed().into_iter().find(|x| x.id == floor).map(|x| x.time);
        assert!(w.get(wall).is_some());
        (t_edit, died)
    }

    /// An edit that removes the floor's only connection collapses it 0.35 s later (the first frame with InternalTime > ExpireTime); an
    /// edit that keeps the connection runs the same check and the floor stays. The edited class here has a synthetic cube with no set bit.
    #[test]
    fn edit_that_removes_the_only_connection_collapses_on_time() {
        static NOTHING: fn(&str) -> Option<ConnectivityCube> = |c| if c == "EDITED_NO_CONNECTION" { Some([0; 6]) } else { None };
        let e = StructuralEnv { collision: &GROUND, cubes: &NOTHING, grid_origin: Vec3::ZERO, anchored: &no_anchor };
        let mut w = BuildingWorld::new();
        let wall = place(&mut w, PieceKind::Wall, IVec3::ZERO, 0, 200.);
        let floor = place(&mut w, PieceKind::Floor, IVec3::Z, 0, 190.); // rests on the wall's top edge
        let (t_edit, died) = edit_and_run(&mut w, &e, wall, floor, "EDITED_NO_CONNECTION");
        let due = t_edit + collapse_delay(1, 0, 0) as f64;
        let t = died.expect("the floor falls");
        assert!(t > due && t - 1. / 60. <= due + 1e-9, "dies on the first frame after {due}: {t}");
        assert!(w.get(floor).is_none());
        // control: the same edit flow to a class with the full wall cube keeps the floor
        let mut w = BuildingWorld::new();
        let wall = place(&mut w, PieceKind::Wall, IVec3::ZERO, 0, 200.);
        let floor = place(&mut w, PieceKind::Floor, IVec3::Z, 0, 190.);
        let (_, died) = edit_and_run(&mut w, &e, wall, floor, "C");
        assert_eq!(died, None);
    }

    /// VERIFIER FIX: an edit that leaves the edited piece ITSELF unsupported collapses it at edit + 0.25 s (its own timer from
    /// `InitializeBuildingActor(2)`), ahead of the neighbours' timers in the timer order.
    #[test]
    fn edit_that_cuts_the_piece_itself_off_collapses_it_after_025s() {
        static NOTHING: fn(&str) -> Option<ConnectivityCube> = |c| if c == "EDITED_NO_CONNECTION" { Some([0; 6]) } else { None };
        let e = StructuralEnv { collision: &GROUND, cubes: &NOTHING, grid_origin: Vec3::ZERO, anchored: &no_anchor };
        let mut w = BuildingWorld::new();
        let base = place(&mut w, PieceKind::Wall, IVec3::ZERO, 0, 200.);
        let upper = place(&mut w, PieceKind::Wall, IVec3::Z, 0, 200.); // held only by the base wall's top edge
        let mut d = crate::damage::BuildingDamage::new();
        let dt = 1. / 60.;
        for _ in 0..60 { w.tick(dt); d.tick(&mut w, &e); }
        assert!(d.structural.is_supported(upper) && !d.structural.is_supported_by_world(upper));
        w.set_edit(upper, crate::world::EditState { class_path: Some("EDITED_NO_CONNECTION".into()), rotation_iterations: 0, mirrored: false });
        w.tick(dt);
        let t_edit = w.time();
        d.tick(&mut w, &e);
        let order: Vec<PieceId> = d.structural.pending().map(|x| x.0).collect();
        assert_eq!(order.first(), Some(&upper), "the new actor's timer is set before the old actor's teardown: {order:?}");
        assert!(order.contains(&base), "the teardown schedules the base wall too");
        for _ in 0..60 { w.tick(dt); d.tick(&mut w, &e); }
        let died: Vec<(PieceId, f64)> = d.take_destroyed().into_iter().map(|x| (x.id, x.time)).collect();
        assert_eq!(died.len(), 1, "{died:?}");
        let (id, t) = died[0];
        let due = t_edit + COLLAPSE_DELAY as f64;
        assert!(id == upper && t > due && t - dt as f64 <= due + 1e-9, "dies on the first frame after {due}: {died:?}");
        assert!(w.get(base).is_some());
    }

    /// The same with the cooked 1.8 data: a real `PBWA_W1_Solid` edit class whose cube no longer reaches the floor on top.
    #[test]
    fn real_wall_edit_that_disconnects_the_floor() {
        let Ok(root) = fn_live::var("FN_TEST_ROOT") else { return };
        let set = fn_pak::PakSet::open_install(std::path::Path::new(&root)).expect("paks");
        let db = crate::edit::EditDb::load(&set).expect("edit db");
        let base = "/Game/Building/ActorBlueprints/Player/Wood/L1/PBWA_W1_Solid.PBWA_W1_Solid_C";
        let mut cubes: HashMap<String, ConnectivityCube> = HashMap::new();
        for c in db.possible_edit_classes(db.class(base).expect("solid wall")) {
            if let Ok(cd) = crate::pieces::load_class(&set, &c.class_path) { cubes.insert(c.class_path.clone(), cd.cube); }
        }
        let mut w = BuildingWorld::new();
        let wall = place(&mut w, PieceKind::Wall, IVec3::ZERO, 0, 200.);
        let floor = place(&mut w, PieceKind::Floor, IVec3::Z, 0, 190.);
        let fv = vox(&w, floor);
        let wp = w.get(wall).unwrap().clone();
        let mut cut: Vec<&String> = cubes.iter().filter(|(_, c)| !connected(&voxel(PieceKind::Wall, **c, wp.location, wp.yaw_deg, false), &fv)).map(|(k, _)| k).collect();
        cut.sort();
        eprintln!("wall edit classes that do not reach a floor on top: {cut:?}");
        let class = cut.first().expect("some edit (e.g. a half wall) drops the top edge").to_string();
        let lookup = |c: &str| cubes.get(c).copied();
        let e = StructuralEnv { collision: &GROUND, cubes: &lookup, grid_origin: Vec3::ZERO, anchored: &no_anchor };
        w.get_mut(wall).unwrap().class_path = base.into();
        w.get_mut(floor).unwrap().class_path = "/Game/Building/ActorBlueprints/Player/Wood/L1/PBWA_W1_Floor.PBWA_W1_Floor_C".into();
        let (t_edit, died) = edit_and_run(&mut w, &e, wall, floor, &class);
        let t = died.expect("the floor falls");
        assert!(t > t_edit + collapse_delay(1, 0, 0) as f64 && t < t_edit + 0.37, "{class}: edit {t_edit}, death {t}");
    }

    /// `FUN_1404fc9a0`: of the groups a dissolve leaves, the largest takes the old group id (its own record is freed), the others keep the
    /// ids they got.
    #[test]
    fn dissolve_renames_the_largest_group_to_the_old_id() {
        // a floor bridge at level 1 over cells 1..5, held by a grounded wall at each end (a: +X edge of cell 0, b: -X edge of cell 6)
        let mut w = BuildingWorld::new();
        let a = place(&mut w, PieceKind::Wall, IVec3::ZERO, 0, 1.);
        let f: Vec<PieceId> = (1..=5).map(|x| place(&mut w, PieceKind::Floor, IVec3::new(x, 0, 1), 0, 1.)).collect();
        let b = place(&mut w, PieceKind::Wall, IVec3::new(6, 0, 0), 2, 1.);
        let mut s = Structural::new();
        s.sync(&w, &env());
        let g = s.group_of(f[0]);
        assert!(g >= 0 && [a, b].iter().chain(&f).all(|&x| s.group_of(x) == g));
        // f[1] (cell 2) dies: left {a, f1} (2 members), right {f3, f4, f5, b} (4 members)
        let dead = w.remove(f[1]).unwrap();
        s.on_piece_died(&w, &dead, 0., &env());
        let right = s.group_of(f[2]);
        let left = s.group_of(f[0]);
        assert_eq!(right, g, "the largest regrouped record is renamed to the old id");
        assert!([f[3], f[4], b].iter().all(|&x| s.group_of(x) == g));
        assert_eq!(s.group_members(g).len(), 4);
        assert!(left >= 0 && left != g && s.group_of(a) == left && s.group_members(left).len() == 2);
        assert!(s.is_supported(f[0]) && s.is_supported(f[4]));
        // the renamed group's own record was freed: the next new group reuses the first free slot
        assert!(s.groups.iter().filter(|x| x.is_some()).count() == 2, "{:?}", s.groups.iter().map(|x| x.as_ref().map(|r| r.members.len())).collect::<Vec<_>>());
    }

    /// UE4.16 `FTimerManager`: equal ExpireTimes fire in the binary heap's order (not by piece id), a timer fires only when
    /// InternalTime > ExpireTime, and timers set before the frame's tick wait in the pending list.
    #[test]
    fn timer_heap_matches_ue_416() {
        let mut t = TimerHeap::default();
        assert_eq!(t.next_expired(1.), None, "frame 1 ticked");
        for id in [10, 20, 30, 40] { t.set(id, 0.25, 1.); } // after the tick: straight into the heap, equal keys never sift
        assert_eq!(t.heap_ids(), vec![10, 20, 30, 40]);
        assert_eq!(t.next_expired(1.25), None, "1.25 is not > 1.25");
        // pop 10: 40 moves to the top, SiftDown picks the RIGHT child on equal keys (30) but 30 < 40 is false: 40 stays
        let order: Vec<PieceId> = std::iter::from_fn(|| t.next_expired(1.3).map(|x| x.0)).collect();
        assert_eq!(order, vec![10, 40, 30, 20]);
        // SetTimer before the frame ticked: pending (ExpireTime = delay), pushed after the fire loop with that frame's InternalTime
        t.set(1, 0.1, 2.);
        t.set(2, 0.1, 2.);
        assert_eq!(t.heap_ids(), Vec::<PieceId>::new());
        assert_eq!(t.next_expired(2.5), None, "pending timers are not in this frame's loop");
        assert_eq!(t.heap_ids(), vec![1, 2]);
        assert!(t.iter().all(|(_, due)| due == 2.5 + 0.1f32 as f64), "ExpireTime = InternalTime of the flushing tick + delay");
        // re-setting a handle removes it first (HeapRemoveAt)
        t.set(1, 0.5, 2.5);
        assert_eq!(t.heap_ids(), vec![2, 1]);
    }

    /// `InitializeBuildingActor(1)` (FUN_1404f4be0 on the new piece): placing a floor on a wall gives the wall a structural timer; the new
    /// piece itself gets none (it is not registered yet).
    #[test]
    fn placement_schedules_the_new_pieces_neighbours() {
        let mut w = BuildingWorld::new();
        let wall = place(&mut w, PieceKind::Wall, IVec3::ZERO, 0, 1.);
        let mut s = Structural::new();
        s.sync(&w, &env());
        assert_eq!(s.pending().count(), 0, "nothing next to the first piece");
        let floor = place(&mut w, PieceKind::Floor, IVec3::Z, 0, 1.);
        s.sync(&w, &env());
        let p: Vec<(PieceId, f64)> = s.pending().collect();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].0, wall);
        assert!(!p.iter().any(|x| x.0 == floor));
        // the check runs and finds the wall stable
        assert_eq!(s.run_next_due(&w, 0., &env()), None);
        assert_eq!(s.run_next_due(&w, 1., &env()), Some(None));
        assert_eq!(s.last_check(wall), SupportCheck::Stable);
    }

    /// The four base cubes equal the cooked `ConnectivityCubeData` the catalog reads (FN_TEST_ROOT = the 1.8 `FortniteGame` dir).
    #[test]
    fn base_cubes_match_the_paks() {
        let Ok(root) = fn_live::var("FN_TEST_ROOT") else { return };
        let set = fn_pak::PakSet::open_install(std::path::Path::new(&root)).expect("paks");
        let cat = crate::pieces::PieceCatalog::load(&set).expect("catalog");
        for k in PieceKind::ALL { assert_eq!(cat.def(k, Material::Wood).unwrap().class.cube, base_cube(k), "{k:?}"); }
    }
}

#[cfg(test)]
mod terrain_tests {
    use super::*;
    use glam::Vec2;
    /// TERRAIN: a 20 deg slope rising along world +X (world cm = UE cm here), 0 at x = 0. A floor slot's "WorldOverlapCheck" (+15 / -18 over 496 x 496 cm)
    /// is supported at every level the slope crosses under the cell, not above it; a wall's (+375 / -18) reaches the slope from 3 m above it; a box over a hole
    /// of the landscape falls back to the plane.
    #[test] fn the_world_overlap_check_touches_the_landscape_where_it_crosses_the_box() {
        let k = 20f32.to_radians().tan();
        let hf = fn_terrain::Heightfield::synthetic(Vec2::splat(-2000.), 100., 2, 21, |x, _| k * x);
        let w = std::sync::Arc::new(fn_terrain::World { hf, map: fn_terrain::Mapping { centre_cm: [0.; 3], anchor: Vec3::ZERO, ref_z_cm: 0. } });
        let g = Ground { terrain: Some(fn_terrain::Local::new(w, 0.)), flat_z: Some(-5000.) };
        // a floor cell centred on x = 512: the slope spans 512 +- 248 -> z 96 .. 277 under it
        let floor = |z: f32| world_overlap_check(&g, &overlap_box(PieceKind::Floor, Vec3::new(512., 0., z), 0), false).touched_world;
        assert!(floor(100.) && floor(192.) && floor(270.) && !floor(0.) && !floor(300.) && !floor(384.), "a floor is supported only where the slope crosses its slab");
        let wall = |z: f32| world_overlap_check(&g, &overlap_box(PieceKind::Wall, Vec3::new(256., 0., z), 0), false).touched_world;
        assert!(wall(0.) && wall(-200.) && !wall(-400.) && !wall(150.), "a wall reaches 375 cm up / 18 cm down");
        // past the landscape (x > 2000 cm): the plane at -5000 cm
        assert!(world_overlap_check(&g, &overlap_box(PieceKind::Floor, Vec3::new(4000., 0., -5000.), 0), false).touched_world);
        assert!(!world_overlap_check(&g, &overlap_box(PieceKind::Floor, Vec3::new(4000., 0., 0.), 0), false).touched_world);
    }
}
