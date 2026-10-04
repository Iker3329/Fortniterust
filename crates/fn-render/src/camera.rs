//! Camera parameters, projection maths and the GPU camera uniform.
//!
//! Conventions match the OpenGL prototype: metres, right-handed Y-up world, the camera looks down -Z in eye space.
//! The projection is a wgpu (depth 0..1) perspective. The horizontal FOV is authored at 16:9 and the vertical FOV is
//! preserved for other aspect ratios ("Hor+", UE MaintainYFOV), see camera.rs of the prototype.
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// ASSUMED: UE4 GNearClippingPlane default (10 cm).
pub const NEAR: f32 = 0.1;
/// Far clip of the main pass (the original uses an infinite far plane; a finite one is used here).
pub const FAR: f32 = 3000.;
/// Athena_PlayerCameraModeMelee FOV (serialized, degrees), authored at REFERENCE_ASPECT.
pub const MELEE_FOV_DEG: f32 = 80.;
pub const REFERENCE_ASPECT: f32 = 16. / 9.;
/// Clip planes of the sky pass only (metres). The star dome is ~1.5e6 m away.
pub const SKY_NEAR: f32 = 0.1;
pub const SKY_FAR: f32 = 4.0e6;

/// Frustum half-extents at the near plane (right, top) for a window of `width` x `height`.
pub fn frustum(fov_deg: f32, width: f32, height: f32, near: f32) -> (f64, f64) {
    let aspect = (width.max(1.) / height.max(1.)) as f64;
    let top = ((fov_deg as f64) * 0.5).to_radians().tan() / REFERENCE_ASPECT as f64 * near as f64;
    (top * aspect, top)
}

#[derive(Clone, Copy, Debug)]
pub struct CameraParams {
    pub eye: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    pub fov_deg: f32,
    pub near: f32,
    pub far: f32,
    pub width: u32,
    pub height: u32,
}
impl CameraParams {
    /// Camera from OpenGL-prototype style yaw/pitch (degrees; yaw 0 faces -Z, positive yaw turns towards +X, pitch up positive).
    pub fn from_yaw_pitch(eye: Vec3, yaw_deg: f32, pitch_deg: f32, width: u32, height: u32) -> Self {
        let (y, p) = (yaw_deg.to_radians(), pitch_deg.to_radians());
        let forward = Vec3::new(y.sin() * p.cos(), p.sin(), -y.cos() * p.cos());
        let right = Vec3::new(y.cos(), 0., y.sin());
        Self { eye, forward, up: right.cross(forward), fov_deg: MELEE_FOV_DEG, near: NEAR, far: FAR, width, height }
    }
    pub fn aspect(&self) -> f32 { self.width.max(1) as f32 / self.height.max(1) as f32 }
    /// tan(half horizontal fov), tan(half vertical fov).
    pub fn tan_half(&self) -> (f32, f32) {
        let (r, t) = frustum(self.fov_deg, self.width as f32, self.height as f32, self.near);
        ((r / self.near as f64) as f32, (t / self.near as f64) as f32)
    }
    pub fn view_matrix(&self) -> Mat4 { Mat4::look_to_rh(self.eye, self.forward, self.up) }
    fn projection(&self, near: f32, far: f32) -> Mat4 {
        let (_, ty) = self.tan_half();
        Mat4::perspective_rh(2. * ty.atan(), self.aspect(), near, far)
    }
    pub fn proj_matrix(&self) -> Mat4 { self.projection(self.near, self.far) }
    /// Projection for the sky pass: same frustum, near 0.1 m and far 4e6 m.
    pub fn sky_proj_matrix(&self) -> Mat4 { self.projection(SKY_NEAR, SKY_FAR) }
    /// Sky pass matrix: wide-clip projection times the view rotation only (the camera sits at the origin).
    pub fn sky_view_proj(&self) -> Mat4 {
        let mut v = self.view_matrix();
        v.w_axis = glam::Vec4::W;
        self.sky_proj_matrix() * v
    }
    pub fn uniform(&self, time: f32) -> CameraUniform {
        let view = self.view_matrix();
        let (tx, ty) = self.tan_half();
        CameraUniform {
            view_proj: (self.proj_matrix() * view).to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            view_inverse: view.inverse().to_cols_array_2d(),
            position: [self.eye.x, self.eye.y, self.eye.z, time],
            viewport: [self.width as f32, self.height as f32, 1. / self.width.max(1) as f32, 1. / self.height.max(1) as f32],
            proj: [tx, ty, self.near, self.far],
        }
    }
}

/// GPU camera block (group 0 binding 0 of the scene pass). Layout mirrors `struct Camera` in shaders/scene.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub view_inverse: [[f32; 4]; 4],
    /// xyz eye position (metres), w = time (seconds).
    pub position: [f32; 4],
    /// width, height, 1/width, 1/height.
    pub viewport: [f32; 4],
    /// tan(half fov x), tan(half fov y), near, far.
    pub proj: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cam(w: u32, h: u32) -> CameraParams { CameraParams::from_yaw_pitch(Vec3::new(1., 2., 3.), 30., -12., w, h) }

    #[test]
    fn uniform_is_a_whole_number_of_vec4s_and_size_is_stable() {
        assert_eq!(std::mem::size_of::<CameraUniform>(), 3 * 64 + 3 * 16);
        assert_eq!(std::mem::size_of::<CameraUniform>() % 16, 0);
    }
    #[test]
    fn eye_maps_to_the_view_origin_and_forward_is_minus_z() {
        let c = cam(1280, 720);
        let u = c.uniform(0.);
        let view = Mat4::from_cols_array_2d(&u.view);
        assert!(view.transform_point3(c.eye).length() < 1e-5);
        let f = view.transform_vector3(c.forward);
        assert!((f - -Vec3::Z).length() < 1e-5, "{f:?}");
        let inv = Mat4::from_cols_array_2d(&u.view_inverse);
        assert!((inv * view - Mat4::IDENTITY).abs_diff_eq(Mat4::ZERO, 1e-4));
    }
    #[test]
    fn near_and_far_planes_map_to_wgpu_depth_zero_and_one() {
        let c = cam(1280, 720);
        let p = c.proj_matrix();
        let near = p.project_point3(Vec3::new(0., 0., -c.near));
        let far = p.project_point3(Vec3::new(0., 0., -c.far));
        assert!(near.z.abs() < 1e-5 && (far.z - 1.).abs() < 1e-4, "{near:?} {far:?}");
    }
    #[test]
    fn sixteen_by_nine_horizontal_fov_is_eighty_degrees() {
        let c = cam(1920, 1080);
        let (tx, _) = c.tan_half();
        assert!((tx - 40f32.to_radians().tan()).abs() < 1e-5, "{tx}");
    }
    #[test]
    fn vertical_fov_is_preserved_for_other_aspect_ratios() {
        let (_, t16) = cam(1920, 1080).tan_half();
        for (w, h) in [(1280, 720), (2560, 1080), (1024, 768), (720, 1280)] {
            let (tx, ty) = cam(w, h).tan_half();
            assert!((ty - t16).abs() < 1e-6, "{w}x{h}");
            assert!((tx / ty - w as f32 / h as f32).abs() < 1e-4, "{w}x{h}");
        }
    }
    #[test]
    fn frustum_edge_projects_to_the_ndc_border() {
        let c = cam(1280, 720);
        let (tx, ty) = c.tan_half();
        let p = c.proj_matrix();
        let d = 10.;
        let q = p.project_point3(Vec3::new(tx * d, ty * d, -d));
        assert!((q.x - 1.).abs() < 1e-4 && (q.y - 1.).abs() < 1e-4, "{q:?}");
    }
    #[test]
    fn yaw_zero_faces_minus_z_and_positive_yaw_turns_right() {
        let c0 = CameraParams::from_yaw_pitch(Vec3::ZERO, 0., 0., 16, 9);
        assert!((c0.forward - -Vec3::Z).length() < 1e-6);
        let c90 = CameraParams::from_yaw_pitch(Vec3::ZERO, 90., 0., 16, 9);
        assert!((c90.forward - Vec3::X).length() < 1e-6);
        assert!((c0.up - Vec3::Y).length() < 1e-6);
    }
    #[test]
    fn sky_matrix_ignores_camera_translation_and_reaches_the_dome() {
        let mut a = cam(1280, 720);
        let m1 = a.sky_view_proj();
        a.eye += Vec3::new(500., 40., -900.);
        let m2 = a.sky_view_proj();
        assert!(m1.abs_diff_eq(m2, 1e-6));
        // a point on the star dome 1.5e6 m ahead must be inside the sky clip volume
        let q = m1.project_point3(a.forward * 1.5e6);
        assert!(q.z > 0. && q.z < 1., "{q:?}");
    }
}
