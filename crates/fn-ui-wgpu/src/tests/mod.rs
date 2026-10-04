//! Headless verification of the wgpu backend: quads/text/clip/9-slice/rotation/mips/batching against a tiny software reference, and GPU-vs-CPU
//! material comparisons. Tests skip (loudly, via eprintln) when no wgpu adapter exists at all.
use crate::UiRenderer;
use fn_ui::assets::{MemAssets, Sampler, Texture, UiAssets};
use fn_ui::draw::TextureId;
use std::sync::{Mutex, OnceLock};

mod quads;
mod materials;
mod soft;

struct Gpu(Mutex<UiRenderer>);
// wgpu handles are Send + Sync; UiRenderer holds only those plus plain data.
unsafe impl Sync for Gpu {}
unsafe impl Send for Gpu {}

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

/// Runs `f` with the shared headless renderer; `None` (after a loud message) when there is no adapter.
pub fn with_gpu<R>(f: impl FnOnce(&mut UiRenderer) -> R) -> Option<R> {
    let g = GPU.get_or_init(|| match UiRenderer::new_headless() {
        Ok(r) => {
            let i = r.adapter_info().unwrap();
            eprintln!("fn-ui-wgpu tests: adapter = {} ({:?}, {:?}, driver {} {})", i.name, i.backend, i.device_type, i.driver, i.driver_info);
            Some(Gpu(Mutex::new(r)))
        }
        Err(e) => { eprintln!("!!!!! fn-ui-wgpu tests SKIPPED: no wgpu adapter (hardware or software): {e} !!!!!"); None }
    });
    let g = g.as_ref()?;
    let mut guard = g.0.lock().unwrap_or_else(|p| p.into_inner());
    // every test owns a fresh UiAssets whose TextureIds restart at 1: start from empty texture caches and the default mode
    guard.clear_caches();
    guard.set_material_mode(crate::MaterialMode::Gpu);
    Some(f(&mut guard))
}

pub fn new_assets() -> UiAssets { UiAssets::new(Box::new(MemAssets::default())) }

pub fn make_tex(assets: &mut UiAssets, w: u32, h: u32, sampler: Sampler, f: impl Fn(u32, u32) -> [u8; 4]) -> TextureId {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h { for x in 0..w { px.extend_from_slice(&f(x, y)); } }
    assets.store.insert(Texture::new(w, h, px, sampler))
}

/// (max abs diff, mean abs diff) over all channels.
pub fn diff(a: &[u8], b: &[u8]) -> (u32, f64) {
    assert_eq!(a.len(), b.len());
    let mut mx = 0u32; let mut sum = 0u64;
    for (x, y) in a.iter().zip(b) { let d = (*x as i32 - *y as i32).unsigned_abs(); mx = mx.max(d); sum += d as u64; }
    (mx, sum as f64 / a.len().max(1) as f64)
}

/// Tiny deterministic generator (xorshift32).
pub struct Rng(pub u32);
impl Rng {
    pub fn next(&mut self) -> u32 { let mut x = self.0; x ^= x << 13; x ^= x >> 17; x ^= x << 5; self.0 = x; x }
    pub fn f(&mut self) -> f32 { (self.next() >> 8) as f32 / (1u32 << 24) as f32 }
    pub fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }
}
