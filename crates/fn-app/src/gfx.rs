//! wgpu surface + device of the game window (Vulkan / DX12 through `fn_render::Gpu`). The scene renderer (`fn_render::Renderer`) and the UI renderer
//! (`fn_ui_wgpu::UiRenderer`) are created from these handles by the loader; this type owns only what a window needs: the surface, its configuration
//! (size, present mode = vsync) and frame acquisition.
//!
//! Size safety (the old crash: a 9000 px wide window -> `Surface::configure` validation error -> panic -> exit 0xC0000409): every size that reaches the
//! surface goes through [`clamp_surface_size`] with the DEVICE's real `max_texture_dimension_2d` (and `platform::MAX_CLIENT`), at creation and at every
//! resize; a window that grew beyond the limit is asked to shrink back. wgpu errors no longer panic: `Gpu::install_error_sink` records them and the next
//! [`Gfx::acquire`] reports [`Acquire::Fatal`] with the wgpu text (the app then exits with code 1 and `main` shows a native box).
use fn_render::{Gpu, GpuErrorSink};
use std::sync::Arc;
use wgpu::{CurrentSurfaceTexture as C, PresentMode, Surface, SurfaceConfiguration};
use winit::dpi::PhysicalSize;
use winit::window::Window;

pub struct Gfx {
    pub window: Arc<Window>,
    pub surface: Surface<'static>,
    pub config: SurfaceConfiguration,
    pub gpu: Gpu,
    pub vsync: bool,
    /// wgpu errors recorded by the uncaptured-error callback (see module docs).
    pub errors: Arc<GpuErrorSink>,
    /// Largest surface edge: min(device limit, `platform::MAX_CLIENT`).
    pub max_dim: u32,
    /// Acquires in a row that came back `Validation` (see [`Gfx::acquire`]).
    validation_streak: u32,
}

/// Operations slower than this are logged as `[slow]` lines (surface reconfigure, renderer target re-creation).
pub const SLOW_MS: f32 = 8.0;

pub enum Acquire { Frame(wgpu::SurfaceTexture), /// Nothing to draw this time (outdated surface reconfigured, occluded, timeout, zero-sized).
    Skip, Fatal(String) }

pub fn present_mode(vsync: bool) -> PresentMode { if vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync } }

/// A surface size the device accepts: each axis within 1..=`max_dim`.
pub fn clamp_surface_size(w: u32, h: u32, max_dim: u32) -> (u32, u32) { (w.clamp(1, max_dim.max(1)), h.clamp(1, max_dim.max(1))) }

impl Gfx {
    pub fn new(window: Arc<Window>, vsync: bool) -> Result<Gfx, String> {
        #[cfg(not(target_arch = "wasm32"))]
        let (surface, gpu) = {
            let instance = Gpu::create_instance();
            let surface = instance.create_surface(window.clone()).map_err(|e| format!("cannot create the window surface: {e}"))?;
            let gpu = Gpu::new(instance, Some(&surface))?;
            (surface, gpu)
        };
        // BROWSER PORT: the adapter and device were awaited before the event loop started, on the page's canvas (the window draws into it)
        #[cfg(target_arch = "wasm32")]
        let (surface, gpu) = crate::web::take_gpu()?;
        let errors = gpu.install_error_sink();
        let max_dim = gpu.max_texture_dimension_2d().min(crate::platform::MAX_CLIENT.0.max(crate::platform::MAX_CLIENT.1));
        let size = window.inner_size();
        let (sw, sh) = clamp_surface_size(size.width, size.height, max_dim);
        let caps = surface.get_capabilities(&gpu.adapter);
        if caps.formats.is_empty() { return Err("the surface reports no formats".into()); }
        let mut config = surface.get_default_config(&gpu.adapter, sw, sh).ok_or("the surface is not supported by the adapter")?;
        // The post pass writes sRGB-encoded values itself and the UI composites in gamma space: both need a NON-sRGB target.
        config.format = fn_ui_wgpu::pick_surface_format(&caps);
        config.present_mode = present_mode(vsync);
        // INVENTORY FIX3: the UI's SBackgroundBlur pass copies the region it blurs out of the back buffer (when the surface allows it)
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) { config.usage |= wgpu::TextureUsages::COPY_SRC; }
        surface.configure(&gpu.device, &config);
        if (sw, sh) != (size.width.max(1), size.height.max(1)) { let _ = window.request_inner_size(PhysicalSize::new(sw, sh)); }
        let gfx = Gfx { window, surface, config, gpu, vsync, errors, max_dim, validation_streak: 0 };
        match gfx.take_error() { Some(e) => Err(e), None => Ok(gfx) }
    }
    pub fn format(&self) -> wgpu::TextureFormat { self.config.format }
    pub fn size(&self) -> (u32, u32) { (self.config.width, self.config.height) }
    /// The first wgpu error recorded since the last call, as one message (None when the device is healthy).
    pub fn take_error(&self) -> Option<String> {
        let all = self.errors.take();
        (!all.is_empty()).then(|| format!("GPU error: {}{}", all[0], if self.errors.total() > 1 { format!(" (+{} more, see the log)", self.errors.total() - 1) } else { String::new() }))
    }
    /// Reconfigure for a new client size, clamped to what the device accepts (an oversized window is asked to shrink back). False for a zero-sized
    /// (minimised) client, which cannot be configured.
    pub fn resize(&mut self, w: u32, h: u32) -> bool {
        if w == 0 || h == 0 { return false; }
        let (cw, ch) = clamp_surface_size(w, h, self.max_dim);
        if (cw, ch) != (w, h) {
            eprintln!("[gfx] window {w}x{h} exceeds the {} px surface limit: using {cw}x{ch}", self.max_dim);
            let _ = self.window.request_inner_size(PhysicalSize::new(cw, ch));
        }
        if (cw, ch) != self.size() { self.config.width = cw; self.config.height = ch; self.configure(); }
        true
    }
    /// `surface.configure` with the current config; logs when it was slow (it recreates the swap chain: tens of ms is normal, more is a finding).
    fn configure(&self) {
        let t = std::time::Instant::now();
        self.surface.configure(&self.gpu.device, &self.config);
        let ms = t.elapsed().as_secs_f32() * 1000.;
        if ms > SLOW_MS { eprintln!("[slow] surface.configure {}x{} {:?} took {ms:.1} ms", self.config.width, self.config.height, self.config.present_mode); }
    }
    pub fn set_vsync(&mut self, on: bool) {
        if on == self.vsync { return; }
        self.vsync = on; self.config.present_mode = present_mode(on);
        self.configure();
    }
    pub fn acquire(&mut self) -> Acquire {
        // BROWSER PORT: WebGPU validation errors arrive asynchronously in the sink (no blocking error scopes on the web), long after the call that
        // caused them, and one invalid object (e.g. a sampler) must not stop every later frame: logged there, fatal natively
        #[cfg(target_arch = "wasm32")]
        if let Some(e) = self.take_error() { eprintln!("[gfx] WebGPU error (reported asynchronously; rendering goes on): {}", e.lines().next().unwrap_or("")); }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(e) = self.take_error() { return Acquire::Fatal(e); }
        // Window changes can arrive before winit's Resized event. Keep the actual
        // client extent in sync before acquiring, including the loading -> world transition
        // (weapons-fork fix for a user-observed surface validation crash).
        let actual = self.window.inner_size();
        if actual.width == 0 || actual.height == 0 { return Acquire::Skip; }
        let previous_size = self.size();
        self.resize(actual.width, actual.height);
        // The caller may already have built UI/camera data using the previous extent.
        if self.size() != previous_size { return Acquire::Skip; }
        #[cfg(not(target_arch = "wasm32"))]
        let (acquired, validation) = {
            let scope = self.gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let acquired = self.surface.get_current_texture();
            (acquired, pollster::block_on(scope.pop()))
        };
        // BROWSER PORT: no per-frame error scope (its result is a promise the frame cannot wait for); surface errors reach the uncaptured-error sink
        #[cfg(target_arch = "wasm32")]
        let (acquired, validation): (_, Option<wgpu::Error>) = (self.surface.get_current_texture(), None);
        if let Some(error) = validation {
            return Acquire::Fatal(format!("surface acquisition failed at {}x{} ({:?}): {error}", self.config.width, self.config.height, self.config.present_mode));
        }
        match acquired {
            C::Success(t) | C::Suboptimal(t) => { self.validation_streak = 0; Acquire::Frame(t) }
            C::Outdated | C::Lost => { self.configure(); Acquire::Skip }
            C::Timeout | C::Occluded => Acquire::Skip,
            // VIDEO FIX (2026-10-06): a lone `Validation` acquire (the swap chain's image acquisition failed, no error text reached the scope or the sink) ended
            // 2 of 7 scripted runs on Vulkan (verification/first_use/run3.log, shots_warm.log). It is treated like a lost surface: reconfigure and skip the
            // frame; only a streak of them is fatal, so a recording session survives a driver / capture-hook hiccup.
            C::Validation => {
                self.validation_streak += 1;
                let detail = self.take_error();
                eprintln!("[gfx] surface acquire returned Validation at {}x{} ({:?}), {} in a row: {}", self.config.width, self.config.height, self.config.present_mode, self.validation_streak, detail.as_deref().unwrap_or("no error text"));
                if self.validation_streak >= 5 { return Acquire::Fatal(detail.unwrap_or_else(|| "surface validation error (5 acquires in a row)".into())); }
                self.configure();
                Acquire::Skip
            }
        }
    }
    pub fn present(&self, frame: wgpu::SurfaceTexture) { self.window.pre_present_notify(); self.gpu.queue.present(frame); }
    pub fn describe(&self) -> String { self.gpu.describe() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn vsync_maps_to_auto_present_modes() { assert_eq!(present_mode(true), PresentMode::AutoVsync); assert_eq!(present_mode(false), PresentMode::AutoNoVsync); }
    #[test] fn surface_sizes_are_clamped_per_axis_to_the_device_limit() {
        assert_eq!(clamp_surface_size(9000, 720, 8192), (8192, 720));
        assert_eq!(clamp_surface_size(1280, 9000, 8192), (1280, 8192));
        assert_eq!(clamp_surface_size(99999, 1, 8192), (8192, 1));
        assert_eq!(clamp_surface_size(u32::MAX, u32::MAX, 8192), (8192, 8192));
        assert_eq!(clamp_surface_size(0, 0, 8192), (1, 1));
        assert_eq!(clamp_surface_size(3000, 3000, 2048), (2048, 2048), "an adapter with a smaller limit wins");
        assert_eq!(clamp_surface_size(640, 360, 0), (1, 1), "a nonsense limit never returns zero");
    }

    /// Real window + real surface at sizes above the limit: the old code panicked inside `Surface::configure` here. Skips (with a note) without a GPU.
    #[cfg(windows)]
    #[test] fn oversized_windows_configure_a_clamped_surface_without_a_gpu_error() {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        let Ok(el) = winit::event_loop::EventLoop::builder().with_any_thread(true).build() else { eprintln!("no event loop: skipped"); return };
        #[allow(deprecated)]
        let window = el.create_window(Window::default_attributes().with_visible(false).with_inner_size(PhysicalSize::new(9000u32, 720u32)));
        let Ok(window) = window else { eprintln!("no window: skipped"); return };
        let window = Arc::new(window);
        let mut gfx = match Gfx::new(window.clone(), false) { Ok(g) => g, Err(e) => { assert!(!e.contains("GPU error"), "{e}"); eprintln!("no GPU: skipped ({e})"); return } };
        let max = gfx.max_dim;
        assert!(gfx.size().0 <= max && gfx.size().1 <= max, "created at {:?} (limit {max}); window was {:?}", gfx.size(), window.inner_size());
        for (w, h) in [(9000u32, 500u32), (1280, 9000), (99999, 1), (1280, 720), (0, 5)] {
            gfx.resize(w, h);
            let (sw, sh) = gfx.size();
            assert!(sw >= 1 && sh >= 1 && sw <= max && sh <= max, "{w}x{h} -> {sw}x{sh}");
            if w > 0 && h > 0 { assert_eq!((sw, sh), clamp_surface_size(w, h, max)); }
            match gfx.acquire() { Acquire::Fatal(e) => panic!("{w}x{h}: {e}"), Acquire::Frame(f) => drop(f), Acquire::Skip => {} }
        }
        assert!(gfx.take_error().is_none(), "no wgpu error was recorded");
        // the safety net: a deliberately invalid configure (what the old code did) no longer panics; it is recorded and `acquire` reports it
        let mut bad = gfx.config.clone(); bad.width = max + 808;
        gfx.surface.configure(&gfx.gpu.device, &bad);
        match gfx.acquire() { Acquire::Fatal(e) => assert!(e.starts_with("GPU error") && e.contains("maximum supported texture size"), "{e}"), _ => panic!("the validation error must surface as Acquire::Fatal") }
    }
}
