//! The 1.8 `Athena_Terrain` landscape as walkable ground: the `terrain.bin` height field (written by `fn-export props backdrop-terrain`),
//! queried with the triangles UE 4.16 builds from it.
//!
//! RECOVERED (cooked `Athena_Terrain` `LandscapeHeightfieldCollisionComponent`s): `CollisionSizeQuads` 127 and `CollisionScale` 1.0, i.e. the collision height
//! field IS the render height map at mip 0 (same 128x128 samples per component), so one triangulation serves render, collision and the distance field.
//! TRIANGULATION (UE 4.16): every quad is split along the diagonal (x, y) -> (x+1, y+1): `LandscapeRender.cpp:1977-1983` emits (i00, i11, i10) and (i00, i01, i11);
//! the PhysX height field (`LandscapeCollision.cpp:503`, rows = mirrored x, columns = y, `tessFlag` left 0 = diagonal (row, col+1)-(row+1, col)) is the same split.
//!
//! Spaces: UE centimetres (x, y, z up) everywhere in this crate except [`Mapping`], which converts to the port's game metres (x, z, y up) the same way
//! `fn-render::backdrop::to_game` does. Pure Rust, no GPU, builds for wasm.
use glam::{Vec2, Vec3};
use std::sync::{Arc, RwLock};

/// One landscape component (`terrain.bin`).
pub struct TerrainComp {
    pub size: usize,
    /// UE cm of sample (0, 0) at raw height 32768.
    pub origin: [f32; 3],
    /// cm per quad in x / y, cm per raw height unit.
    pub step: [f32; 3],
    pub heights: Vec<u16>,
    /// `(layer name index, weights)`.
    pub layers: Vec<(usize, Vec<u8>)>,
}
pub struct TerrainSet { pub layer_names: Vec<String>, pub comps: Vec<TerrainComp> }

impl TerrainComp {
    /// World UE height (cm) of grid sample (x, y).
    pub fn z_cm(&self, x: usize, y: usize) -> f32 { self.origin[2] + (self.heights[y * self.size + x] as f32 - 32768.) * self.step[2] }
}

fn rd_u32(b: &[u8], p: &mut usize) -> Result<u32, String> {
    let v = b.get(*p..*p + 4).ok_or("truncated")?;
    *p += 4;
    Ok(u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
}
fn rd_f32(b: &[u8], p: &mut usize) -> Result<f32, String> { Ok(f32::from_bits(rd_u32(b, p)?)) }

/// `terrain.bin` written by `fn-export props backdrop-terrain`.
pub fn parse_terrain(b: &[u8]) -> Result<TerrainSet, String> {
    if b.len() < 16 || &b[..4] != b"FNTH" { return Err("terrain.bin: bad magic".into()); }
    let mut p = 4;
    if rd_u32(b, &mut p)? != 1 { return Err("terrain.bin: version".into()); }
    let (n, nl) = (rd_u32(b, &mut p)? as usize, rd_u32(b, &mut p)? as usize);
    let mut layer_names = vec![];
    for _ in 0..nl {
        let l = *b.get(p).ok_or("truncated")? as usize;
        layer_names.push(String::from_utf8_lossy(b.get(p + 1..p + 1 + l).ok_or("truncated")?).into_owned());
        p += 1 + l;
    }
    let mut comps = vec![];
    for _ in 0..n {
        let size = rd_u32(b, &mut p)? as usize;
        let mut v = [0f32; 6];
        for x in v.iter_mut() { *x = rd_f32(b, &mut p)?; }
        let hb = b.get(p..p + size * size * 2).ok_or("truncated heights")?;
        let heights: Vec<u16> = hb.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        p += size * size * 2;
        let nlay = *b.get(p).ok_or("truncated")? as usize;
        p += 1;
        let mut layers = vec![];
        for _ in 0..nlay {
            let li = *b.get(p).ok_or("truncated")? as usize;
            let w = b.get(p + 1..p + 1 + size * size).ok_or("truncated weights")?.to_vec();
            p += 1 + size * size;
            layers.push((li, w));
        }
        comps.push(TerrainComp { size, origin: [v[0], v[1], v[2]], step: [v[3], v[4], v[5]], heights, layers });
    }
    Ok(TerrainSet { layer_names, comps })
}

/// Height of the UE 4.16 landscape triangle pair of one quad at fractional position (tx, ty) in [0, 1]^2 (corner heights h00 h10 h01 h11),
/// with the quad's face normal in "height per quad" units: diagonal (0,0)-(1,1).
#[inline]
pub fn quad_height(h00: f32, h10: f32, h01: f32, h11: f32, tx: f32, ty: f32) -> f32 {
    if tx >= ty { h00 + tx * (h10 - h00) + ty * (h11 - h10) } else { h00 + ty * (h01 - h00) + tx * (h11 - h01) }
}

pub fn component_holes(set: &TerrainSet) -> Vec<Vec<bool>> { (0..set.comps.len()).map(|ci| comp_holes(set, ci)).collect() }
pub fn comp_holes(set: &TerrainSet, ci: usize) -> Vec<bool> {
    let c = &set.comps[ci];
    let n = c.size * c.size;
    let Some(vi) = set.layer_names.iter().position(|n| n == "DataLayer" || n == "DataLayer__") else { return vec![false; n] };
    let Some((_, vw)) = c.layers.iter().find(|(li, _)| *li == vi) else { return vec![false; n] };
    vw.iter().take(n).map(|&v| v >= 128).collect()
}

/// The landscape with an O(1) component lookup.
pub struct Heightfield {
    pub set: TerrainSet,
    /// UE cm of the component lattice origin (the smallest component origin).
    lat0: Vec2,
    /// cm per quad (x, y) - every component has the same.
    pub step: Vec2,
    /// Quads per component edge.
    quads: usize,
    nx: usize,
    ny: usize,
    index: Vec<u32>,
    holes: Vec<Vec<bool>>,
    /// UE cm bounds of all components (x, y).
    pub min: Vec2,
    pub max: Vec2,
}

const NONE: u32 = u32::MAX;

impl Heightfield {
    pub fn new(set: TerrainSet) -> Result<Heightfield, String> {
        let c0 = set.comps.first().ok_or("terrain without components")?;
        let (size, step) = (c0.size, Vec2::new(c0.step[0], c0.step[1]));
        if set.comps.iter().any(|c| c.size != size || (c.step[0] - step.x).abs() > 1e-3 || (c.step[1] - step.y).abs() > 1e-3) {
            return Err("terrain components differ in size / spacing".into());
        }
        let quads = size - 1;
        let edge = step * quads as f32;
        let lat0 = set.comps.iter().fold(Vec2::splat(f32::MAX), |m, c| m.min(Vec2::new(c.origin[0], c.origin[1])));
        let max = set.comps.iter().fold(Vec2::splat(f32::MIN), |m, c| m.max(Vec2::new(c.origin[0], c.origin[1]) + edge));
        let cell = |c: &TerrainComp| (((c.origin[0] - lat0.x) / edge.x).round() as usize, ((c.origin[1] - lat0.y) / edge.y).round() as usize);
        let (nx, ny) = set.comps.iter().fold((0, 0), |(a, b), c| { let (x, y) = cell(c); (a.max(x + 1), b.max(y + 1)) });
        let mut index = vec![NONE; nx * ny];
        for (i, c) in set.comps.iter().enumerate() {
            let (x, y) = cell(c);
            let o = Vec2::new(c.origin[0], c.origin[1]) - (lat0 + Vec2::new(x as f32, y as f32) * edge);
            if o.abs().max_element() > 1. { return Err(format!("component {i} is off the component lattice by {o:?} cm")); }
            if index[y * nx + x] != NONE { return Err(format!("components {} and {i} overlap", index[y * nx + x])); }
            index[y * nx + x] = i as u32;
        }
        let holes = component_holes(&set);
        Ok(Heightfield { set, lat0, step, quads, nx, ny, index, holes, min: lat0, max })
    }

    /// Global quad lattice coordinates of a UE xy (fractional).
    #[inline]
    fn lattice(&self, x: f32, y: f32) -> Vec2 { (Vec2::new(x, y) - self.lat0) / self.step }

    /// Corner heights (cm) of global quad (qx, qy), `None` where no component covers it.
    fn quad(&self, qx: i64, qy: i64) -> Option<[f32; 4]> {
        if qx < 0 || qy < 0 { return None; }
        let q = self.quads as i64;
        let (cx, cy) = ((qx / q) as usize, (qy / q) as usize);
        if cx >= self.nx || cy >= self.ny { return None; }
        let ci = self.index[cy * self.nx + cx];
        if ci == NONE { return None; }
        let c = &self.set.comps[ci as usize];
        let (lx, ly) = ((qx % q) as usize, (qy % q) as usize);
        if self.holes[ci as usize].get(ly * c.size + lx).copied().unwrap_or(false) { return None; }
        Some([c.z_cm(lx, ly), c.z_cm(lx + 1, ly), c.z_cm(lx, ly + 1), c.z_cm(lx + 1, ly + 1)])
    }

    pub fn hole(&self, x: f32, y: f32) -> bool {
        let l = self.lattice(x, y);
        let (qx, qy) = (l.x.floor() as i64, l.y.floor() as i64);
        if qx < 0 || qy < 0 { return false; }
        let q = self.quads as i64;
        let (cx, cy) = ((qx / q) as usize, (qy / q) as usize);
        if cx >= self.nx || cy >= self.ny { return false; }
        let ci = self.index[cy * self.nx + cx];
        if ci == NONE { return false; }
        let c = &self.set.comps[ci as usize];
        self.holes[ci as usize].get(((qy % q) as usize) * c.size + (qx % q) as usize).copied().unwrap_or(false)
    }

    /// Landscape height (UE cm) at a UE xy, `None` outside the landscape.
    pub fn height_cm(&self, x: f32, y: f32) -> Option<f32> { self.sample(x, y).map(|s| s.0) }

    /// Height (cm) and the up-facing triangle normal (UE space) at a UE xy.
    pub fn sample(&self, x: f32, y: f32) -> Option<(f32, Vec3)> {
        let l = self.lattice(x, y);
        let (mut qx, mut qy) = (l.x.floor() as i64, l.y.floor() as i64);
        // the far edge of the landscape belongs to the last quad
        let (ex, ey) = ((self.nx * self.quads) as i64, (self.ny * self.quads) as i64);
        if qx == ex && l.x <= ex as f32 { qx -= 1; }
        if qy == ey && l.y <= ey as f32 { qy -= 1; }
        let [h00, h10, h01, h11] = self.quad(qx, qy)?;
        let (tx, ty) = (l.x - qx as f32, l.y - qy as f32);
        let h = quad_height(h00, h10, h01, h11, tx, ty);
        let (dx, dy) = if tx >= ty { (h10 - h00, h11 - h10) } else { (h11 - h01, h01 - h00) };
        let n = Vec3::new(-dx / self.step.x, -dy / self.step.y, 1.).normalize();
        Some((h, n))
    }

    /// Calls `f` with every landscape triangle (UE cm, counter-clockwise seen from above) whose quad overlaps the UE xy rectangle `lo..hi`.
    pub fn triangles_in(&self, lo: Vec2, hi: Vec2, mut f: impl FnMut([Vec3; 3])) {
        let (a, b) = (self.lattice(lo.x, lo.y), self.lattice(hi.x, hi.y));
        let (x0, y0, x1, y1) = (a.x.floor() as i64, a.y.floor() as i64, b.x.floor() as i64, b.y.floor() as i64);
        for qy in y0..=y1 {
            for qx in x0..=x1 {
                let Some([h00, h10, h01, h11]) = self.quad(qx, qy) else { continue };
                let p = |ix: i64, iy: i64, h: f32| Vec3::new(self.lat0.x + ix as f32 * self.step.x, self.lat0.y + iy as f32 * self.step.y, h);
                let (p00, p10, p01, p11) = (p(qx, qy, h00), p(qx + 1, qy, h10), p(qx, qy + 1, h01), p(qx + 1, qy + 1, h11));
                f([p00, p10, p11]);
                f([p00, p11, p01]);
            }
        }
    }

    /// First landscape hit of the ray `from + dir * t`, t in 0..=max (UE cm; `dir` normalised): (t, surface normal).
    /// Marches at a quarter quad and bisects the sign change of (ray z - landscape z): exact on the piecewise-planar surface up to 1/64 cm.
    pub fn ray(&self, from: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3)> {
        let f = |t: f32| { let p = from + dir * t; self.height_cm(p.x, p.y).map(|h| p.z - h) };
        let h_step = self.step.min_element() * 0.25;
        // horizontal rays step a quarter quad; steep rays a quarter quad of height change at most
        let dt = h_step / dir.truncate().length().max(0.25);
        let (mut t0, mut g0) = (0f32, f(0.));
        if g0.is_some_and(|g| g < 0.) { return None; }
        while t0 < max {
            let t1 = (t0 + dt).min(max);
            let g1 = f(t1);
            if let (Some(a), Some(b)) = (g0, g1) {
                if a >= 0. && b < 0. {
                    let (mut lo, mut hi) = (t0, t1);
                    for _ in 0..24 { let m = 0.5 * (lo + hi); if f(m).is_some_and(|g| g >= 0.) { lo = m } else { hi = m } }
                    let p = from + dir * hi;
                    return Some((lo, self.sample(p.x, p.y).map(|s| s.1).unwrap_or(Vec3::Z)));
                }
            }
            t0 = t1;
            g0 = g1;
        }
        None
    }

    /// Smallest distance from the segment `a`..`b` to the landscape within `reach` cm (UE space), signed: negative when the segment's lower end is under the
    /// surface (depth along the normal). Returns (distance, closest landscape point, triangle normal). `None` when nothing is within `reach`.
    pub fn segment_distance(&self, a: Vec3, b: Vec3, reach: f32) -> Option<(f32, Vec3, Vec3)> {
        let low = if a.z <= b.z { a } else { b };
        if let Some((h, n)) = self.sample(low.x, low.y) {
            if low.z < h {
                let d = (low.z - h) * n.z;
                return Some((d, low - n * d, n));
            }
        }
        let (lo, hi) = (a.truncate().min(b.truncate()) - Vec2::splat(reach), a.truncate().max(b.truncate()) + Vec2::splat(reach));
        let mut best: Option<(f32, Vec3, Vec3)> = None;
        let zlo = a.z.min(b.z);
        let (slo, shi) = (a.truncate().min(b.truncate()), a.truncate().max(b.truncate()));
        self.triangles_in(lo, hi, |t| {
            let bound = best.map_or(reach, |b| b.0);
            // vertical lower bound: the segment's bottom above the triangle's top
            let top = t[0].z.max(t[1].z).max(t[2].z);
            if zlo - top >= bound { return; }
            // horizontal lower bound: the gap between the xy boxes of the segment and the triangle
            let (tlo, thi) = (t[0].truncate().min(t[1].truncate()).min(t[2].truncate()), t[0].truncate().max(t[1].truncate()).max(t[2].truncate()));
            if (tlo - shi).max(slo - thi).max(Vec2::ZERO).length() >= bound { return; }
            let (d, p) = segment_triangle(a, b, &t);
            if d < bound {
                let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                best = Some((d, p, n));
            }
        });
        best
    }
}

impl Heightfield {
    /// Exact (min, max) landscape height (cm) over the UE xy rectangle `lo..hi`: every triangle clipped to the rectangle (the surface is piecewise planar, so the
    /// extremes are vertices of the clipped polygons). `None` when no component covers any of it.
    pub fn height_range(&self, lo: Vec2, hi: Vec2) -> Option<(f32, f32)> {
        let mut r: Option<(f32, f32)> = None;
        self.triangles_in(lo, hi, |t| {
            let mut poly: Vec<Vec3> = t.to_vec();
            // Sutherland-Hodgman against x >= lo.x, x <= hi.x, y >= lo.y, y <= hi.y
            for (axis, bound, keep_above) in [(0, lo.x, true), (0, hi.x, false), (1, lo.y, true), (1, hi.y, false)] {
                let inside = |p: &Vec3| if keep_above { p[axis] >= bound } else { p[axis] <= bound };
                let mut out = Vec::with_capacity(poly.len() + 2);
                for i in 0..poly.len() {
                    let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
                    if inside(&p) { out.push(p); }
                    if inside(&p) != inside(&q) { let s = (bound - p[axis]) / (q[axis] - p[axis]); out.push(p + (q - p) * s); }
                }
                poly = out;
                if poly.is_empty() { return; }
            }
            for p in poly { r = Some(r.map_or((p.z, p.z), |(a, b)| (a.min(p.z), b.max(p.z)))); }
        });
        r
    }

    /// A synthetic landscape for tests and tools: `n x n` components of `size` samples, `step` cm quads, sample (0, 0) of the first component at UE xy `origin`,
    /// heights `f(x, y)` (UE cm, quantised to 1/16 cm, range +-2048 cm).
    pub fn synthetic(origin: Vec2, step: f32, n: usize, size: usize, f: impl Fn(f32, f32) -> f32) -> Heightfield {
        let q = size - 1;
        let mut comps = vec![];
        for cy in 0..n {
            for cx in 0..n {
                let o = origin + Vec2::new((cx * q) as f32, (cy * q) as f32) * step;
                let heights = (0..size * size).map(|i| { let (x, y) = (o.x + (i % size) as f32 * step, o.y + (i / size) as f32 * step); (32768. + f(x, y) * 16.).round().clamp(0., 65535.) as u16 }).collect();
                comps.push(TerrainComp { size, origin: [o.x, o.y, 0.], step: [step, step, 1. / 16.], heights, layers: vec![] });
            }
        }
        Heightfield::new(TerrainSet { layer_names: vec![], comps }).expect("synthetic landscape")
    }
}

/// Closest point of triangle `t` to `p` (Ericson, Real-Time Collision Detection 5.1.5).
pub fn closest_on_triangle(p: Vec3, t: &[Vec3; 3]) -> Vec3 {
    let (a, b, c) = (t[0], t[1], t[2]);
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0. && d2 <= 0. { return a; }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0. && d4 <= d3 { return b; }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0. && d1 >= 0. && d3 <= 0. { return a + ab * (d1 / (d1 - d3)); }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0. && d5 <= d6 { return c; }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0. && d2 >= 0. && d6 <= 0. { return a + ac * (d2 / (d2 - d6)); }
    let va = d3 * d6 - d5 * d4;
    if va <= 0. && (d4 - d3) >= 0. && (d5 - d6) >= 0. { return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6))); }
    let denom = 1. / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// Distance between segment `a`..`b` and triangle `t` with the closest triangle point. The distance to a convex set is convex along the segment, so a
/// golden-section search finds the minimum (to ~1e-3 of the segment length; the pawn's segment is under 70 cm).
pub fn segment_triangle(a: Vec3, b: Vec3, t: &[Vec3; 3]) -> (f32, Vec3) {
    let d = |s: f32| { let p = a + (b - a) * s; let q = closest_on_triangle(p, t); ((p - q).length(), q) };
    if (b - a).length_squared() < 1e-6 { return d(0.); }
    const R: f32 = 0.618_034;
    let (mut lo, mut hi) = (0f32, 1f32);
    let (mut m1, mut m2) = (hi - R * (hi - lo), lo + R * (hi - lo));
    let (mut f1, mut f2) = (d(m1).0, d(m2).0);
    for _ in 0..16 {
        if f1 < f2 { hi = m2; m2 = m1; f2 = f1; m1 = hi - R * (hi - lo); f1 = d(m1).0; } else { lo = m1; m1 = m2; f1 = f2; m2 = lo + R * (hi - lo); f2 = d(m2).0; }
    }
    [d(0.), d(1.), d(0.5 * (lo + hi))].into_iter().fold((f32::MAX, Vec3::ZERO), |m, v| if v.0 < m.0 { v } else { m })
}

/// UE cm <-> game metres, as `fn-render::backdrop::to_game`: game = (anchor.x + (ue.x - centre.x) / 100, (ue.z - ref_z) / 100, anchor.z + (ue.y - centre.y) / 100).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapping { pub centre_cm: [f32; 3], pub anchor: Vec3, pub ref_z_cm: f32 }
impl Mapping {
    pub fn to_game(&self, ue: Vec3) -> Vec3 {
        Vec3::new(self.anchor.x + (ue.x - self.centre_cm[0]) * 0.01, (ue.z - self.ref_z_cm) * 0.01, self.anchor.z + (ue.y - self.centre_cm[1]) * 0.01)
    }
    pub fn to_ue(&self, g: Vec3) -> Vec3 {
        Vec3::new((g.x - self.anchor.x) * 100. + self.centre_cm[0], (g.z - self.anchor.z) * 100. + self.centre_cm[1], g.y * 100. + self.ref_z_cm)
    }
}

/// Longest step (cm) of [`World::sweep_sphere`] and its search reach. CHOSEN: one 2 m landscape quad.
pub const SWEEP_STEP_CM: f32 = 200.;

/// The installed landscape and its mapping (one per loaded world).
pub struct World { pub hf: Heightfield, pub map: Mapping }

impl World {
    /// Ground height (game metres, y) at game (x, z); `None` off the landscape.
    pub fn ground_y(&self, x: f32, z: f32) -> Option<f32> {
        let u = self.map.to_ue(Vec3::new(x, 0., z));
        self.hf.height_cm(u.x, u.y).map(|h| (h - self.map.ref_z_cm) * 0.01)
    }
    pub fn hole(&self, x: f32, z: f32) -> bool { let u = self.map.to_ue(Vec3::new(x, 0., z)); self.hf.hole(u.x, u.y) }
    /// Ground height and up normal in game space.
    pub fn ground(&self, x: f32, z: f32) -> Option<(f32, Vec3)> {
        let u = self.map.to_ue(Vec3::new(x, 0., z));
        self.hf.sample(u.x, u.y).map(|(h, n)| ((h - self.map.ref_z_cm) * 0.01, Vec3::new(n.x, n.z, n.y)))
    }
    /// First landscape hit of a game-space ray (metres): (t, game-space normal).
    pub fn ray(&self, from: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3)> {
        let d = Vec3::new(dir.x, dir.z, dir.y);
        self.hf.ray(self.map.to_ue(from), d, max * 100.).map(|(t, n)| (t * 0.01, Vec3::new(n.x, n.z, n.y)))
    }
    /// Sphere of radius `r` swept from `a` to `b` (game metres): the fraction of the move at first contact, `None` when it stays clear. Sphere tracing on the exact
    /// point-to-triangle distance (steps of the current clearance, never more than [`SWEEP_STEP_CM`]); contact = clearance under 0.05 cm.
    pub fn sweep_sphere(&self, a: Vec3, b: Vec3, r: f32) -> Option<f32> {
        let (ua, ub) = (self.map.to_ue(a), self.map.to_ue(b));
        let (d, rc) = (ub - ua, r * 100.);
        let len = d.length();
        let clear = |p: Vec3| self.hf.segment_distance(p, p, rc + SWEEP_STEP_CM).map_or(SWEEP_STEP_CM, |x| x.0 - rc);
        if len < 1e-3 { return (clear(ua) <= 0.05).then_some(0.); }
        let dir = d / len;
        let mut t = 0f32;
        for _ in 0..256 {
            let g = clear(ua + dir * t);
            if g <= 0.05 { return Some((t / len).clamp(0., 1.)); }
            if t >= len { return None; }
            t = (t + g).min(len);
        }
        None
    }
    /// Game-space xz bounds (min, max) of the landscape.
    pub fn bounds(&self) -> (Vec2, Vec2) {
        let (a, b) = (self.map.to_game(self.hf.min.extend(0.)), self.map.to_game(self.hf.max.extend(0.)));
        (Vec2::new(a.x.min(b.x), a.z.min(b.z)), Vec2::new(a.x.max(b.x), a.z.max(b.z)))
    }
}

/// The landscape seen from a translated centimetre space with UE axes (cm, Z up): `local + offset = UE cm`. The movement scene and the building world are such
/// spaces (scene X = game x * 100, Y = (game z - z_offset) * 100, Z = game y * 100), so [`Local::new`] derives the offset from the [`Mapping`]:
/// `offset = to_ue(game (0, 0, z_offset))` (the mapping is a pure translation once the axes are swapped).
#[derive(Clone)]
pub struct Local { pub world: Arc<World>, pub offset: Vec3 }
impl std::fmt::Debug for Local {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "Local {{ offset: {:?}, {} comps }}", self.offset, self.world.hf.set.comps.len()) }
}
impl Local {
    /// The local space whose origin is the game point (0, 0, `z_offset_m`).
    pub fn new(world: Arc<World>, z_offset_m: f32) -> Local { let offset = world.map.to_ue(Vec3::new(0., 0., z_offset_m)); Local { world, offset } }
    /// Height (local cm) and the up triangle normal at a local xy; `None` off the landscape (a hole or past its edge).
    #[inline]
    pub fn sample(&self, x: f32, y: f32) -> Option<(f32, Vec3)> { self.world.hf.sample(x + self.offset.x, y + self.offset.y).map(|(h, n)| (h - self.offset.z, n)) }
    pub fn hole(&self, x: f32, y: f32) -> bool { self.world.hf.hole(x + self.offset.x, y + self.offset.y) }
    /// [`Heightfield::segment_distance`] in local cm.
    pub fn segment_distance(&self, a: Vec3, b: Vec3, reach: f32) -> Option<(f32, Vec3, Vec3)> {
        self.world.hf.segment_distance(a + self.offset, b + self.offset, reach).map(|(d, p, n)| (d, p - self.offset, n))
    }
    /// [`Heightfield::height_range`] over a local xy rectangle, local cm.
    pub fn height_range(&self, lo: Vec2, hi: Vec2) -> Option<(f32, f32)> {
        let o = self.offset.truncate();
        self.world.hf.height_range(lo + o, hi + o).map(|(a, b)| (a - self.offset.z, b - self.offset.z))
    }
    /// True when components cover the four corners of the local xy rectangle (components are whole squares; a rectangle smaller than one has no hole inside).
    pub fn covers(&self, lo: Vec2, hi: Vec2) -> bool { [lo, hi, Vec2::new(lo.x, hi.y), Vec2::new(hi.x, lo.y)].iter().all(|p| self.sample(p.x, p.y).is_some()) }
}

static WORLD: RwLock<Option<Arc<World>>> = RwLock::new(None);

/// Installs the landscape every ground query of the process uses (the loader, once the world centre is known). `None` restores the flat world.
pub fn install(w: Option<Arc<World>>) { *WORLD.write().unwrap_or_else(|e| e.into_inner()) = w; }
/// The installed landscape, if any.
pub fn world() -> Option<Arc<World>> { WORLD.read().unwrap_or_else(|e| e.into_inner()).clone() }
/// Ground height (game metres) at game (x, z): the landscape when one is installed, else the flat world's 0.
pub fn ground_y(x: f32, z: f32) -> f32 { world().and_then(|w| w.ground_y(x, z)).unwrap_or(0.) }

#[cfg(test)]
mod tests {
    use super::*;

    /// 2x2 components of 3x3 samples (2 quads), 100 cm quads, heights = f(global sample).
    fn grid(f: impl Fn(usize, usize) -> u16) -> Heightfield {
        let mut comps = vec![];
        for (cx, cy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let heights = (0..9).map(|i| f(cx * 2 + i % 3, cy * 2 + i / 3)).collect();
            comps.push(TerrainComp { size: 3, origin: [-1000. + cx as f32 * 200., 500. + cy as f32 * 200., 0.], step: [100., 100., 1.], heights, layers: vec![] });
        }
        Heightfield::new(TerrainSet { layer_names: vec![], comps }).unwrap()
    }

    #[test]
    fn samples_are_exact_and_the_quad_split_is_the_ue_diagonal() {
        let hf = grid(|x, y| 32768 + (x * 10 + y * 100) as u16);
        assert_eq!(hf.height_cm(-1000., 500.), Some(0.));
        assert_eq!(hf.height_cm(-1000. + 300., 500. + 200.), Some(30. + 200.));
        // a saddle quad: only the (0,0)-(1,1) diagonal gives h00/h11 along it
        let hf = grid(|x, y| if (x + y) % 2 == 0 { 32768 } else { 32868 });
        let mid = hf.height_cm(-1000. + 50., 500. + 50.).unwrap();
        assert!(mid.abs() < 1e-3, "{mid}: on the 00-11 diagonal both corners are 0");
        assert!((hf.height_cm(-1000. + 75., 500. + 25.).unwrap() - 50.).abs() < 1e-3);
        assert_eq!(hf.height_cm(-1001., 500.), None);
        assert!(hf.height_cm(-1000. + 400., 500. + 400.).is_some(), "far edge inside");
    }

    #[test]
    fn normal_ray_and_segment_agree_with_the_surface() {
        let hf = grid(|x, _| 32768 + (x * 100) as u16); // slope 1 cm per cm along x
        let (h, n) = hf.sample(-850., 650.).unwrap();
        assert!((h - 150.).abs() < 1e-3);
        assert!((n - Vec3::new(-1., 0., 1.).normalize()).length() < 1e-4, "{n}");
        let (t, _) = hf.ray(Vec3::new(-850., 650., 1000.), Vec3::NEG_Z, 5000.).unwrap();
        assert!((t - 850.).abs() < 0.05, "{t}");
        assert!(hf.ray(Vec3::new(-850., 650., 1000.), Vec3::Z, 5000.).is_none());
        // a vertical segment 100 cm above the slope: distance 100 * cos 45
        let (d, _, nn) = hf.segment_distance(Vec3::new(-850., 650., 250.), Vec3::new(-850., 650., 300.), 300.).unwrap();
        assert!((d - 100. * std::f32::consts::FRAC_1_SQRT_2).abs() < 0.2, "{d}");
        assert!(nn.z > 0.7);
        let (d, ..) = hf.segment_distance(Vec3::new(-850., 650., 140.), Vec3::new(-850., 650., 200.), 300.).unwrap();
        assert!(d < 0., "penetrating: {d}");
    }

    #[test]
    fn height_range_is_exact_and_local_is_a_translation() {
        let hf = Heightfield::synthetic(Vec2::new(-1000., -1000.), 100., 2, 11, |x, y| 0.5 * x + 0.25 * y);
        let (a, b) = hf.height_range(Vec2::new(-150., -50.), Vec2::new(250., 75.)).unwrap();
        assert!((a - (-75. - 12.5)).abs() < 0.1 && (b - (125. + 18.75)).abs() < 0.1, "{a} {b}");
        assert!(hf.height_range(Vec2::splat(5000.), Vec2::splat(5100.)).is_none());
        let m = Mapping { centre_cm: [300., -200., 50.], anchor: Vec3::new(1., 0., -10.), ref_z_cm: 40. };
        let l = Local::new(Arc::new(World { hf, map: m }), 12.);
        // local (X, Y, Z) cm = game (x * 100, (z - 12) * 100, y * 100)
        let g = Vec3::new(2.5, 0.7, 4.);
        assert!((Vec3::new(g.x * 100., (g.z - 12.) * 100., g.y * 100.) + l.offset - m.to_ue(g)).length() < 1e-2);
        let (h, _) = l.sample(250., -1700.).unwrap();
        assert!((h * 0.01 - l.world.ground_y(2.5, -5.).unwrap()).abs() < 1e-4, "{h}");
    }

    #[test]
    fn mapping_round_trips() {
        let m = Mapping { centre_cm: [51152.1, 71639.9, 1912.], anchor: Vec3::new(0., 0., -10.), ref_z_cm: 768. };
        let u = Vec3::new(1000., -2000., 900.);
        assert!((m.to_ue(m.to_game(u)) - u).length() < 0.05);
    }
}

#[cfg(test)]
mod real {
    use super::*;
    /// The exported 1.8 landscape (`FN_TERRAIN_BIN`, default the shared assets folder); skipped when absent.
    #[test]
    fn the_athena_landscape_sits_on_one_component_lattice() {
        let p = fn_live::var("FN_TERRAIN_BIN").unwrap_or_else(|_| r"C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime\assets\props\backdrop\terrain.bin".into());
        let Ok(b) = std::fs::read(&p) else { eprintln!("skip: no {p}"); return };
        let hf = Heightfield::new(parse_terrain(&b).unwrap()).unwrap();
        eprintln!("TERRAIN {} comps, lattice {}x{}, bounds {:?}..{:?} cm", hf.set.comps.len(), hf.nx, hf.ny, hf.min, hf.max);
        // every sample of every component is reproduced by the lookup
        let mut worst = 0f32;
        for c in &hf.set.comps {
            for y in (0..c.size).step_by(7) {
                for x in (0..c.size).step_by(7) {
                    let Some(h) = hf.height_cm(c.origin[0] + x as f32 * c.step[0], c.origin[1] + y as f32 * c.step[1]) else { continue };
                    worst = worst.max((h - c.z_cm(x, y)).abs());
                }
            }
        }
        // under 1 cm: f32 positions at 1.5 km (ULP 0.008 cm) and borders shared by neighbouring components (raw unit 50 / 128 = 0.39 cm)
        assert!(worst < 1., "{worst}");
    }
}
