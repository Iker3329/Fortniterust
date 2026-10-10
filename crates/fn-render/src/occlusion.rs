//! OCCLUSION (2026-10-07): view occlusion culling of the placed statics (fn-app statics_render.rs) from the frame's hierarchical Z buffer.
//!
//! 1.8 culls occluded primitives every frame (`r.AllowOcclusionQueries` 1 in the cooked ini, `FDeferredShadingSceneRenderer::RenderOcclusion`
//! @1418025b0; results read a frame late, `r.NumBufferedOcclusionQueries`). The port does the cheap equivalent with the HZB the renderer already builds
//! for SSR / SSAO (`BuildHZB`, UE device Z, the FURTHEST depth of each block; the same buffer UE's own `FHZBOcclusionTester` reads @1419a4520): one small mip
//! is copied to a mappable buffer each frame, mapped without stalling, and a later frame tests a placement's bounding sphere against it on the CPU, as
//! `FHZBOcclusionTester` does on the GPU (the sphere's nearest device Z against the furthest occluder depth of every HZB texel its screen rectangle covers).
//! CONSERVATIVE by construction: anything not wholly inside that frame's view, nearer than its near plane, or under a sky / empty texel is visible; the
//! sphere grows by the distance the eye moved since (parallax), and a snapshot older than [`MAX_AGE`] frames or from an eye more than [`MAX_MOVE_M`] away
//! is not used, and HZB texels nearer than [`NEAR_OCCLUDER_M`] (the pawn, the held item: moving occluders) never occlude. Only the view's draws are culled (shadow casters and the distance-field scene keep every placement, as UE's whole-scene shadows do).
//! `FN_OCCLUSION=0` turns it off.
use glam::{Mat4, Vec3, Vec4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// HZB texels of the readback mip along the larger axis (at most): mip `m` is chosen so its width is <= this.
const READ_WIDTH: u32 = 128;
/// A snapshot older than this many frames is not used.
pub const MAX_AGE: u64 = if cfg!(target_arch = "wasm32") { 4 } else { 2 };
/// A snapshot whose eye is farther than this from the current eye (port metres) is not used.
pub const MAX_MOVE_M: f32 = 4.0;
/// HZB texels nearer than this (port metres) never occlude: the player pawn and the held item (moving occluders a few metres from the third-person camera)
/// may have moved since the snapshot.
pub const NEAR_OCCLUDER_M: f32 = 6.0;

pub fn early_pickup() -> bool { static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *ON.get_or_init(|| fn_live::var("FN_OCCLUSION_PICKUP").map_or(true, |v| v != "0")) }
pub fn enabled() -> bool { static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *ON.get_or_init(|| fn_live::var("FN_OCCLUSION").map_or(true, |v| v != "0")) }

/// One frame's HZB mip on the CPU: `data` = UE device Z (0 = nothing / sky), row-major `w` x `h`; the camera it was rendered with.
pub struct HzbSnapshot {
    pub w: u32, pub h: u32, pub data: Vec<f32>,
    /// Buffer pixels per texel of this mip (2^(mip + 1)), the view size in pixels.
    pub texel_px: f32, pub view: (u32, u32),
    pub view_proj: Mat4, pub eye: Vec3, pub forward: Vec3, pub near: f32,
    /// The UE near plane (m) of the device Z (reflection.rs `UE_NEAR_M`).
    pub ue_near: f32,
    pub frame: u64,
}

struct Slot { buf: wgpu::Buffer, state: Arc<AtomicBool>, busy: bool, meta: Option<Meta> }
#[derive(Clone)]
struct Meta { w: u32, h: u32, row_bytes: u32, texel_px: f32, view: (u32, u32), view_proj: Mat4, eye: Vec3, forward: Vec3, near: f32, ue_near: f32, frame: u64 }

/// The readback ring (owned by the renderer).
pub struct Occlusion { slots: Mutex<Vec<Slot>>, pending: Mutex<Option<usize>>, frame: u64 }

static LATEST: Mutex<Option<Arc<HzbSnapshot>>> = Mutex::new(None);
static FRAME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The newest mapped snapshot, when it is fresh enough to use this frame.
pub fn latest() -> Option<Arc<HzbSnapshot>> {
    if !enabled() { return None; }
    let s = LATEST.lock().ok()?.clone()?;
    (FRAME.load(Ordering::Relaxed).saturating_sub(s.frame) <= MAX_AGE).then_some(s)
}

/// REPORT ONLY (2026-10-08): the age in frames of the newest mapped snapshot whatever [`MAX_AGE`] says (`None` = none yet): tells a stale readback from no
/// readback when [`latest`] returns nothing.
pub fn newest_age() -> Option<u64> { let s = LATEST.lock().ok()?.clone()?; Some(FRAME.load(Ordering::Relaxed).saturating_sub(s.frame)) }

impl Occlusion {
    pub fn new() -> Occlusion { Occlusion { slots: Mutex::new(vec![]), pending: Mutex::new(None), frame: 0 } }
    /// Start of a frame: picks up every mapped readback (the newest becomes [`latest`]).
    pub fn begin_frame(&mut self, device: &wgpu::Device) {
        self.frame += 1;
        FRAME.store(self.frame, Ordering::Relaxed);
        self.pickup(device);
    }
    /// PERF (2026-10-08): picks up every readback mapped by now (the newest becomes [`latest`]) without starting a frame. The app calls it right before the
    /// statics are culled (`Renderer::occlusion_pickup`): a readback that completed during the frame's CPU work is used one frame earlier (the walk saw 38 % of
    /// its frames with only a too-old snapshot). Same snapshots, same tests; `FN_OCCLUSION_PICKUP=0` = only at the start of the renderer's frame (before).
    pub fn pickup(&mut self, device: &wgpu::Device) {
        if enabled() { let _ = device.poll(wgpu::PollType::Poll); } // (runs the map callbacks; never waits)
        let Ok(mut slots) = self.slots.lock() else { return };
        let mut best: Option<HzbSnapshot> = None;
        for s in slots.iter_mut() {
            if !s.busy || !s.state.load(Ordering::Acquire) { continue; }
            if let Some(m) = s.meta.take() {
                let Ok(r) = s.buf.slice(..).get_mapped_range() else { s.buf.unmap(); s.busy = false; s.state.store(false, Ordering::Release); continue };
                let mut data = Vec::with_capacity((m.w * m.h) as usize);
                for y in 0..m.h as usize { let row = &r[y * m.row_bytes as usize..]; for x in 0..m.w as usize { data.push(f16_to_f32(u16::from_le_bytes([row[2 * x], row[2 * x + 1]]))); } }
                drop(r);
                if best.as_ref().map_or(true, |b| b.frame < m.frame) { best = Some(HzbSnapshot { w: m.w, h: m.h, data, texel_px: m.texel_px, view: m.view, view_proj: m.view_proj, eye: m.eye, forward: m.forward, near: m.near, ue_near: m.ue_near, frame: m.frame }); }
            }
            s.buf.unmap();
            s.busy = false;
            s.state.store(false, Ordering::Release);
        }
        if let Some(b) = best { if let Ok(mut l) = LATEST.lock() { if l.as_ref().map_or(true, |o| o.frame < b.frame) { *l = Some(Arc::new(b)); } } }
    }
    /// Records the copy of this frame's HZB mip into a free readback buffer (none free: skipped this frame). `hzb` = the HZB texture (all mips), `view` = the
    /// view size, `cam` = the camera the depth was rendered with, `ue_near` = the device Z's near plane.
    pub fn encode_copy(&self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, hzb: &wgpu::Texture, view: (u32, u32), cam: &crate::camera::CameraParams, ue_near: f32) {
        if !enabled() { return; }
        let mips = hzb.mip_level_count();
        let mut m = 0;
        while m + 1 < mips && (hzb.width() >> m).max(1) > READ_WIDTH { m += 1; }
        let (w, h) = ((hzb.width() >> m).max(1), (hzb.height() >> m).max(1));
        let row_bytes = (w * 2).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let size = (row_bytes * h) as u64;
        let Ok(mut slots) = self.slots.lock() else { return };
        let make = || device.create_buffer(&wgpu::BufferDescriptor { label: Some("hzb occlusion readback"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        // a free buffer of this size, else a new one (at most 4 in flight), else a free one of another size re-created, else skip this frame
        let i = if let Some(i) = slots.iter().position(|s| !s.busy && s.buf.size() == size) { i }
            else if slots.len() < 4 { slots.push(Slot { buf: make(), state: Arc::new(AtomicBool::new(false)), busy: false, meta: None }); slots.len() - 1 }
            else if let Some(i) = slots.iter().position(|s| !s.busy) { slots[i].buf = make(); i }
            else { return };
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: hzb, mip_level: m, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &slots[i].buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row_bytes), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
        let mut c = cam.clone(); c.jitter = [0., 0.];
        slots[i].busy = true;
        slots[i].meta = Some(Meta { w, h, row_bytes, texel_px: (1u32 << (m + 1)) as f32, view, view_proj: c.proj_matrix() * c.view_matrix(), eye: cam.eye, forward: cam.forward.normalize(), near: cam.near, ue_near, frame: self.frame });
        if let Ok(mut p) = self.pending.lock() { *p = Some(i); }
    }
    /// After the frame's submit: maps the buffer copied this frame (read by a later [`Self::begin_frame`]).
    pub fn after_submit(&self) {
        let Some(i) = self.pending.lock().ok().and_then(|mut p| p.take()) else { return };
        let Ok(slots) = self.slots.lock() else { return };
        let st = slots[i].state.clone();
        slots[i].buf.slice(..).map_async(wgpu::MapMode::Read, move |r| { if r.is_ok() { st.store(true, Ordering::Release); } });
    }
}
impl Default for Occlusion { fn default() -> Self { Self::new() } }

impl HzbSnapshot {
    /// Whether a sphere (port world metres) seen from `eye_now` is hidden behind that frame's depth (see the module doc for the conservative rules).
    pub fn sphere_occluded(&self, center: Vec3, radius: f32, eye_now: Vec3) -> bool {
        let moved = (eye_now - self.eye).length();
        if moved > MAX_MOVE_M { return false; }
        let r = radius + moved;
        // nearest view depth of the sphere (UE device Z = near / view depth)
        let zmin = (center - self.eye).dot(self.forward) - r;
        if zmin <= self.near.max(0.01) { return false; }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for k in 0..8 {
            let p = center + Vec3::new(if k & 1 == 0 { -r } else { r }, if k & 2 == 0 { -r } else { r }, if k & 4 == 0 { -r } else { r });
            let c: Vec4 = self.view_proj * p.extend(1.0);
            if c.w <= self.near * 0.5 { return false; }
            let (nx, ny) = (c.x / c.w, c.y / c.w);
            x0 = x0.min(nx); x1 = x1.max(nx); y0 = y0.min(ny); y1 = y1.max(ny);
        }
        // wholly inside that frame's view only (what lay outside it was never in its depth)
        if x0 < -1. || y0 < -1. || x1 > 1. || y1 > 1. { return false; }
        let (vw, vh) = (self.view.0 as f32, self.view.1 as f32);
        let px = |nx: f32| (nx * 0.5 + 0.5) * vw;
        let py = |ny: f32| (0.5 - ny * 0.5) * vh;
        // one texel of margin around the rectangle (TAA jitter, rounding)
        let tx0 = ((px(x0) / self.texel_px).floor() as i32 - 1).max(0) as u32;
        let tx1 = ((px(x1) / self.texel_px).floor() as i32 + 1).min(self.w as i32 - 1).max(0) as u32;
        let ty0 = ((py(y1) / self.texel_px).floor() as i32 - 1).max(0) as u32;
        let ty1 = ((py(y0) / self.texel_px).floor() as i32 + 1).min(self.h as i32 - 1).max(0) as u32;
        if (tx1 - tx0 + 1) * (ty1 - ty0 + 1) > 4096 { return false; } // (a huge rectangle: not worth the scan)
        let obj_z = self.ue_near / zmin * (1. + 1. / 256.);
        let near_z = self.ue_near / NEAR_OCCLUDER_M;
        for y in ty0..=ty1 { for x in tx0..=tx1 {
            // the furthest occluder of the texel must be in front of the sphere's nearest point
            let t = self.data[(y * self.w + x) as usize];
            if t <= obj_z || t >= near_z { return false; }
        } }
        true
    }
}

/// IEEE half -> f32.
fn f16_to_f32(h: u16) -> f32 {
    let s = ((h >> 15) as u32) << 31;
    let e = ((h >> 10) & 0x1f) as u32;
    let m = (h & 0x3ff) as u32;
    let bits = if e == 0 { if m == 0 { s } else { let mut e2 = 127 - 15 + 1; let mut m2 = m; while m2 & 0x400 == 0 { m2 <<= 1; e2 -= 1; } s | (e2 << 23) | ((m2 & 0x3ff) << 13) } }
        else if e == 31 { s | 0x7f80_0000 | (m << 13) } else { s | ((e + 127 - 15) << 23) | (m << 13) };
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn half_conversion() {
        for v in [0.0f32, 1.0, 0.5, 0.001, 0.1, 65504.0, 6.1e-5] { let h = half_bits(v); assert!((f16_to_f32(h) - v).abs() <= v * 1e-3 + 1e-7, "{v}"); }
    }
    fn half_bits(v: f32) -> u16 { let b = v.to_bits(); let e = ((b >> 23) & 0xff) as i32 - 127 + 15; if v == 0. { 0 } else if e <= 0 { ((v / 5.960464477539063e-8).round() as u16) } else { ((e as u16) << 10) | ((b >> 13) & 0x3ff) as u16 } }
    /// A sphere behind a wall that fills the snapshot is occluded; in front of it, beside the view or after the eye moved far, it is not.
    #[test] fn sphere_against_a_wall() {
        let cam = crate::camera::CameraParams { eye: Vec3::ZERO, forward: Vec3::NEG_Z, up: Vec3::Y, fov_deg: 90., near: 0.1, far: 1000., width: 256, height: 128, jitter: [0.; 2] };
        let wall_z = 0.1 / 20.; // a wall 20 m away everywhere (UE device Z with a 0.1 m near)
        let s = HzbSnapshot { w: 64, h: 32, data: vec![wall_z; 64 * 32], texel_px: 4., view: (256, 128), view_proj: cam.proj_matrix() * cam.view_matrix(), eye: cam.eye, forward: cam.forward, near: 0.1, ue_near: 0.1, frame: 1 };
        assert!(s.sphere_occluded(Vec3::new(0., 0., -40.), 2., Vec3::ZERO));
        assert!(!s.sphere_occluded(Vec3::new(0., 0., -10.), 2., Vec3::ZERO), "in front of the wall");
        assert!(!s.sphere_occluded(Vec3::new(60., 0., -40.), 2., Vec3::ZERO), "outside that view");
        assert!(!s.sphere_occluded(Vec3::new(0., 0., -40.), 2., Vec3::new(10., 0., 0.)), "the eye moved too far");
        let mut sky = s; sky.data[16 * 64 + 32] = 0.; assert!(!sky.sphere_occluded(Vec3::new(0., 0., -40.), 2., Vec3::ZERO), "a sky texel in the rectangle");
    }
}
