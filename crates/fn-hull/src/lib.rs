//! `fn-hull`: convex hulls rebuilt from point sets.
//!
//! The cooked 1.8 collision data (`fn-propdata` `Shape::Convex`) stores the hull VERTICES only; the faces live in the PhysX blob, which is not decoded.
//! This crate rebuilds the polytope (incremental 3D hull, exact for the 8-18 point hulls the engine bakes) and offers what the game needs:
//!
//! * [`Hull::sdf`] signed distance (negative inside), [`Hull::closest`] nearest surface point + face normal,
//! * [`Hull::ray`] entering ray hit (player standing on a rock, floor probes),
//! * [`HullSet::trace_sphere`] sphere sweep (camera boom, pickaxe trace) by sphere tracing on the 1-Lipschitz distance,
//! * [`Hull::mapped`] / [`HullSet::mapped`] rigid / affine re-expression (axis swaps are reflections: `flip` re-winds the faces).
//!
//! Units are whatever the caller uses (metres in the game, centimetres inside the movement scene); epsilons are relative to the hull size.
use glam::Vec3;

/// One convex polytope. `tris` wind counter-clockwise seen from OUTSIDE; `planes[i]` belongs to `tris[i]` (`n . p <= d` inside).
#[derive(Clone, Debug, Default)]
pub struct Hull {
    pub verts: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
    pub planes: Vec<(Vec3, f32)>,
    /// Unique undirected edges (vertex index pairs), for exact segment-vs-hull distances.
    pub edges: Vec<[u32; 2]>,
    /// Bounding sphere (centre of the AABB, radius to the farthest vertex) and AABB.
    pub center: Vec3,
    pub radius: f32,
    pub min: Vec3,
    pub max: Vec3,
}

/// Nearest surface feature of a hull: signed distance (negative inside), the surface point and the outward face normal.
#[derive(Clone, Copy, Debug)]
pub struct Closest {
    pub dist: f32,
    pub point: Vec3,
    pub normal: Vec3,
}

struct Face {
    v: [u32; 3],
    n: Vec3,
    d: f32,
    alive: bool,
}

fn make_face(pts: &[Vec3], a: u32, b: u32, c: u32, inside: Vec3) -> Option<Face> {
    let (pa, pb, pc) = (pts[a as usize], pts[b as usize], pts[c as usize]);
    let n = (pb - pa).cross(pc - pa).normalize_or_zero();
    if n == Vec3::ZERO {
        return None;
    }
    let d = n.dot(pa);
    if n.dot(inside) - d > 0. {
        Some(Face { v: [a, c, b], n: -n, d: -d, alive: true })
    } else {
        Some(Face { v: [a, b, c], n, d, alive: true })
    }
}

/// Closest points between the segments `p1..q1` and `p2..q2` (Ericson 5.1.9): (parameter on the first segment, distance). Both segments must have length.
pub fn segment_segment(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (f32, f32) {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.length_squared(), d2.length_squared(), d2.dot(r));
    let (s, t);
    if a <= 1e-12 && e <= 1e-12 {
        return (0., r.length());
    }
    if a <= 1e-12 {
        s = 0.;
        t = (f / e).clamp(0., 1.);
    } else {
        let c = d1.dot(r);
        if e <= 1e-12 {
            t = 0.;
            s = (-c / a).clamp(0., 1.);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > 1e-12 { ((b * f - c * e) / denom).clamp(0., 1.) } else { 0. };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0. {
                t0 = 0.;
                s0 = (-c / a).clamp(0., 1.);
            } else if t0 > 1. {
                t0 = 1.;
                s0 = ((b - c) / a).clamp(0., 1.);
            }
            s = s0;
            t = t0;
        }
    }
    ((s), ((p1 + d1 * s) - (p2 + d2 * t)).length())
}

/// Closest point on triangle `abc` to `p` (Ericson, Real-Time Collision Detection 5.1.5).
pub fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0. && d2 <= 0. {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0. && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0. && d1 >= 0. && d3 <= 0. {
        let v = d1 / (d1 - d3);
        return a + ab * v;
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0. && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0. && d2 >= 0. && d6 <= 0. {
        let w = d2 / (d2 - d6);
        return a + ac * w;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0. && (d4 - d3) >= 0. && (d5 - d6) >= 0. {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let denom = 1. / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

impl Hull {
    /// Convex hull of `input` (duplicates / interior points allowed). `None` when the points are fewer than 4, not finite, or (nearly) coplanar.
    pub fn from_points(input: &[Vec3]) -> Option<Hull> {
        let pts: Vec<Vec3> = input.iter().copied().filter(|p| p.is_finite()).collect();
        if pts.len() < 4 {
            return None;
        }
        let (mut lo, mut hi) = (pts[0], pts[0]);
        for p in &pts {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let scale = (hi - lo).max_element();
        if scale <= 0. {
            return None;
        }
        let eps = scale * 1e-5;
        // initial tetrahedron from extreme points
        let i0 = (0..pts.len()).min_by(|&a, &b| pts[a].x.total_cmp(&pts[b].x)).unwrap_or(0);
        let i1 = (0..pts.len()).max_by(|&a, &b| (pts[a] - pts[i0]).length_squared().total_cmp(&(pts[b] - pts[i0]).length_squared()))?;
        let e = pts[i1] - pts[i0];
        if e.length() <= eps {
            return None;
        }
        let i2 = (0..pts.len()).max_by(|&a, &b| e.cross(pts[a] - pts[i0]).length_squared().total_cmp(&e.cross(pts[b] - pts[i0]).length_squared()))?;
        let n = e.cross(pts[i2] - pts[i0]);
        if n.length() <= eps * e.length() {
            return None;
        }
        let nn = n.normalize();
        let i3 = (0..pts.len()).max_by(|&a, &b| nn.dot(pts[a] - pts[i0]).abs().total_cmp(&nn.dot(pts[b] - pts[i0]).abs()))?;
        if nn.dot(pts[i3] - pts[i0]).abs() <= eps {
            return None;
        }
        let inside = (pts[i0] + pts[i1] + pts[i2] + pts[i3]) * 0.25;
        let (a, b, c, d) = (i0 as u32, i1 as u32, i2 as u32, i3 as u32);
        let mut faces: Vec<Face> = [(a, b, c), (a, b, d), (a, c, d), (b, c, d)].iter().filter_map(|&(x, y, z)| make_face(&pts, x, y, z, inside)).collect();
        if faces.len() != 4 {
            return None;
        }
        for (i, p) in pts.iter().enumerate() {
            let i = i as u32;
            if i == a || i == b || i == c || i == d {
                continue;
            }
            let visible: Vec<usize> = (0..faces.len()).filter(|&f| faces[f].alive && faces[f].n.dot(*p) - faces[f].d > eps).collect();
            if visible.is_empty() {
                continue;
            }
            let mut edges: Vec<(u32, u32)> = vec![];
            for &f in &visible {
                let v = faces[f].v;
                edges.extend([(v[0], v[1]), (v[1], v[2]), (v[2], v[0])]);
            }
            let horizon: Vec<(u32, u32)> = edges.iter().copied().filter(|&(x, y)| !edges.contains(&(y, x))).collect();
            for &f in &visible {
                faces[f].alive = false;
            }
            for (x, y) in horizon {
                if let Some(f) = make_face(&pts, x, y, i, inside) {
                    faces.push(f);
                }
            }
            if faces.len() > 4096 {
                faces.retain(|f| f.alive);
            }
        }
        let tris: Vec<[u32; 3]> = faces.iter().filter(|f| f.alive).map(|f| f.v).collect();
        Some(Hull::finish(pts, tris))
    }

    /// Builds planes and bounds from vertices + triangles (windings are fixed to point outward from the vertex centroid).
    fn finish(verts: Vec<Vec3>, mut tris: Vec<[u32; 3]>) -> Hull {
        // only the vertices a face uses
        let mut used = vec![false; verts.len()];
        for t in &tris {
            for &i in t {
                used[i as usize] = true;
            }
        }
        let mut remap = vec![u32::MAX; verts.len()];
        let mut vs = vec![];
        for (i, v) in verts.iter().enumerate() {
            if used[i] {
                remap[i] = vs.len() as u32;
                vs.push(*v);
            }
        }
        for t in &mut tris {
            for i in t.iter_mut() {
                *i = remap[*i as usize];
            }
        }
        let centroid = if vs.is_empty() { Vec3::ZERO } else { vs.iter().copied().sum::<Vec3>() / vs.len() as f32 };
        let mut planes = Vec::with_capacity(tris.len());
        for t in &mut tris {
            let (a, b, c) = (vs[t[0] as usize], vs[t[1] as usize], vs[t[2] as usize]);
            let mut n = (b - a).cross(c - a).normalize_or_zero();
            if n.dot(centroid - a) > 0. {
                t.swap(1, 2);
                n = -n;
            }
            planes.push((n, n.dot(a)));
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for v in &vs {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        if vs.is_empty() {
            lo = Vec3::ZERO;
            hi = Vec3::ZERO;
        }
        let center = (lo + hi) * 0.5;
        let radius = vs.iter().map(|v| (*v - center).length()).fold(0., f32::max);
        let mut edges: Vec<[u32; 2]> = tris.iter().flat_map(|t| [[t[0].min(t[1]), t[0].max(t[1])], [t[1].min(t[2]), t[1].max(t[2])], [t[2].min(t[0]), t[2].max(t[0])]]).collect();
        edges.sort_unstable();
        edges.dedup();
        Hull { verts: vs, tris, planes, edges, center, radius, min: lo, max: hi }
    }

    /// The same hull with every vertex passed through `f` (must be affine). A reflection (axis swap, negative scale) needs `flip = true`.
    pub fn mapped(&self, f: impl Fn(Vec3) -> Vec3, flip: bool) -> Hull {
        let verts: Vec<Vec3> = self.verts.iter().map(|&v| f(v)).collect();
        let tris = self.tris.iter().map(|t| if flip { [t[0], t[2], t[1]] } else { *t }).collect();
        Hull::finish(verts, tris)
    }

    /// Lower bound of the distance from `p` to the hull: the distance to its AABB (0 inside the box).
    #[inline]
    pub fn aabb_dist(&self, p: Vec3) -> f32 {
        (self.min - p).max(p - self.max).max(Vec3::ZERO).length()
    }
    /// Lower bound of the distance between the AABB of the segment `a..b` and the hull's AABB (0 when they overlap).
    #[inline]
    pub fn aabb_gap(&self, lo: Vec3, hi: Vec3) -> f32 {
        (self.min - hi).max(lo - self.max).max(Vec3::ZERO).length()
    }
    /// Signed distance from `p` (negative inside, exact: the nearest plane inside, the nearest triangle outside).
    pub fn sdf(&self, p: Vec3) -> f32 {
        let mut m = f32::MIN;
        for (n, d) in &self.planes {
            m = m.max(n.dot(p) - d);
        }
        if m <= 0. {
            return m;
        }
        let mut best = f32::MAX;
        for t in &self.tris {
            let q = closest_on_triangle(p, self.verts[t[0] as usize], self.verts[t[1] as usize], self.verts[t[2] as usize]);
            best = best.min((p - q).length_squared());
        }
        best.sqrt()
    }

    /// Nearest surface feature. At rims (several faces within 1e-3 of the best distance) `dir` (the sweep direction) picks the face whose normal opposes it
    /// most; pass `Vec3::ZERO` to take the first.
    pub fn closest(&self, p: Vec3, dir: Vec3) -> Closest {
        let mut m = (f32::MIN, 0usize);
        for (i, (n, d)) in self.planes.iter().enumerate() {
            let v = n.dot(p) - d;
            if v > m.0 {
                m = (v, i);
            }
        }
        if m.0 <= 0. {
            let (n, _) = self.planes[m.1];
            return Closest { dist: m.0, point: p - n * m.0, normal: n };
        }
        let tri = |i: usize| {
            let t = &self.tris[i];
            let q = closest_on_triangle(p, self.verts[t[0] as usize], self.verts[t[1] as usize], self.verts[t[2] as usize]);
            ((p - q).length(), q)
        };
        let mut best = (f32::MAX, Vec3::ZERO, 0usize);
        for i in 0..self.tris.len() {
            let (d, q) = tri(i);
            if d < best.0 {
                best = (d, q, i);
            }
        }
        if dir != Vec3::ZERO {
            let tol = best.0 + 1e-3 * (1. + self.radius * 1e-2);
            for i in 0..self.tris.len() {
                if self.planes[i].0.dot(dir) < self.planes[best.2].0.dot(dir) - 1e-6 {
                    let (d, q) = tri(i);
                    if d <= tol {
                        best = (best.0.min(d), q, i);
                    }
                }
            }
        }
        Closest { dist: best.0, point: best.1, normal: self.planes[best.2].0 }
    }

    /// Parameter `t` (0..1) of the point of the segment `a..b` nearest to the hull, EXACT, when the segment does not touch the hull; `None` when it overlaps it
    /// (the caller then needs the deepest-penetration point instead). The nearest pair of a segment and a convex polytope is an endpoint against the hull, or the
    /// segment interior against an edge (against a face only when parallel, which an endpoint ties): so it is the best of 2 endpoint distances and the edge distances.
    pub fn segment_nearest_param(&self, a: Vec3, b: Vec3) -> Option<f32> {
        let d = b - a;
        let mut separated = false;
        let (mut t0, mut t1) = (0f32, 1f32);
        for (n, off) in &self.planes {
            let denom = n.dot(d);
            let dist = n.dot(a) - off;
            if denom.abs() < 1e-9 {
                if dist > 0. {
                    separated = true;
                    break;
                }
                continue;
            }
            let t = -dist / denom;
            if denom < 0. {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                separated = true;
                break;
            }
        }
        if !separated {
            return None;
        }
        if d.length_squared() < 1e-12 {
            return Some(0.);
        }
        let mut best = (self.sdf(a), 0f32);
        let db = self.sdf(b);
        if db < best.0 {
            best = (db, 1.);
        }
        for e in &self.edges {
            let (s, dist) = segment_segment(a, b, self.verts[e[0] as usize], self.verts[e[1] as usize]);
            if dist < best.0 {
                best = (dist, s);
            }
        }
        Some(best.1)
    }

    /// First entry of the ray `origin + t * dir` (unit `dir`) into the hull for `0 <= t <= max_t`: (t, outward normal). `None` when it misses or starts inside.
    pub fn ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<(f32, Vec3)> {
        let (mut t0, mut t1, mut nin) = (0f32, max_t, Vec3::ZERO);
        for (n, d) in &self.planes {
            let denom = n.dot(dir);
            let dist = n.dot(origin) - d;
            if denom.abs() < 1e-9 {
                if dist > 0. {
                    return None;
                }
                continue;
            }
            let t = -dist / denom;
            if denom < 0. {
                if t > t0 {
                    t0 = t;
                    nin = *n;
                }
            } else if t < t1 {
                t1 = t;
            }
            if t0 > t1 {
                return None;
            }
        }
        if nin == Vec3::ZERO {
            None
        } else {
            Some((t0, nin))
        }
    }
}

/// The convex pieces of one prop (an oak is 9 of them) with a common bounding sphere.
#[derive(Clone, Debug, Default)]
pub struct HullSet {
    pub hulls: Vec<Hull>,
    pub center: Vec3,
    pub radius: f32,
}

impl HullSet {
    pub fn new(hulls: Vec<Hull>) -> HullSet {
        if hulls.is_empty() {
            return HullSet::default();
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for h in &hulls {
            lo = lo.min(h.min);
            hi = hi.max(h.max);
        }
        let center = (lo + hi) * 0.5;
        let radius = hulls.iter().map(|h| (h.center - center).length() + h.radius).fold(0., f32::max);
        HullSet { hulls, center, radius }
    }
    pub fn is_empty(&self) -> bool {
        self.hulls.is_empty()
    }
    pub fn mapped(&self, f: impl Fn(Vec3) -> Vec3 + Copy, flip: bool) -> HullSet {
        HullSet::new(self.hulls.iter().map(|h| h.mapped(f, flip)).collect())
    }
    /// Smallest signed distance over the pieces (`f32::MAX` when empty).
    pub fn sdf(&self, p: Vec3) -> f32 {
        let mut best = f32::MAX;
        for h in &self.hulls {
            if (p - h.center).length() - h.radius >= best || h.aabb_dist(p) >= best {
                continue;
            }
            best = best.min(h.sdf(p));
        }
        best
    }
    /// Smallest horizontal-plane-free distance between the segment `a..b` and the bounding sphere centre (broad phase).
    pub fn segment_clearance(&self, a: Vec3, b: Vec3) -> f32 {
        let ab = b - a;
        let l2 = ab.length_squared();
        let t = if l2 > 0. { ((self.center - a).dot(ab) / l2).clamp(0., 1.) } else { 0. };
        (a + ab * t - self.center).length() - self.radius
    }
    /// Sphere sweep of `radius` from `from` to `to`: fraction of the segment at first contact (`Some(0.)` when it starts overlapping), `None` when clear.
    /// Sphere tracing on the exact (1-Lipschitz) distance of only those pieces whose bounding sphere the swept sphere can reach.
    pub fn trace_sphere(&self, from: Vec3, to: Vec3, radius: f32) -> Option<f32> {
        if self.hulls.is_empty() || self.segment_clearance(from, to) > radius {
            return None;
        }
        let seg = to - from;
        let len = seg.length();
        let seg_clear = |h: &Hull| {
            let l2 = seg.length_squared();
            let t = if l2 > 0. { ((h.center - from).dot(seg) / l2).clamp(0., 1.) } else { 0. };
            (from + seg * t - h.center).length() - h.radius
        };
        let (slo, shi) = (from.min(to) - Vec3::splat(radius), from.max(to) + Vec3::splat(radius));
        let cands: Vec<&Hull> = self.hulls.iter().filter(|h| seg_clear(h) <= radius && h.aabb_gap(slo, shi) <= 0.).collect();
        if cands.is_empty() {
            return None;
        }
        let dist = |p: Vec3| {
            let mut best = f32::MAX;
            for h in &cands {
                if (p - h.center).length() - h.radius >= best || h.aabb_dist(p) >= best {
                    continue;
                }
                best = best.min(h.sdf(p));
            }
            best
        };
        if dist(from) <= radius {
            return Some(0.);
        }
        if len < 1e-6 {
            return None;
        }
        let dir = seg / len;
        let mut t = 0f32;
        for _ in 0..1024 {
            let d = dist(from + dir * t) - radius;
            if d <= 1e-4 {
                return Some((t / len).clamp(0., 1.));
            }
            t += d.max(2e-3);
            if t >= len {
                // the last step may land exactly on the end
                return if dist(to) - radius <= 1e-4 { Some(1.) } else { None };
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(h: f32) -> Vec<Vec3> {
        let mut v = vec![];
        for x in [-h, h] {
            for y in [-h, h] {
                for z in [-h, h] {
                    v.push(Vec3::new(x, y, z));
                }
            }
        }
        v
    }
    struct Rng(u32);
    impl Rng {
        fn f(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            (self.0 >> 8) as f32 / (1u32 << 24) as f32
        }
        fn v(&mut self, s: f32) -> Vec3 {
            Vec3::new(self.f() - 0.5, self.f() - 0.5, self.f() - 0.5) * 2. * s
        }
    }

    #[test]
    fn a_cube_has_12_triangles_and_the_exact_distance_field() {
        let h = Hull::from_points(&cube(1.)).unwrap();
        assert_eq!(h.tris.len(), 12, "6 quads as 12 triangles");
        assert!((h.sdf(Vec3::ZERO) + 1.).abs() < 1e-5);
        assert!((h.sdf(Vec3::new(3., 0., 0.)) - 2.).abs() < 1e-5);
        assert!((h.sdf(Vec3::new(2., 2., 0.)) - 2f32.sqrt()).abs() < 1e-5, "edge region");
        assert!((h.sdf(Vec3::new(2., 2., 2.)) - 3f32.sqrt()).abs() < 1e-5, "corner region");
        assert!((h.sdf(Vec3::new(0.5, 0.2, -0.9)) + 0.1).abs() < 1e-5);
    }
    #[test]
    fn every_input_point_is_inside_and_planes_point_outward() {
        let mut r = Rng(0x2545_F491);
        for case in 0..40 {
            let n = 5 + case % 40;
            let pts: Vec<Vec3> = (0..n).map(|_| r.v(2.)).collect();
            let h = Hull::from_points(&pts).unwrap();
            for p in &pts {
                assert!(h.sdf(*p) <= 1e-4, "case {case}: a hull point is {} outside", h.sdf(*p));
            }
            let c = h.verts.iter().copied().sum::<Vec3>() / h.verts.len() as f32;
            assert!(h.planes.iter().all(|(n, d)| n.dot(c) - d < 0.), "centroid inside every plane");
            // closed surface: every directed edge has its reverse
            let mut e: Vec<(u32, u32)> = vec![];
            for t in &h.tris {
                e.extend([(t[0], t[1]), (t[1], t[2]), (t[2], t[0])]);
            }
            assert!(e.iter().all(|&(a, b)| e.contains(&(b, a))), "case {case}: open mesh");
            // Euler: V - E + F = 2
            assert_eq!(h.verts.len() as i32 - (e.len() / 2) as i32 + h.tris.len() as i32, 2, "case {case}");
        }
    }
    #[test]
    fn coplanar_and_degenerate_inputs_are_none_not_a_panic() {
        assert!(Hull::from_points(&[Vec3::ZERO; 8]).is_none());
        assert!(Hull::from_points(&[Vec3::new(0., 0., 0.), Vec3::X, Vec3::Y, Vec3::new(1., 1., 0.)]).is_none());
        assert!(Hull::from_points(&[Vec3::ZERO, Vec3::X, Vec3::Y]).is_none());
        assert!(Hull::from_points(&[Vec3::NAN; 6]).is_none());
    }
    #[test]
    fn sdf_matches_a_brute_force_distance_to_the_vertices_and_faces() {
        let mut r = Rng(77);
        let pts: Vec<Vec3> = (0..14).map(|_| r.v(1.5)).collect();
        let h = Hull::from_points(&pts).unwrap();
        for _ in 0..200 {
            let q = r.v(4.);
            let d = h.sdf(q);
            // outside: no vertex is nearer than the distance, and the closest point lies on the surface
            if d > 0. {
                assert!(pts.iter().all(|p| (*p - q).length() >= d - 1e-4));
                let c = h.closest(q, Vec3::ZERO);
                assert!((c.dist - d).abs() < 1e-4 && h.sdf(c.point).abs() < 1e-3);
            }
        }
    }
    #[test]
    fn closest_picks_the_face_that_opposes_the_sweep_at_a_rim() {
        let h = Hull::from_points(&cube(1.)).unwrap();
        let p = Vec3::new(1., 1., 3.); // above the top rim edge x=1, y=1? that is the edge (1,1,z) at z=1 corner region
        let top = h.closest(Vec3::new(1.0, 0.0, 1.0 + 1e-5) + Vec3::new(0., 0., 0.), Vec3::new(0., 0., -1.));
        assert!(top.normal.z > 0.9, "dropping onto the rim reports the top face: {:?}", top.normal);
        let side = h.closest(Vec3::new(1.0 + 1e-5, 0.0, 1.0), Vec3::new(-1., 0., 0.));
        assert!(side.normal.x > 0.9, "walking into the rim reports the side face: {:?}", side.normal);
        let _ = p;
    }
    #[test]
    fn rays_enter_at_the_analytic_point_and_miss_or_start_inside_cleanly() {
        let h = Hull::from_points(&cube(1.)).unwrap();
        let (t, n) = h.ray(Vec3::new(0.2, 0.3, 5.), Vec3::NEG_Z, 10.).unwrap();
        assert!((t - 4.).abs() < 1e-5 && n.z > 0.99);
        assert!(h.ray(Vec3::new(3., 0., 5.), Vec3::NEG_Z, 10.).is_none());
        assert!(h.ray(Vec3::new(0., 0., 5.), Vec3::NEG_Z, 3.).is_none(), "too short");
        assert!(h.ray(Vec3::ZERO, Vec3::NEG_Z, 10.).is_none(), "starts inside");
        assert!(h.ray(Vec3::new(0., 0., 5.), Vec3::Z, 10.).is_none(), "pointing away");
        // parallel to a face and outside its slab
        assert!(h.ray(Vec3::new(2., 0., 0.), Vec3::Y, 10.).is_none());
    }
    #[test]
    fn mapping_with_an_axis_swap_keeps_distances_when_flipped() {
        let mut r = Rng(5);
        let pts: Vec<Vec3> = (0..12).map(|_| r.v(1.)).collect();
        let h = Hull::from_points(&pts).unwrap();
        let swap = |v: Vec3| Vec3::new(v.x, v.z, v.y) * 100. + Vec3::new(5., 0., 0.);
        let m = h.mapped(swap, true);
        for _ in 0..50 {
            let q = r.v(2.);
            assert!((h.sdf(q) * 100. - m.sdf(swap(q))).abs() < 1e-2, "{} vs {}", h.sdf(q) * 100., m.sdf(swap(q)));
        }
        let c = m.verts.iter().copied().sum::<Vec3>() / m.verts.len() as f32;
        assert!(m.planes.iter().all(|(n, d)| n.dot(c) - d < 0.));
    }
    #[test]
    fn the_nearest_point_of_a_vertical_segment_matches_a_brute_force_search_and_none_means_overlap() {
        let mut r = Rng(0xC0FFEE);
        let (mut hits, mut overlaps) = (0, 0);
        for case in 0..300 {
            let pts: Vec<Vec3> = (0..8 + case % 10).map(|_| r.v(1.2)).collect();
            let h = Hull::from_points(&pts).unwrap();
            let (x, y) = (r.f() * 3.2 - 1.6, r.f() * 3.2 - 1.6);
            let (z0, len) = (r.f() * 4. - 2., 0.2 + r.f() * 2.);
            let (a, b) = (Vec3::new(x, y, z0), Vec3::new(x, y, z0 + len));
            let brute = (0..=2000).map(|i| h.sdf(a + (b - a) * (i as f32 / 2000.))).fold(f32::MAX, f32::min);
            match h.segment_nearest_param(a, b) {
                Some(t) => { hits += 1; let d = h.sdf(a + (b - a) * t); assert!(brute > -1e-3 && (d - brute).abs() < 2e-3, "case {case}: exact {d} brute {brute}"); }
                None => { overlaps += 1; assert!(brute <= 2e-3, "case {case}: reported overlap but the brute-force distance is {brute}"); }
            }
        }
        assert!(hits > 50 && overlaps > 20, "{hits} disjoint {overlaps} overlapping");
    }
    #[test]
    fn a_sphere_sweep_stops_at_the_analytic_contact_and_clears_when_offset() {
        let set = HullSet::new(vec![Hull::from_points(&cube(1.)).unwrap()]);
        let t = set.trace_sphere(Vec3::new(-5., 0., 0.), Vec3::new(5., 0., 0.), 0.25).unwrap();
        assert!((t * 10. - (5. - 1. - 0.25)).abs() < 2e-3, "{}", t * 10.);
        assert!(set.trace_sphere(Vec3::new(-5., 2., 0.), Vec3::new(5., 2., 0.), 0.25).is_none(), "1 m clear of the face");
        assert!(set.trace_sphere(Vec3::new(-5., 1.2, 0.), Vec3::new(5., 1.2, 0.), 0.25).is_some(), "a 0.25 sphere 0.2 above the top grazes it");
        assert_eq!(set.trace_sphere(Vec3::new(0.5, 0., 0.), Vec3::new(5., 0., 0.), 0.1), Some(0.));
        assert!(set.trace_sphere(Vec3::new(5., 0., 0.), Vec3::new(10., 0., 0.), 0.1).is_none(), "moving away");
        // corner: the contact distance is measured to the corner point
        let t = set.trace_sphere(Vec3::new(-4., -4., 0.), Vec3::new(0., 0., 0.), 0.5).unwrap();
        let hit = Vec3::new(-4., -4., 0.) * (1. - t);
        assert!((set.sdf(hit) - 0.5).abs() < 2e-3, "stops 0.5 from the surface, got {}", set.sdf(hit));
    }
    #[test]
    fn sets_combine_pieces_and_have_a_bounding_sphere_that_contains_them() {
        let a = Hull::from_points(&cube(1.)).unwrap();
        let b = a.mapped(|v| v + Vec3::new(10., 0., 0.), false);
        let s = HullSet::new(vec![a, b]);
        assert!(s.sdf(Vec3::new(10., 0., 0.)) < 0. && s.sdf(Vec3::ZERO) < 0. && s.sdf(Vec3::new(5., 0., 0.)) > 3.9);
        for h in &s.hulls {
            assert!((h.center - s.center).length() + h.radius <= s.radius + 1e-4);
        }
        assert!(HullSet::default().trace_sphere(Vec3::ZERO, Vec3::X, 1.).is_none() && HullSet::default().sdf(Vec3::ZERO) == f32::MAX);
    }
}
