//! Synthetic textured static meshes (a broadleaf tree with masked two-sided foliage cards and 3 LODs, a rock, a checker-alpha quad). They
//! exercise the whole static-mesh path (alpha test, two-sided lighting, alpha-tested shadows, LOD, instancing) without any exported asset:
//! used by the tests, by `fn-viewer --prop-grid N` when no `--props` folder is given, and as the fallback look when real assets are missing.
use crate::static_mesh::*;
use glam::{Mat3, Vec3};
use image::{Rgba, RgbaImage};

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 17; self.0 ^= self.0 << 5; (self.0 >> 8) as f32 / (1u32 << 24) as f32 }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.next() }
}

#[derive(Default)]
struct Builder { v: Vec<StaticVertex>, i: Vec<u32> }
impl Builder {
    fn vert(&mut self, p: Vec3, n: Vec3, uv: [f32; 2], color: [u8; 4]) -> u32 {
        self.v.push(StaticVertex { pos: p.to_array(), normal: n.normalize_or_zero().to_array(), uv, color });
        self.v.len() as u32 - 1
    }
    fn quad(&mut self, c: [Vec3; 4], n: [Vec3; 4], color: [u8; 4]) {
        let uv = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
        let k: Vec<u32> = (0..4).map(|j| self.vert(c[j], n[j], uv[j], color)).collect();
        self.i.extend_from_slice(&[k[0], k[1], k[2], k[0], k[2], k[3]]);
    }
}

/// Leaf-cluster alpha texture: many small leaves with soft green variation, transparent between them (256 x 256 sRGB).
pub fn leaf_texture(seed: u32) -> RgbaImage {
    let n = 256u32;
    let mut img = RgbaImage::from_pixel(n, n, Rgba([60, 110, 30, 0]));
    let mut r = Rng(seed.wrapping_mul(2654435761).max(1));
    for _ in 0..70 {
        let (cx, cy) = (r.range(0., n as f32), r.range(0., n as f32));
        let (rx, ry, ang) = (r.range(9., 17.), r.range(4., 7.), r.range(0., std::f32::consts::PI));
        let shade = r.range(0.6, 1.2);
        let col = [(40. * shade) as u8, (120. * shade).min(255.) as u8, (28. * shade) as u8];
        let (s, c) = ang.sin_cos();
        for y in (cy as i32 - 20)..(cy as i32 + 20) {
            for x in (cx as i32 - 20)..(cx as i32 + 20) {
                let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
                if (u / rx).powi(2) + (v / ry).powi(2) <= 1. {
                    let (px, py) = (x.rem_euclid(n as i32) as u32, y.rem_euclid(n as i32) as u32);
                    let vein = if v.abs() < 0.7 { 1.25 } else { 1. };
                    img.put_pixel(px, py, Rgba([(col[0] as f32 * vein) as u8, (col[1] as f32 * vein).min(255.) as u8, (col[2] as f32 * vein) as u8, 255]));
                }
            }
        }
    }
    img
}
/// Bark-like vertical streak texture (opaque, 64 x 128).
pub fn bark_texture() -> RgbaImage {
    let mut r = Rng(77);
    let cols: Vec<f32> = (0..64).map(|_| r.range(0.55, 1.1)).collect();
    RgbaImage::from_fn(64, 128, |x, y| { let k = cols[x as usize] * (0.9 + 0.1 * ((y as f32 * 0.4 + x as f32).sin())); Rgba([(95. * k) as u8, (66. * k) as u8, (42. * k) as u8, 255]) })
}
/// Rocky noise texture (opaque, 128 x 128, tileable enough for a prop).
pub fn rock_texture() -> RgbaImage {
    let mut r = Rng(1234);
    let g: Vec<f32> = (0..16 * 16).map(|_| r.range(0.0, 1.0)).collect();
    RgbaImage::from_fn(128, 128, |x, y| {
        let (fx, fy) = (x as f32 / 8., y as f32 / 8.);
        let (ix, iy, tx, ty) = (fx as usize % 16, fy as usize % 16, fx.fract(), fy.fract());
        let s = |a: usize, b: usize| g[(b % 16) * 16 + a % 16];
        let v = (s(ix, iy) * (1. - tx) + s(ix + 1, iy) * tx) * (1. - ty) + (s(ix, iy + 1) * (1. - tx) + s(ix + 1, iy + 1) * tx) * ty;
        let k = 0.55 + 0.4 * v;
        Rgba([(120. * k) as u8, (116. * k) as u8, (108. * k) as u8, 255])
    })
}
/// Alpha checker (8 x 8 squares of 8 texels, every other one transparent) for alpha-test / two-sided lighting tests.
pub fn checker_alpha_texture() -> RgbaImage {
    RgbaImage::from_fn(64, 64, |x, y| if (x / 8 + y / 8) % 2 == 0 { Rgba([200, 200, 60, 255]) } else { Rgba([60, 60, 200, 0]) })
}

pub fn foliage_material(name: &str, seed: u32) -> StaticMaterialData {
    StaticMaterialData {
        name: name.into(), base_color: Some(leaf_texture(seed)), alpha_mode: AlphaMode::Masked, alpha_cutoff: 0.5, two_sided: true,
        roughness: 0.8, specular: 0.0, translucency: 0.45, sway: 0.03, flutter: 0.03, ..Default::default()
    }
}
pub fn bark_material(name: &str) -> StaticMaterialData {
    StaticMaterialData { name: name.into(), base_color: Some(bark_texture()), roughness: 0.9, specular: 0.1, sway: 0.03, ..Default::default() }
}
pub fn rock_material(name: &str) -> StaticMaterialData {
    StaticMaterialData { name: name.into(), base_color: Some(rock_texture()), roughness: 0.8, specular: 0.3, ..Default::default() }
}

fn trunk(b: &mut Builder, r0: f32, r1: f32, h: f32, sides: usize) {
    for k in 0..sides {
        let (a0, a1) = (std::f32::consts::TAU * k as f32 / sides as f32, std::f32::consts::TAU * (k + 1) as f32 / sides as f32);
        let p = |a: f32, r: f32, y: f32| Vec3::new(r * a.cos(), y, r * a.sin());
        let (n0, n1) = (Vec3::new(a0.cos(), 0.1, a0.sin()), Vec3::new(a1.cos(), 0.1, a1.sin()));
        // CCW seen from outside (a grows towards +z at +x): c1, c0, c3, c2
        b.quad([p(a1, r0, 0.), p(a0, r0, 0.), p(a0, r1, h), p(a1, r1, h)], [n1, n0, n0, n1], [255, 255, 255, 255]);
    }
}

/// `cards` leaf cards of `size` metres scattered in a sphere of radius `rad` around `(0, cy, 0)`; normals point away from the centre.
fn canopy(b: &mut Builder, cards: usize, size: f32, cy: f32, rad: f32, seed: u32) {
    let mut r = Rng(seed.max(1));
    let center = Vec3::new(0., cy, 0.);
    for _ in 0..cards {
        let dir = loop { let d = Vec3::new(r.range(-1., 1.), r.range(-0.6, 1.), r.range(-1., 1.)); if d.length() > 0.2 && d.length() <= 1. { break d.normalize(); } };
        let p = center + dir * rad * r.range(0.35, 1.0);
        let normal = dir;
        let tangent = normal.cross(Vec3::new(r.range(-1., 1.), r.range(-1., 1.), r.range(-1., 1.))).normalize_or_zero();
        let tangent = if tangent == Vec3::ZERO { Vec3::X } else { tangent };
        let bitan = normal.cross(tangent);
        let (sx, sy) = (size * r.range(0.8, 1.2) * 0.5, size * r.range(0.8, 1.2) * 0.5);
        let roll = Mat3::from_axis_angle(normal, r.range(0., std::f32::consts::TAU));
        let (t, bt) = (roll * tangent * sx, roll * bitan * sy);
        let nrm = (normal + Vec3::Y * 0.4).normalize();
        b.quad([p - t - bt, p + t - bt, p + t + bt, p - t + bt], [nrm; 4], [255, 255, 255, 255]);
    }
}

/// Broadleaf tree ~8 m tall: bark trunk + masked canopy cards; LOD 0 (48 cards), LOD 1 (16 bigger cards), LOD 2 (6 cards, thin trunk).
/// Material slots: 0 = bark, 1 = foliage. Cull distance 300 m.
pub fn tree_mesh(name: &str, seed: u32) -> StaticMeshData {
    let mut lods = vec![];
    for (k, (cards, size, sides, limit)) in [(48usize, 2.2f32, 8usize, 35.0f32), (16, 3.6, 6, 90.), (6, 5.5, 4, 1e9)].into_iter().enumerate() {
        let (mut bark, mut leaf) = (Builder::default(), Builder::default());
        trunk(&mut bark, 0.32, 0.12, 5.8, sides);
        canopy(&mut leaf, cards, size, 6.6, 2.7, seed.wrapping_add(k as u32 * 101));
        let nb = bark.v.len() as u32;
        let mut vertices = bark.v;
        let mut indices = bark.i;
        let bark_idx = indices.len() as u32;
        indices.extend(leaf.i.iter().map(|i| i + nb));
        vertices.extend(leaf.v);
        let leaf_idx = indices.len() as u32 - bark_idx;
        lods.push(StaticLod { vertices, indices, sections: vec![StaticSection { first_index: 0, index_count: bark_idx, material: 0 }, StaticSection { first_index: bark_idx, index_count: leaf_idx, material: 1 }], max_distance: limit });
    }
    StaticMeshData { name: name.into(), lods, cull_distance: 300., fade_range: 0., cast_shadow: true, fort_default_cull: false }
}

/// Lumpy rock ~1.6 m tall (icosphere, 2 subdivisions, radial noise, flat-ish base), 2 LODs. Material slot 0. Cull distance 220 m.
pub fn rock_mesh(name: &str, seed: u32) -> StaticMeshData {
    let mut lods = vec![];
    for (subdiv, limit) in [(2usize, 45.0f32), (1, 1e9)] {
        let t = (1. + 5f32.sqrt()) / 2.;
        let mut pts: Vec<Vec3> = [(-1., t, 0.), (1., t, 0.), (-1., -t, 0.), (1., -t, 0.), (0., -1., t), (0., 1., t), (0., -1., -t), (0., 1., -t), (t, 0., -1.), (t, 0., 1.), (-t, 0., -1.), (-t, 0., 1.)].iter().map(|p| Vec3::new(p.0, p.1, p.2).normalize()).collect();
        let mut tris: Vec<[usize; 3]> = vec![[0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8], [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1]];
        for _ in 0..subdiv {
            let mut next = vec![];
            let mid = |pts: &mut Vec<Vec3>, a: usize, b: usize| { let m = ((pts[a] + pts[b]) * 0.5).normalize(); pts.push(m); pts.len() - 1 };
            for tr in &tris { let (a, b, c) = (mid(&mut pts, tr[0], tr[1]), mid(&mut pts, tr[1], tr[2]), mid(&mut pts, tr[2], tr[0])); next.extend([[tr[0], a, c], [tr[1], b, a], [tr[2], c, b], [a, b, c]]); }
            tris = next;
        }
        // radial noise shared by direction, so every LOD has the same silhouette
        let shape = |d: Vec3| -> Vec3 {
            let n = ((d.x * 3.1 + seed as f32).sin() * (d.y * 2.7 + 1.3).cos() + (d.z * 4.3 - seed as f32 * 0.3).sin() * 0.6) * 0.18;
            let mut p = d * (1. + n);
            p.y = p.y.max(-0.55);
            Vec3::new(p.x * 1.0, p.y * 0.8 + 0.5, p.z * 0.9)
        };
        let mut b = Builder::default();
        for tr in &tris {
            let p: Vec<Vec3> = tr.iter().map(|&i| shape(pts[i])).collect();
            let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            let c = (p[0] + p[1] + p[2]) / 3.;
            let n = if n.dot(c - Vec3::new(0., 0.5, 0.)) < 0. { -n } else { n };
            for (j, q) in p.iter().enumerate() {
                let uv = [q.x * 0.5 + q.y * 0.3, q.z * 0.5 + q.y * 0.3];
                let _ = j;
                b.vert(*q, n, uv, [255, 255, 255, 255]);
            }
            let k = b.v.len() as u32;
            if (p[1] - p[0]).cross(p[2] - p[0]).dot(c - Vec3::new(0., 0.5, 0.)) >= 0. { b.i.extend_from_slice(&[k - 3, k - 2, k - 1]); } else { b.i.extend_from_slice(&[k - 3, k - 1, k - 2]); }
        }
        let n = b.i.len() as u32;
        lods.push(StaticLod { vertices: b.v, indices: b.i, sections: vec![StaticSection { first_index: 0, index_count: n, material: 0 }], max_distance: limit });
    }
    StaticMeshData { name: name.into(), lods, cull_distance: 220., fade_range: 0., cast_shadow: true, fort_default_cull: false }
}

/// A 2 m x 2 m vertical quad (base at y = 0) with the checker alpha texture, normal +Z, one LOD, one masked two-sided material slot.
pub fn checker_quad_mesh(name: &str) -> StaticMeshData {
    let mut b = Builder::default();
    let n = Vec3::Z;
    b.quad([Vec3::new(-1., 0., 0.), Vec3::new(1., 0., 0.), Vec3::new(1., 2., 0.), Vec3::new(-1., 2., 0.)], [n; 4], [255; 4]);
    let c = b.i.len() as u32;
    StaticMeshData { name: name.into(), lods: vec![StaticLod { vertices: b.v, indices: b.i, sections: vec![StaticSection { first_index: 0, index_count: c, material: 0 }], max_distance: 1e9 }], cull_distance: 0., fade_range: 0., cast_shadow: true, fort_default_cull: false }
}
pub fn checker_quad_material() -> StaticMaterialData {
    StaticMaterialData { name: "checker".into(), base_color: Some(checker_alpha_texture()), alpha_mode: AlphaMode::Masked, alpha_cutoff: 0.5, two_sided: true, specular: 0., ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_has_three_lods_with_fewer_triangles_and_two_sections() {
        let t = tree_mesh("t", 3);
        assert_eq!(t.lods.len(), 3);
        let tris: Vec<usize> = t.lods.iter().map(|l| l.indices.len() / 3).collect();
        assert!(tris[0] > tris[1] && tris[1] > tris[2], "{tris:?}");
        assert!(t.lods.iter().all(|l| l.sections.len() == 2 && l.sections.iter().map(|s| s.index_count).sum::<u32>() as usize == l.indices.len()));
        assert!(t.lods.iter().all(|l| l.indices.iter().all(|&i| (i as usize) < l.vertices.len())));
        let (_, r, h) = mesh_bounds(&t);
        assert!(h > 8. && r > 3., "{r} {h}");
    }
    #[test]
    fn trunk_triangles_wind_counter_clockwise_seen_from_outside() {
        let t = tree_mesh("t", 3);
        let l = &t.lods[0];
        let s = &l.sections[0];
        for tri in l.indices[..s.index_count as usize].chunks(3) {
            let p: Vec<Vec3> = tri.iter().map(|&i| Vec3::from(l.vertices[i as usize].pos)).collect();
            let gn = (p[1] - p[0]).cross(p[2] - p[0]);
            let radial = Vec3::new((p[0].x + p[1].x + p[2].x) / 3., 0., (p[0].z + p[1].z + p[2].z) / 3.);
            assert!(gn.dot(radial) > 0., "trunk face points inward");
        }
    }
    #[test]
    fn rock_normals_point_outward_and_winding_matches() {
        let m = rock_mesh("r", 1);
        let l = &m.lods[0];
        for tri in l.indices.chunks(3) {
            let p: Vec<Vec3> = tri.iter().map(|&i| Vec3::from(l.vertices[i as usize].pos)).collect();
            let gn = (p[1] - p[0]).cross(p[2] - p[0]);
            let c = (p[0] + p[1] + p[2]) / 3. - Vec3::new(0., 0.5, 0.);
            assert!(gn.dot(c) >= 0., "winding must be CCW seen from outside");
            assert!(Vec3::from(l.vertices[tri[0] as usize].normal).dot(c) >= 0.);
        }
    }
    #[test]
    fn leaf_texture_is_partly_transparent() {
        let img = leaf_texture(1);
        let c = coverage(&img, 0.5);
        assert!(c > 0.15 && c < 0.85, "coverage {c}");
    }
}
