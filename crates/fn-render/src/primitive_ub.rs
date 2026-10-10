//! PRIMITIVE UB: the `Primitive` uniform buffer (`FPrimitiveUniformShaderParameters`) of the original material programs, filled exactly as UE4.16
//! `GetPrimitiveUniformShaderParameters` (PrimitiveUniformShaderParameters.h:35) does from what `FPrimitiveSceneProxy::UpdateUniformBuffer` (PrimitiveSceneProxy.cpp:251) passes.
//! One builder for every program that reads `Primitive` (storm, guns, scope, pickups, building, lobby): the old copies filled 16 float4s with orientation in 9.xyz,
//! no flags / orientation / scale / local bounds and a half-box-diagonal radius.
//!
//! Layout (uniform buffer struct packing: FVector / FVector4 / FMatrix align to 16 bytes, float / uint32 to 4; UniformBuffer.h:430):
//! ```text
//!  0..3  LocalToWorld (row-vector rows)            4..7  WorldToLocal
//!  8     ObjectWorldPositionAndRadius (world bounds origin, sphere radius)
//!  9     xyz ObjectBounds (world box extent)       w LocalToWorldDeterminantSign
//! 10     xyz ActorWorldPosition                    w DecalReceiverMask
//! 11     x PerObjectGBufferData  y UseSingleSampleShadowFromStationaryLights  z UseEditorDepthTest
//! 12     ObjectOrientation (unit Z axis, w = 1: FVector -> FVector4 default W)
//! 13     NonUniformScale (w 0)                    14     InvNonUniformScale (w 0)
//! 15     xyz LocalObjectBoundsMin                 16     xyz LocalObjectBoundsMax, w LightingChannelMask (uint32 bits)
//! 17     x LpvBiasMultiplier                      (18 float4 = 288 bytes)
//! ```
use glam::{Mat4, Vec3};

pub const PRIMITIVE_FLOAT4S: usize = 18;

/// `FBoxSphereBounds` (origin, box extent, sphere radius), UE cm.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct BoxSphereBounds { pub origin: [f32; 3], pub extent: [f32; 3], pub radius: f32 }
impl BoxSphereBounds {
    /// `FBoxSphereBounds::TransformBy(const FMatrix&)` (BoxSphereBounds.cpp:16): origin transformed, extent = sum of |extent_i * row_i|, radius scaled by the
    /// largest row length. `m` is the glam (column-vector) matrix: its columns are the UE rows.
    pub fn transform_by(&self, m: &Mat4) -> BoxSphereBounds {
        let rows = [m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate()];
        let o = Vec3::from(self.origin);
        let origin = rows[0] * o.x + rows[1] * o.y + rows[2] * o.z + m.w_axis.truncate();
        let extent = (rows[0] * self.extent[0]).abs() + (rows[1] * self.extent[1]).abs() + (rows[2] * self.extent[2]).abs();
        let max_sq = rows.iter().map(|r| *r * *r).fold(Vec3::ZERO, |a, b| a + b).max_element();
        BoxSphereBounds { origin: origin.to_array(), extent: extent.to_array(), radius: max_sq.sqrt() * self.radius }
    }
    /// `FBoxSphereBounds(const FBox&)` (BoxSphereBounds.h:62): origin = box centre, extent = half size, sphere radius = `BoxExtent.Size()`.
    pub fn from_box(min: [f32; 3], max: [f32; 3]) -> BoxSphereBounds {
        let e: [f32; 3] = std::array::from_fn(|k| (max[k] - min[k]) * 0.5);
        BoxSphereBounds { origin: std::array::from_fn(|k| (max[k] + min[k]) * 0.5), extent: e, radius: (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt() }
    }
    /// `FBoxSphereBounds(Box.TransformBy(M))`: an `FBox` moved by `m` (`FBox::TransformBy`, Box.cpp: centre transformed, extent = sum of |extent_i * row_i|), then
    /// re-boxed (the sphere is the new box's, not the scaled old sphere as in [`Self::transform_by`]).
    pub fn box_transform_by(min: [f32; 3], max: [f32; 3], m: &Mat4) -> BoxSphereBounds { let b = Self::from_box(min, max).transform_by(m); Self::from_box(b.min(), b.max()) }
    /// `GetBoxExtrema(0)` / `(1)`.
    pub fn min(&self) -> [f32; 3] { std::array::from_fn(|k| self.origin[k] - self.extent[k]) }
    pub fn max(&self) -> [f32; 3] { std::array::from_fn(|k| self.origin[k] + self.extent[k]) }
}

/// The proxy flags `GetPrimitiveUniformShaderParameters` takes. `Default` = a component with engine defaults: `bReceivesDecals` true (PrimitiveComponent.cpp:169),
/// no distance-field / capsule representation, no forced single-sample shadowing, `bUseEditorDepthTest` true (PrimitiveSceneProxy.cpp:85), lighting channel 0 only
/// (`GetDefaultLightingChannelMask` = 1), `LpvBiasMultiplier` 1 (PrimitiveComponent.cpp:174).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimitiveFlags { pub receives_decals: bool, pub has_distance_field: bool, pub has_capsule: bool, pub single_sample_shadow_from_stationary: bool, pub use_editor_depth_test: bool, pub lighting_channel_mask: u32, pub lpv_bias_multiplier: f32 }
impl Default for PrimitiveFlags {
    fn default() -> Self { PrimitiveFlags { receives_decals: true, has_distance_field: false, has_capsule: false, single_sample_shadow_from_stationary: false, use_editor_depth_test: true, lighting_channel_mask: 1, lpv_bias_multiplier: 1.0 } }
}

/// `GetPrimitiveUniformShaderParameters`. `l2w` = local -> world (glam column-vector matrix; its columns are the UE row-vector rows).
pub fn primitive_uniform(l2w: &Mat4, actor_position: [f32; 3], world: &BoxSphereBounds, local: &BoxSphereBounds, flags: &PrimitiveFlags) -> Vec<[f32; 4]> {
    let mut v = vec![[0f32; 4]; PRIMITIVE_FLOAT4S];
    let cols = [l2w.x_axis, l2w.y_axis, l2w.z_axis, l2w.w_axis];
    for i in 0..4 { v[i] = cols[i].to_array(); }
    let inv = l2w.inverse();
    let icols = [inv.x_axis, inv.y_axis, inv.z_axis, inv.w_axis];
    for i in 0..4 { v[4 + i] = icols[i].to_array(); }
    v[8] = [world.origin[0], world.origin[1], world.origin[2], world.radius];
    // RotDeterminant of the 3x3 part; FloatSelect(x, 1, -1) = x >= 0 ? 1 : -1
    let det = glam::Mat3::from_mat4(*l2w).determinant();
    v[9] = [world.extent[0], world.extent[1], world.extent[2], if det >= 0.0 { 1.0 } else { -1.0 }];
    v[10] = [actor_position[0], actor_position[1], actor_position[2], if flags.receives_decals { 1.0 } else { 0.0 }];
    v[11] = [(2 * flags.has_capsule as i32 + flags.has_distance_field as i32) as f32 / 3.0, if flags.single_sample_shadow_from_stationary { 1.0 } else { 0.0 }, if flags.use_editor_depth_test { 1.0 } else { 0.0 }, 0.0];
    // GetUnitAxis(Z) = GetScaledAxis(Z).GetSafeNormal() (zero below SMALL_NUMBER squared length)
    let z = cols[2].truncate();
    let z = if z.length_squared() > 1e-8 { z.normalize() } else { Vec3::ZERO };
    v[12] = [z.x, z.y, z.z, 1.0];
    let s = [cols[0].truncate().length(), cols[1].truncate().length(), cols[2].truncate().length()];
    v[13] = [s[0], s[1], s[2], 0.0];
    // KINDA_SMALL_NUMBER = 1e-4
    let inv_s = |x: f32| if x > 1e-4 { 1.0 / x } else { 0.0 };
    v[14] = [inv_s(s[0]), inv_s(s[1]), inv_s(s[2]), 0.0];
    let (mn, mx) = (local.min(), local.max());
    v[15] = [mn[0], mn[1], mn[2], 0.0];
    v[16] = [mx[0], mx[1], mx[2], f32::from_bits(flags.lighting_channel_mask)];
    v[17] = [flags.lpv_bias_multiplier, 0.0, 0.0, 0.0];
    v
}

/// The common case of a mesh component: `FPrimitiveSceneProxy` with `Bounds` = `CalcBounds(LocalToWorld)` = the mesh bounds transformed (BoundsScale 1,
/// StaticMeshComponent.cpp:471) and `LocalBounds` = the mesh bounds; the actor position is the component's owner location.
pub fn mesh_primitive(l2w: &Mat4, mesh_bounds: &BoxSphereBounds, actor_position: [f32; 3], flags: &PrimitiveFlags) -> Vec<[f32; 4]> {
    primitive_uniform(l2w, actor_position, &mesh_bounds.transform_by(l2w), mesh_bounds, flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn layout_follows_get_primitive_uniform_shader_parameters() {
        // the safe-zone cylinder: mesh bounds +-50 cm (sphere 70.71), world scale (0.02 R, 0.02 R, 400), R = 1000 m
        let local = BoxSphereBounds { origin: [0.; 3], extent: [50.; 3], radius: 70.71 };
        let l2w = Mat4::from_translation(Vec3::new(10., 20., 30.)) * Mat4::from_scale(Vec3::new(2000., 2000., 400.));
        let p = mesh_primitive(&l2w, &local, [10., 20., 30.], &PrimitiveFlags::default());
        assert_eq!(p.len(), 18);
        assert_eq!(p[3], [10., 20., 30., 1.]);
        assert!((p[8][3] - 70.71 * 2000.).abs() < 1e-2, "sphere radius = cooked radius * max axis scale");
        assert_eq!(p[9], [100_000., 100_000., 20_000., 1.]);
        assert_eq!(p[10], [10., 20., 30., 1.]);
        assert_eq!(p[11], [0., 0., 1., 0.]);
        assert_eq!(p[12], [0., 0., 1., 1.]);
        assert_eq!(p[13], [2000., 2000., 400., 0.]);
        assert_eq!(p[15], [-50., -50., -50., 0.]);
        assert_eq!(p[16][3].to_bits(), 1);
        assert_eq!(p[17][0], 1.0);
        // mirrored transform: negative determinant sign
        let m = mesh_primitive(&Mat4::from_scale(Vec3::new(-1., 1., 1.)), &local, [0.; 3], &PrimitiveFlags::default());
        assert_eq!(m[9][3], -1.0);
    }
}
