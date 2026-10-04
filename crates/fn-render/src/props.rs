//! Prop instances (the old runtime's procedural trees and rocks, `fn_combat::prop_mesh`) as world-space renderer vertices.
//!
//! `Renderer::set_prop_instances(&[PropInstance])` is the one-call API: it expands every instance (position, yaw about Y, uniform scale,
//! shake sway) into flat-normal, vertex-coloured triangles and uploads them; the renderer draws them with the static-prop pipeline
//! (`fs_prop`: sun diffuse with cascaded shadows, sky hemisphere ambient, height fog; they cast and receive shadows).
use crate::renderer::PropVertex;
use fn_combat::{prop_mesh, Harvestable, Kind};
use glam::{Quat, Vec3};

/// One placed prop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropInstance {
    pub kind: Kind,
    /// World position of the prop's base (metres, y up).
    pub pos: Vec3,
    /// Rotation about +Y in radians.
    pub yaw: f32,
    /// Uniform scale (1 = the old runtime's size).
    pub scale: f32,
    /// Remaining shake time in seconds (`Harvestable::shake`, 0.3 right after a hit); drives the sway of `fn_combat::prop_wobble`.
    pub shake: f32,
}
impl PropInstance {
    pub fn new(kind: Kind, pos: Vec3) -> Self { PropInstance { kind, pos, yaw: 0., scale: 1., shake: 0. } }
    pub fn with_yaw(mut self, yaw: f32) -> Self { self.yaw = yaw; self }
    pub fn with_scale(mut self, scale: f32) -> Self { self.scale = scale; self }
    pub fn with_shake(mut self, shake: f32) -> Self { self.shake = shake; self }
    /// An instance for a live `fn_combat` harvestable (yaw 0, scale 1, its current shake).
    pub fn from_harvestable(h: &Harvestable) -> Self { PropInstance { kind: h.kind, pos: h.pos, yaw: 0., scale: 1., shake: h.shake } }
    /// Every live prop of a combat world.
    pub fn from_world(world: &fn_combat::World) -> Vec<PropInstance> { world.props.iter().filter(|p| p.alive()).map(Self::from_harvestable).collect() }
}

/// Cached prop-local templates (computing `prop_mesh` allocates; the renderer keeps one of these).
pub struct PropTemplates { tree: Vec<([Vec3; 3], [f32; 3])>, rock: Vec<([Vec3; 3], [f32; 3])> }
impl Default for PropTemplates { fn default() -> Self { PropTemplates { tree: prop_mesh(Kind::Tree), rock: prop_mesh(Kind::Rock) } } }
impl PropTemplates {
    pub fn mesh(&self, kind: Kind) -> &[([Vec3; 3], [f32; 3])] { match kind { Kind::Tree => &self.tree, Kind::Rock => &self.rock } }
    pub fn triangles_per(&self, kind: Kind) -> usize { self.mesh(kind).len() }

    /// Appends the world-space triangles of `inst` to `out` (flat outward normals, linear vertex colour).
    pub fn append(&self, inst: &PropInstance, out: &mut Vec<PropVertex>) {
        let height = Harvestable::new(inst.kind, Vec3::ZERO).height * inst.scale;
        let wobble = fn_combat::prop_wobble(&Harvestable { shake: inst.shake, ..Harvestable::new(inst.kind, inst.pos) }) * inst.scale;
        let rot = Quat::from_rotation_y(inst.yaw);
        let core = Vec3::new(0., height * 0.4, 0.);
        for (tri, color) in self.mesh(inst.kind) {
            let l = tri.map(|v| rot * (v * inst.scale));
            let mut n = (l[1] - l[0]).cross(l[2] - l[0]).normalize_or_zero();
            if n.dot((l[0] + l[1] + l[2]) / 3. - core) < 0. { n = -n; }
            for v in l {
                let sway = v.y / height.max(0.1) * wobble;
                let w = inst.pos + v + Vec3::new(sway, 0., 0.);
                out.push(PropVertex { pos: w.to_array(), normal: n.to_array(), color: *color });
            }
        }
    }
    pub fn build(&self, instances: &[PropInstance]) -> Vec<PropVertex> {
        let mut out = Vec::with_capacity(instances.iter().map(|i| self.triangles_per(i.kind) * 3).sum());
        for i in instances { self.append(i, &mut out); }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_combat::{prop_normal, prop_vertex, World};

    /// Reference expansion written exactly like fn_combat's own helpers (no yaw/scale).
    fn reference(world: &World) -> Vec<PropVertex> {
        let mut out = vec![];
        for p in world.props.iter().filter(|p| p.alive()) {
            for (tri, color) in prop_mesh(p.kind) {
                let n = prop_normal(p, &tri);
                for v in tri { out.push(PropVertex { pos: prop_vertex(p, v).to_array(), normal: n.to_array(), color }); }
            }
        }
        out
    }
    #[test]
    fn identity_instances_match_the_combat_reference_including_shake() {
        let mut w = World::sandbox();
        w.props[0].shake = 0.2;
        let got = PropTemplates::default().build(&PropInstance::from_world(&w));
        let want = reference(&w);
        assert_eq!(got.len(), want.len());
        for (a, b) in got.iter().zip(&want) {
            for k in 0..3 { assert!((a.pos[k] - b.pos[k]).abs() < 1e-5 && (a.normal[k] - b.normal[k]).abs() < 1e-5 && a.color[k] == b.color[k]); }
        }
    }
    #[test]
    fn yaw_and_scale_move_vertices_and_keep_unit_outward_normals() {
        let t = PropTemplates::default();
        let a = t.build(&[PropInstance::new(Kind::Tree, Vec3::new(10., 0., 5.)).with_yaw(1.0).with_scale(2.0)]);
        assert!(a.iter().all(|v| (Vec3::from(v.normal).length() - 1.).abs() < 1e-4));
        let top = a.iter().map(|v| v.pos[1]).fold(0f32, f32::max);
        assert!((top - 6.6 * 2.0).abs() < 1e-4, "scaled apex {top}");
        // every vertex stays within the scaled canopy radius of the trunk axis
        assert!(a.iter().all(|v| ((v.pos[0] - 10.).powi(2) + (v.pos[2] - 5.).powi(2)).sqrt() <= 1.9 * 2.0 + 1e-3));
        // normals face away from the core
        assert!(a.chunks(3).all(|c| { let m = (Vec3::from(c[0].pos) + Vec3::from(c[1].pos) + Vec3::from(c[2].pos)) / 3. - Vec3::new(10., 4.5 * 2.0 * 0.4, 5.); Vec3::from(c[0].normal).dot(m) >= 0. }));
    }
    #[test]
    fn triangle_counts_and_empty_list() {
        let t = PropTemplates::default();
        assert!(t.build(&[]).is_empty());
        let v = t.build(&[PropInstance::new(Kind::Rock, Vec3::ZERO), PropInstance::new(Kind::Tree, Vec3::X)]);
        assert_eq!(v.len(), 3 * (t.triangles_per(Kind::Rock) + t.triangles_per(Kind::Tree)));
    }
}
