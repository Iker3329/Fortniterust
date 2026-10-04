//! `AnimNode_AnimDynamics` (UE 4.16, AnimGraphRuntime) for the Battle Royale character parts' own AnimBlueprints.
//!
//! WHAT IS DATA: every field of [`AnimDynamicsParams`] is read from the 1.8 packages (the part AnimBP's class default object, struct
//! `AnimNode_AnimDynamics`), see `fn_pose::part_anim`. The Alpha input pin of each node is an exposed input evaluated from the AnimBP's own
//! bytecode (`MapRangeClamped(PawnSpeed2D, ...)`), also in `fn_pose::part_anim`.
//!
//! WHAT IS NOT DATA (engine code, only present in the 1.8 exe's `FAnimPhys` solver, NOT decompiled here): the integrator and the way a body
//! is built from the node. This module is a position-based re-implementation of the node's documented behaviour, with these explicit,
//! non-recovered constants in [`SolverAssumptions`]: body mass (unit mass; box inertia from `BoxExtents`), linear / angular damping, gravity
//! (the world gravity, 1.8 DefaultEngine.ini DefaultGravityZ -2800 cm/s^2 along component -Z, scaled by `GravityScale`), the largest simulated step. Wind (`bEnableWind`, `WindScale`) needs
//! the level's wind source (not in the data): it contributes nothing. `bChain`, spherical limits, planar limits with entries and
//! `Cone` angular constraints do not occur in the Battle Royale part AnimBPs and are reported by [`AnimDynamicsParams::unsupported`].
//!
//! Semantics ported: a node simulates ONE rigid body (a box of `BoxExtents`, centre of mass at the bound bone origin) attached to the animated
//! bound bone (the kinematic base, the node's input pose) through a joint at `LocalJointOffset` (bone-local). `ConstraintSetup` bounds the joint's
//! linear displacement (per axis `Free` / `Limited [min, max]` / `Locked`, in the bone's axes) and the body's rotation relative to the animated
//! bone (`Angular`: per axis min / max degrees). `bAngularSpring` / `AngularSpringConstant` pull the rotation back to `AngularTarget`. The result is
//! blended over the input pose by `Alpha` (`AlphaScaleBias` applied) like every skeletal control.
//!
//! Space: the physics runs in UE space (cm, Z up, left-handed) like the original; [`to_ue`] / [`from_ue`] convert the evaluator's glTF-space
//! component transforms (`(x, y, z)` -> `(x, z, y)`, quaternion `(x, y, z, w)` -> `(x, z, y, -w)`).
use glam::{Mat3, Quat, Vec3};

/// Evaluator (glTF, metres) component-space transform -> UE (cm) position / rotation.
pub fn to_ue(p: Vec3, q: Quat) -> (Vec3, Quat) { (Vec3::new(p.x, p.z, p.y) * 100., Quat::from_xyzw(q.x, q.z, q.y, -q.w).normalize()) }
/// Inverse of [`to_ue`].
pub fn from_ue(p: Vec3, q: Quat) -> (Vec3, Quat) { (Vec3::new(p.x, p.z, p.y) * 0.01, Quat::from_xyzw(q.x, q.z, q.y, -q.w).normalize()) }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LinearLimitType { Free, #[default] Limited, Locked }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AngularConstraintType { #[default] Angular, Cone }

/// `FAnimPhysConstraintSetup` (angles in degrees, lengths in cm, as serialized).
#[derive(Clone, Debug, PartialEq)]
pub struct ConstraintSetup {
    pub angular_type: AngularConstraintType,
    /// 0 = X, 1 = Y, 2 = Z (`AnimPhysTwistAxis`).
    pub twist_axis: u8,
    pub angular_min: [f32; 3],
    pub angular_max: [f32; 3],
    pub angular_target: [f32; 3],
    pub angular_target_axis: u8,
    pub cone_angle: f32,
    pub linear_types: [LinearLimitType; 3],
    pub linear_min: [f32; 3],
    pub linear_max: [f32; 3],
    pub linear_fully_locked: bool,
}
impl Default for ConstraintSetup {
    fn default() -> Self {
        ConstraintSetup { angular_type: AngularConstraintType::Angular, twist_axis: 1, angular_min: [0.; 3], angular_max: [0.; 3], angular_target: [0.; 3], angular_target_axis: 1, cone_angle: 0.,
            linear_types: [LinearLimitType::Limited; 3], linear_min: [0.; 3], linear_max: [0.; 3], linear_fully_locked: false }
    }
}

/// `AnimNode_AnimDynamics` properties (UE 4.16 field names, serialized units).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimDynamicsParams {
    pub bound_bone: String,
    pub chain: bool,
    pub chain_end: Option<String>,
    pub box_extents: [f32; 3],
    pub local_joint_offset: [f32; 3],
    pub gravity_scale: f32,
    pub linear_spring: bool,
    pub linear_spring_constant: f32,
    pub angular_spring: bool,
    pub angular_spring_constant: f32,
    pub constraint: ConstraintSetup,
    pub pre_iterations: u32,
    pub post_iterations: u32,
    pub use_planar_limit: bool,
    pub planar_limit_count: usize,
    pub use_spherical_limits: bool,
    pub spherical_limit_count: usize,
    pub enable_wind: bool,
    pub wind_scale: f32,
    pub override_linear_damping: bool,
    pub linear_damping_override: f32,
    pub override_angular_damping: bool,
    pub angular_damping_override: f32,
    pub external_force: [f32; 3],
    pub simulation_space_component: bool,
    pub relative_space_bone: Option<String>,
    pub lod_threshold: i32,
    /// `Alpha` default (the exposed pin overrides it per frame) and `AlphaScaleBias` (scale, bias).
    pub alpha: f32,
    pub alpha_scale: f32,
    pub alpha_bias: f32,
    pub do_update: bool,
    pub do_eval: bool,
}
impl Default for AnimDynamicsParams {
    fn default() -> Self {
        AnimDynamicsParams { bound_bone: String::new(), chain: false, chain_end: None, box_extents: [5.; 3], local_joint_offset: [0.; 3], gravity_scale: 1., linear_spring: false, linear_spring_constant: 0.,
            angular_spring: false, angular_spring_constant: 0., constraint: ConstraintSetup::default(), pre_iterations: 4, post_iterations: 1, use_planar_limit: true, planar_limit_count: 0,
            use_spherical_limits: false, spherical_limit_count: 0, enable_wind: false, wind_scale: 1., override_linear_damping: false, linear_damping_override: 0., override_angular_damping: false,
            angular_damping_override: 0., external_force: [0.; 3], simulation_space_component: true, relative_space_bone: None, lod_threshold: -1, alpha: 1., alpha_scale: 1., alpha_bias: 0., do_update: true, do_eval: true }
    }
}
impl AnimDynamicsParams {
    /// Parts of the node this port does not simulate (non-empty = the data uses something [`AnimDynamicsSim`] ignores).
    pub fn unsupported(&self) -> Vec<String> {
        let mut u = vec![];
        if self.chain { u.push("bChain".into()); }
        if self.use_spherical_limits && self.spherical_limit_count > 0 { u.push("spherical limits".into()); }
        if self.use_planar_limit && self.planar_limit_count > 0 { u.push("planar limits".into()); }
        if self.constraint.angular_type == AngularConstraintType::Cone { u.push("Cone angular constraint".into()); }
        if !self.simulation_space_component { u.push("world / bone-relative simulation space".into()); }
        if self.external_force != [0.; 3] { u.push("ExternalForce".into()); }
        u
    }
    /// `AlphaScaleBias` applied to an Alpha pin value, clamped to 0..1 (`ActualAlpha`).
    pub fn actual_alpha(&self, alpha: f32) -> f32 { (alpha * self.alpha_scale + self.alpha_bias).clamp(0., 1.) }
}

/// Engine constants of the `FAnimPhys` solver that are not in the packages (see the module docs). Reported, never hidden.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolverAssumptions {
    pub mass: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// cm/s^2 along component -Z before `GravityScale` (UE default `DefaultGravityZ`).
    pub gravity_cm_s2: f32,
    pub max_step: f32,
}
impl Default for SolverAssumptions { fn default() -> Self { SolverAssumptions { mass: 1., linear_damping: 0.7, angular_damping: 0.7, gravity_cm_s2: -fn_movement::params::WORLD_GRAVITY_Z, max_step: 1. / 30. } } }

#[derive(Clone, Copy, Debug)]
struct Body { p: Vec3, q: Quat, v: Vec3, w: Vec3 }

/// The simulation state of one node.
#[derive(Clone, Debug, Default)]
pub struct AnimDynamicsSim { body: Option<Body> }

fn axis(v: [f32; 3], i: usize) -> f32 { v[i] }
/// Rotation vector (axis * angle, shortest path) of a quaternion.
fn rotvec(q: Quat) -> Vec3 {
    let q = if q.w < 0. { -q } else { q };
    let s = Vec3::new(q.x, q.y, q.z);
    let sl = s.length();
    if sl < 1e-7 { return s * 2.; }
    s / sl * (2. * sl.atan2(q.w))
}
fn from_rotvec(v: Vec3) -> Quat { let a = v.length(); if a < 1e-9 { Quat::IDENTITY } else { Quat::from_axis_angle(v / a, a) } }

fn inv_inertia_world(q: Quat, ext: [f32; 3], mass: f32) -> Mat3 {
    // uniform box, ext = full side lengths: I_x = m/12 (ey^2 + ez^2) ...
    let ix = mass / 12. * (ext[1] * ext[1] + ext[2] * ext[2]);
    let iy = mass / 12. * (ext[0] * ext[0] + ext[2] * ext[2]);
    let iz = mass / 12. * (ext[0] * ext[0] + ext[1] * ext[1]);
    let inv = Mat3::from_diagonal(Vec3::new(1. / ix.max(1e-6), 1. / iy.max(1e-6), 1. / iz.max(1e-6)));
    let r = Mat3::from_quat(q);
    r * inv * r.transpose()
}

impl AnimDynamicsSim {
    pub fn new() -> Self { Self::default() }
    pub fn reset(&mut self) { self.body = None; }
    pub fn is_initialised(&self) -> bool { self.body.is_some() }

    /// One tick. `base` = the node's input pose of the bound bone, UE space (cm). `dt` seconds. Returns the simulated bone transform (UE space);
    /// the caller blends it with the input by `Alpha`. The first call (or after [`reset`](Self::reset)) starts at rest on the base.
    pub fn step(&mut self, prm: &AnimDynamicsParams, a: &SolverAssumptions, base: (Vec3, Quat), dt: f32) -> (Vec3, Quat) {
        let (bp, bq) = base;
        let Some(mut b) = self.body else {
            self.body = Some(Body { p: bp, q: bq, v: Vec3::ZERO, w: Vec3::ZERO });
            return base;
        };
        if !prm.do_update { return (b.p, b.q); }
        let dt = dt.clamp(0., a.max_step);
        if dt <= 0. { return (b.p, b.q); }
        let c = &prm.constraint;
        let off = Vec3::from(prm.local_joint_offset);
        let ext = prm.box_extents;
        let lin_damp = if prm.override_linear_damping { prm.linear_damping_override } else { a.linear_damping };
        let ang_damp = if prm.override_angular_damping { prm.angular_damping_override } else { a.angular_damping };

        // forces: gravity at the centre of mass, optional linear spring on the joint, angular spring toward the target
        b.v += Vec3::new(0., 0., -a.gravity_cm_s2 * prm.gravity_scale) * dt;
        if prm.linear_spring {
            let jb = b.p + b.q * off;
            let ja = bp + bq * off;
            b.v += (ja - jb) * (prm.linear_spring_constant / a.mass) * dt;
        }
        if prm.angular_spring {
            let rel = rotvec(bq.conjugate() * b.q);
            let target = Vec3::from(c.angular_target) * std::f32::consts::PI / 180.;
            let tau = bq * ((target - rel) * prm.angular_spring_constant);
            b.w += inv_inertia_world(b.q, ext, a.mass) * tau * dt;
        }
        b.v /= 1. + lin_damp * dt;
        b.w /= 1. + ang_damp * dt;

        let (p0, q0) = (b.p, b.q);
        b.p += b.v * dt;
        let dq = Quat::from_xyzw(b.w.x, b.w.y, b.w.z, 0.) * b.q;
        b.q = Quat::from_xyzw(b.q.x + 0.5 * dt * dq.x, b.q.y + 0.5 * dt * dq.y, b.q.z + 0.5 * dt * dq.z, b.q.w + 0.5 * dt * dq.w).normalize();

        let solve = |b: &mut Body| {
            // angular range relative to the animated bone, per axis in the bone frame
            if c.angular_type == AngularConstraintType::Angular {
                let rel = rotvec(bq.conjugate() * b.q);
                let lim = |i: usize| (axis(c.angular_min, i).to_radians(), axis(c.angular_max, i).to_radians());
                let clamped = Vec3::new(rel.x.clamp(lim(0).0.min(lim(0).1), lim(0).1.max(lim(0).0)), rel.y.clamp(lim(1).0.min(lim(1).1), lim(1).1.max(lim(1).0)), rel.z.clamp(lim(2).0.min(lim(2).1), lim(2).1.max(lim(2).0)));
                if (clamped - rel).length_squared() > 1e-12 { b.q = (bq * from_rotvec(clamped)).normalize(); }
            }
            // joint linear displacement in the bone frame
            let jb = b.p + b.q * off;
            let ja = bp + bq * off;
            let r = bq.conjugate() * (jb - ja);
            let mut r2 = r;
            for i in 0..3 {
                let (lo, hi) = (c.linear_min[i].min(c.linear_max[i]), c.linear_max[i].max(c.linear_min[i]));
                let v = r[i];
                r2[i] = if c.linear_fully_locked { 0. } else { match c.linear_types[i] { LinearLimitType::Free => v, LinearLimitType::Limited => v.clamp(lo, hi), LinearLimitType::Locked => 0. } };
            }
            let corr = bq * (r2 - r);
            let len = corr.length();
            if len > 1e-7 {
                // generalised inverse mass at the joint point; the base is kinematic
                let arm = b.q * off;
                let n = corr / len;
                let iw = inv_inertia_world(b.q, ext, a.mass);
                let axn = arm.cross(n);
                let w_eff = 1. / a.mass + axn.dot(iw * axn);
                let lambda = len / w_eff.max(1e-9);
                b.p += n * (lambda / a.mass);
                let dth = iw * arm.cross(n * lambda);
                let dq = Quat::from_xyzw(dth.x, dth.y, dth.z, 0.) * b.q;
                b.q = Quat::from_xyzw(b.q.x + 0.5 * dq.x, b.q.y + 0.5 * dq.y, b.q.z + 0.5 * dq.z, b.q.w + 0.5 * dq.w).normalize();
            }
        };
        let pure_translate = |b: &mut Body| {
            // final exact joint fix without rotating the body (keeps the angular range satisfied)
            let jb = b.p + b.q * off;
            let ja = bp + bq * off;
            let r = bq.conjugate() * (jb - ja);
            let mut r2 = r;
            for i in 0..3 {
                let (lo, hi) = (c.linear_min[i].min(c.linear_max[i]), c.linear_max[i].max(c.linear_min[i]));
                r2[i] = if c.linear_fully_locked { 0. } else { match c.linear_types[i] { LinearLimitType::Free => r[i], LinearLimitType::Limited => r[i].clamp(lo, hi), LinearLimitType::Locked => 0. } };
            }
            b.p += bq * (r2 - r);
        };
        for _ in 0..prm.pre_iterations.max(1) { solve(&mut b); }
        pure_translate(&mut b);
        b.v = (b.p - p0) / dt;
        let dq = b.q * q0.conjugate();
        b.w = rotvec(dq) / dt;
        for _ in 0..prm.post_iterations { solve(&mut b); }
        pure_translate(&mut b);
        self.body = Some(b);
        (b.p, b.q)
    }

    /// Blend of the simulated transform over the input (`Alpha`): position lerp, rotation slerp.
    pub fn blend(input: (Vec3, Quat), sim: (Vec3, Quat), alpha: f32) -> (Vec3, Quat) {
        let alpha = alpha.clamp(0., 1.);
        if alpha <= 1e-4 { return input; }
        (input.0.lerp(sim.0, alpha), input.1.slerp(sim.1, alpha).normalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn grenade() -> AnimDynamicsParams {
        let mut p = AnimDynamicsParams { bound_bone: "dyn_grenade01".into(), box_extents: [10., 10., 20.], local_joint_offset: [0., 0., 2.], gravity_scale: 0.2, angular_spring: true, angular_spring_constant: 1000., pre_iterations: 16, post_iterations: 4, ..Default::default() };
        p.constraint.angular_min = [0., -20., 0.];
        p.constraint.angular_max = [20., 0., 0.];
        p.constraint.linear_min = [0., 0., -0.5];
        p.constraint.linear_max = [0., 0., 0.5];
        p
    }
    #[test]
    fn rest_on_a_static_base_stays_inside_the_limits() {
        let prm = grenade();
        let a = SolverAssumptions::default();
        let base = (Vec3::new(10., 20., 100.), Quat::from_rotation_x(0.3));
        let mut s = AnimDynamicsSim::new();
        let mut out = base;
        for _ in 0..600 { out = s.step(&prm, &a, base, 1. / 60.); }
        let rel = rotvec(base.1.conjugate() * out.1);
        assert!(rel.x >= -1e-3 && rel.x <= 20f32.to_radians() + 1e-3 && rel.y <= 1e-3 && rel.y >= -20f32.to_radians() - 1e-3 && rel.z.abs() < 1e-3, "{rel:?}");
        let r = base.1.conjugate() * ((out.0 + out.1 * Vec3::new(0., 0., 2.)) - (base.0 + base.1 * Vec3::new(0., 0., 2.)));
        assert!(r.x.abs() < 1e-2 && r.y.abs() < 1e-2 && r.z.abs() <= 0.51, "{r:?}");
    }
    #[test]
    fn first_tick_is_the_input_and_zero_alpha_is_a_bypass() {
        let prm = grenade();
        let mut s = AnimDynamicsSim::new();
        let base = (Vec3::ONE, Quat::IDENTITY);
        assert_eq!(s.step(&prm, &SolverAssumptions::default(), base, 1. / 60.), base);
        assert_eq!(AnimDynamicsSim::blend(base, (Vec3::ZERO, Quat::IDENTITY), 0.), base);
    }
    #[test]
    fn locked_rotation_pouch_slides_along_z_only() {
        let mut prm = AnimDynamicsParams { bound_bone: "dyn_pouch_r".into(), box_extents: [5.; 3], local_joint_offset: [0., 0., -5.], gravity_scale: 0.1, ..Default::default() };
        prm.constraint.linear_max = [0., 0., 2.];
        let a = SolverAssumptions::default();
        let mut s = AnimDynamicsSim::new();
        let base = (Vec3::ZERO, Quat::IDENTITY);
        let mut out = base;
        for _ in 0..300 { out = s.step(&prm, &a, base, 1. / 60.); }
        assert!(out.0.x.abs() < 1e-2 && out.0.y.abs() < 1e-2 && out.0.z >= -1e-2 && out.0.z <= 2.01, "{:?}", out.0);
        assert!(rotvec(out.1).length() < 1e-2);
    }
    #[test]
    fn ue_conversion_round_trips() {
        let (p, q) = (Vec3::new(0.1, 1.2, -0.3), Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.2, 0.9));
        let (up, uq) = to_ue(p, q);
        let (p2, q2) = from_ue(up, uq);
        assert!((p - p2).length() < 1e-5 && q.dot(q2).abs() > 0.99999);
    }
}
