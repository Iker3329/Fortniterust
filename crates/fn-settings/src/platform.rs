//! Platform seam of the settings apply layer. The old `settings::win` module (Win32 + WGL calls) is replaced by traits; the front-end crate
//! (`fn-app`: winit + wgpu) implements them. The behaviour the old Win32 code had, which a `PlatformHost` implementation must reproduce:
//!
//! * **Display modes** (`display_modes`): the monitor's real modes (`EnumDisplaySettingsEx`, flags 0), only >= [`MIN_RES`], 32-bit colour,
//!   non-interlaced. `display_sizes` additionally keeps only sizes the OS accepts as an exclusive mode (`CDS_TEST` probe) and dedupes/sorts.
//! * **Windowed**: restore any exclusive mode, standard framed window, client size `fit_windowed_client(w, h, work area, frame)` centred in the
//!   monitor's work area (never larger than the screen, a zero axis falls back to 1280x720, never below [`MIN_RES`] unless the screen is smaller).
//! * **WindowedFullscreen**: restore any exclusive mode, borderless popup covering the whole monitor.
//! * **Fullscreen**: exclusive mode on the monitor the window is on, refresh rate = `pick_refresh(modes, w, h, current_hz)` (current rate when the
//!   new size offers it, else the highest); an `Err` leaves the window untouched. `restore_display` undoes it on that same monitor.
//! * **Vsync**: swap interval 1/0. `set_swap_interval` returns false when the platform cannot do it (the old WGL extension missing); the caller then
//!   reports an error and the frame limiter paces instead.
//! * **Vblank wait** (`wait_vblank`): block until the compositor's next refresh and return an HRESULT-like code (negative = failure). Used by
//!   [`crate::VblankWait`] to avoid the driver's spin wait.
//! * **Cursor** (`set_cursor_captured`): captured = hidden and pinned to the client centre (raw-input look); released = the stock arrow.
//!   Capturing is only legal while the window is foreground, gameplay has started and the run is not scripted (`fn_game::host::may_capture`).
use crate::{Level, Runtime, VideoHooks, WindowMode, DISPLAY_HZ, PostFlags};

/// One mode of a display: size and refresh rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayMode { pub width: u32, pub height: u32, pub hz: u32 }

/// Pure: distinct (width, height) pairs of `modes`, sorted by width then height. `valid` filters (the live code passes an exclusive-mode probe).
pub fn distinct_sizes(modes: &[DisplayMode], mut valid: impl FnMut(u32, u32) -> bool) -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = modes.iter().map(|m| (m.width, m.height)).collect(); v.sort_unstable(); v.dedup(); v.retain(|&(w, h)| valid(w, h)); v
}
/// Pure: refresh rate to use for a w x h exclusive mode: `prefer_hz` when the display offers it at that size, else the highest offered, else 0 (let the OS choose).
pub fn pick_refresh(modes: &[DisplayMode], w: u32, h: u32, prefer_hz: u32) -> u32 {
    let hz: Vec<u32> = modes.iter().filter(|m| m.width == w && m.height == h).map(|m| m.hz).collect();
    if hz.contains(&prefer_hz) { prefer_hz } else { hz.into_iter().max().unwrap_or(0) }
}
/// Mip level a texture with `levels` mips starts at when `skip` top levels are dropped (never past the last level). Was `assets::base_level_for`.
pub fn base_level_for(skip: u32, levels: u32) -> u32 { skip.min(levels.saturating_sub(1)) }

/// Window / display / cursor services of the host OS. Every method has a no-op default so partial implementations (tests, headless) are fine.
pub trait PlatformHost {
    /// The monitor's supported modes (see the module docs for the filter).
    fn display_modes(&mut self) -> Vec<DisplayMode> { vec![] }
    /// The monitor's current (desktop) mode.
    fn current_display_mode(&mut self) -> Option<DisplayMode> { None }
    /// Whether the OS would accept `w`x`h` as an exclusive mode (a test, nothing is applied).
    fn exclusive_mode_supported(&mut self, _w: u32, _h: u32) -> bool { true }
    /// Valid (width, height) sizes for the Resolution row: distinct, sorted, accepted by the OS.
    fn display_sizes(&mut self) -> Vec<(u32, u32)> { let modes = self.display_modes(); distinct_sizes(&modes, |w, h| self.exclusive_mode_supported(w, h)) }
    /// Window mode + resolution (see the module docs).
    fn apply_display(&mut self, _mode: WindowMode, _w: u32, _h: u32) -> Result<(), String> { Ok(()) }
    /// Undoes an exclusive mode switch on the monitor it was applied to.
    fn restore_display(&mut self) {}
    /// Sets the swap interval; false when unsupported.
    fn set_swap_interval(&mut self, _on: bool) -> bool { true }
    fn swap_interval(&self) -> Option<i32> { None }
    /// Blocks until the next compositor refresh; negative = failure.
    fn wait_vblank(&mut self) -> i32 { 0 }
    /// Pins + hides (true) or releases + shows (false) the cursor.
    fn set_cursor_captured(&mut self, _captured: bool) {}
    fn cursor_captured(&self) -> bool { false }
    /// Whether the game window is the foreground window (keyboard/mouse focus).
    fn window_focused(&self) -> bool { true }
}

/// A host that does nothing and remembers what it was asked to do: headless runs and tests.
#[derive(Default, Debug)]
pub struct NullPlatform { pub modes: Vec<DisplayMode>, pub current: Option<DisplayMode>, pub swap_interval_supported: bool, pub log: Vec<String>, captured: bool }
impl NullPlatform {
    pub fn new(modes: Vec<DisplayMode>) -> Self { let current = modes.first().copied(); NullPlatform { modes, current, swap_interval_supported: true, log: vec![], captured: false } }
}
impl NullPlatform {
    /// Fixed 16:9-ish size list used by the UI demo and the UI tests (every size at 60 Hz, desktop = 1920x1080).
    pub const STATIC_SIZES: [(u32, u32); 9] = [(1280, 720), (1360, 768), (1366, 768), (1600, 900), (1920, 1080), (2560, 1080), (2560, 1440), (3440, 1440), (3840, 2160)];
    pub fn static_16_9() -> Self {
        let modes: Vec<DisplayMode> = Self::STATIC_SIZES.iter().map(|&(w, h)| DisplayMode { width: w, height: h, hz: 60 }).collect();
        let current = modes.iter().copied().find(|m| (m.width, m.height) == (1920, 1080));
        NullPlatform { modes, current, swap_interval_supported: true, log: vec![], captured: false }
    }
}
impl PlatformHost for NullPlatform {
    fn display_modes(&mut self) -> Vec<DisplayMode> { self.modes.clone() }
    fn current_display_mode(&mut self) -> Option<DisplayMode> { self.current }
    fn apply_display(&mut self, mode: WindowMode, w: u32, h: u32) -> Result<(), String> { self.log.push(format!("display {mode:?} {w}x{h}")); Ok(()) }
    fn restore_display(&mut self) { self.log.push("restore_display".into()); }
    fn set_swap_interval(&mut self, on: bool) -> bool { self.log.push(format!("swap_interval {on}")); self.swap_interval_supported }
    fn set_cursor_captured(&mut self, captured: bool) { self.captured = captured; self.log.push(format!("cursor_captured {captured}")); }
    fn cursor_captured(&self) -> bool { self.captured }
}

/// Live video hooks: the window/vsync/cap/FPS parts go to the [`PlatformHost`] and the [`Runtime`] flags, the camera far plane is set here, and
/// every renderer-bound setting (shadows, post stages, gamma, anti-aliasing, texture quality) is forwarded to the renderer's own [`VideoHooks`].
/// (The old `win::LiveVideo` poked `post::Post`, `shadow::Shadow` and `assets::set_texture_quality` directly.)
pub struct LiveVideo<'a, P: PlatformHost + ?Sized, R: VideoHooks + ?Sized> { pub platform: &'a mut P, pub renderer: &'a mut R, pub runtime: &'a mut Runtime }
impl<P: PlatformHost + ?Sized, R: VideoHooks + ?Sized> VideoHooks for LiveVideo<'_, P, R> {
    fn set_display(&mut self, mode: WindowMode, w: u32, h: u32) -> Result<(), String> {
        let r = self.platform.apply_display(mode, w, h);
        if let Some(m) = self.platform.current_display_mode() { DISPLAY_HZ.store(m.hz, std::sync::atomic::Ordering::Relaxed); }
        r
    }
    fn set_anti_aliasing(&mut self, level: Level) -> Result<(), String> { self.renderer.set_anti_aliasing(level) }
    fn set_texture_quality(&mut self, level: Level) -> Result<(), String> { self.renderer.set_texture_quality(level) }
    fn set_vsync(&mut self, on: bool) -> Result<(), String> {
        if let Some(m) = self.platform.current_display_mode() { DISPLAY_HZ.store(m.hz, std::sync::atomic::Ordering::Relaxed); }
        let ok = self.platform.set_swap_interval(on);
        self.runtime.vsync = on && ok;
        if on && !ok { Err("swap interval unavailable; frame limiter used instead".into()) } else { Ok(()) }
    }
    fn set_frame_cap(&mut self, fps: u32) -> Result<(), String> { self.runtime.frame_cap = fps; Ok(()) }
    fn set_shadow_map_size(&mut self, size: Option<u32>) -> Result<(), String> { self.renderer.set_shadow_map_size(size) }
    fn set_shadow_quality(&mut self, level: Level) -> Result<(), String> { self.renderer.set_shadow_quality(level) }
    fn set_post(&mut self, f: PostFlags) -> Result<(), String> { self.renderer.set_post(f) }
    fn set_post_quality(&mut self, l: Level) -> Result<(), String> { self.renderer.set_post_quality(l) }
    fn set_display_gamma(&mut self, g: f32) -> Result<(), String> { self.renderer.set_display_gamma(g) }
    fn set_far_plane(&mut self, m: f32) -> Result<(), String> { fn_camera::set_far_plane(m); self.renderer.set_far_plane(m) }
    fn set_view_distance_scale(&mut self, s: f32) -> Result<(), String> { self.renderer.set_view_distance_scale(s) }
    fn set_screen_percentage(&mut self, p: f32) -> Result<(), String> { self.renderer.set_screen_percentage(p) }
    fn set_effects_quality(&mut self, l: Level) -> Result<(), String> { self.renderer.set_effects_quality(l) }
    fn set_show_fps(&mut self, on: bool) -> Result<(), String> { self.runtime.show_fps = on; self.renderer.set_show_fps(on) }
    fn set_motion_blur(&mut self, on: bool) -> Result<(), String> { self.renderer.set_motion_blur(on) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Settings, ApplyReport, MIN_RES};
    #[derive(Default)] struct Rend { calls: Vec<String> }
    impl VideoHooks for Rend {
        fn set_shadow_map_size(&mut self, s: Option<u32>) -> Result<(), String> { self.calls.push(format!("shadows {s:?}")); Ok(()) }
        fn set_texture_quality(&mut self, l: Level) -> Result<(), String> { self.calls.push(format!("textures {l}")); Ok(()) }
    }
    #[test] fn live_video_routes_window_parts_to_the_platform_and_the_rest_to_the_renderer() {
        let mut p = NullPlatform::new(vec![DisplayMode { width: 1920, height: 1080, hz: 144 }]); let mut r = Rend::default(); let mut rt = Runtime::default(); let mut s = Settings::default();
        let rep: ApplyReport = { let mut hooks = LiveVideo { platform: &mut p, renderer: &mut r, runtime: &mut rt }; s.apply_video(&mut hooks) };
        assert!(rep.ok(), "{:?}", rep.failed);
        assert!(p.log.iter().any(|l| l.starts_with("display ")) && p.log.contains(&"swap_interval true".to_string()), "{:?}", p.log);
        assert!(r.calls.iter().any(|c| c.starts_with("shadows")) && r.calls.iter().any(|c| c.starts_with("textures")), "{:?}", r.calls);
        assert!(rt.vsync && DISPLAY_HZ.load(std::sync::atomic::Ordering::Relaxed) == 144);
    }
    #[test] fn missing_swap_interval_reports_an_error_and_disables_runtime_vsync() {
        let mut p = NullPlatform::new(vec![]); p.swap_interval_supported = false; let mut r = Rend::default(); let mut rt = Runtime::default();
        let mut hooks = LiveVideo { platform: &mut p, renderer: &mut r, runtime: &mut rt };
        assert!(hooks.set_vsync(true).is_err()); assert!(!hooks.runtime.vsync); assert!(hooks.set_vsync(false).is_ok());
    }
    #[test] fn static_16_9_host_lists_the_demo_sizes_and_reports_1080p_as_the_desktop() {
        let mut p = NullPlatform::static_16_9();
        assert_eq!(p.display_sizes(), NullPlatform::STATIC_SIZES.to_vec());
        assert!(p.display_sizes().iter().all(|&(w, h)| w >= MIN_RES.0 && h >= MIN_RES.1));
        assert_eq!(p.current_display_mode(), Some(DisplayMode { width: 1920, height: 1080, hz: 60 }));
    }
    #[test] fn display_sizes_default_impl_dedupes_and_probes() {
        let m = |w, h, hz| DisplayMode { width: w, height: h, hz };
        let mut p = NullPlatform::new(vec![m(1920, 1080, 60), m(1920, 1080, 144), m(1280, 720, 60)]);
        assert_eq!(p.display_sizes(), vec![(1280, 720), (1920, 1080)]);
    }
}
