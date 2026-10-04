//! Procedural props of the old runtime (placeholder trees and rocks, `fn_combat::prop_mesh`) as renderer vertices / instances.
use fn_combat::Kind;
use fn_render::PropInstance;
use glam::Vec3;

/// World-space triangle list of every live prop (shake sway included), one flat normal per triangle, vertex colour as albedo.
/// Reference expansion straight from fn_combat's helpers; the viewer itself feeds `Renderer::set_prop_instances`, and a test proves both agree.
#[cfg(test)]
use fn_combat::{prop_mesh, prop_normal, prop_vertex, World};
#[cfg(test)]
use fn_render::PropVertex;
#[cfg(test)]
pub fn prop_vertices(world: &World) -> Vec<PropVertex> {
    let mut out = vec![];
    for p in world.props.iter().filter(|p| p.alive()) {
        for (tri, color) in prop_mesh(p.kind) {
            let n = prop_normal(p, &tri);
            for v in tri {
                let w = prop_vertex(p, v);
                out.push(PropVertex { pos: w.to_array(), normal: n.to_array(), color });
            }
        }
    }
    out
}

/// `n` deterministic pseudo-random props (about 2/3 trees, 1/3 rocks) with random yaw and 0.8..1.3 scale, scattered over the ground in
/// front of the spawn (x -45..45, z -50..20), keeping a clear 3 m disc around the pawn and the sandbox props; every 7th one is shaking.
pub fn scatter(n: usize, seed: u32) -> Vec<PropInstance> {
    let mut s = seed.wrapping_mul(2654435761).wrapping_add(12345);
    let mut rnd = move || { s ^= s << 13; s ^= s >> 17; s ^= s << 5; (s >> 8) as f32 / (1u32 << 24) as f32 };
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let (x, z) = (rnd() * 90. - 45., rnd() * 70. - 50.);
        let pos = Vec3::new(x, 0., z);
        if (pos - Vec3::new(0., 0., 12.)).length() < 3. { continue; }
        let kind = if rnd() < 0.67 { Kind::Tree } else { Kind::Rock };
        let k = out.len();
        out.push(PropInstance::new(kind, pos).with_yaw(rnd() * std::f32::consts::TAU).with_scale(0.8 + rnd() * 0.5).with_shake(if k % 7 == 0 { 0.2 } else { 0. }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sandbox_has_two_trees_and_a_rock_with_unit_normals() {
        let w = World::sandbox();
        let v = prop_vertices(&w);
        assert_eq!(v.len() % 3, 0);
        assert!(v.len() > 3 * 16 * 3);
        assert!(v.iter().all(|p| (glam::Vec3::from(p.normal).length() - 1.).abs() < 1e-4));
    }
    #[test]
    fn instances_reproduce_the_reference_expansion() {
        let mut w = World::sandbox();
        w.props[1].shake = 0.25;
        let want = prop_vertices(&w);
        let got = fn_render::PropTemplates::default().build(&PropInstance::from_world(&w));
        assert_eq!(got.len(), want.len());
        assert!(got.iter().zip(&want).all(|(a, b)| (0..3).all(|k| (a.pos[k] - b.pos[k]).abs() < 1e-5 && (a.normal[k] - b.normal[k]).abs() < 1e-5)));
    }
    #[test]
    fn scatter_is_deterministic_and_counts_match() {
        assert_eq!(scatter(50, 1), scatter(50, 1));
        assert_ne!(scatter(50, 1), scatter(50, 2));
        assert_eq!(scatter(50, 1).len(), 50);
        assert!(scatter(0, 1).is_empty());
        assert!(scatter(50, 1).iter().any(|p| p.kind == Kind::Rock) && scatter(50, 1).iter().any(|p| p.shake > 0.));
    }
}
