//! Collision scene for the character capsule (UE units: centimetres, X/Y horizontal, Z up).
//!
//! The world the capsule collides with is (a) the infinite baseplate, the plane Z = 0, and (b) the solids of revolution that
//! propcol.rs generates for trunks and rocks (`propcol::Blocker`, converted from controller metres to centimetres). Canopies
//! are camera-only and are not in this scene.
//!
//! This module plays the role of UE's `UWorld::SweepSingleByChannel / LineTraceSingleByChannel / OverlapBlockingTestByChannel`
//! for one vertical capsule. Everything here is geometry; the movement algorithms live in movement_cmc.rs.
//!
//! Method: the signed gap between a capsule and a convex solid of revolution is evaluated exactly (the signed distance of a
//! convex set is convex along the capsule's vertical axis, so the nearest axis point is found by ternary search; the distance
//! in the (radius, height) half-plane to the profile polygon is exact). A sweep is sphere-traced (the gap is 1-Lipschitz in the
//! capsule position, so stepping by gap - SKIN never skips a surface) and bisected to the contact. No tunnelling is possible at
//! any step size (verified in movement.rs tests at 0.125 s steps and 40 m/s).
//!
//! UE mapping notes (ASSUMED, no collision data exists for these placeholder props): `Hit::normal` is the capsule-surface
//! normal (axis point minus impact point, like UE's capsule sweeps) and `Hit::impact_normal` is the face normal of the nearest
//! profile edge (the two differ at rims, exactly where UE's ShouldCheckForValidLandingSpot looks).
use glam::{Vec2, Vec3};

/// Contact skin (cm): a swept capsule stops this far from a surface (PhysX-like contact offset; ASSUMED 0.1 cm).
pub const SKIN: f32 = 0.1;
const MOVING_IN: f32 = -1e-3;

/// UE `FHitResult` subset used by the movement component.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub blocking: bool,
    pub start_penetrating: bool,
    /// Fraction of the requested move completed before the hit.
    pub time: f32,
    pub depth: f32,
    /// Capsule centre at the contact.
    pub location: Vec3,
    pub impact_point: Vec3,
    pub normal: Vec3,
    pub impact_normal: Vec3,
    pub trace_start: Vec3,
    pub trace_end: Vec3,
}
impl Default for Hit { fn default() -> Self { Hit::none() } }
impl Hit {
    pub fn none() -> Self {
        Hit { blocking: false, start_penetrating: false, time: 1., depth: 0., location: Vec3::ZERO, impact_point: Vec3::ZERO, normal: Vec3::Z, impact_normal: Vec3::Z, trace_start: Vec3::ZERO, trace_end: Vec3::ZERO }
    }
    /// UE `IsValidBlockingHit`.
    pub fn valid_blocking(&self) -> bool { self.blocking && !self.start_penetrating }
}

/// Vertical capsule: centre, radius, half height (hemispheres included).
#[derive(Clone, Copy, Debug)]
pub struct Capsule { pub c: Vec3, pub r: f32, pub hh: f32 }
impl Capsule {
    /// Z range of the segment joining the two hemisphere centres.
    pub fn seg(&self) -> (f32, f32) { (self.c.z - (self.hh - self.r).max(0.), self.c.z + (self.hh - self.r).max(0.)) }
    pub fn at(&self, c: Vec3) -> Self { Capsule { c, ..*self } }
}

/// Convex frustum of revolution about the vertical axis through `c`: heights z0..z1, radii r0 -> r1 (propcol::Blocker in cm).
#[derive(Clone, Copy, Debug)]
pub struct Frustum { pub c: Vec2, pub z0: f32, pub z1: f32, pub r0: f32, pub r1: f32 }

struct Feature { dist: f32, point: Vec3, face_n: Vec3 }

fn seg_closest(p: Vec2, a: Vec2, b: Vec2) -> (f32, Vec2) {
    let ab = b - a; let l2 = ab.length_squared();
    let t = if l2 > 0. { ((p - a).dot(ab) / l2).clamp(0., 1.) } else { 0. };
    let q = a + ab * t; ((p - q).length(), q)
}

impl Frustum {
    pub fn max_r(&self) -> f32 { self.r0.max(self.r1) }
    /// Signed distance (negative inside) from `p`, with the nearest surface point and that face's outward normal.
    /// When the nearest point is shared by two faces (a rim or edge), `dir` (the sweep direction) picks the face whose normal opposes it most -
    /// UE's `FindGeomOpNormal` rule for `FHitResult::ImpactNormal` (a capsule dropped onto a rim reports the walkable top face, not the side).
    fn closest(&self, p: Vec3, dir: Vec3) -> Feature {
        let d = Vec2::new(p.x - self.c.x, p.y - self.c.y); let rho = d.length();
        let u = if rho > 1e-6 { d / rho } else { Vec2::X };
        let q = Vec2::new(rho, p.z);
        let (a, b, c, e) = (Vec2::new(0., self.z0), Vec2::new(self.r0, self.z0), Vec2::new(self.r1, self.z1), Vec2::new(0., self.z1));
        let side = Vec2::new(self.z1 - self.z0, -(self.r1 - self.r0)).normalize_or_zero();
        let side = if side == Vec2::ZERO { Vec2::X } else { side };
        let edges = [(a, b, Vec2::new(0., -1.)), (b, c, side), (c, e, Vec2::new(0., 1.))];
        let mut best = (f32::MAX, Vec2::ZERO, Vec2::X);
        let cand: [(f32, Vec2, Vec2); 3] = edges.map(|(s, t, n)| { let (dist, pt) = seg_closest(q, s, t); (dist, pt, n) });
        for &(dist, pt, n) in &cand { if dist < best.0 { best = (dist, pt, n); } }
        if dir != Vec3::ZERO {
            let opp = |n: Vec2| u.x * n.x * dir.x + u.y * n.x * dir.y + n.y * dir.z;
            for &(dist, pt, n) in &cand { if dist <= best.0 + 1e-3 && opp(n) < opp(best.2) - 1e-6 { best = (best.0, pt, n); } }
        }
        let inside = p.z >= self.z0 && p.z <= self.z1 && rho <= self.r0 + (self.r1 - self.r0) * (p.z - self.z0) / (self.z1 - self.z0).max(1e-6);
        let to3 = |v: Vec2, radial: bool| -> Vec3 { if radial { Vec3::new(u.x * v.x, u.y * v.x, v.y) } else { Vec3::new(self.c.x + u.x * v.x, self.c.y + u.y * v.x, v.y) } };
        Feature { dist: if inside { -best.0 } else { best.0 }, point: to3(best.1, false), face_n: to3(best.2, true) }
    }
}

/// Smallest gap (cm, negative = penetration) between the capsule and the scene, with the contact data.
pub struct Probe { pub gap: f32, pub normal: Vec3, pub point: Vec3, pub face_n: Vec3 }

#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub solids: Vec<Frustum>,
    /// Convex hulls (the cooked 1.8 collision convexes of the real props) in the same space as `solids`: centimetres, Z up. `Scene::hulls_to_cm` converts
    /// controller-space metres. Everything that sweeps / probes / casts down treats them like another solid.
    pub hulls: Vec<fn_hull::Hull>,
}

impl Scene {
    /// Converts propcol blockers (controller metres: centre = (x, y), spans y_lo..y_hi) to UE centimetres.
    pub fn from_blockers(b: &[Blocker]) -> Scene {
        Scene { solids: b.iter().map(|k| Frustum { c: k.centre * 100., z0: k.y_lo * 100., z1: k.y_hi * 100., r0: k.r_lo * 100., r1: k.r_hi * 100. }).collect(), hulls: vec![] }
    }
    /// Hulls given in controller metres -> scene centimetres.
    pub fn hulls_to_cm(h: &[fn_hull::Hull]) -> Vec<fn_hull::Hull> { h.iter().map(|k| k.mapped(|v| v * 100., false)).collect() }

    pub fn probe(&self, cap: &Capsule) -> Probe { self.probe_dir(cap, Vec3::ZERO) }
    pub fn probe_dir(&self, cap: &Capsule, dir: Vec3) -> Probe {
        // Baseplate: the plane Z = 0, walkable, normal +Z.
        let mut best = Probe { gap: cap.c.z - cap.hh, normal: Vec3::Z, point: Vec3::new(cap.c.x, cap.c.y, 0.), face_n: Vec3::Z };
        let (zl, zh) = cap.seg();
        for f in &self.solids {
            // Conservative lower bound of the gap from the bounding cylinders: skip solids that cannot beat the best so far.
            let hsep = Vec2::new(cap.c.x - f.c.x, cap.c.y - f.c.y).length() - f.max_r() - cap.r;
            let vsep = (f.z0 - (zh + cap.r)).max((zl - cap.r) - f.z1);
            if hsep.max(vsep) >= best.gap { continue; }
            let sd = |z: f32| f.closest(Vec3::new(cap.c.x, cap.c.y, z), Vec3::ZERO).dist;
            let (mut a, mut b) = (zl, zh);
            for _ in 0..28 { let m1 = a + (b - a) / 3.; let m2 = b - (b - a) / 3.; if sd(m1) < sd(m2) { b = m2 } else { a = m1 } }
            let z = 0.5 * (a + b); let p = Vec3::new(cap.c.x, cap.c.y, z);
            let ft = f.closest(p, dir); let gap = ft.dist - cap.r;
            if gap < best.gap {
                let normal = if ft.dist > 1e-4 { (p - ft.point).normalize_or_zero() } else { ft.face_n };
                let normal = if normal == Vec3::ZERO { ft.face_n } else { normal };
                best = Probe { gap, normal, point: ft.point, face_n: ft.face_n };
            }
        }
        for h in &self.hulls {
            // lower bound of the gap: the capsule axis segment to the hull's bounding sphere, and the capsule's box to the hull's box (canopy discs far above the head
            // are rejected by the box, which a sphere around a wide, flat hull cannot do)
            let cz = h.center.z.clamp(zl, zh);
            let lb = (Vec3::new(cap.c.x, cap.c.y, cz) - h.center).length() - h.radius - cap.r;
            if lb >= best.gap { continue; }
            if h.aabb_gap(Vec3::new(cap.c.x, cap.c.y, zl), Vec3::new(cap.c.x, cap.c.y, zh)) - cap.r >= best.gap { continue; }
            // nearest point of the capsule axis: exact (endpoints + edges) when the axis is clear of the hull; when it overlaps, the signed distance of a convex
            // set is convex along the (vertical) axis, so a ternary search finds the deepest point
            let (pa, pb) = (Vec3::new(cap.c.x, cap.c.y, zl), Vec3::new(cap.c.x, cap.c.y, zh));
            let zn = match h.segment_nearest_param(pa, pb) {
                Some(t) => zl + (zh - zl) * t,
                None => {
                    let sd = |z: f32| h.sdf(Vec3::new(cap.c.x, cap.c.y, z));
                    let (mut a, mut b) = (zl, zh);
                    for _ in 0..22 { let m1 = a + (b - a) / 3.; let m2 = b - (b - a) / 3.; if sd(m1) < sd(m2) { b = m2 } else { a = m1 } }
                    0.5 * (a + b)
                }
            };
            let p = Vec3::new(cap.c.x, cap.c.y, zn);
            let ft = h.closest(p, dir); let gap = ft.dist - cap.r;
            if gap < best.gap {
                let normal = if ft.dist > 1e-4 { (p - ft.point).normalize_or_zero() } else { ft.normal };
                let normal = if normal == Vec3::ZERO { ft.normal } else { normal };
                best = Probe { gap, normal, point: ft.point, face_n: ft.normal };
            }
        }
        best
    }

    /// UE `OverlapBlockingTestByChannel`.
    pub fn overlaps(&self, cap: &Capsule) -> bool { self.probe(cap).gap < 0. }

    /// UE `SweepSingleByChannel` for the capsule translated by `delta`.
    pub fn sweep(&self, cap: Capsule, delta: Vec3) -> Hit {
        let len = delta.length();
        let mut out = Hit::none(); out.trace_start = cap.c; out.trace_end = cap.c + delta;
        if len < 1e-6 { return out; }
        let dir = delta / len;
        let p0 = self.probe_dir(&cap, dir);
        let mk = |t: f32, pr: &Probe, pen: bool| Hit { blocking: true, start_penetrating: pen, time: t / len, depth: if pen { -pr.gap } else { 0. }, location: cap.c + dir * t, impact_point: pr.point, normal: pr.normal, impact_normal: pr.face_n, trace_start: out.trace_start, trace_end: out.trace_end };
        if p0.gap < 0. {
            // Initially overlapping: a move that leaves the geometry is allowed, anything else reports a start-penetrating hit.
            if p0.normal.dot(dir) > 0. { return out; }
            return mk(0., &p0, true);
        }
        let (mut t, mut prev) = (0f32, 0f32);
        for _ in 0..400 {
            let pr = self.probe_dir(&cap.at(cap.c + dir * t), dir);
            if pr.gap <= SKIN && pr.normal.dot(dir) < MOVING_IN {
                if t <= 0. { return mk(0., &pr, false); }
                let (mut lo, mut hi) = (prev, t);
                for _ in 0..26 { let m = 0.5 * (lo + hi); if self.probe(&cap.at(cap.c + dir * m)).gap <= SKIN { hi = m } else { lo = m } }
                let pr_hi = self.probe_dir(&cap.at(cap.c + dir * hi), dir);
                return mk(lo, &pr_hi, false);
            }
            if t >= len { return out; }
            prev = t;
            let step = if pr.gap > SKIN { (pr.gap - SKIN).max(0.05) } else { 0.5 };
            t = (t + step).min(len);
        }
        out
    }

    /// UE `LineTraceSingleByChannel` straight down from `from`: (distance, impact point, surface normal).
    pub fn line_down(&self, from: Vec3, dist: f32) -> Option<(f32, Vec3, Vec3)> {
        let mut best: Option<(f32, Vec3, Vec3)> = None;
        let mut consider = |z: f32, n: Vec3, at: Vec3| {
            if from.z >= z && from.z - z <= dist && best.map_or(true, |b| from.z - z < b.0) { best = Some((from.z - z, at, n)); }
        };
        consider(0., Vec3::Z, Vec3::new(from.x, from.y, 0.));
        for f in &self.solids {
            let rho = Vec2::new(from.x - f.c.x, from.y - f.c.y).length();
            if rho > f.max_r() { continue; }
            if rho <= f.r1 { consider(f.z1, Vec3::Z, Vec3::new(from.x, from.y, f.z1)); }
            else if f.r0 > f.r1 && rho <= f.r0 {
                let z = f.z0 + (rho - f.r0) / (f.r1 - f.r0) * (f.z1 - f.z0);
                let s = Vec2::new(f.z1 - f.z0, -(f.r1 - f.r0)).normalize_or_zero();
                let u = if rho > 1e-6 { Vec2::new(from.x - f.c.x, from.y - f.c.y) / rho } else { Vec2::X };
                consider(z, Vec3::new(u.x * s.x, u.y * s.x, s.y), Vec3::new(from.x, from.y, z));
            }
        }
        for h in &self.hulls {
            if (Vec2::new(from.x, from.y) - Vec2::new(h.center.x, h.center.y)).length() > h.radius { continue; }
            if let Some((t, n)) = h.ray(from, Vec3::NEG_Z, dist) { consider(from.z - t, n, Vec3::new(from.x, from.y, from.z - t)); }
        }
        best
    }
}

// ---- Player-vs-prop blocker (moved here from the old propcol.rs so the movement crate does not depend on the prop crate) ----
/// ASSUMED: UE ACharacter default capsule radius (34 cm), metres.
pub const PLAYER_RADIUS: f32 = 0.34;
/// Serialized: PlayerPawn_Athena_Generic.CollisionCylinder.CapsuleHalfHeight = 75 cm, metres (fn-props has a test that this equals
/// `2 * fn_camera::STAND_HALF_HEIGHT`, the value the old propcol.rs derived it from).
pub const PLAYER_HEIGHT: f32 = 2. * 0.75;

/// Player-vs-prop blocker in controller space (controller position y = world z - `z_offset`): a frustum of revolution about a vertical axis.
#[derive(Clone, Copy, Debug)]
pub struct Blocker { pub centre: Vec2, pub y_lo: f32, pub y_hi: f32, pub r_lo: f32, pub r_hi: f32 }

#[allow(dead_code)] // the player collides through `Scene` (capsule sweeps); keep_out/resolve remain as the conservative cylinder reference
impl Blocker {
    /// Radius (player radius included) the capsule centre must keep from the axis for a body spanning `feet..feet+PLAYER_HEIGHT`;
    /// `None` when the capsule is entirely above/below this solid.
    pub fn keep_out(&self, feet: f32) -> Option<f32> {
        let (lo, hi) = (feet.max(self.y_lo), (feet + PLAYER_HEIGHT).min(self.y_hi));
        if lo > hi { return None; }
        let at = |y: f32| { let t = ((y - self.y_lo) / (self.y_hi - self.y_lo).max(1e-6)).clamp(0., 1.); self.r_lo + (self.r_hi - self.r_lo) * t };
        Some(at(lo).max(at(hi)) + PLAYER_RADIUS)
    }
    /// Pushes `pos` out of the blocker and removes the velocity component into it (slide along the surface).
    pub fn resolve(&self, pos: &mut Vec2, vel: &mut Vec2, feet: f32) {
        let Some(r) = self.keep_out(feet) else { return };
        let d = *pos - self.centre; let len = d.length();
        if len >= r { return; }
        let n = if len > 1e-5 { d / len } else if vel.length_squared() > 1e-8 { -vel.normalize() } else { Vec2::X };
        *pos = self.centre + n * r;
        let into = vel.dot(n);
        if into < 0. { *vel -= n * into; }
    }
}


#[cfg(test)]
mod hull_tests {
    use super::*;
    use fn_hull::Hull;
    /// Axis-aligned box hull (cm, Z up): `lo..hi`.
    fn boxh(lo: Vec3, hi: Vec3) -> Hull {
        let mut v = vec![];
        for x in [lo.x, hi.x] { for y in [lo.y, hi.y] { for z in [lo.z, hi.z] { v.push(Vec3::new(x, y, z)); } } }
        Hull::from_points(&v).unwrap()
    }
    fn scene() -> Scene { Scene { solids: vec![], hulls: vec![boxh(Vec3::new(-100., -100., 150.), Vec3::new(100., 100., 450.))] } }
    fn cap(x: f32, y: f32, z: f32) -> Capsule { Capsule { c: Vec3::new(x, y, z), r: 34., hh: 75. } }

    #[test] fn the_capsule_gap_to_a_hull_box_is_the_analytic_one() {
        let s = scene();
        // beside the box: 300 - 100 - 34
        assert!((s.probe(&cap(300., 0., 300.)).gap - 166.).abs() < 0.05);
        // diagonal in front of an edge: distance to the vertical edge (100, 100) minus the radius
        let g = s.probe(&cap(200., 200., 300.)).gap; assert!((g - (2f32.sqrt() * 100. - 34.)).abs() < 0.05, "{g}");
        // above the box top (z = 450): the lowest point of the capsule is c.z - hh
        let g = s.probe(&cap(0., 0., 700.)).gap; assert!((g - (700. - 75. - 450.)).abs() < 0.05, "{g}");
        // the capsule is cleared from the plane too (baseplate z = 0): a capsule inside the box overlaps
        assert!(s.overlaps(&cap(0., 0., 300.)) && !s.overlaps(&cap(300., 0., 300.)));
        // an inclined approach reports the face normal of the nearest face
        let p = s.probe(&cap(300., 0., 300.)); assert!(p.normal.x > 0.99 && p.face_n.x > 0.99, "{:?} {:?}", p.normal, p.face_n);
    }
    #[test] fn a_sweep_stops_at_the_hull_and_a_clear_sweep_does_not() {
        let s = scene();
        let h = s.sweep(cap(400., 0., 150.), Vec3::new(-300., 0., 0.));
        assert!(h.blocking && !h.start_penetrating, "{h:?}");
        assert!((h.time * 300. - (400. - 100. - 34.)).abs() < 0.3, "stops {} cm along, wanted 266", h.time * 300.);
        assert!(h.normal.x > 0.99);
        // below the box (z of the capsule axis < 50 - ...): walking under the 50 cm gap is blocked by the capsule height, 2 m beside is clear
        assert!(!s.sweep(cap(400., 300., 150.), Vec3::new(-800., 0., 0.)).blocking);
    }
    #[test] fn a_floor_probe_lands_on_the_top_of_a_hull_and_misses_beside_it() {
        let s = scene();
        let (d, p, n) = s.line_down(Vec3::new(20., -30., 600.), 700.).unwrap();
        assert!((d - 150.).abs() < 1e-3 && (p.z - 450.).abs() < 1e-3 && n.z > 0.99, "{d} {p:?} {n:?}");
        let (d, _, _) = s.line_down(Vec3::new(150., 0., 600.), 700.).unwrap(); assert!((d - 600.).abs() < 1e-3, "beside the box the baseplate is the floor");
        assert!(s.line_down(Vec3::new(20., 0., 600.), 100.).is_none(), "too far");
    }
    #[test] fn the_player_controller_stops_at_a_hull_wall_slides_along_it_and_can_stand_on_a_low_one() {
        use crate::controller::Controller;
        // a 3 m tall wall of a hull, 2 m wide, at controller y = -4 (metres): its +y face is at -3
        let wall = boxh(Vec3::new(-1., -5., 0.), Vec3::new(1., -3., 3.));
        let mut c = Controller::default(); c.hulls = vec![wall.clone()];
        for _ in 0..240 { c.step(Vec2::new(0., -1.), false, true, 1. / 60.); }
        assert!((c.position.y - (-3. + PLAYER_RADIUS)).abs() < 0.03, "stands {} m from the face at y = -3", c.position.y);
        assert!(c.velocity.length() < 0.3);
        // diagonal against the wall: keeps sliding sideways, never inside
        let mut c = Controller::default(); c.hulls = vec![wall]; c.position = Vec2::new(-0.5, -1.);
        let mut min_gap = f32::MAX;
        for _ in 0..300 { c.step(Vec2::new(0.5, -1.).normalize(), false, true, 1. / 60.); let sc = Scene { solids: vec![], hulls: Scene::hulls_to_cm(&c.hulls) }; min_gap = min_gap.min(sc.probe(&c.component().capsule()).gap); }
        assert!(min_gap > -0.2, "penetrated {min_gap} cm");
        assert!(c.position.x > 1.2, "slid past the 1 m half width: {:?}", c.position);
        // a 30 cm tall hull block is stepped onto (max step height 45 cm): the floor under the feet becomes its top
        let step = boxh(Vec3::new(-1., -5., 0.), Vec3::new(1., -3., 0.3));
        let mut c = Controller::default(); c.hulls = vec![step];
        let mut top = 0f32; let mut on_it = false;
        for _ in 0..120 { c.step(Vec2::new(0., -1.), false, false, 1. / 60.); top = top.max(c.height); on_it |= c.position.y < -3.5 && c.position.y > -4.5 && (c.height - 0.3).abs() < 0.02; }
        assert!((top - 0.3).abs() < 0.02 && on_it, "walked up onto the block: highest floor {top}");
    }
}
