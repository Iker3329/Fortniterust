//! The `View` and `Primitive` uniform buffers the original base-pass programs read, filled in Unreal conventions (centimetres, left handed, Z up, row-vector matrices,
//! translated world = world + PreViewTranslation, PreViewTranslation = -eye) from the port's camera (metres, right handed, Y up). The member indices are the ones the
//! recovered shaders read (`layout.json` of the programs, verified by the `view_matrices_*` tests and by running the translated programs against the interpreter).
//!
//! Depth convention: the renderer's depth buffer is the wgpu standard-Z one (near -> 0, far -> 1, `clip.w` = view depth) so `clip.z = A * w + B` with
//! `A = far / (far - near)`, `B = -far * near / (far - near)`; `InvDeviceZToWorldZTransform` is the inverse of that (SceneDepth = 1 / (z * c2 - c3)).
use glam::{DMat4, DVec3, DVec4, Vec3};

/// The port's camera for one frame (what `fn_render::CameraParams` holds, metres / port world axes).
#[derive(Clone, Copy, Debug)]
pub struct ViewParams {
    pub eye: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    /// tan of the half field of view (x, y).
    pub tan_half: (f32, f32),
    pub near_m: f32,
    pub far_m: f32,
    pub width: u32,
    pub height: u32,
    /// `View.GameTime` / `RealTime` (seconds).
    pub time: f32,
}

pub const VIEW_FLOAT4S: usize = 178;
pub const PRIMITIVE_FLOAT4S: usize = 16;
/// Member indices (float4 slots) of the `View` uniform buffer that the FX programs read.
pub mod view_slot {
    pub const TRANSLATED_WORLD_TO_CLIP: usize = 0;
    pub const WORLD_TO_CLIP: usize = 4;
    /// `ViewToTranslatedWorld` rows 0..2: the camera right / up / forward axes in world space (the billboard materials read them).
    pub const VIEW_TO_TRANSLATED_WORLD: usize = 12;
    /// `CameraViewToTranslatedWorld` rows 0..2 (same axes without an HMD).
    pub const CAMERA_VIEW_TO_TRANSLATED_WORLD: usize = 20;
    pub const SV_POSITION_TO_TRANSLATED_WORLD: usize = 40;
    pub const VIEW_FORWARD: usize = 52;
    /// `ViewUp`, `ViewRight`, then the HMD no-roll variants (equal to them without an HMD).
    pub const VIEW_UP: usize = 53;
    pub const VIEW_RIGHT: usize = 54;
    pub const HMD_NO_ROLL_UP: usize = 55;
    pub const HMD_NO_ROLL_RIGHT: usize = 56;
    /// `ViewOrigin` (world space, cm).
    pub const WORLD_VIEW_ORIGIN: usize = 59;
    pub const INV_DEVICE_Z_TO_WORLD_Z: usize = 57;
    pub const SCREEN_POSITION_SCALE_BIAS: usize = 58;
    /// translated-world camera origin (0: PreViewTranslation = -ViewOrigin)
    pub const TRANSLATED_VIEW_ORIGIN: usize = 60;
    pub const PRE_VIEW_TRANSLATION: usize = 62;
    pub const VIEW_RECT_MIN: usize = 120;
    pub const VIEW_SIZE_AND_INV_SIZE: usize = 121;
    pub const REAL_TIME: usize = 131;
    pub const PREV_TIME: usize = 128;
}

/// Port world (metres, Y up) -> Unreal world (centimetres, Z up): the `fn-propdata` / glTF mapping `(x, y, z) -> (x, z, y) * 100`.
pub fn to_ue(p: Vec3) -> DVec3 { DVec3::new(p.x as f64, p.z as f64, p.y as f64) * 100.0 }
/// Direction mapping (no scale).
pub fn dir_to_ue(d: Vec3) -> DVec3 { DVec3::new(d.x as f64, d.z as f64, d.y as f64) }

/// The clip transform as a row-vector matrix on translated world positions: `clip = (rel, 1) * M`.
pub struct Clip { pub m: DMat4, pub eye: DVec3, pub fwd: DVec3, pub up: DVec3, pub right: DVec3, pub a: f64, pub b: f64, pub near: f64, pub far: f64 }

impl ViewParams {
    pub fn clip(&self) -> Clip {
        let eye = to_ue(self.eye);
        let fwd = dir_to_ue(self.forward).normalize();
        let up0 = dir_to_ue(self.up).normalize();
        // UE: right = up x forward; re-orthogonalise the up vector
        let right = up0.cross(fwd).normalize();
        let up = fwd.cross(right).normalize();
        let (sx, sy) = (1.0 / self.tan_half.0 as f64, 1.0 / self.tan_half.1 as f64);
        let (near, far) = (self.near_m as f64 * 100.0, self.far_m as f64 * 100.0);
        let a = far / (far - near);
        let b = -far * near / (far - near);
        // columns of the glam matrix are the float4 rows used by the shader: cb[i] = row i of M (x*row0 + y*row1 + z*row2 + row3)
        let row = |i: usize| DVec4::new(right[i] * sx, up[i] * sy, fwd[i] * a, fwd[i]);
        let m = DMat4::from_cols(DVec4::new(row(0).x, row(1).x, row(2).x, 0.0), DVec4::new(row(0).y, row(1).y, row(2).y, 0.0), DVec4::new(row(0).z, row(1).z, row(2).z, b), DVec4::new(row(0).w, row(1).w, row(2).w, 0.0));
        Clip { m, eye, fwd, up, right, a, b, near, far }
    }

    /// The 178 float4 slots of the `View` uniform buffer.
    pub fn view_buffer(&self) -> Vec<[f32; 4]> {
        let c = self.clip();
        let mut v = vec![[0f32; 4]; VIEW_FLOAT4S];
        let rows = |m: &DMat4, at: usize, v: &mut Vec<[f32; 4]>| { for i in 0..4 { let r = m.row(i); v[at + i] = [r.x as f32, r.y as f32, r.z as f32, r.w as f32]; } };
        // `DMat4::row(i)` of the matrix built from the shader rows' transpose: row i of the glam matrix = shader float4 i
        rows(&c.m, view_slot::TRANSLATED_WORLD_TO_CLIP, &mut v);
        // world -> clip: rel = p - eye  =>  clip = p*R + (-eye*R) + row3
        let t = DVec4::new(-c.eye.x, -c.eye.y, -c.eye.z, 1.0);
        let row3 = c.m.row(3);
        let r0 = c.m.row(0); let r1 = c.m.row(1); let r2 = c.m.row(2);
        let new3 = r0 * t.x + r1 * t.y + r2 * t.z + row3;
        let w = DMat4::from_cols(DVec4::new(r0.x, r1.x, r2.x, new3.x), DVec4::new(r0.y, r1.y, r2.y, new3.y), DVec4::new(r0.z, r1.z, r2.z, new3.z), DVec4::new(r0.w, r1.w, r2.w, new3.w));
        rows(&w, view_slot::WORLD_TO_CLIP, &mut v);
        // SvPosition -> translated world: (x_px, y_px, z, 1) * S * M^-1
        let (wd, ht) = (self.width.max(1) as f64, self.height.max(1) as f64);
        let s = DMat4::from_cols(DVec4::new(2.0 / wd, 0.0, 0.0, -1.0), DVec4::new(0.0, -2.0 / ht, 0.0, 1.0), DVec4::new(0.0, 0.0, 1.0, 0.0), DVec4::new(0.0, 0.0, 0.0, 1.0));
        // shader-row matrices multiply row vectors: (p*S)*Minv  ==  with glam column vectors: (Minv^T * S^T) p^T ; build via explicit row products
        let mrows = to_rows(&s); let minv = invert_rows(&to_rows(&c.m));
        let prod = mul_rows(&mrows, &minv);
        for i in 0..4 { v[view_slot::SV_POSITION_TO_TRANSLATED_WORLD + i] = [prod[i][0] as f32, prod[i][1] as f32, prod[i][2] as f32, prod[i][3] as f32]; }
        v[view_slot::VIEW_FORWARD] = [c.fwd.x as f32, c.fwd.y as f32, c.fwd.z as f32, 0.0];
        // SceneDepth = z*c0 + c1 + 1/(z*c2 - c3), z = a + b / depth
        v[view_slot::INV_DEVICE_Z_TO_WORLD_Z] = [0.0, 0.0, (1.0 / c.b) as f32, (c.a / c.b) as f32];
        v[view_slot::SCREEN_POSITION_SCALE_BIAS] = [0.5, -0.5, 0.5, 0.5];
        let col = |d: DVec3| [d.x as f32, d.y as f32, d.z as f32, 0.0];
        v[view_slot::VIEW_UP] = col(c.up); v[view_slot::VIEW_RIGHT] = col(c.right); v[view_slot::HMD_NO_ROLL_UP] = col(c.up); v[view_slot::HMD_NO_ROLL_RIGHT] = col(c.right);
        v[view_slot::WORLD_VIEW_ORIGIN] = col(c.eye);
        v[view_slot::TRANSLATED_VIEW_ORIGIN] = [0.0; 4];
        v[view_slot::PRE_VIEW_TRANSLATION] = [-c.eye.x as f32, -c.eye.y as f32, -c.eye.z as f32, 0.0];
        v[view_slot::VIEW_RECT_MIN] = [0.0; 4];
        v[view_slot::VIEW_SIZE_AND_INV_SIZE] = [wd as f32, ht as f32, (1.0 / wd) as f32, (1.0 / ht) as f32];
        v[view_slot::REAL_TIME] = [0.0, 0.0, self.time, self.time];
        v[view_slot::PREV_TIME][3] = self.time;
        for (i, axis) in [c.right, c.up, c.fwd].into_iter().enumerate() { v[view_slot::VIEW_TO_TRANSLATED_WORLD + i] = col(axis); v[view_slot::CAMERA_VIEW_TO_TRANSLATED_WORLD + i] = col(axis); }
        let _ = w;
        v
    }
}

type Rows = [[f64; 4]; 4];
fn to_rows(m: &DMat4) -> Rows { std::array::from_fn(|i| { let r = m.row(i); [r.x, r.y, r.z, r.w] }) }
fn mul_rows(a: &Rows, b: &Rows) -> Rows { std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| a[i][k] * b[k][j]).sum())) }
fn invert_rows(m: &Rows) -> Rows {
    // Gauss-Jordan
    let mut a = *m; let mut inv: Rows = std::array::from_fn(|i| std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 }));
    for c in 0..4 {
        let p = (c..4).max_by(|&x, &y| a[x][c].abs().partial_cmp(&a[y][c].abs()).unwrap()).unwrap();
        a.swap(c, p); inv.swap(c, p);
        let d = a[c][c];
        for j in 0..4 { a[c][j] /= d; inv[c][j] /= d; }
        for r in 0..4 { if r != c { let f = a[r][c]; for j in 0..4 { a[r][j] -= f * a[c][j]; inv[r][j] -= f * inv[c][j]; } } }
    }
    inv
}

/// The `Primitive` uniform buffer of a component at `position` (UE cm) with uniform `scale`: `LocalToWorld` (rows 0..3, row-vector convention: scaled axes, then the
/// translation), the determinant sign (element 9.w) and the object position (element 10.xyz); the members the recovered programs read.
pub fn primitive_buffer(position: DVec3, scale: f32) -> Vec<[f32; 4]> {
    let mut v = vec![[0f32; 4]; PRIMITIVE_FLOAT4S];
    v[0] = [scale, 0., 0., 0.]; v[1] = [0., scale, 0., 0.]; v[2] = [0., 0., scale, 0.];
    v[3] = [position.x as f32, position.y as f32, position.z as f32, 1.];
    v[9] = [1. / scale.max(1e-6), 1. / scale.max(1e-6), 1. / scale.max(1e-6), 1.0];
    v[10] = [position.x as f32, position.y as f32, position.z as f32, 0.0];
    v
}
/// `Primitive` of a mesh component rotated by `axes` (X, Y, Z world directions of the local axes; unit scale): `LocalToWorld` rows (axes, translation), determinant sign, object
/// position and `InvNonUniformScale` (element 14).
pub fn primitive_buffer_local(position: DVec3, axes: [DVec3; 3]) -> Vec<[f32; 4]> {
    let mut v = vec![[0f32; 4]; PRIMITIVE_FLOAT4S];
    for (i, a) in axes.iter().enumerate() { v[i] = [a.x as f32, a.y as f32, a.z as f32, 0.]; }
    v[3] = [position.x as f32, position.y as f32, position.z as f32, 1.];
    v[9] = [1., 1., 1., 1.];
    v[10] = [position.x as f32, position.y as f32, position.z as f32, 0.];
    v[14] = [1., 1., 1., 0.];
    v
}
/// `FParticleSpriteUniformParameters` ("SpriteVF", 10 float4) of a camera facing, unrotated, non-flipbook sprite emitter (`ScreenAlignment` PSA_Square, normals
/// `CameraFacing`): TangentSelector (1,0,0,0) selects the camera right / up axes, `SubImageSize` (1,1,1,1), `RotationScale` 1 / `RotationBias` 0, pivot (-0.5,-0.5).
pub fn sprite_vf_buffer() -> Vec<[f32; 4]> {
    let mut v = vec![[0f32; 4]; 10];
    v[0] = [1., 0., 0., 0.]; v[1] = [0., 0., 1., 0.];          // AxisLockRight / AxisLockUp (unused: selector.z = 0)
    v[2] = [1., 0., 0., 0.];                                    // TangentSelector
    v[4] = [0., 0., 1., 0.];                                    // NormalsCylinderUnitDirection (unused with camera facing normals)
    v[5] = [1., 1., 1., 1.];                                    // SubImageSize
    v[8] = [1., 0., 0., 0.];                                    // RotationScale, RotationBias, NormalsMode 0
    v[9] = [-0.5, -0.5, 0., 0.];                                // PivotOffset
    v
}

pub fn bytes(v: &[[f32; 4]]) -> Vec<u8> { v.iter().flat_map(|r| r.iter().flat_map(|f| f.to_le_bytes())).collect() }

#[cfg(test)]
mod tests {
    use super::*;
    fn view() -> ViewParams { ViewParams { eye: Vec3::new(1., 2., 3.), forward: Vec3::new(0.3, -0.1, -1.).normalize(), up: Vec3::Y, tan_half: (0.84, 0.47), near_m: 0.1, far_m: 3000., width: 1280, height: 720, time: 1.5 } }
    fn apply(v: &[[f32; 4]], at: usize, p: [f32; 4]) -> [f32; 4] { let mut o = [0f32; 4]; for i in 0..4 { for j in 0..4 { o[j] += p[i] * v[at + i][j]; } } o }

    /// The clip position the recovered vertex shader computes (translated world through `TranslatedWorldToClip`) equals the port camera's own projection of the point.
    #[test] fn view_matrices_match_the_port_camera_projection() {
        let vp = view(); let v = vp.view_buffer();
        let cam = fn_camera_stub::project(&vp, Vec3::new(2., 1.5, -2.));
        let ue = to_ue(Vec3::new(2., 1.5, -2.)); let rel = [(ue.x - vp.eye.x as f64 * 100.0) as f32, (ue.y - vp.eye.z as f64 * 100.0) as f32, (ue.z - vp.eye.y as f64 * 100.0) as f32, 1.0];
        let c = apply(&v, view_slot::TRANSLATED_WORLD_TO_CLIP, rel);
        let ndc = [c[0] / c[3], c[1] / c[3], c[2] / c[3]];
        for k in 0..3 { assert!((ndc[k] - cam[k]).abs() < 2e-4, "axis {k}: {ndc:?} vs {cam:?}"); }
        // clip.w is the view depth in cm
        let depth_m = (Vec3::new(2., 1.5, -2.) - vp.eye).dot(vp.forward);
        assert!((c[3] / 100. - depth_m).abs() < 1e-3);
        // WorldToClip of the untranslated point agrees
        let world = [ue.x as f32, ue.y as f32, ue.z as f32, 1.0];
        let c2 = apply(&v, view_slot::WORLD_TO_CLIP, world);
        for k in 0..4 { assert!((c2[k] - c[k]).abs() < 0.05 * c[3].abs().max(1.) * 1e-3 + 0.05, "{c2:?} vs {c:?}"); }
    }
    /// SvPosition (pixel, depth) maps back to the translated world point and the depth transform recovers the view depth.
    #[test] fn sv_position_round_trip_and_scene_depth() {
        let vp = view(); let v = vp.view_buffer();
        let p = Vec3::new(2., 1.5, -2.);
        let cam = fn_camera_stub::project(&vp, p);
        let (px, py) = ((cam[0] * 0.5 + 0.5) * vp.width as f32, (0.5 - cam[1] * 0.5) * vp.height as f32);
        let h = apply(&v, view_slot::SV_POSITION_TO_TRANSLATED_WORLD, [px, py, cam[2], 1.0]);
        let rel = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
        let ue = to_ue(p); let want = [(ue.x - vp.eye.x as f64 * 100.0) as f32, (ue.y - vp.eye.z as f64 * 100.0) as f32, (ue.z - vp.eye.y as f64 * 100.0) as f32];
        for k in 0..3 { assert!((rel[k] - want[k]).abs() < 0.2, "{rel:?} vs {want:?}"); }
        let t = v[view_slot::INV_DEVICE_Z_TO_WORLD_Z];
        let z = cam[2]; let depth = z * t[0] + t[1] + 1. / (z * t[2] - t[3]);
        let want_depth = (p - vp.eye).dot(vp.forward) * 100.;
        assert!((depth - want_depth).abs() / want_depth < 1e-3, "{depth} vs {want_depth}");
    }
    mod fn_camera_stub {
        use super::*;
        /// The port's projection (`CameraParams`): `Mat4::look_to_rh` + `perspective_rh`, ndc of a world point (metres).
        pub fn project(v: &ViewParams, p: Vec3) -> [f32; 3] {
            let view = glam::Mat4::look_to_rh(v.eye, v.forward, v.up);
            let proj = glam::Mat4::perspective_rh(2. * v.tan_half.1.atan(), v.tan_half.0 / v.tan_half.1, v.near_m, v.far_m);
            let c = proj * view * glam::Vec4::new(p.x, p.y, p.z, 1.);
            [c.x / c.w, c.y / c.w, c.z / c.w]
        }
    }
}




