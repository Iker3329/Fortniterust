//! PERF (2026-10-08): the prop index (fn_props::index) must give exactly the linear scans' results: same sets, same order.
//! Own test binary: the lib tests flip the global `set_disabled` switch, which would race these comparisons.
use fn_combat::{Harvestable, Hull, HullSet, Kind, Resource};
use fn_props::{player_blockers, player_blockers_in, player_hulls, player_hulls_in, trace, trace_ids, PropIndex};
use glam::{Vec2, Vec3};
use std::sync::Arc;

struct Rng(u32);
impl Rng {
    fn f(&mut self) -> f32 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 17; self.0 ^= self.0 << 5; (self.0 >> 8) as f32 / (1u32 << 24) as f32 }
    fn r(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }
}
/// A prop of 1-3 box hulls around `pos` (half sizes up to `size`).
fn hull_prop(rng: &mut Rng, pos: Vec3, size: f32) -> Harvestable {
    let mut hulls = vec![];
    for _ in 0..1 + (rng.f() * 3.) as usize {
        let c = pos + Vec3::new(rng.r(-size, size), 0., rng.r(-size, size));
        let h = Vec3::new(rng.r(0.2, size), rng.r(0.5, 6.), rng.r(0.2, size));
        let mut v = vec![];
        for x in [-h.x, h.x] { for y in [0., h.y] { for z in [-h.z, h.z] { v.push(c + Vec3::new(x, y, z)); } } }
        hulls.push(Hull::from_points(&v).unwrap());
    }
    Harvestable::real(Kind::Tree, Resource::Wood, pos, 0., 1., 600., 0, Some(Arc::new(HullSet::new(hulls))))
}
/// ~4000 props over 1.2 km: mostly small hull props, a few huge ones (landscape-sized pieces), some placeholders, ~10 % destroyed.
fn world(seed: u32) -> Vec<Harvestable> {
    let mut rng = Rng(seed);
    let mut v = vec![];
    for i in 0..4000 {
        let pos = Vec3::new(rng.r(-600., 600.), rng.r(-5., 30.), rng.r(-600., 600.));
        let mut p = if i % 97 == 0 { Harvestable::new(if i % 2 == 0 { Kind::Tree } else { Kind::Rock }, pos) }
            else if i % 501 == 0 { hull_prop(&mut rng, pos, 300.) }
            else { let s = rng.r(0.5, 12.); hull_prop(&mut rng, pos, s) };
        if rng.f() < 0.1 { p.health = 0.; }
        v.push(p);
    }
    v
}
fn camera_ids_linear(props: &[Harvestable], f: Vec3, m: f32) -> Vec<u32> {
    (0..props.len() as u32).filter(|&i| props[i as usize].hull.as_ref().map_or(true, |h| (h.center - f).length() - h.radius <= m)).collect()
}
fn camera_ids_indexed(props: &[Harvestable], ix: &PropIndex, f: Vec3, m: f32) -> Vec<u32> {
    let mut ids = vec![];
    ix.hull_candidates(Vec2::new(f.x, f.z), m, &mut ids);
    ids.retain(|&i| props[i as usize].hull.as_ref().map_or(true, |h| (h.center - f).length() - h.radius <= m));
    ids.extend_from_slice(ix.plain());
    ids.sort_unstable();
    ids
}

#[test]
fn indexed_queries_return_exactly_the_linear_scans() {
    for seed in [0x1234_5678, 0x0BAD_F00D] {
        let props = world(seed);
        let ix = PropIndex::build(&props);
        assert!(ix.matches(&props));
        let mut rng = Rng(seed ^ 0x5555);
        let (mut nonempty, mut cam_total) = (0, 0);
        for _ in 0..3000 {
            // query points inside, at the border of and outside the populated area (controller space: y = world z - 12)
            let around = Vec2::new(rng.r(-700., 700.), rng.r(-700., 700.));
            let range = [0., 6., 6., 40., rng.r(0., 80.)][(rng.f() * 5.) as usize];
            let lin = player_hulls(&props, 12., around, range);
            let ind = player_hulls_in(&props, Some(&ix), 12., around, range);
            assert_eq!(format!("{lin:?}"), format!("{ind:?}"), "player_hulls at {around:?} range {range}");
            if !lin.is_empty() { nonempty += 1; }
            // the camera's prop set and its sphere sweeps
            let feet = Vec3::new(around.x, rng.r(-5., 30.), around.y + 12.);
            let (cl, ci) = (camera_ids_linear(&props, feet, 40.), camera_ids_indexed(&props, &ix, feet, 40.));
            assert_eq!(cl, ci, "camera props at {feet:?}");
            cam_total += cl.len();
            let (a, b) = (feet + Vec3::Y * 1.5, feet + Vec3::new(rng.r(-4., 4.), rng.r(-1., 3.), rng.r(-4., 4.)));
            assert_eq!(trace(&props, a, b, 0.08).map(f32::to_bits), trace_ids(&props, &ci, a, b, 0.08).map(f32::to_bits), "trace {a:?} -> {b:?}");
            // floor below: every hull prop whose bounding circle contains the point is a candidate
            let w = Vec2::new(feet.x, feet.z);
            let mut cand = vec![];
            ix.hull_candidates(w, 0., &mut cand);
            for (i, p) in props.iter().enumerate() {
                if let Some(h) = &p.hull { if (Vec2::new(h.center.x, h.center.z) - w).length() <= h.radius { assert!(cand.binary_search(&(i as u32)).is_ok(), "floor candidate {i} missing at {w:?}"); } }
            }
            assert!(cand.windows(2).all(|p| p[0] < p[1]), "candidates ascending, no duplicates");
        }
        assert!(nonempty > 100 && cam_total > 3000, "the queries must hit props ({nonempty}, {cam_total})");
        assert_eq!(format!("{:?}", player_blockers(&props, 12.)), format!("{:?}", player_blockers_in(&props, Some(&ix), 12.)));
    }
}

#[test]
fn the_index_notices_a_changed_props_list() {
    let props = world(7);
    let ix = PropIndex::build(&props);
    let mut other = props.clone();
    assert!(!ix.matches(&other), "another buffer");
    let ix2 = PropIndex::build(&other);
    // same buffer, same length, one prop replaced in place (the scripted `clear` + `push` of a single prop)
    let mut one = vec![props[3].clone()];
    let ix1 = PropIndex::build(&one);
    one.clear(); one.push(props[4].clone());
    assert!(!ix1.matches(&one), "a replaced prop in a short list");
    // destroying a prop keeps the index valid (queries test alive())
    other[10].health = 0.;
    assert!(ix2.matches(&other));
    assert!(PropIndex::build(&[]).matches(&[]));
}

#[test]
fn empty_and_placeholder_only_worlds() {
    let props = vec![Harvestable::new(Kind::Tree, Vec3::new(1., 0., 2.)), Harvestable::new(Kind::Rock, Vec3::new(-3., 0., 5.))];
    let ix = PropIndex::build(&props);
    assert_eq!(ix.plain(), &[0, 1]);
    let mut c = vec![9];
    ix.hull_candidates(Vec2::ZERO, 40., &mut c);
    assert!(c.is_empty());
    assert_eq!(format!("{:?}", player_blockers(&props, 12.)), format!("{:?}", player_blockers_in(&props, Some(&ix), 12.)));
}

/// PERF (2026-10-08): the hull cache hands out exactly what `mapped` builds, across queries that keep, add and drop props.
#[test]
fn cached_player_hulls_equal_fresh_ones() {
    let props = world(99);
    let ix = PropIndex::build(&props);
    let mut cache = fn_props::HullMapCache::default();
    let mut rng = Rng(4242);
    let mut at = Vec2::new(0., 0.);
    for i in 0..2000 {
        // a walk with occasional jumps: consecutive queries share most props
        at += Vec2::new(rng.r(-1., 1.), rng.r(-1., 1.));
        if i % 200 == 0 { at = Vec2::new(rng.r(-600., 600.), rng.r(-600., 600.)); }
        let fresh = player_hulls(&props, 12., at, 6.);
        let cached = fn_props::player_hulls_cached(&props, Some(&ix), 12., at, 6., Some(&mut cache));
        assert_eq!(format!("{fresh:?}"), format!("{cached:?}"), "query {i} at {at:?}");
        let lin_cached = fn_props::player_hulls_cached(&props, None, 12., at, 6., Some(&mut cache));
        assert_eq!(format!("{fresh:?}"), format!("{lin_cached:?}"));
    }
}

#[test]
fn segment_traces_match_the_linear_scan() {
    for seed in [7u32, 99, 4242] {
        let props = world(seed);
        let ix = PropIndex::build(&props);
        let mut rng = Rng(seed ^ 0x5bd1e995);
        for n in 0..1500 {
            let a = Vec3::new(rng.r(-650., 650.), rng.r(-5., 40.), rng.r(-650., 650.));
            let len = if n % 3 == 0 { rng.r(0.5, 4.) } else { rng.r(4., 90.) };
            let dir = Vec3::new(rng.r(-1., 1.), rng.r(-0.4, 0.4), rng.r(-1., 1.)).normalize_or(Vec3::X);
            let b = a + dir * len;
            let r = [0., 0.08, 0.15, 0.4][n % 4];
            assert_eq!(fn_props::trace_in(&props, Some(&ix), a, b, r), trace(&props, a, b, r), "seed {seed} segment {n}");
            let ids = fn_props::segment_ids(Some(&ix), a, b, r).unwrap();
            assert_eq!(fn_combat::trace_cylinders_ids(&props, &ids, a, dir, len, r), fn_combat::trace_cylinders(&props, a, dir, len, r), "seed {seed} cylinders {n}");
        }
    }
}
