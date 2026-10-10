//! The 1.8 build grid: tile size / height, the per-piece placement offsets, and the snap of a target point to a piece transform.
//!
//! One fixed world grid: [`snap_transform`] rounds ABSOLUTE world coordinates (`FMath::GridSnap` on the location itself, no player or
//! origin term: `ABuildingActor` vfunc_251 @14046f020 -> `FUN_140477b70`). The coordinates are the level's UE world space (cm, Z up).
//! Cells are centred on multiples of 512 (X/Y); floor levels are multiples of 384 (Z). Walls stand on the cell edges (pivot = edge
//! midpoint at the floor level), floors / ramps / cones fill a cell (pivot = the midpoint of one cell edge, the piece extends 512
//! along its local +Y from it: `BaseLocToPivotOffset` (0,-256,0) / `CentroidOffset` (0,256,..)).
//!
//! Constants (1.8 exe, native constructors; float bits in parentheses):
//! * `SnapGridSize` 512 (0x44000000) and `VertSnapGridSize` 384 (0x43C00000): `ABuildingActor::ABuildingActor` @140458440 (+0x6D8 / +0x6DC).
//! * per piece (`ABuildingWall` @14049fe20, `ABuildingFloor` @140459d10, `ABuildingStairs` @14049f370 (calls the floor ctor first),
//!   `ABuildingRoof` @14045b660): `BuildingPlacementDistance` (+0xBF0), `CentroidOffset` (+0x6EC), `BaseLocToPivotOffset` (+0x6F8),
//!   the wall flag (+0x554 bit 0x10, rotate-in-place), all listed in [`geom`]. The float constants they load are
//!   DAT_14401ee34 = 128, DAT_14401ee38 = 192, DAT_143ff9048 = 384, DAT_143ff903c = 256, DAT_143ff904c = 512, DAT_14404e220 = 90.
use glam::{IVec3, Vec3};

use crate::pieces::PieceKind;
use crate::world::GridKey;

/// `ABuildingActor::SnapGridSize` (cm).
pub const SNAP_GRID_SIZE: f32 = 512.;
/// `ABuildingActor::VertSnapGridSize` (cm).
pub const VERT_SNAP_GRID_SIZE: f32 = 384.;
/// Rotation grid of the snap (pitch, yaw, roll), DAT_1451803e0..e8 = (0, 90, 90); pitch is then forced to 0.
pub const ROTATION_GRID: [f32; 3] = [0., 90., 90.];
/// DAT_14404e220: the snapped yaw gets +90 before the pivot offset is applied; also the step of one rotation iteration.
pub const YAW_STEP: f32 = 90.;
/// `FUN_140833cb0` (BuildingPlaceTrace): the target is `ImpactPoint + ImpactNormal * 10` (DAT_143fe3714).
pub const TRACE_NORMAL_PUSH: f32 = 10.;
/// DAT_143ff9010 = 1e-4 (`KINDA_SMALL_NUMBER`) used by the preview's "moved?" tests.
pub const KINDA_SMALL: f32 = 1e-4;

/// UE `FRotator` (degrees).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rot { pub pitch: f32, pub yaw: f32, pub roll: f32 }
impl Rot {
    pub fn yaw(yaw: f32) -> Rot { Rot { pitch: 0., yaw, roll: 0. } }
    /// `FRotator::GetNormalized` (each axis to (-180, 180]).
    pub fn normalized(self) -> Rot { Rot { pitch: normalize_axis(self.pitch), yaw: normalize_axis(self.yaw), roll: normalize_axis(self.roll) } }
    /// `FRotator::Vector` (unit direction).
    pub fn vector(self) -> Vec3 {
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        let (sy, cy) = self.yaw.to_radians().sin_cos();
        Vec3::new(cp * cy, cp * sy, sp)
    }
}

/// `FRotator::NormalizeAxis`: ClampAxis (fmod 360, negative + 360) then > 180 -> -360.
pub fn normalize_axis(a: f32) -> f32 {
    let mut a = a % 360.;
    if a < 0. { a += 360.; }
    if a > 180. { a -= 360.; }
    a
}

/// `FMath::GridSnap(Location, Grid)` = `FloorToFloat((Location + 0.5 * Grid) / Grid) * Grid` (the exe inlines it as
/// `(int)ROUND(2 * ((Grid * 0.5 + x) / Grid) - 0.5) >> 1`, i.e. FloorToInt).
pub fn grid_snap(x: f32, grid: f32) -> f32 {
    if grid == 0. { return x; }
    ((x + 0.5 * grid) / grid).floor() * grid
}

/// Rotates `v` by a yaw-only rotator (UE `FRotator(0, yaw, 0).RotateVector`: x' = x cos - y sin, y' = x sin + y cos).
pub fn rotate_yaw(v: Vec3, yaw_deg: f32) -> Vec3 {
    let (s, c) = yaw_deg.to_radians().sin_cos();
    Vec3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z)
}

/// Native per-class placement data (see the module docs for the addresses).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieceGeom {
    /// `EFortBuildingType` (+0x3F0).
    pub building_type: u8,
    /// +0x554 bit 0x10: walls rotate in place (vfunc_251 adds the rotation iterations to the yaw without moving the pivot).
    pub rotates_in_place: bool,
    /// `CentroidOffset` (+0x6EC).
    pub centroid_offset: Vec3,
    /// `BaseLocToPivotOffset` (+0x6F8).
    pub base_loc_to_pivot: Vec3,
    /// `BuildingPlacementDistance` (+0xBF0): half extents of the box the build target is clamped into.
    pub placement_distance: Vec3,
}

pub fn geom(kind: PieceKind) -> PieceGeom {
    match kind {
        // ABuildingWall @14049fe20: BPD (128,128,384), CentroidOffset (0,0,192), BaseLocToPivotOffset.Y = -256 (0xC3800000), +0x554 |= 0x10.
        // (It also writes SnapOffset.Y = 10 (+0x6E4 = 0x41200000); SnapOffset is not read by the placement path.)
        PieceKind::Wall => PieceGeom { building_type: 0, rotates_in_place: true, centroid_offset: Vec3::new(0., 0., 192.), base_loc_to_pivot: Vec3::new(0., -256., 0.), placement_distance: Vec3::new(128., 128., 384.) },
        // ABuildingFloor @140459d10: BPD (192,192,384), CentroidOffset (0,256,0), BaseLocToPivotOffset.Y = -256.
        PieceKind::Floor => PieceGeom { building_type: 1, rotates_in_place: false, centroid_offset: Vec3::new(0., 256., 0.), base_loc_to_pivot: Vec3::new(0., -256., 0.), placement_distance: Vec3::new(192., 192., 384.) },
        // ABuildingStairs @14049f370 (after the floor ctor): BPD (512,512,384), CentroidOffset (0,256,192); BaseLocToPivotOffset stays the floor's.
        PieceKind::Stairs => PieceGeom { building_type: 5, rotates_in_place: false, centroid_offset: Vec3::new(0., 256., 192.), base_loc_to_pivot: Vec3::new(0., -256., 0.), placement_distance: Vec3::new(512., 512., 384.) },
        // ABuildingRoof @14045b660: BPD (512,512,384), CentroidOffset (0,256,0), BaseLocToPivotOffset.Y = -256.
        PieceKind::Roof => PieceGeom { building_type: 6, rotates_in_place: false, centroid_offset: Vec3::new(0., 256., 0.), base_loc_to_pivot: Vec3::new(0., -256., 0.), placement_distance: Vec3::new(512., 512., 384.) },
    }
}

/// `ABuildingActor` vfunc_251 @14046f020 (the preview's grid snap), `iterations` = `AFortPlayerController::BuildPreviewRotationIterations`.
///
/// 1. rotation snapped to (0, 90, 90) then Pitch = 0; 2. `Loc.Z -= CentroidOffset.Z`; 3. `FUN_140477b70`: X/Y/Z GridSnap 512, then Z
/// GridSnap 384 (from the unsnapped Z); 4. Yaw += 90; 5. Loc += Rot.RotateVector(BaseLocToPivotOffset); 6. walls: Yaw += 90 * iterations;
/// others (`FUN_1404642c0`): rotate about the snapped grid point by 90 * iterations; 7. rotator normalized.
pub fn snap_transform(kind: PieceKind, target: Vec3, view: Rot, iterations: u8) -> (Vec3, Rot) {
    let g = geom(kind);
    let mut rot = Rot { pitch: grid_snap(view.pitch, ROTATION_GRID[0]), yaw: grid_snap(view.yaw, ROTATION_GRID[1]), roll: grid_snap(view.roll, ROTATION_GRID[2]) };
    rot.pitch = 0.;
    let z = target.z - g.centroid_offset.z;
    let mut loc = Vec3::new(grid_snap(target.x, SNAP_GRID_SIZE), grid_snap(target.y, SNAP_GRID_SIZE), grid_snap(z, VERT_SNAP_GRID_SIZE));
    rot.yaw += YAW_STEP;
    loc += rotate_yaw(g.base_loc_to_pivot, rot.yaw);
    let turn = iterations as f32 * YAW_STEP;
    if g.rotates_in_place {
        rot.yaw += turn;
    } else {
        let new_yaw = normalize_axis(rot.yaw + turn);
        loc = loc - rotate_yaw(g.base_loc_to_pivot, rot.yaw) + rotate_yaw(g.base_loc_to_pivot, new_yaw);
        rot.yaw = new_yaw;
    }
    (loc, rot.normalized())
}

/// `ABuildingActor::GetCentroid` @14046ced0: actor location + actor rotation * `CentroidOffset`.
pub fn centroid(kind: PieceKind, loc: Vec3, yaw: f32) -> Vec3 { loc + rotate_yaw(geom(kind).centroid_offset, yaw) }

/// `ABuildingSMActor` vfunc_281 @140477f30 (the structural-grid location): walls -> the actor location; other pieces ->
/// location - rotation * `BaseLocToPivotOffset` (the cell centre at the floor level).
pub fn grid_location(kind: PieceKind, loc: Vec3, yaw: f32) -> Vec3 {
    let g = geom(kind);
    if g.rotates_in_place { loc } else { loc - rotate_yaw(g.base_loc_to_pivot, yaw) }
}

/// Structural grid cell of a point (`FUN_1404ea060`): `RoundToInt((p - origin) / (512, 512, 384))`, written by the exe as
/// `(int)ROUND(2 f + 0.5) >> 1` = FloorToInt(f + 0.5) (DAT_144040c68 = 1/512, DAT_144040c6c = 1/384). The structural grid origin
/// (+0x3C4) is taken as the world origin, the same lattice the placement snap uses.
pub fn cell_of(p: Vec3) -> IVec3 {
    let f = |v: f32, inv: f32| (v * inv + 0.5).floor() as i32;
    IVec3::new(f(p.x, 1. / SNAP_GRID_SIZE), f(p.y, 1. / SNAP_GRID_SIZE), f(p.z, 1. / VERT_SNAP_GRID_SIZE))
}
/// Centre of a cell at its floor level.
pub fn cell_origin(c: IVec3) -> Vec3 { Vec3::new(c.x as f32 * SNAP_GRID_SIZE, c.y as f32 * SNAP_GRID_SIZE, c.z as f32 * VERT_SNAP_GRID_SIZE) }

/// Which side of its cell a wall pivot lies on (`FUN_1404e1e80`): 0 = -X, 1 = +X, 2 = +Y, 3 = -Y.
pub fn wall_side(pivot: Vec3, cell: IVec3) -> u8 {
    let o = cell_origin(cell);
    let (dx, dy) = (pivot.x - o.x, pivot.y - o.y);
    if dy.abs() < dx.abs() { if dx > 0. { 1 } else { 0 } } else if dy <= 0. { 3 } else { 2 }
}

/// Occupancy slot of the structural grid (`UBuildingStructuralSupportSystem` @1404d3af0 registers type 0 in the wall map,
/// type 1 in the floor map and types 6 / 5 / 7 / 11 (roof, stairs, pillar, centre-cell actor) in ONE centre-cell map: a ramp and a
/// cone cannot share a cell; a floor and a ramp / cone can).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// Edge wall: `axis` 0 = the edge at x = (ix - 0.5) * 512 (wall spans Y), 1 = the edge at y = (iy - 0.5) * 512 (spans X);
    /// `idx` the edge lattice coordinate (FUN_1404f0160: side 0 -> x, side 1 -> x + 1, side 2 -> y + 1, side 3 -> y).
    Wall { axis: u8, idx: IVec3 },
    /// Floor plane at the cell's floor level (`FUN_1404de610`; a floor not exactly on a level would take the next plane up).
    Floor(IVec3),
    Center(IVec3),
}

/// Grid key and slot of a piece transform.
pub fn key_for(kind: PieceKind, loc: Vec3, yaw: f32) -> (GridKey, Slot) {
    let gl = grid_location(kind, loc, yaw);
    let cell = cell_of(gl);
    let quarter = ((normalize_axis(yaw).rem_euclid(360.) / 90.).round() as i32).rem_euclid(4) as u8;
    match kind {
        PieceKind::Wall => {
            let side = wall_side(gl, cell);
            let (axis, idx) = match side {
                0 => (0, cell),
                1 => (0, cell + IVec3::X),
                2 => (1, cell + IVec3::Y),
                _ => (1, cell),
            };
            (GridKey { kind, cell: idx, rot: axis }, Slot::Wall { axis, idx })
        }
        PieceKind::Floor => {
            let on_level = on_level(gl, cell);
            let c = if on_level { cell } else { cell + IVec3::Z };
            (GridKey { kind, cell: c, rot: quarter }, Slot::Floor(c))
        }
        PieceKind::Stairs | PieceKind::Roof => (GridKey { kind, cell, rot: quarter }, Slot::Center(cell)),
    }
}

/// `FUN_1404e1de0`: true when the point is on the cell's floor level (`RoundToInt((z - levelZ) / 384) * 384 < 1e-4`).
pub fn on_level(p: Vec3, cell: IVec3) -> bool {
    let lz = cell.z as f32 * VERT_SNAP_GRID_SIZE;
    (((p.z - lz) / VERT_SNAP_GRID_SIZE + 0.5).floor()) * VERT_SNAP_GRID_SIZE < KINDA_SMALL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_snap_is_floor_half_up() {
        assert_eq!(grid_snap(255.9, 512.), 0.);
        assert_eq!(grid_snap(256., 512.), 512.);
        assert_eq!(grid_snap(-256., 512.), 0.);
        assert_eq!(grid_snap(-256.1, 512.), -512.);
        assert_eq!(grid_snap(191., 384.), 0.);
        assert_eq!(grid_snap(192., 384.), 384.);
    }

    /// Looking along +X (yaw 0) at a point inside the cell centred on (1024, 0): the wall lands on that cell's far (+X) edge,
    /// the floor fills the cell, the ramp rises away from the viewer, all sharing the one world lattice.
    #[test]
    fn wall_floor_ramp_on_the_same_cell() {
        let t = Vec3::new(1100., 30., 100.);
        let (wl, wr) = snap_transform(PieceKind::Wall, t, Rot::yaw(0.), 0);
        assert!((wl - Vec3::new(1024. + 256., 0., 0.)).length() < 1e-3, "{wl}"); // Z: GridSnap(100 - 192, 384) = 0
        assert_eq!(wr.yaw, 90.);
        let (fl, fr) = snap_transform(PieceKind::Floor, t, Rot::yaw(0.), 0);
        assert!((fl - Vec3::new(1024. + 256., 0., 0.)).length() < 1e-3, "{fl}");
        assert!((centroid(PieceKind::Floor, fl, fr.yaw) - Vec3::new(1024., 0., 0.)).length() < 1e-3);
        let (sl, sr) = snap_transform(PieceKind::Stairs, t, Rot::yaw(0.), 0);
        assert!((centroid(PieceKind::Stairs, sl, sr.yaw) - Vec3::new(1024., 0., 192.)).length() < 1e-3);
        // a target 200 cm up rounds the floor to the next level (GridSnap(200, 384) = 384), the wall stays (GridSnap(8, 384) = 0)
        let (fl2, _) = snap_transform(PieceKind::Floor, Vec3::new(1100., 30., 200.), Rot::yaw(0.), 0);
        assert_eq!(fl2.z, 384.);
        assert_eq!(snap_transform(PieceKind::Wall, Vec3::new(1100., 30., 200.), Rot::yaw(0.), 0).0.z, 0.);
    }
}
