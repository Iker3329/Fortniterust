//! STORM HOOK: the numbers and the CPU-side buffer contents of the 1.8 Battle Royale storm visuals (the safe-zone wall and the storm cloud sheet).
//!
//! The renderer draws nothing storm-specific itself: the wall (`SafeZoneIndicator` -> `SafeZoneCylinder` mesh with `M_SafeZoneSphere`) and the 28 `StormQuadMain`
//! cloud quads of `BP_SafeZoneStorm_1322` (`M_SafeZoneStorm`) go through the generic FX pass ([`crate::fx`]) with the ORIGINAL base-pass programs; the application
//! (`fn-matshader::storm`) builds that frame from the plain data here. This module only holds what is recovered about the indicator Blueprint / native actor and the
//! math that turns a zone (centre, radius, opacity) into the uniform buffers the original programs read.
//!
//! RECOVERED (package data + exe):
//! * `AFortSafeZoneIndicator::Tick` (exe `14044f390`) sets the actor scale to `(Radius, Radius, 1000)` (`DAT_143fe3734` = 1000.0, the same constant as the `Slate.ToolTipWrapWidth`
//!   default) and the actor location to the interpolated zone centre. The `SafeZoneCylinder` component (mesh `SafeZoneCylinder`: a unit-ish tube, bounds +-50 x +-50 x +-50 cm)
//!   has relative scale `(0.02, 0.02, 0.4)`, so its world scale is `(0.02 R, 0.02 R, 400)`: a tube of radius `R` cm and height 400 m centred on the zone centre's height.
//! * `SafeZoneIndicator` Blueprint (`ReceiveTick`): `MPC_SafeZoneClouds.SafeZoneLoc` = actor location as a linear colour, `SafeZoneScale` = component world scale X /
//!   `InitialSafeZoneWorldScale` (3700), `MasterOpacity` from the `StormFadeIn` timeline (0 -> 1 linearly over 25 s, started by `ReceiveBeginPlay`).
//! * `MPC_SafeZoneClouds` layout (UE 4.16 collection uniform buffer: scalars packed four per float4, then the vectors): float4 0 = `(SafeZoneScale, MasterOpacity, 0, 0)`,
//!   float4 1 = `SafeZoneLoc`.
//! * `BP_SafeZoneStorm` has 28 `StaticMeshComponent`s showing `StormQuadMain` (all attached to the root): a 4 x 4 inner grid of scale 2 at z = 250 m and a 12 quad outer ring of scale 4 at z = 260 m (614 m / 1229 m cells);
//!   `BP_SafeZoneStorm_1322` sits at (-289.965, 168.029, 0) cm.
use glam::{Mat4, Vec3};

/// `InitialSafeZoneWorldScale` (`SafeZoneIndicator` CDO).
pub const INITIAL_SAFE_ZONE_WORLD_SCALE: f32 = 3700.0;
/// Actor scale Z the native `Tick` applies (`DAT_143fe3734`).
pub const ACTOR_SCALE_Z: f32 = 1000.0;
/// `SafeZoneCylinder_GEN_VARIABLE.RelativeScale3D`.
pub const CYLINDER_RELATIVE_SCALE: [f32; 3] = [0.02, 0.02, 0.4];
/// `BP_SafeZoneStorm_1322` root (level instance, `DefaultSceneRoot.RelativeLocation`), UE cm.
pub const STORM_ACTOR_CM: [f32; 3] = [-289.96533, 168.0288, 0.0];

/// A zone as the indicator holds it, in the FX space (Unreal axes / centimetres, already shifted onto the port's baseplate, see `fn-app/src/storm_hook.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StormParams {
    pub center_cm: [f32; 3],
    pub radius_cm: f32,
    /// `MPC_SafeZoneClouds.MasterOpacity`.
    pub master_opacity: f32,
}

impl StormParams {
    /// Actor scale the native tick sets.
    pub fn actor_scale(&self) -> [f32; 3] { [self.radius_cm, self.radius_cm, ACTOR_SCALE_Z] }
    /// `K2_GetComponentScale` of the cylinder component.
    pub fn cylinder_world_scale(&self) -> [f32; 3] { let a = self.actor_scale(); [a[0] * CYLINDER_RELATIVE_SCALE[0], a[1] * CYLINDER_RELATIVE_SCALE[1], a[2] * CYLINDER_RELATIVE_SCALE[2]] }
    /// `MPC_SafeZoneClouds.SafeZoneScale`.
    pub fn safe_zone_scale(&self) -> f32 { self.cylinder_world_scale()[0] / INITIAL_SAFE_ZONE_WORLD_SCALE }
    /// Contents of the `MaterialCollection0` buffer of `MPC_SafeZoneClouds` (2 float4s).
    pub fn collection_buffer(&self) -> [[f32; 4]; 2] {
        [[self.safe_zone_scale(), self.master_opacity, 0.0, 0.0], [self.center_cm[0], self.center_cm[1], self.center_cm[2], 1.0]]
    }
    /// `LocalToWorld` of the cylinder component (UE local cm -> FX world cm): scale (0.02 R, 0.02 R, 400), translation = the zone centre.
    pub fn cylinder_local_to_world(&self) -> Mat4 {
        Mat4::from_translation(Vec3::from(self.center_cm)) * Mat4::from_scale(Vec3::from(self.cylinder_world_scale()))
    }
}


// The storm's own `Primitive` builder was removed (it put the orientation in 9.xyz, left 11..13 / 15..17 zero, used 16 float4s and a half-box-diagonal radius):
// every draw now uses the shared UE4.16 `GetPrimitiveUniformShaderParameters` port, `crate::primitive_ub`.
pub use crate::primitive_ub::PRIMITIVE_FLOAT4S;

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn the_first_phase_is_scale_one_at_radius_185000() {
        // SafeZoneScale = 0.02 R / 3700 -> 1 at R = 185000 cm (the Blueprint's "initial" scale); the data's first phase is 150000 cm
        let p = StormParams { center_cm: [32224.0, -25744.0, 1312.0], radius_cm: 185_000.0, master_opacity: 1.0 };
        assert!((p.safe_zone_scale() - 1.0).abs() < 1e-6);
        let q = StormParams { radius_cm: 150_000.0, ..p };
        assert!((q.safe_zone_scale() - 150_000.0 * 0.02 / 3700.0).abs() < 1e-6);
        assert_eq!(q.collection_buffer()[1], [32224.0, -25744.0, 1312.0, 1.0]);
    }
    #[test] fn the_cylinder_is_a_tube_of_radius_r_and_400_m() {
        let p = StormParams { center_cm: [10.0, 20.0, 30.0], radius_cm: 1000.0, master_opacity: 1.0 };
        let m = p.cylinder_local_to_world();
        // the mesh's rim (local x = 50) lands at R from the centre, its top (local z = 50) 200 m above it
        let rim = m.transform_point3(Vec3::new(50.0, 0.0, 0.0));
        assert!((rim - Vec3::new(1010.0, 20.0, 30.0)).length() < 1e-3, "{rim:?}");
        let top = m.transform_point3(Vec3::new(0.0, 0.0, 50.0));
        assert!((top.z - (30.0 + 20_000.0)).abs() < 1e-2, "{top:?}");
    }
    // (the Primitive layout test lives with the shared builder: `crate::primitive_ub::tests`)
}
