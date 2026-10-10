//! ARCH AUDIT (2026-10-07): does the collision the runtime builds (one `fn_hull::Hull` per cooked `FKConvexElem`, the same rule as fn-app `props::hulls_of`)
//! follow the RENDER mesh of every exported prop kind? A grid of rays along the three axes is cast through each kind's bounds; a ray that enters a hull but
//! no render triangle is "phantom" collision (a hull bridging an opening: an arch, the gap under a truck), a ray that meets triangles but no hull is a hole.
//! Run: `FN_PROPS_DIR=<assets>/props cargo test -p fn-props --test hull_vs_render -- --ignored --nocapture` (needs the exported props; skipped without).
use fn_hull::Hull;
use fn_propdata::{PropCatalog, Shape};
use glam::{Quat, Vec3};

fn hulls(c: &fn_propdata::Collision) -> Vec<Hull> {
    let ue = |p: Vec3| Vec3::new(p.x, p.z, p.y) * 0.01;
    c.shapes.iter().filter_map(|s| match s {
        Shape::Convex { vertices, transform_translation, transform_rotation, transform_scale, .. } => {
            let (r, sc, t) = (Quat::from_array(*transform_rotation).normalize(), Vec3::from(*transform_scale), Vec3::from(*transform_translation));
            Hull::from_points(&vertices.iter().map(|v| ue(r * (Vec3::from(*v) * sc) + t)).collect::<Vec<_>>())
        }
        Shape::Box { center, rotation, half_extent } => {
            let (r, c, h) = (Quat::from_array(*rotation).normalize(), Vec3::from(*center), Vec3::from(*half_extent));
            let mut v = vec![]; for x in [-1., 1.] { for y in [-1., 1.] { for z in [-1., 1.] { v.push(ue(c + r * (h * Vec3::new(x, y, z)))); } } }
            Hull::from_points(&v)
        }
        _ => None,
    }).collect()
}

fn tri_hit(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> bool {
    let (e1, e2) = (b - a, c - a); let p = d.cross(e2); let det = e1.dot(p);
    if det.abs() < 1e-12 { return false; }
    let inv = 1. / det; let s = o - a; let u = s.dot(p) * inv; if !(0. ..=1.).contains(&u) { return false; }
    let q = s.cross(e1); let v = d.dot(q) * inv; if v < 0. || u + v > 1. { return false; }
    e2.dot(q) * inv >= 0.
}

#[test]
#[ignore]
fn collision_follows_the_render_mesh() {
    let Ok(dir) = std::env::var("FN_PROPS_DIR") else { eprintln!("FN_PROPS_DIR unset: skipped"); return };
    let cat = PropCatalog::load(&dir).expect("catalog");
    let mut rows = vec![];
    for k in &cat.kinds {
        let hs = hulls(&k.collision);
        let Ok(lods) = k.load_lods() else { continue }; let Some(lod) = lods.first() else { continue };
        let pos: Vec<Vec3> = lod.positions.iter().map(|p| Vec3::from(*p)).collect();
        let tris: Vec<[Vec3; 3]> = lod.sections.iter().flat_map(|s| s.indices.chunks_exact(3).map(|t| [pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]]).collect::<Vec<_>>()).collect();
        let (mn, mx) = lod.bounds(); let (mn, mx) = (Vec3::from(mn) - Vec3::splat(0.05), Vec3::from(mx) + Vec3::splat(0.05));
        let n = 24; let (mut phantom, mut hole, mut both) = (0, 0, 0);
        for axis in 0..3 {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            for i in 0..n { for j in 0..n {
                let mut o = Vec3::ZERO; o[axis] = mn[axis] - 0.1; o[u] = mn[u] + (mx[u] - mn[u]) * (i as f32 + 0.5) / n as f32; o[v] = mn[v] + (mx[v] - mn[v]) * (j as f32 + 0.5) / n as f32;
                let mut d = Vec3::ZERO; d[axis] = 1.; let len = mx[axis] - mn[axis] + 0.2;
                let col = hs.iter().any(|h| h.ray(o, d, len).is_some() || h.sdf(o) <= 0.);
                let ren = tris.iter().any(|t| tri_hit(o, d, t[0], t[1], t[2]));
                match (col, ren) { (true, false) => phantom += 1, (false, true) => hole += 1, (true, true) => both += 1, _ => {} }
            } }
        }
        let tot = (phantom + both).max(1) as f32;
        rows.push((phantom as f32 / tot, k.id.clone(), hs.len(), k.collision.shapes.len(), phantom, hole, both, k.collision.trace_flag.clone()));
    }
    rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    println!("{:<44} {:>6} {:>5} {:>7} {:>5} {:>5}", "kind", "hulls", "elems", "phantom", "hole", "both");
    for r in &rows { println!("{:<44} {:>6} {:>5} {:>6.0}% {:>5} {:>5} {}", r.1, r.2, r.3, r.0 * 100., r.5, r.6, r.7); }
}
