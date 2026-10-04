//! Synthetic test scene: sky gradient with an HDR sun, a checkered ground plane with a real perspective depth ramp, and eight colour-bar poles
//! at increasing distances (5 m .. 1.2 km), plus a camera pair for the motion-blur tests. Metres, `cm_per_unit = 100`.
use crate::cpu::Image;
use crate::params::*;
use glam::{Mat4, Vec3};

pub const NEAR: f32 = 0.1;
pub const FAR: f32 = 6000.0;
pub const EYE_HEIGHT: f32 = 1.7;
pub const VFOV_DEG: f32 = 50.0;

pub struct Camera {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub aspect: f32,
}

impl Camera {
    pub fn view(&self) -> Mat4 {
        let fwd = Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos());
        Mat4::look_to_rh(self.pos, fwd, Vec3::Y)
    }
    pub fn proj(&self) -> Mat4 { Mat4::perspective_rh(VFOV_DEG.to_radians(), self.aspect, NEAR, FAR) }
    pub fn view_proj(&self) -> Mat4 { self.proj() * self.view() }
}

/// `clip_to_prev_clip` for `(ndc.xy, device_z, 1)` of the current frame.
pub fn clip_to_prev(cur: &Camera, prev: &Camera) -> Mat4 { prev.view_proj() * cur.view_proj().inverse() }

pub struct Scene {
    pub color: Image,
    /// device depth in `.x`
    pub depth: Image,
    /// rg = NDC velocity (cur - prev), a = 1 for the moving object
    pub velocity: Image,
    pub params: PostFxParams,
    pub cur: Camera,
    pub prev: Camera,
}

const BARS: [[f32; 3]; 8] = [[1.0, 1.0, 1.0], [1.0, 1.0, 0.0], [0.0, 1.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.25, 0.25, 0.25]];
const POLE_DIST: [f32; 8] = [5.0, 12.0, 30.0, 70.0, 150.0, 300.0, 600.0, 1200.0];

pub struct SceneOptions {
    pub width: usize,
    pub height: usize,
    /// yaw change between the previous and the current frame, radians (camera motion blur)
    pub yaw_delta: f32,
    /// camera translation per frame along its right vector (m)
    pub strafe: f32,
    /// NDC velocity of the moving object (the red pole), `[0, 0]` = none
    pub object_velocity: [f32; 2],
    pub exposure: f32,
}

impl Default for SceneOptions {
    fn default() -> Self { SceneOptions { width: 1280, height: 720, yaw_delta: 0.0, strafe: 0.0, object_velocity: [0.0, 0.0], exposure: 1.0 } }
}

pub fn build(o: &SceneOptions) -> Scene {
    let (w, h) = (o.width, o.height);
    let aspect = w as f32 / h as f32;
    let cur = Camera { pos: Vec3::new(0.0, EYE_HEIGHT, 0.0), yaw: 0.0, pitch: 0.0, aspect };
    let prev = Camera { pos: cur.pos - Vec3::new(o.strafe, 0.0, 0.0), yaw: cur.yaw - o.yaw_delta, pitch: 0.0, aspect };
    let tan_y = (VFOV_DEG.to_radians() * 0.5).tan();
    let tan_x = tan_y * aspect;
    let dt = DepthTransform::from_perspective(NEAR, FAR);
    let mut color = Image::new(w, h);
    let mut depth = Image::new(w, h);
    let mut velocity = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let ndc = [(x as f32 + 0.5) / w as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / h as f32 * 2.0];
            // ray in view space (camera looks down -z), distance along the view axis = z
            let ray_slope_y = ndc[1] * tan_y;
            let mut z = FAR;
            let mut col = [0.0f32; 3];
            if ray_slope_y < 0.0 {
                let zg = EYE_HEIGHT / -ray_slope_y;
                if zg < FAR {
                    z = zg;
                    let wx = ndc[0] * tan_x * zg;
                    let checker = ((wx / 4.0).floor() + (zg / 4.0).floor()).rem_euclid(2.0);
                    let base = 0.10 + 0.14 * checker;
                    let fogmix = 1.0 - (-zg / 800.0).exp();
                    for c in 0..3 { col[c] = base * (1.0 - fogmix) + [0.55, 0.75, 1.1][c] * fogmix; }
                } else { col = [0.55, 0.75, 1.1]; }
            } else {
                // sky: brighter toward the horizon, HDR sun disc
                let t = (ray_slope_y / tan_y).clamp(0.0, 1.0);
                col = [0.55 * (1.0 - t) + 0.05 * t, 0.75 * (1.0 - t) + 0.12 * t, 1.10 * (1.0 - t) + 0.35 * t];
                let (sx, sy) = (0.45, 0.35);
                let d2 = (ndc[0] - sx) * (ndc[0] - sx) * aspect * aspect + (ndc[1] - sy) * (ndc[1] - sy);
                if d2 < 0.0016 { col = [30.0, 27.0, 20.0]; }
            }
            // poles: vertical bars 1.2 m wide, 6 m tall, centred at world x = (i - 3.5) * spacing
            for i in 0..8 {
                let zp = POLE_DIST[i];
                let cx = (i as f32 - 3.5) * zp * 0.18;
                let half_w = 0.6 + zp * 0.01;
                let wx = ndc[0] * tan_x * zp;
                let wy = ray_slope_y * zp + EYE_HEIGHT;
                if (wx - cx).abs() < half_w && (0.0..6.0 + zp * 0.02).contains(&wy) && zp < z {
                    z = zp;
                    col = [BARS[i][0] * 2.5 + 0.02, BARS[i][1] * 2.5 + 0.02, BARS[i][2] * 2.5 + 0.02];
                    if i == 5 && (o.object_velocity[0] != 0.0 || o.object_velocity[1] != 0.0) {
                        velocity.px[y * w + x] = [o.object_velocity[0], o.object_velocity[1], 0.0, 1.0];
                    }
                }
            }
            let d = if z >= FAR { 1.0 } else { dt.device_z_from_linear(z) };
            color.px[y * w + x] = [col[0], col[1], col[2], 1.0];
            depth.px[y * w + x] = [d, 0.0, 0.0, 0.0];
        }
    }
    let rows = [
        [tan_x, 0.0, 0.0, 0.0], // right * tan_x  (basis permuted so .z is world up: x = right, y = forward, z = up)
        [0.0, 0.0, tan_y, 0.0], // up * tan_y
        [0.0, 1.0, 0.0, 0.0],   // forward
    ];
    let view = ViewParams { width: w as u32, height: h as u32, screen_to_world_rows: rows, clip_to_prev_clip: clip_to_prev(&cur, &prev) };
    let params = PostFxParams::athena_day(dt, view, o.exposure, 100.0);
    Scene { color, depth, velocity, params, cur, prev }
}
