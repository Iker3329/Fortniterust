//! GDF PARTIAL UPDATES (2026-10-06): which texels of the four global distance field clipmaps the DFAO recomposites each frame, ported from UE4.16
//! `GlobalDistanceField.cpp` `ComputeUpdateRegionsAndUpdateViewState` (:865-1190) and its helpers (`AddUpdateRegionForAxis` :702, `AddUpdateRegionForPrimitive`
//! :727, `TrimOverlappingAxis` :759, `GetUpdateFrequencyForClipmap` :822, `ShouldUpdateClipmapThisFrame` :850). The composite / grid-cull dispatches of every
//! region and the scrolled (toroidal) addressing live in `distance_field_ao.rs` / `distance_field_ao.wgsl`.
//!
//! # 1.8 values
//! `r.AOGlobalDistanceFieldPartialUpdates` 1, `r.AOGlobalDistanceFieldStaggeredUpdates` 1, `r.AOGlobalDistanceFieldCacheMostlyStaticSeparately` 1: the
//! cvar registrations in the 1.8 exe (`dyninit_CVarAOGlobalDistanceFieldPartialUpdates` @14013cb60 -> global @1451b7e28, `...StaggeredUpdatess` @14013cc40 ->
//! @1451b7e2c, `...CacheMostlyStaticSeparately` @14013caf0 -> @1451b7e20) point at .data words that hold 1 (crates/fn-exe-defaults/reports/exe-defaults.json reads
//! RVAs 0x51b7e20 / 0x51b7e28 / 0x51b7e2c = 1; NOTE: that report pairs these three RVAs with the wrong cvar names, the decompiled registrations above are the
//! authority). UE4.16 GlobalDistanceField.cpp:29-50 initialises all three to 1 as well.
//!
//! # Mostly-static cache
//! With `CacheMostlyStaticSeparately` UE keeps a second set of clipmaps with the mostly-static primitives only and composites the often-moving ones on top of
//! it into the clipmaps the cone trace reads (`USE_PARENT_DISTANCE_FIELD`). Both sets get the same camera-movement regions; a modified primitive dirties
//! its own cache's regions. The cone trace reads min(static composite, movable composite) = the composite of every object, which is what the port's single
//! cache holds, recomputed over the union of the regions (every port distance-field object is a static mesh: none is `IsOftenMoving`). Same texel values.
use glam::{IVec3, Vec3, Vec4};

/// `r.AOGlobalDistanceFieldPartialUpdates` (1.8: 1).
pub const PARTIAL_UPDATES: bool = true;
/// `r.AOGlobalDistanceFieldStaggeredUpdates` (1.8: 1).
pub const STAGGERED_UPDATES: bool = true;
/// "Only use partial updates with small numbers of primitive modifications" (GlobalDistanceField.cpp:961).
pub const MAX_PARTIAL_PRIMITIVE_MODIFICATIONS: usize = 100;

/// `FBox` (float, UE cm = DF space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb { pub min: Vec3, pub max: Vec3 }
impl Aabb {
    /// `FBox::IsInsideOrOn` (Box.h:364).
    pub fn inside_or_on(&self, p: Vec3) -> bool { p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y && p.z >= self.min.z && p.z <= self.max.z }
    /// `FBox::Intersect` (Box.h:565): touching boxes intersect.
    pub fn intersect(&self, o: &Aabb) -> bool {
        !(self.min.x > o.max.x || o.min.x > self.max.x || self.min.y > o.max.y || o.min.y > self.max.y || self.min.z > o.max.z || o.min.z > self.max.z)
    }
}

/// `FVolumeUpdateRegion`: world bounds and the texels (`CellsSize`) a composite dispatch covers, starting at `bounds.min`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UpdateRegion { pub bounds: Aabb, pub cells: IVec3 }

/// `GetUpdateFrequencyForClipmap` (frequency, phase).
pub fn update_frequency(clip: usize) -> (u32, u32) {
    if clip == 0 || !STAGGERED_UPDATES { (1, 0) } else if clip == 1 { (2, 0) } else if clip == 2 { (4, 1) } else { (4, 3) }
}
/// `ShouldUpdateClipmapThisFrame`.
pub fn should_update_this_frame(clip: usize, update_index: u32) -> bool { let (f, p) = update_frequency(clip); update_index % f == p }

/// `AddUpdateRegionForAxis`: the slab of the clipmap the camera movement exposed along `axis`.
pub fn add_update_region_for_axis(movement: IVec3, clipmap: &Aabb, cell: f32, axis: usize, res: i32, out: &mut Vec<UpdateRegion>) {
    let mut r = UpdateRegion { bounds: *clipmap, cells: IVec3::splat(res) };
    r.cells[axis] = movement[axis].abs().min(res);
    if movement[axis] > 0 {
        // positive movement: the min of that axis moves up to contain the newly exposed area
        r.bounds.min[axis] = (clipmap.max[axis] - movement[axis] as f32 * cell).max(clipmap.min[axis]);
    } else if movement[axis] < 0 {
        r.bounds.max[axis] = (clipmap.min[axis] - movement[axis] as f32 * cell).min(clipmap.max[axis]);
    }
    if r.cells[axis] > 0 { out.push(r); }
}

/// `AddUpdateRegionForPrimitive`: the object's influence box (`Bounds.W + MaxSphereQueryRadius` around its bounding sphere), snapped to cells, clamped.
pub fn add_update_region_for_primitive(b: Vec4, max_sphere_query_radius: f32, clipmap: &Aabb, cell: f32, out: &mut Vec<UpdateRegion>) {
    let c = b.truncate();
    let bb_min = c - Vec3::splat(b.w) - Vec3::splat(max_sphere_query_radius);
    let mut min = Vec3::ZERO;
    for k in 0..3 { min[k] = (cell * (bb_min[k] / cell).floor()).max(clipmap.min[k]); }
    let mut max = min + Vec3::splat(((b.w + max_sphere_query_radius) * 2. / cell).ceil() * cell);
    for k in 0..3 { max[k] = max[k].min(clipmap.max[k]); }
    let size = max - min;
    // FMath::TruncToInt(x / CellSize + .5f)
    let cells = IVec3::new((size.x / cell + 0.5) as i32, (size.y / cell + 0.5) as i32, (size.z / cell + 0.5) as i32);
    if cells.x > 0 && cells.y > 0 && cells.z > 0 { out.push(UpdateRegion { bounds: Aabb { min, max }, cells }); }
}

/// `TrimOverlappingAxis`.
fn trim_overlapping_axis(trim: usize, cell: f32, other: &UpdateRegion, r: &mut UpdateRegion) {
    let (a0, a1) = ((trim + 1) % 3, (trim + 2) % 3);
    // entirely contained in 2d
    if r.bounds.max[a0] <= other.bounds.max[a0] && r.bounds.min[a0] >= other.bounds.min[a0] && r.bounds.max[a1] <= other.bounds.max[a1] && r.bounds.min[a1] >= other.bounds.min[a1] {
        if r.bounds.min[trim] >= other.bounds.min[trim] && r.bounds.min[trim] <= other.bounds.max[trim] {
            // min on this axis is inside the other region: clip it
            r.bounds.min[trim] = other.bounds.max[trim];
        } else {
            // otherwise the max must be inside, because the two volumes intersect
            r.bounds.max[trim] = other.bounds.min[trim];
        }
        r.cells[trim] = ((r.bounds.max[trim] - r.bounds.min[trim]).max(0.) / cell + 0.5) as i32;
    }
}

/// The partial-update regions of one clipmap update (`bLocalUsePartialUpdates` branch, GlobalDistanceField.cpp:963-1066): the movement slabs, the modified
/// primitives' boxes, fully contained regions dropped, overlapping ones trimmed. `None` = do a full update instead (more texels than one full update).
pub fn partial_update_regions(movement: IVec3, clipmap: &Aabb, cell: f32, res: i32, modified: &[Vec4], max_sphere_query_radius: f32) -> Option<Vec<UpdateRegion>> {
    let mut regions: Vec<UpdateRegion> = Vec::new();
    for axis in 0..3 { add_update_region_for_axis(movement, clipmap, cell, axis, res, &mut regions); }
    for b in modified { add_update_region_for_primitive(*b, max_sphere_query_radius, clipmap, cell, &mut regions); }
    // trim fully contained update regions
    let mut i = 0;
    while i < regions.len() {
        let r = regions[i];
        let contained = (0..regions.len()).any(|j| j != i && regions[j].bounds.inside_or_on(r.bounds.min) && regions[j].bounds.inside_or_on(r.bounds.max));
        if contained { regions.remove(i); } else { i += 1; }
    }
    // trim overlapping regions
    let mut i = 0;
    while i < regions.len() {
        let mut empty = false;
        for j in 0..regions.len() {
            if j == i { continue; }
            let other = regions[j];
            if other.bounds.intersect(&regions[i].bounds) {
                let r = &mut regions[i];
                trim_overlapping_axis(0, cell, &other, r);
                trim_overlapping_axis(1, cell, &other, r);
                trim_overlapping_axis(2, cell, &other, r);
                if r.cells.x == 0 || r.cells.y == 0 || r.cells.z == 0 { empty = true; break; }
            }
        }
        if empty { regions.remove(i); } else { i += 1; }
    }
    // fall back to a full update if the partial updates were going to do more work
    let texels: i64 = regions.iter().map(|r| r.cells.x as i64 * r.cells.y as i64 * r.cells.z as i64).sum();
    if texels >= (res as i64).pow(3) { return None; }
    Some(regions)
}

/// `FGlobalDistanceFieldClipmapState` of the view: where the last full update put the toroidal origin, where the clipmap is centred now, and the
/// primitive modifications accumulated since its last update.
#[derive(Clone, Debug, Default)]
pub struct ClipmapState {
    pub initialized: bool,
    pub full_update_origin: IVec3,
    pub last_partial_update_origin: IVec3,
    pub modified: Vec<Vec4>,
}

/// One clipmap's step of `ComputeUpdateRegionsAndUpdateViewState` for this frame: `Some(regions)` when it updates (empty = nothing changed), `None` when its
/// staggered turn is not this frame. `force` = `bForceFullUpdate` (first use / the atlas re-created: every object re-added).
#[allow(clippy::too_many_arguments)]
pub fn update_clipmap(state: &mut ClipmapState, clip: usize, update_index: u32, force_in: bool, view_origin: Vec3, extent: f32, res: i32, max_sphere_query_radius: f32) -> Option<Vec<UpdateRegion>> {
    let cell = extent * 2. / res as f32;
    let force = force_in || !state.initialized;
    if !(should_update_this_frame(clip, update_index) || force) { return None; }
    let grid = IVec3::new((view_origin.x / cell).floor() as i32, (view_origin.y / cell).floor() as i32, (view_origin.z / cell).floor() as i32);
    let snapped = grid.as_vec3() * cell;
    let clipmap = Aabb { min: snapped - Vec3::splat(extent), max: snapped + Vec3::splat(extent) };
    let use_partial = PARTIAL_UPDATES && !force;
    if !use_partial {
        // store the location of the full update
        state.full_update_origin = grid;
        state.initialized = true;
    }
    let local_partial = use_partial && state.modified.len() < MAX_PARTIAL_PRIMITIVE_MODIFICATIONS;
    let regions = local_partial.then(|| partial_update_regions(grid - state.last_partial_update_origin, &clipmap, cell, res, &state.modified, max_sphere_query_radius)).flatten()
        .unwrap_or_else(|| vec![UpdateRegion { bounds: clipmap, cells: IVec3::splat(res) }]);
    state.modified.clear();
    state.last_partial_update_origin = grid;
    Some(regions)
}

#[cfg(test)]
mod tests {
    use super::*;
    const RES: i32 = 128;

    #[test]
    fn staggered_frequencies_match_ue() {
        // clipmap 0 every frame, 1 every 2nd (phase 0), 2 / 3 every 4th (phases 1 / 3): at most 2 clipmaps per frame
        let n: Vec<usize> = (0..8u32).map(|i| (0..4).filter(|&c| should_update_this_frame(c, i)).count()).collect();
        assert!(n.iter().all(|&k| k <= 2), "{n:?}");
        assert!(should_update_this_frame(2, 1) && should_update_this_frame(3, 3) && !should_update_this_frame(3, 1));
    }

    #[test]
    fn camera_movement_updates_only_the_exposed_slabs() {
        let mut st = ClipmapState::default();
        let extent = 2500.;
        let cell = extent * 2. / RES as f32;
        // first frame: full update, origin stored
        let r = update_clipmap(&mut st, 0, 0, false, Vec3::new(100., 200., 50.), extent, RES, 600.).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].cells, IVec3::splat(RES));
        let full = st.full_update_origin;
        // move by 2 cells in +x and 1 cell in -z: one 2-cell slab on x, one 1-cell slab on z, overlap trimmed away
        let r = update_clipmap(&mut st, 0, 1, false, Vec3::new(100. + 2. * cell, 200., 50. - cell), extent, RES, 600.).unwrap();
        assert_eq!(st.full_update_origin, full, "a partial update keeps the toroidal origin");
        let texels: i32 = r.iter().map(|x| x.cells.x * x.cells.y * x.cells.z).sum();
        assert!(texels <= 2 * RES * RES + RES * RES, "{r:?}");
        assert!(r.iter().any(|x| x.cells.x == 2 && x.cells.y == RES), "{r:?}");
        // no movement, nothing modified: nothing to do
        let r = update_clipmap(&mut st, 0, 2, false, Vec3::new(100. + 2. * cell, 200., 50. - cell), extent, RES, 600.).unwrap();
        assert!(r.is_empty());
        // a teleport larger than the clipmap falls back to one full region
        let r = update_clipmap(&mut st, 0, 3, false, Vec3::new(1e6, 0., 0.), extent, RES, 600.).unwrap();
        assert_eq!(r, vec![UpdateRegion { bounds: r[0].bounds, cells: IVec3::splat(RES) }]);
    }

    #[test]
    fn regions_tile_the_newly_exposed_cells_exactly_once() {
        // every texel of the moved clipmap that was outside the previous one is covered by exactly one region; nothing else is
        let extent = 2500.;
        let cell = extent * 2. / RES as f32;
        let mut st = ClipmapState::default();
        update_clipmap(&mut st, 0, 0, false, Vec3::ZERO, extent, RES, 600.).unwrap();
        let old = st.last_partial_update_origin;
        let eye = Vec3::new(3.4 * cell, -2.2 * cell, 5.7 * cell);
        let regions = update_clipmap(&mut st, 0, 1, false, eye, extent, RES, 600.).unwrap();
        let new = st.last_partial_update_origin;
        let mut covered = std::collections::HashMap::new();
        for r in &regions {
            for z in 0..r.cells.z { for y in 0..r.cells.y { for x in 0..r.cells.x {
                let p = r.bounds.min + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * cell;
                let g = (p / cell).floor().as_ivec3();
                *covered.entry(g).or_insert(0) += 1;
            }}}
        }
        assert!(covered.values().all(|&k| k == 1), "a texel was composited twice");
        let half = RES / 2;
        let inside = |g: IVec3, o: IVec3| (0..3).all(|k| g[k] >= o[k] - half && g[k] < o[k] + half);
        for z in new.z - half..new.z + half { for y in new.y - half..new.y + half { for x in new.x - half..new.x + half {
            let g = IVec3::new(x, y, z);
            assert_eq!(covered.contains_key(&g), !inside(g, old), "texel {g:?}");
        }}}
    }

    #[test]
    fn a_modified_primitive_dirties_its_influence_box() {
        let extent = 2500.;
        let cell = extent * 2. / RES as f32;
        let mut st = ClipmapState::default();
        update_clipmap(&mut st, 0, 0, false, Vec3::ZERO, extent, RES, 600.).unwrap();
        st.modified.push(Vec4::new(300., -200., 0., 150.));
        let r = update_clipmap(&mut st, 0, 1, false, Vec3::ZERO, extent, RES, 600.).unwrap();
        assert_eq!(r.len(), 1);
        // (150 + 600) * 2 / cell rounded up, in cells
        let n = ((150f32 + 600.) * 2. / cell).ceil() as i32;
        assert_eq!(r[0].cells, IVec3::splat(n));
        assert!(r[0].bounds.inside_or_on(Vec3::new(300., -200., 0.)));
        assert!(st.modified.is_empty());
        // 100 or more modifications: a full update
        st.modified = (0..100).map(|i| Vec4::new(i as f32 * 10., 0., 0., 10.)).collect();
        let r = update_clipmap(&mut st, 0, 2, false, Vec3::ZERO, extent, RES, 600.).unwrap();
        assert_eq!(r[0].cells, IVec3::splat(RES));
    }

    #[test]
    fn staggered_clipmaps_skip_frames_but_force_updates_them() {
        let mut st = ClipmapState::default();
        assert!(update_clipmap(&mut st, 3, 0, false, Vec3::ZERO, 20000., RES, 600.).is_some(), "uninitialized: forced");
        assert!(update_clipmap(&mut st, 3, 0, false, Vec3::ZERO, 20000., RES, 600.).is_none(), "not its turn");
        assert!(update_clipmap(&mut st, 3, 3, false, Vec3::ZERO, 20000., RES, 600.).is_some());
        assert!(update_clipmap(&mut st, 3, 0, true, Vec3::ZERO, 20000., RES, 600.).is_some(), "bForceFullUpdate");
    }
}
