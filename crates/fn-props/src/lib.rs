//! Collision for the placeholder harvestables (trees, rocks): camera probe sweeps and player capsule push-out.
//!
//! SOURCE OF TRUTH: the solids are generated from the same constants as the rendered meshes (`combat::TREE_TRUNK`,
//! `TREE_CANOPIES`, `ROCK_RINGS`, ...) and from the live `combat::World::props` list (dead/harvested props have no collision),
//! and a test checks that every rendered vertex lies inside the union of the solids.
//!
//! Every solid is a convex frustum of revolution (cone / truncated cone) about the prop's vertical axis; the distance from a
//! point to it is computed exactly in the (radius, height) half-plane. Sphere sweeps use sphere tracing (the distance is
//! 1-Lipschitz, so stepping by distance - radius never skips a surface) followed by bisection to the contact point.
//!
//! RECOVERED vs ASSUMED: no tree/rock collision data exists in the 1.8 data used here (the meshes are procedural
//! placeholders), so every shape and rule below is ASSUMED. What IS recovered is the camera probe itself (8 cm sphere + four
//! +/-1 degree rays, camera.rs). Assumptions: trunk = frustum, canopy = cones (camera-only, the player walks/jumps through
//! them), rocks = a stack of frusta over the largest jittered ring radius (conservative hull), player capsule radius 42 cm
//! (AFortPawn ctor @1407c9ff0 writes it; no cooked pawn overrides it). The player no longer uses the push-out
//! `Blocker::resolve` below: movement_cmc.rs sweeps a real capsule (42 cm radius, 75 cm half height, crouched 40 clamped to 42) against
//! these solids (`movement_scene::Scene::from_blockers`), with step-up, slopes, perching and ledges; `Blocker::keep_out` stays as the
//! conservative cylinder bound the camera/prop tests use.
use glam::{Vec2, Vec3};
use fn_combat::{self as combat, Harvestable, Kind};
use std::sync::atomic::{AtomicBool, Ordering};
pub mod index;
pub use index::PropIndex;

// PLAYER_RADIUS / PLAYER_HEIGHT and `Blocker` live in fn-movement (the movement crate must not depend on this one); re-exported here.
pub use fn_movement::scene::{Blocker, PLAYER_HEIGHT, PLAYER_RADIUS};

static DISABLED: AtomicBool = AtomicBool::new(false);
/// Test hook (`--camera-prop-test --no-prop-collision`): switches prop collision off for the camera AND the player so the
/// "before" captures can be taken with the same build and the same script.
pub fn set_disabled(off: bool) { DISABLED.store(off, Ordering::Relaxed); }
pub fn disabled() -> bool { DISABLED.load(Ordering::Relaxed) }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part { Trunk, Canopy, Rock }

/// Frustum of revolution: axis through `base` (prop origin), spanning heights `h0..h1` with radii `r0` -> `r1`.
#[derive(Clone, Copy, Debug)]
pub struct Solid { pub part: Part, pub base: Vec3, pub h0: f32, pub h1: f32, pub r0: f32, pub r1: f32 }

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a; let l2 = ab.length_squared();
    let t = if l2 > 0. { ((p - a).dot(ab) / l2).clamp(0., 1.) } else { 0. };
    (p - (a + ab * t)).length()
}

impl Solid {
    pub fn player_blocks(&self) -> bool { self.part != Part::Canopy }
    pub fn y_lo(&self) -> f32 { self.base.y + self.h0 }
    pub fn y_hi(&self) -> f32 { self.base.y + self.h1 }
    pub fn max_radius(&self) -> f32 { self.r0.max(self.r1) }
    /// Signed distance from `p` to the solid (negative inside). Exact for this convex solid of revolution.
    pub fn sdf(&self, p: Vec3) -> f32 {
        let rho = ((p.x - self.base.x).powi(2) + (p.z - self.base.z).powi(2)).sqrt();
        let y = p.y - self.base.y;
        let (a, b, c, d) = (Vec2::new(0., self.h0), Vec2::new(self.r0, self.h0), Vec2::new(self.r1, self.h1), Vec2::new(0., self.h1));
        let q = Vec2::new(rho, y);
        let outer = seg_dist(q, a, b).min(seg_dist(q, b, c)).min(seg_dist(q, c, d));
        let inside = y >= self.h0 && y <= self.h1 && rho <= self.r0 + (self.r1 - self.r0) * (y - self.h0) / (self.h1 - self.h0).max(1e-6);
        if inside { -outer.min(seg_dist(q, d, a)) } else { outer }
    }
    /// First contact of a sphere of `radius` swept from `from` to `to`, as a fraction of the segment.
    /// `Some(0.)` when the sphere already overlaps the solid at `from`.
    pub fn trace(&self, from: Vec3, to: Vec3, radius: f32) -> Option<f32> {
        let seg = to - from; let len = seg.length();
        if self.sdf(from) <= radius { return Some(0.); }
        if len < 1e-6 { return None; }
        // Broad phase: horizontal distance from the axis to the segment against the widest radius.
        let (f2, s2) = (Vec2::new(from.x - self.base.x, from.z - self.base.z), Vec2::new(seg.x, seg.z));
        let l2 = s2.length_squared();
        let tc = if l2 > 0. { (-f2.dot(s2) / l2).clamp(0., 1.) } else { 0. };
        if (f2 + s2 * tc).length() > self.max_radius() + radius + 0.01 { return None; }
        let dir = seg / len;
        let free = |t: f32| self.sdf(from + dir * t) - radius;
        let bisect = |mut lo: f32, mut hi: f32| { for _ in 0..28 { let m = 0.5 * (lo + hi); if free(m) <= 0. { hi = m } else { lo = m } } lo / len };
        let (mut t, mut prev_out) = (0f32, 0f32);
        for _ in 0..4096 {
            let d = free(t);
            if d <= 0. { return Some(bisect(prev_out, t)); }
            prev_out = t;
            if t >= len { return None; }
            t = (t + d.max(0.004)).min(len);
        }
        None
    }
}

/// Collision solids of one prop in world space (empty for harvested/dead props).
pub fn solids(p: &Harvestable) -> Vec<Solid> {
    if !p.alive() || p.hull.is_some() { return vec![]; } // a real prop collides through its convex hulls (`trace` / `clearance` / `player_hulls`)
    let s = |part, h0, h1, r0, r1| Solid { part, base: p.pos, h0, h1, r0, r1 };
    match p.kind {
        Kind::Tree => {
            let (tr0, tr1, th) = combat::TREE_TRUNK;
            let mut v = vec![s(Part::Trunk, 0., th, tr0, tr1)];
            for (r, y0, y1, _) in combat::TREE_CANOPIES { v.push(s(Part::Canopy, y0, y1, r, 0.)); }
            v
        }
        Kind::Rock => {
            // The mesh scales ring vertices by 1 +/- ROCK_JITTER: use the largest scale so the hull contains every vertex.
            let k = 1. + combat::ROCK_JITTER;
            let r = combat::ROCK_RINGS;
            vec![s(Part::Rock, r[0].1, r[1].1, r[0].0 * k, r[1].0 * k),
                 s(Part::Rock, r[1].1, r[2].1, r[1].0 * k, r[2].0 * k),
                 s(Part::Rock, r[2].1, combat::ROCK_APEX.1, r[2].0 * k, combat::ROCK_APEX.0)]
        }
    }
}

/// Camera probe: earliest sphere-sweep contact fraction against every prop (None = clear). Matches
/// `camera::Camera::update`'s trace signature.
pub fn trace(props: &[Harvestable], from: Vec3, to: Vec3, radius: f32) -> Option<f32> { trace_iter(props.iter(), from, to, radius) }
/// PERF (2026-10-08): [`trace`] over the props `ids` (indices into `props`, ascending, e.g. from [`PropIndex`]): the same tests in the same order.
pub fn trace_ids(props: &[Harvestable], ids: &[u32], from: Vec3, to: Vec3, radius: f32) -> Option<f32> { trace_iter(ids.iter().map(|&i| &props[i as usize]), from, to, radius) }
pub fn segment_ids(idx: Option<&PropIndex>, from: Vec3, to: Vec3, radius: f32) -> Option<Vec<u32>> {
    let ix = idx.filter(|_| index::enabled())?;
    let mut ids = Vec::new();
    ix.segment_candidates(Vec2::new(from.x, from.z), Vec2::new(to.x, to.z), radius, &mut ids);
    ids.extend_from_slice(ix.plain());
    ids.sort_unstable();
    Some(ids)
}
pub fn trace_in(props: &[Harvestable], idx: Option<&PropIndex>, from: Vec3, to: Vec3, radius: f32) -> Option<f32> {
    let Some(ids) = segment_ids(idx, from, to, radius) else { return trace(props, from, to, radius) };
    let r = trace_ids(props, &ids, from, to, radius);
    if index::verify() { let lin = trace(props, from, to, radius); if lin != r { eprintln!("[prop index] MISMATCH trace {r:?} indexed vs {lin:?} linear"); } }
    r
}
fn trace_iter<'a>(props: impl Iterator<Item = &'a Harvestable> + Clone, from: Vec3, to: Vec3, radius: f32) -> Option<f32> {
    if disabled() { return None; }
    let mut best: Option<f32> = None;
    for p in props.clone().filter(|p| p.alive()) {
        if let Some(h) = &p.hull { if let Some(t) = h.trace_sphere(from, to, radius) { best = Some(best.map_or(t, |b| b.min(t))); } }
    }
    for p in props { if p.hull.is_some() || !p.alive() { continue; } for s in solids(p) {
        // ASSUMED: canopies are leaves (non-solid for the player). When the probe starts INSIDE one (the pawn's head is in the
        // foliage after a jump) that canopy is ignored for this sweep, so the pawn stays visible instead of the boom collapsing
        // to zero for the whole jump; trunks and rocks always block, and a canopy still blocks a boom that enters it from outside.
        if s.part == Part::Canopy && s.sdf(from) <= radius { continue; }
        if let Some(t) = s.trace(from, to, radius) { best = Some(best.map_or(t, |b| b.min(t))); }
    } }
    best
}

/// Smallest signed distance from `p` to any solid, optionally ignoring canopies. `f32::MAX` with no props.
pub fn clearance(props: &[Harvestable], p: Vec3, include_canopy: bool) -> f32 {
    let mut m = f32::MAX;
    for pr in props.iter().filter(|p| p.alive()) { if let Some(h) = &pr.hull { m = m.min(h.sdf(p)); } }
    for pr in props { for s in solids(pr) { if include_canopy || s.part != Part::Canopy { m = m.min(s.sdf(p)); } } }
    m
}

/// Player blockers from the live props (trunks and rocks only: canopies are camera-only). `z_offset` converts world z to
/// controller y (main.rs: 12).
pub fn player_blockers(props: &[Harvestable], z_offset: f32) -> Vec<Blocker> { player_blockers_iter(props.iter(), z_offset) }
/// PERF (2026-10-08): [`player_blockers`] with the index: only hull-less props have frusta (`solids` is empty for a hull prop), so only
/// [`PropIndex::plain`] is visited, in the same order. `None` = the linear scan.
pub fn player_blockers_in(props: &[Harvestable], idx: Option<&PropIndex>, z_offset: f32) -> Vec<Blocker> {
    let Some(ix) = idx else { return player_blockers(props, z_offset) };
    let out = player_blockers_iter(ix.plain().iter().map(|&i| &props[i as usize]), z_offset);
    if index::verify() { let lin = player_blockers(props, z_offset); if format!("{lin:?}") != format!("{out:?}") { eprintln!("[prop index] MISMATCH player_blockers: {} indexed vs {} linear", out.len(), lin.len()); } }
    out
}
fn player_blockers_iter<'a>(props: impl Iterator<Item = &'a Harvestable>, z_offset: f32) -> Vec<Blocker> {
    if disabled() { return vec![]; }
    let mut out = vec![];
    for p in props { for s in solids(p) { if s.player_blocks() {
        out.push(Blocker { centre: Vec2::new(s.base.x, s.base.z - z_offset), y_lo: s.y_lo(), y_hi: s.y_hi(), r_lo: s.r0, r_hi: s.r1 });
    } } }
    out
}

/// The convex hulls of the real props that can touch the player, in CONTROLLER space (metres: x, world z - `z_offset`, up) for `Controller::hulls`:
/// only pieces whose bounding sphere comes within `range` metres of `around` (controller x / y). Dead props and disabled collision give none.
pub fn player_hulls(props: &[Harvestable], z_offset: f32, around: Vec2, range: f32) -> Vec<fn_hull::Hull> { player_hulls_iter(props.iter(), z_offset, around, range, None) }
/// PERF (2026-10-08): [`player_hulls`] over the index's candidates (ascending, the same per-prop test). `None` = the linear scan.
pub fn player_hulls_in(props: &[Harvestable], idx: Option<&PropIndex>, z_offset: f32, around: Vec2, range: f32) -> Vec<fn_hull::Hull> { player_hulls_cached(props, idx, z_offset, around, range, None) }
/// PERF (2026-10-08): [`player_hulls_in`], taking the controller-space copies of the hulls from `cache` (filled with the very same `mapped` calls, so the
/// same values) instead of re-mapping (`Hull::finish`: planes, edges) every tick.
pub fn player_hulls_cached(props: &[Harvestable], idx: Option<&PropIndex>, z_offset: f32, around: Vec2, range: f32, cache: Option<&mut HullMapCache>) -> Vec<fn_hull::Hull> {
    let Some(ix) = idx else { return player_hulls_iter(props.iter(), z_offset, around, range, cache) };
    let mut ids = vec![];
    ix.hull_candidates(Vec2::new(around.x, around.y + z_offset), range, &mut ids);
    let out = player_hulls_iter(ids.iter().map(|&i| &props[i as usize]), z_offset, around, range, cache);
    if index::verify() { let lin = player_hulls(props, z_offset, around, range); if format!("{lin:?}") != format!("{out:?}") { eprintln!("[prop index] MISMATCH player_hulls: {} indexed vs {} linear", out.len(), lin.len()); } }
    out
}
/// PERF (2026-10-08): the controller-space hulls of the props the last [`player_hulls_cached`] query used, by `HullSet` (the `Arc` is held, so its address
/// cannot be reused by another set while cached). Entries not used by a query are dropped after it.
#[derive(Clone, Debug, Default)]
pub struct HullMapCache { sets: std::collections::HashMap<usize, (std::sync::Arc<fn_hull::HullSet>, Vec<Option<fn_hull::Hull>>, bool)>, z_offset_bits: u32 }
fn player_hulls_iter<'a>(props: impl Iterator<Item = &'a Harvestable>, z_offset: f32, around: Vec2, range: f32, mut cache: Option<&mut HullMapCache>) -> Vec<fn_hull::Hull> {
    if disabled() { return vec![]; }
    if let Some(c) = cache.as_deref_mut() { if c.z_offset_bits != z_offset.to_bits() { c.sets.clear(); c.z_offset_bits = z_offset.to_bits(); } for e in c.sets.values_mut() { e.2 = false; } }
    let mut out = vec![];
    let w = Vec2::new(around.x, around.y + z_offset);
    for p in props {
        let Some(set) = &p.hull else { continue };
        if !p.alive() || (Vec2::new(set.center.x, set.center.z) - w).length() - set.radius > range { continue; }
        for (k, h) in set.hulls.iter().enumerate() {
            if (Vec2::new(h.center.x, h.center.z) - w).length() - h.radius > range { continue; }
            let map = || h.mapped(|v| Vec3::new(v.x, v.z - z_offset, v.y), true);
            match cache.as_deref_mut() {
                None => out.push(map()),
                Some(c) => {
                    let e = c.sets.entry(std::sync::Arc::as_ptr(set) as usize).or_insert_with(|| (set.clone(), vec![None; set.hulls.len()], true));
                    e.2 = true;
                    out.push(e.1[k].get_or_insert_with(map).clone());
                }
            }
        }
    }
    if let Some(c) = cache { c.sets.retain(|_, e| e.2); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_camera::{self as camera, Camera, Input, MELEE, View};
    use fn_movement::Controller;

    fn tree_at(x: f32, z: f32) -> Harvestable { Harvestable::new(Kind::Tree, Vec3::new(x, 0., z)) }
    fn rock_at(x: f32, z: f32) -> Harvestable { Harvestable::new(Kind::Rock, Vec3::new(x, 0., z)) }
    /// Deterministic xorshift in [0,1).
    struct Rng(u32);
    impl Rng { fn f(&mut self) -> f32 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 17; self.0 ^= self.0 << 5; (self.0 >> 8) as f32 / (1u32 << 24) as f32 } fn r(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() } }
    fn world_trace(props: &[Harvestable]) -> impl Fn(Vec3, Vec3, f32) -> Option<f32> + '_ {
        move |a, b, r| match (camera::trace_ground(a, b, r), trace(props, a, b, r)) { (Some(x), Some(y)) => Some(x.min(y)), (x, y) => x.or(y) }
    }

    // ---- shared source of truth ----
    #[test] fn every_rendered_vertex_is_inside_the_collision_solids() {
        for (kind, p) in [(Kind::Tree, tree_at(1., 2.)), (Kind::Rock, rock_at(-3., 4.))] {
            let sol = solids(&p);
            for (tri, _) in combat::prop_mesh(kind) { for v in tri {
                let w = p.pos + v;
                let d = sol.iter().map(|s| s.sdf(w)).fold(f32::MAX, f32::min);
                assert!(d <= 1e-4, "{kind:?} vertex {v:?} is {d} m outside the colliders");
            } }
        }
    }
    #[test] fn dead_props_have_no_collision() {
        let mut p = tree_at(0., 0.); p.health = 0.;
        assert!(solids(&p).is_empty() && trace(&[p.clone()], Vec3::new(-3., 1., 0.), Vec3::new(3., 1., 0.), 0.08).is_none());
        assert!(player_blockers(&[p], 12.).is_empty());
    }

    // ---- sweep exactness ----
    #[test] fn sphere_sweep_hits_trunk_at_the_analytic_distance() {
        let p = [tree_at(0., 0.)];
        // From z=-3 toward +z at y=1: trunk radius at y=1 is 0.4 - 0.1/3, sphere radius 0.08.
        let r = 0.4 - 0.1 / 3.; let want = (3. - r - 0.08) / 6.;
        let t = trace(&p, Vec3::new(0., 1., -3.), Vec3::new(0., 1., 3.), 0.08).unwrap();
        assert!((t - want).abs() < 2e-3, "{t} vs {want}");
        assert!(trace(&p, Vec3::new(1., 1., -3.), Vec3::new(1., 1., 3.), 0.08).is_none(), "ray 1 m off axis passes the 0.4 m trunk");
        assert_eq!(trace(&p, Vec3::new(0.1, 1., 0.), Vec3::new(0., 1., 3.), 0.08), Some(0.), "start inside reports 0");
    }
    #[test] fn canopy_blocks_the_probe_above_the_trunk_and_clears_below() {
        let p = [tree_at(0., 0.)];
        assert!(trace(&p, Vec3::new(-4., 3., 0.), Vec3::new(4., 3., 0.), 0.08).is_some(), "canopy at 3 m height");
        assert!(trace(&p, Vec3::new(-4., 2., 0.5), Vec3::new(4., 2., 0.5), 0.08).is_none(), "below the canopy, beside the trunk");
        let t = trace(&p, Vec3::new(-4., 3., 0.), Vec3::new(4., 3., 0.), 0.08).unwrap();
        // canopy 1 radius at y=3: 1.9*(1-0.6/2.6); sphere stops 0.08 short measured radially (cone slope makes it slightly more)
        let rr = 1.9 * (1. - 0.6 / 2.6); assert!((4. - t * 8. - rr).abs() < 0.2, "stops near the canopy skin: {}", 4. - t * 8.);
    }
    #[test] fn rock_hull_blocks_and_clears_above() {
        let p = [rock_at(0., 0.)];
        assert!(trace(&p, Vec3::new(-4., 0.5, 0.), Vec3::new(4., 0.5, 0.), 0.08).is_some());
        assert!(trace(&p, Vec3::new(-4., 1.8, 0.), Vec3::new(4., 1.8, 0.), 0.08).is_none());
    }

    // ---- camera: never inside any prop ----
    fn props_cluster() -> Vec<Harvestable> { vec![tree_at(-3.2, 0.), rock_at(3., -1.), tree_at(0.5, -4.), tree_at(1.5, 3.), rock_at(-1., 5.)] }
    #[test] fn camera_never_inside_any_prop_for_random_views_and_positions() {
        let props = props_cluster(); let mut rng = Rng(0x1234_5678);
        let blockers = player_blockers(&props, 0.);
        let (mut cases, mut pawn_in_canopy) = (0, 0);
        while cases < 1500 {
            let feet_xz = Vec2::new(rng.r(-7., 7.), rng.r(-8., 8.));
            // Valid player positions only: the player capsule cannot be inside a trunk or rock.
            let h = if rng.f() < 0.5 { 0. } else { rng.r(0., 1.84) };
            if blockers.iter().any(|b| b.keep_out(h).map_or(false, |r| (feet_xz - b.centre).length() < r - 1e-3)) { continue; }
            let (yaw, pitch, crouched) = (rng.r(0., 360.), rng.r(-89., 89.), rng.f() < 0.2);
            let feet = Vec3::new(feet_xz.x, h, feet_xz.y);
            let mut cam = Camera::default();
            for frame in 0..80 {
                let v = cam.update(&MELEE, Input { feet, crouched, yaw_deg: yaw, pitch_deg: pitch, dt: 1. / 120. }, world_trace(&props));
                let solid_clear = clearance(&props, v.eye, false);
                assert!(solid_clear >= 0.08 - 0.01, "case {cases} frame {frame}: eye {:?} is inside a trunk/rock (clearance {solid_clear}), yaw {yaw} pitch {pitch} feet {feet:?}", v.eye);
                let all_clear = clearance(&props, v.eye, true);
                if all_clear < 0.08 - 0.01 {
                    // Only allowed when the pawn's own capsule axis is inside the same (camera-only, non-solid) canopy: the pawn is in
                    // the foliage, so the camera shares it (canopy ignored for that sweep, see `trace`).
                    assert!(clearance(&props, v.safe, true) < 0.08, "case {cases}: eye in canopy while the pawn is not (clearance {all_clear})");
                    if frame == 79 { pawn_in_canopy += 1; }
                }
                assert!(v.eye.y >= 0.08 - 1e-3);
            }
            cases += 1;
        }
        eprintln!("pawn-inside-canopy cases: {pawn_in_canopy}/1500");
    }

    // ---- camera: boom shortening is continuous ----
    fn boom_len(v: &View) -> f32 { (v.eye - v.safe).length() }
    #[test] fn boom_shortens_and_recovers_without_pops_while_backing_into_a_trunk() {
        let props = [tree_at(0.31, 6.)];
        let mut cam = Camera::default(); let mut feet = Vec3::new(0., 0., 0.);
        let (mut last, mut max_drop, mut max_rise, mut min_len) = (None::<f32>, 0f32, 0f32, f32::MAX);
        for i in 0..600 {
            // 5.5 m/s sprint: backwards (+z, toward the trunk behind the camera) for 100 steps (4.6 m, stopping 1.4 m short of
            // the trunk axis, like the player collision would), then forwards.
            let vz = if i < 100 { 5.5 } else { -5.5 };
            feet.z += vz / 120.;
            let v = cam.update(&MELEE, Input { feet, crouched: false, yaw_deg: 0., pitch_deg: -12., dt: 1. / 120. }, world_trace(&props));
            let l = boom_len(&v); min_len = min_len.min(l);
            if let Some(p) = last { max_drop = max_drop.max(p - l); max_rise = max_rise.max(l - p); }
            last = Some(l);
        }
        // The pawn itself moves 4.6 cm/frame; the boom may shorten by that plus the trunk approach, never by a pop.
        assert!(max_drop < 0.06, "boom dropped {max_drop} m in one frame");
        // Blend-out (assumed 0.15 s): the kept fraction rises by at most dt/0.15 of the remaining gap per frame.
        assert!(max_rise < 0.08, "boom grew {max_rise} m in one frame");
        assert!(min_len < 2.0, "the trunk must actually shorten the boom (min {min_len})");
    }
    #[test] fn blend_out_after_a_silhouette_pass_is_gradual() {
        // Orbit the view around a tree 1.5 m from the pawn: block-in is instant by design (probe), blend-out is bounded.
        let props = [tree_at(0.31, 1.7)]; let mut cam = Camera::default(); let mut last = None::<f32>;
        let (mut max_rise, mut max_drop) = (0f32, 0f32);
        for i in 0..720 {
            let yaw = (i as f32 * 1.0) % 360.; // 120 deg/s
            let v = cam.update(&MELEE, Input { feet: Vec3::ZERO, crouched: false, yaw_deg: yaw, pitch_deg: -12., dt: 1. / 120. }, world_trace(&props));
            let l = boom_len(&v);
            if let Some(p) = last { max_rise = max_rise.max(l - p); max_drop = max_drop.max(p - l); }
            last = Some(l);
        }
        assert!(max_rise < 0.09, "blend-out must be gradual, rose {max_rise} m in one frame");
        eprintln!("silhouette pass: largest one-frame block-in {max_drop:.3} m (instant by design), largest blend-out step {max_rise:.3} m");
    }
    #[test] fn jumping_beside_a_tree_keeps_the_camera_out_of_the_trunk_and_recovers_after_landing() {
        // The reported case: the head goes through the canopy during a jump next to a tree.
        for tree in [Vec2::new(0.8, 1.4), Vec2::new(0.0, 2.9), Vec2::new(-1.2, 0.3)] {
            let props = [tree_at(tree.x, tree.y)]; let mut c = Controller::default(); c.blockers = player_blockers(&props, 0.);
            let mut cam = Camera::default();
            let (mut hidden, mut frames, mut max_change, mut last, mut min_solid) = (0, 0, 0f32, None::<f32>, f32::MAX);
            for i in 0..240 {
                c.step(Vec2::ZERO, i == 20, false, 1. / 120.);
                let v = cam.update(&MELEE, Input { feet: Vec3::new(c.position.x, c.height, c.position.y), crouched: false, yaw_deg: 0., pitch_deg: -12., dt: 1. / 120. }, world_trace(&props));
                let l = boom_len(&v); if let Some(p) = last { max_change = max_change.max((l - p).abs()); } last = Some(l);
                min_solid = min_solid.min(clearance(&props, v.eye, false));
                if c.height > 0. { frames += 1; if v.hide_pawn { hidden += 1; } }
                if std::env::var_os("PROPCOL_DEBUG").is_some() && i % 6 == 0 { eprintln!("i {i} h {:.2} kept {:.3} safe.y {:.2} safe-clr {:.3} eye {:?}", c.height, v.blocked_pct, v.safe.y, clearance(&props, v.safe, true), v.eye); }
            }
            eprintln!("tree {tree:?}: pawn hidden {hidden}/{frames} airborne frames, largest one-frame boom change {max_change:.2} m, min trunk clearance {min_solid:.3}");
            assert!(min_solid >= 0.07);
            // Honest limitation: with the canopy blocking the camera, a jump that puts the head beside/in the cone collapses the boom
            // and hides the pawn for part of the arc (reported above, not hidden); the boom must fully recover after landing.
            assert!(last.unwrap() > 1.5, "boom recovers after landing: {last:?}");
        }
    }
    #[test] fn hide_pawn_threshold_still_applies_with_props() {
        // Tree trunk 0.7 m behind the pawn at yaw 0: boom is almost fully blocked -> pawn hidden.
        let props = [tree_at(0.31, 0.75)]; let mut cam = Camera::default(); let mut v = View::default();
        for _ in 0..120 { v = cam.update(&MELEE, Input { feet: Vec3::new(0., 0., 0.), crouched: false, yaw_deg: 0., pitch_deg: -12., dt: 1. / 120. }, world_trace(&props)); }
        eprintln!("boom kept {:.3} hide {}", v.blocked_pct, v.hide_pawn);
        assert!(v.blocked_pct < 0.5, "{}", v.blocked_pct);
        assert_eq!(v.hide_pawn, v.blocked_pct <= MELEE.hide_pawn_pct);
    }

    // ---- player cannot penetrate ----
    fn blockers_for(props: &[Harvestable]) -> Vec<Blocker> { player_blockers(props, 12.) }
    /// Exact gap (metres, negative = penetration) between the player's capsule (movement_cmc.rs, 42 cm radius, standing or crouched half height)
    /// and the blockers + baseplate, from the same scene the movement component collides with.
    fn capsule_gap(c: &Controller) -> f32 {
        fn_movement::scene::Scene::from_blockers(&c.blockers).probe(&c.component().capsule()).gap * 0.01
    }
    fn run_controller(props: &[Harvestable], start: Vec2, dir: Vec2, secs: f32, dt: f32, jump_at: Option<f32>) -> (Controller, f32) {
        let mut c = Controller::default(); c.position = start; c.blockers = blockers_for(props);
        let mut min_gap = f32::MAX; let mut t = 0.;
        while t < secs {
            c.step(dir, jump_at.map_or(false, |j| t >= j && t < j + 0.05), true, dt);
            min_gap = min_gap.min(capsule_gap(&c));
            t += dt;
        }
        (c, min_gap)
    }
    #[test] fn player_cannot_walk_through_a_trunk_at_any_frame_rate() {
        let props = [tree_at(0., 12. - 4.)]; // controller-space centre (0,-4)
        for dt in [1. / 240., 1. / 120., 1. / 60., 1. / 30., 0.05] {
            let (c, gap) = run_controller(&props, Vec2::new(0., 0.), Vec2::new(0., -1.), 3., dt, None);
            assert!(gap > -1e-3, "dt {dt}: penetrated by {}", -gap);
            // Stands against the trunk: 0.4 trunk base radius + PLAYER_RADIUS (42 cm, AFortPawn ctor @1407c9ff0).
            assert!((c.position.y - (-4. + 0.4 + PLAYER_RADIUS)).abs() < 0.02, "dt {dt}: {:?}", c.position);
            assert!(c.velocity.length() < 0.5, "walking into a wall must stop dead-on");
        }
    }
    #[test] fn player_slides_along_the_surface_instead_of_sticking() {
        let props = [tree_at(0., 12. - 4.)];
        // Aim 0.3 m right of the axis: should deflect around and keep making progress past the tree.
        let (c, gap) = run_controller(&props, Vec2::new(0.3, 0.), Vec2::new(0., -1.), 4., 1. / 120., None);
        assert!(gap > -1e-3);
        assert!(c.position.y < -4. - 0.3, "slid around and past the trunk instead of stalling: {:?}", c.position);
        // Oblique approach keeps its tangential velocity component.
        let (c, gap) = run_controller(&props, Vec2::new(-2., -2.), Vec2::new(1., -0.2).normalize(), 0.9, 1. / 120., None);
        assert!(gap > -1e-3 && c.velocity.length() > 2., "{:?}", c.velocity);
    }
    #[test] fn rocks_block_the_player_but_a_jump_clears_the_top() {
        let props = [rock_at(0., 12. - 4.)];
        let (_, gap) = run_controller(&props, Vec2::new(0., 0.), Vec2::new(0., -1.), 3., 1. / 120., None);
        assert!(gap > -1e-3, "penetrated rock by {}", -gap);
        // Rock top is 1.4 m; the jump apex is 1.84 m, so feet can clear it only for part of the arc (still blocked while feet < 1.4 m).
        let bl = blockers_for(&props);
        assert!(bl.iter().all(|b| b.keep_out(1.5).is_none()), "feet above the 1.4 m rock are not blocked");
        assert!(bl[0].keep_out(0.).unwrap() > 1.1);
    }
    #[test] fn canopies_do_not_block_the_player() {
        let props = [tree_at(0., 12. - 4.)];
        assert!(blockers_for(&props).len() == 1, "only the trunk blocks the player");
        // Walk under the canopy 1 m from the trunk axis: free.
        let (c, _) = run_controller(&props, Vec2::new(1.0, 0.), Vec2::new(0., -1.), 2., 1. / 120., None);
        assert!(c.position.y < -4. - 1., "walked under the canopy");
    }
    #[test] fn random_walks_never_penetrate_any_prop() {
        let props = props_cluster().into_iter().map(|mut p| { p.pos.z += 12.; p }).collect::<Vec<_>>();
        let mut rng = Rng(0xBEEF_1234);
        for case in 0..60 {
            let mut c = Controller::default(); c.position = Vec2::new(rng.r(-8., 8.), rng.r(-10., 8.));
            c.blockers = blockers_for(&props);
            // Start outside every blocker.
            if c.blockers.iter().any(|b| b.keep_out(0.).map_or(false, |r| (c.position - b.centre).length() < r)) { continue; }
            let mut dir = Vec2::X;
            for step in 0..1500 {
                if step % 90 == 0 { let a = rng.r(0., std::f32::consts::TAU); dir = Vec2::new(a.cos(), a.sin()); }
                let dt = [1. / 60., 1. / 144., 0.03][step % 3];
                c.step(dir, step % 250 == 0, step % 2 == 0, dt);
                assert!(capsule_gap(&c) >= -1e-4, "case {case} step {step}: capsule inside a blocker by {} m", -capsule_gap(&c));
            }
        }
    }
}

#[cfg(test)]
mod hull_tests {
    use super::*;
    use fn_combat::{Hull, HullSet, Resource};
    use std::sync::Arc;
    fn boxp(pos: Vec3, half: Vec3, h: f32) -> Harvestable {
        let mut v = vec![];
        for x in [-half.x, half.x] { for z in [-half.z, half.z] { for y in [0., h] { v.push(pos + Vec3::new(x, y, z)); } } }
        Harvestable::real(Kind::Tree, Resource::Wood, pos, 0., 1., 600., 0, Some(Arc::new(HullSet::new(vec![Hull::from_points(&v).unwrap()]))))
    }
    #[test] fn a_hull_prop_collides_through_its_hulls_not_through_placeholder_frusta() {
        let p = boxp(Vec3::new(0., 0., 4.), Vec3::new(0.5, 0., 0.5), 3.);
        assert!(solids(&p).is_empty() && player_blockers(&[p.clone()], 12.).is_empty(), "no frusta for a real prop");
        // camera probe: a sphere sweep along z at height 1 from z = 0 toward the box front face z = 3.5
        let t = trace(&[p.clone()], Vec3::new(0., 1., 0.), Vec3::new(0., 1., 8.), 0.08).unwrap();
        assert!((t * 8. - (3.5 - 0.08)).abs() < 5e-3, "{}", t * 8.);
        assert!(trace(&[p.clone()], Vec3::new(2., 1., 0.), Vec3::new(2., 1., 8.), 0.08).is_none(), "1.5 m beside the 0.5 m half width");
        assert!((clearance(&[p.clone()], Vec3::new(2., 1., 4.), true) - 1.5).abs() < 1e-4);
        // a destroyed prop is gone for all of them
        let mut d = p; d.health = 0.;
        assert!(trace(&[d.clone()], Vec3::new(0., 1., 0.), Vec3::new(0., 1., 8.), 0.08).is_none() && clearance(&[d.clone()], Vec3::new(0., 1., 4.), true) == f32::MAX && player_hulls(&[d], 12., Vec2::ZERO, 50.).is_empty());
    }
    #[test] fn player_hulls_are_in_controller_space_and_only_the_near_ones() {
        let near = boxp(Vec3::new(0., 0., 8.), Vec3::new(0.5, 0., 0.5), 3.); // world z 8 = controller y -4
        let far = boxp(Vec3::new(100., 0., 8.), Vec3::new(0.5, 0., 0.5), 3.);
        let hs = player_hulls(&[near.clone(), far], 12., Vec2::ZERO, 6.);
        assert_eq!(hs.len(), 1, "the far prop is out of range");
        let h = &hs[0];
        // controller space: (x, world z - 12, up)
        assert!((h.center - Vec3::new(0., -4., 1.5)).length() < 1e-4, "{:?}", h.center);
        assert!((h.max.y - (-3.5)).abs() < 1e-4 && (h.max.z - 3.).abs() < 1e-4, "{:?}", h.max);
        // the movement scene (cm) agrees with the camera world: the controller at y = -3.5 + 0.42 (pawn capsule radius, AFortPawn ctor @1407c9ff0) touches the +y face of the box (world z 7.5..8.5 = controller y -4.5..-3.5)
        let sc = fn_movement::scene::Scene { solids: vec![], hulls: fn_movement::scene::Scene::hulls_to_cm(&hs), terrain: None };
        let cap = fn_movement::scene::Capsule { c: Vec3::new(0., -3.5 * 100. + fn_movement::params::CAPSULE_RADIUS, 100.), r: fn_movement::params::CAPSULE_RADIUS, hh: 75. };
        assert!(sc.probe(&cap).gap.abs() < 0.2, "{}", sc.probe(&cap).gap);
        set_disabled(true); assert!(player_hulls(&[near.clone()], 12., Vec2::ZERO, 6.).is_empty() && trace(&[near], Vec3::new(0., 1., 0.), Vec3::new(0., 1., 8.), 0.08).is_none()); set_disabled(false);
    }
}

#[cfg(test)]
mod consts_tests {
    #[test] fn player_height_is_twice_the_camera_stand_half_height() { assert!((super::PLAYER_HEIGHT - 2. * fn_camera::STAND_HALF_HEIGHT).abs() < 1e-6); }
}

