//! `fn-ui-demo --live`: the real screens driven by real input in a winit window.
//!
//! What runs here is exactly what `fn-app` wires up, minus audio and the 3D renderer:
//! * winit events -> [`winput::translate`] -> `MenuSystem::input` (pointer, wheel, keys with left/right modifiers, focus);
//! * the menu owns the Settings screens and their working copy; once a frame `MenuSystem::frame_with` gets a `UiContext` made of the game's
//!   real `fn_settings::Settings`, a [`PlatformHost`] over the window (display modes of the monitor, window mode / size on Apply) and the apply layer
//!   (`StandardBackend`: the real `apply_video` / `apply_input` / `apply_audio` / `apply_game`, the ini is written atomically);
//! * Apply, Restore, Back, the Esc stack and the 15 s "keep these video settings?" dialog all work: change Window Mode, press Apply, answer Yes / No or wait;
//! * behind the menu the HUD is drawn from a `HudState` produced by `fn-hud-adapter` from a real `GameSim` (WASD walks, Left / Right turn: compass and minimap
//!   follow) and the settings: Video > Show FPS, Game > HUD toggles act on it (after Apply).
//!
//! Keys (menu closed): Esc menu, WASD walk, Left/Right turn, 1-6 quickbar, I sample inventory, [ ] health, ; ' shield, L loading screen.
//! Options: `--live [--size WxH] [--assets dir|pak|auto|layered] [--install <FortniteGame>] [--ini <file>|--no-save] [--frames N] [--script]`.
//! `--script` drives the whole flow by itself through `MenuSystem::input` in the real window (open Settings, change Vsync, Apply, switch Window Mode, Apply, let the
//! keep-video dialog answer No, rebind a key, Esc out dropping it), prints the result and exits non-zero on a failed check. The window goes borderless fullscreen for a moment.
use crate::winput;
use crate::window::{create_gfx, Gfx};
use fn_game::{FrameInput, GameSim};
use fn_hud_adapter::{hud_state, FrameClock, HudInputs};
use fn_ui::menu::{lower, HostRequest, MenuSystem, StandardBackend, UiContext as HostCx};
use fn_ui::scene::UiContext;
use fn_ui::screens::hud::{Hud, HudItem, HudLoading};
use fn_ui::settings::{DisplayMode, FrameLimiter, NullPlatform, PlatformHost, Settings, VideoHooks, WindowMode};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::{application::ApplicationHandler, dpi::PhysicalSize, event::*, event_loop::{ActiveEventLoop, EventLoop}, keyboard::{KeyCode, PhysicalKey}, window::{Fullscreen, Window, WindowId}};

/// The renderer-side video hooks of the demo (everything renderer-bound is a no-op: there is no 3D scene here).
struct DemoVideo;
impl VideoHooks for DemoVideo {}

/// `PlatformHost` over the demo window. Exclusive fullscreen is NOT performed (that would change the monitor's video mode): Fullscreen and
/// Windowed Fullscreen both become a borderless window covering the monitor; Windowed restores the frame and resizes the client.
struct DemoPlatform { window: Option<Arc<Window>>, vsync: bool, focused: bool }
impl DemoPlatform {
    fn monitor_modes(&self) -> Vec<DisplayMode> {
        let Some(m) = self.window.as_ref().and_then(|w| w.current_monitor()) else { return NullPlatform::static_16_9().modes };
        let mut v: Vec<DisplayMode> = m.video_modes().map(|vm| { let s = vm.size(); DisplayMode { width: s.width, height: s.height, hz: (vm.refresh_rate_millihertz() + 500) / 1000 } })
            .filter(|m| m.width >= 640 && m.height >= 360).collect();
        v.sort_by_key(|m| (m.width, m.height, m.hz));
        if v.is_empty() { NullPlatform::static_16_9().modes } else { v }
    }
}
impl PlatformHost for DemoPlatform {
    fn display_modes(&mut self) -> Vec<DisplayMode> { self.monitor_modes() }
    fn current_display_mode(&mut self) -> Option<DisplayMode> {
        let m = self.window.as_ref()?.current_monitor()?;
        let s = m.size();
        Some(DisplayMode { width: s.width, height: s.height, hz: m.refresh_rate_millihertz().map_or(0, |mhz| (mhz + 500) / 1000) })
    }
    fn apply_display(&mut self, mode: WindowMode, w: u32, h: u32) -> Result<(), String> {
        let win = self.window.as_ref().ok_or("no window")?;
        match mode {
            WindowMode::Windowed => { win.set_fullscreen(None); let _ = win.request_inner_size(PhysicalSize::new(w.max(640), h.max(360))); }
            WindowMode::WindowedFullscreen | WindowMode::Fullscreen => win.set_fullscreen(Some(Fullscreen::Borderless(None))),
        }
        Ok(())
    }
    fn set_swap_interval(&mut self, on: bool) -> bool { self.vsync = on; true }
    fn window_focused(&self) -> bool { self.focused }
}

struct Live {
    gfx: Option<Gfx>, ctx: UiContext, menu: MenuSystem,
    settings: Settings, platform: DemoPlatform, backend: StandardBackend<DemoVideo>,
    sim: GameSim, hud: Hud, inputs: HudInputs, clock: FrameClock, limiter: FrameLimiter,
    size: (u32, u32), last: Instant, frames: u64, max_frames: Option<u64>, loading_since: Option<f32>,
    items_on: bool, vsync_applied: Option<bool>,
    script: Option<Script>,
}

impl Live {
    fn new(args: &[String]) -> Result<Self, String> {
        let size = args.iter().position(|a| a == "--size").and_then(|i| args.get(i + 1)).and_then(|s| s.split_once('x')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).unwrap_or((1280, 720));
        let max_frames = args.iter().position(|a| a == "--frames").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok());
        let ini: Option<PathBuf> = if args.iter().any(|a| a == "--no-save") { None } else {
            Some(args.iter().position(|a| a == "--ini").and_then(|i| args.get(i + 1)).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("fn-ui-demo-settings.ini")))
        };
        let (settings, report) = match &ini { Some(p) => Settings::load(p), None => (Settings::default(), fn_ui::settings::LoadReport { status: fn_ui::settings::LoadStatus::Missing, path: PathBuf::new(), file_version: None, warnings: vec![], backup: None }) };
        println!("settings: {:?} {} {}", report.status, ini.as_ref().map_or("(not saved)".to_owned(), |p| p.display().to_string()), report.warnings.join("; "));
        let ctx = crate::make_context_default(args, "auto")?;
        let mut menu = MenuSystem::new(ctx.fonts.clone());
        fn_ui::menu::pages::register_all_into(&mut menu);
        let backend = StandardBackend::new(DemoVideo, ini);
        Ok(Live {
            gfx: None, ctx, menu, settings, platform: DemoPlatform { window: None, vsync: true, focused: true }, backend,
            sim: GameSim::new(), hud: Hud::new(), inputs: HudInputs::default(), clock: FrameClock::default(), limiter: FrameLimiter::new(),
            size, last: Instant::now(), frames: 0, max_frames, loading_since: None, items_on: false, vsync_applied: None,
            script: args.iter().any(|a| a == "--script").then(Script::new),
        })
    }

    fn init(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        let g = create_gfx(el, self.size, "fn-ui-demo (live)")?;
        self.platform.window = Some(g.window.clone());
        // the stored resolution starts as the window size so the first Apply does not resize anything by itself
        let sz = g.window.inner_size();
        let _ = self.settings.set("video.resolution", fn_ui::settings::Value::Size(sz.width, sz.height));
        let _ = self.settings.drain_changes();
        self.gfx = Some(g);
        self.menu.open();
        println!("live: Esc toggles the menu; menu closed: WASD walk, Left/Right turn, 1-6 quickbar, I inventory, [ ] health, ; ' shield, L loading");
        Ok(())
    }

    /// Hotkeys of the game side while the menu is closed (the router already tracked the key).
    fn game_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5 | KeyCode::Digit6 => {
                let i = match code { KeyCode::Digit1 => 0, KeyCode::Digit2 => 1, KeyCode::Digit3 => 2, KeyCode::Digit4 => 3, KeyCode::Digit5 => 4, _ => 5 };
                self.inputs.selected = i;
            }
            KeyCode::KeyI => {
                self.items_on = !self.items_on;
                self.inputs.items = if self.items_on {
                    let mut it: [Option<HudItem>; 6] = Default::default();
                    it[0] = Some(HudItem::pickaxe());
                    for (i, (rar, n)) in [(1u8, 3u32), (2, 120), (3, 7), (4, 24)].into_iter().enumerate() { it[i + 1] = Some(HudItem::new(rar, "", Some(n))); }
                    Some(it)
                } else { None };
                self.inputs.selected = 0;
            }
            KeyCode::BracketLeft => self.inputs.vitals.0 = self.inputs.vitals.0.saturating_sub(10),
            KeyCode::BracketRight => self.inputs.vitals.0 = (self.inputs.vitals.0 + 10).min(100),
            KeyCode::Semicolon => self.inputs.vitals.1 = self.inputs.vitals.1.saturating_sub(10),
            KeyCode::Quote => self.inputs.vitals.1 = (self.inputs.vitals.1 + 10).min(100),
            KeyCode::KeyL => self.loading_since = if self.loading_since.is_some() { None } else { Some(0.) },
            _ => {}
        }
    }

    fn frame(&mut self, el: &ActiveEventLoop) {
        let dt = self.last.elapsed().as_secs_f32().clamp(0., 0.1);
        self.last = Instant::now();
        let Some((w, h)) = self.gfx.as_ref().map(|g| (g.config.width, g.config.height)) else { return };
        // ---- game side
        let held = |m: &MenuSystem, c: u16| m.router().is_held(c);
        if !self.menu.is_open() {
            let k = |c: u16| held(&self.menu, c) as i32 as f32;
            let inp = FrameInput { forward: k(0x57) - k(0x53), side: k(0x44) - k(0x41), yaw_delta: (k(0x27) - k(0x25)) * 120. * dt, ..Default::default() };
            self.sim.step_real(&inp, dt);
        }
        self.clock.tick(dt);
        // ---- scripted input (--script)
        if let Some(mut s) = self.script.take() { s.tick(self, dt); if !s.done { self.script = Some(s); } else { println!("script: {}", if s.failures.is_empty() { "OK".to_owned() } else { format!("FAILED: {}", s.failures.join("; ")) }); std::process::exit(if s.failures.is_empty() { 0 } else { 1 }); } }
        // ---- UI
        {
            let mut cx = HostCx::new(&mut self.settings, &mut self.platform, &mut self.backend);
            self.menu.frame_with(&mut cx, w as f32, h as f32, dt);
        }
        for r in self.menu.take_requests() { match r { HostRequest::QuitToDesktop => { el.exit(); return; } HostRequest::Closed => {} } }
        let Some(g) = self.gfx.as_mut() else { return };
        // swap interval / frame cap chosen through Settings
        if self.vsync_applied != Some(self.platform.vsync) {
            self.vsync_applied = Some(self.platform.vsync);
            g.config.present_mode = if self.platform.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
            g.surface.configure(&g.device, &g.config);
        }
        // ---- HUD from the game + settings through the adapter
        self.inputs.frame = self.clock.stats();
        self.inputs.loading = self.loading_since.map(|t| { let done = ((t / 4.) * 20.).min(20.) as usize; HudLoading { done, total: 20, label: format!("Loading HUD ({done}/20)"), tip: 0, fade: 0. } });
        if let Some(t) = self.loading_since.as_mut() { *t += dt; }
        let st = hud_state(&self.sim, &self.settings, &self.inputs);
        self.hud.set_state(&st);
        self.hud.tick(dt);
        let mut list = fn_ui::draw::RenderList::new();
        self.hud.present(&mut self.ctx, w as f32, h as f32, &mut list);
        lower::lower(self.menu.parts().0, &mut self.ctx, w as f32, h as f32, &mut list);
        // ---- draw
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => { g.surface.configure(&g.device, &g.config); return; }
        };
        let view = frame.texture.create_view(&Default::default());
        g.ui.render(&list, &mut self.ctx.assets, &view, w, h, Some([0.25, 0.35, 0.25, 1.]));
        g.ui.queue().present(frame);
        self.frames += 1;
        if self.frames % 30 == 0 { g.window.set_title(&format!("fn-ui-demo (live) {}x{} {:.0} fps  menu: {}", w, h, self.clock.stats().fps, self.menu.stack_names().join(" > "))); }
        let (cap, vsync) = (self.backend.runtime.frame_cap, self.backend.runtime.vsync);
        self.limiter.throttle(cap, vsync);
        if self.max_frames.map_or(false, |m| self.frames >= m) { println!("live: {} frames, {:.0} fps, menu stack {:?}, missing assets {:?}", self.frames, self.clock.stats().fps, self.menu.stack_names(), self.ctx.assets.missing); el.exit(); }
    }
}

impl ApplicationHandler for Live {
    fn resumed(&mut self, el: &ActiveEventLoop) { if self.gfx.is_none() { if let Err(e) = self.init(el) { eprintln!("error: {e}"); el.exit(); } } }
    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match &ev {
            WindowEvent::CloseRequested => { el.exit(); return; }
            WindowEvent::Resized(s) => { if let Some(g) = self.gfx.as_mut() { g.config.width = s.width.max(1); g.config.height = s.height.max(1); g.surface.configure(&g.device, &g.config); } return; }
            WindowEvent::Focused(f) => self.platform.focused = *f,
            WindowEvent::RedrawRequested => { self.frame(el); if let Some(g) = &self.gfx { g.window.request_redraw(); } return; }
            _ => {}
        }
        let was_open = self.menu.is_open();
        if let Some(ie) = winput::translate(&ev) {
            let out = self.menu.input(ie);
            if out.escape_edge && !self.menu.is_open() { self.menu.open(); }
            // game hotkeys: only for a fresh key press while the menu was closed (and is still closed: Esc just opened it)
            if let WindowEvent::KeyboardInput { event: KeyEvent { physical_key: PhysicalKey::Code(code), state: ElementState::Pressed, repeat: false, .. }, .. } = &ev {
                if !was_open && !self.menu.is_open() { self.game_key(*code); }
            }
        }
    }
    fn about_to_wait(&mut self, _el: &ActiveEventLoop) { if let Some(g) = &self.gfx { g.window.request_redraw(); } }
}

// ---- the scripted flow (--script) ---------------------------------------------------------------------------------------------------
enum Target { Text(&'static str), Entry(&'static str), Arrow(&'static str, bool) }
enum Act { Click(Target), Key(u16), Wait(f32), Check(&'static str, fn(&Live) -> bool) }
struct Script { acts: Vec<Act>, i: usize, phase: u32, wait: f32, pos: (f32, f32), failures: Vec<String>, done: bool }
impl Script {
    fn new() -> Self {
        use Act::*; use Target::*;
        let acts = vec![
            Wait(0.5), Check("menu open", |l| l.menu.is_open() && l.menu.stack_names() == ["pause"]),
            Click(Entry("SettingsButton")), Wait(0.3), Check("Settings screen", |l| l.menu.stack_names() == ["pause", "settings"]),
            Click(Arrow("Vsync", true)), Check("working copy changed, game untouched", |l| l.menu.session().working.video.vsync != l.settings.video.vsync),
            Click(Text("Apply")), Wait(0.6), Check("Apply reached the game", |l| l.menu.session().working.video.vsync == l.settings.video.vsync && !l.menu.session().is_dirty()),
            Click(Arrow("Window Mode", false)), Click(Text("Apply")), Wait(0.8),
            Check("keep-video dialog", |l| l.menu.stack_names().last().map(String::as_str) == Some("settings-modal")),
            Check("window is borderless fullscreen", |l| l.settings.video.window_mode == WindowMode::WindowedFullscreen && l.platform.window.as_ref().map_or(false, |w| w.fullscreen().is_some())),
            Wait(1.0), Click(Text("No")), Wait(0.8),
            Check("reverted to windowed", |l| l.settings.video.window_mode == WindowMode::Windowed && l.platform.window.as_ref().map_or(false, |w| w.fullscreen().is_none())),
            Click(Text("Input")), Wait(0.3), Click(Text("Space Bar")), Wait(0.2), Check("capturing", |l| l.menu.state().is_listening()),
            Key(0x43), Wait(0.3), Check("rebound to C", |l| l.menu.session().working.input.bindings.get(fn_ui::settings::A_JUMP)[0].name() == "C"),
            Key(0x1B), Wait(0.5), Check("Esc went back and dropped the unapplied binding", |l| !l.menu.session().is_dirty() && l.menu.stack_names() == ["pause"]),
            Key(0x1B), Wait(0.5), Check("second Esc closes the menu", |l| !l.menu.is_open()),
        ];
        Script { acts, i: 0, phase: 0, wait: 0., pos: (0., 0.), failures: Vec::new(), done: false }
    }
    fn rect_of(l: &Live, label: &str) -> Option<fn_ui::layout::Rect> {
        use fn_ui::menu::Cmd;
        l.menu.parts().0.cmds.iter().find_map(|c| if let Cmd::Text { x, top, box_h, text, font, px, .. } = c { if text == label { Some(fn_ui::layout::Rect::new(*x, *top, l.menu.fonts().measure(*font, text, *px as f32), *box_h)) } else { None } } else { None })
    }
    fn resolve(l: &Live, t: &Target) -> Option<(f32, f32)> {
        use fn_ui::menu::pages::slate::*;
        let g = l.gfx.as_ref()?; let (w, h) = (g.config.width as f32, g.config.height as f32);
        match t {
            Target::Text(s) => Self::rect_of(l, s).map(|r| (r.x + r.w * 0.5, r.y + r.h * 0.5)),
            Target::Entry(wd) => fn_ui::menu::mainmenu::entry_rect(w, h, fn_ui::menu::mainmenu::MenuContext::Match, wd).map(|r| (r.x + r.w * 0.5, r.y + r.h * 0.5)),
            Target::Arrow(label, right) => {
                let r = Self::rect_of(l, label)?; let s = fn_ui::layout::Viewport::new(w, h).scale;
                let cx = r.x + r.w + LABEL_GAP * s;
                Some((if *right { cx + (CONTROL_MIN_W - ARROW_PAD - ARROW_W * 0.5) * s } else { cx + (ARROW_PAD + ARROW_W * 0.5) * s }, r.y + r.h * 0.5))
            }
        }
    }
    fn fail(&mut self, what: &str) { println!("script: FAIL {what}"); self.failures.push(what.to_owned()); }
    /// One frame of the script: a click is move -> press -> release -> settle over four frames; the positions come from what the menu drew.
    fn tick(&mut self, l: &mut Live, dt: f32) {
        use fn_ui::router::InputEvent as E;
        if self.i >= self.acts.len() { self.done = true; return; }
        if self.wait > 0. { self.wait -= dt; return; }
        let mut next = false;
        match &self.acts[self.i] {
            Act::Wait(s) => { self.wait = *s; next = true; }
            Act::Check(name, f) => { if !f(l) { let s = l.menu.session(); println!("script: state: status {:?}, changed {:?}, forced_dirty {}, request {:?}, stack {:?}", s.status().map(|x| x.text), s.changed_ids(), s.forced_dirty, s.pending_request(), l.menu.stack_names()); self.fail(name); } else { println!("script: ok   {name}"); } next = true; }
            Act::Key(code) => {
                let c = *code;
                if self.phase == 0 { l.menu.input(E::Key { vk: c, pressed: true, repeat: false }); self.phase = 1; }
                else { l.menu.input(E::Key { vk: c, pressed: false, repeat: false }); self.phase = 0; next = true; }
            }
            Act::Click(t) => match self.phase {
                0 => match Self::resolve(l, t) { Some(p) => { self.pos = p; l.menu.input(E::PointerMoved { x: p.0, y: p.1 }); self.phase = 1; } None => { self.fail("click target not drawn"); next = true; } },
                1 => { l.menu.input(E::PointerButton { button: fn_ui::input::MouseButton::Left, pressed: true }); self.phase = 2; }
                2 => { l.menu.input(E::PointerButton { button: fn_ui::input::MouseButton::Left, pressed: false }); self.phase = 3; }
                _ => { self.phase = 0; self.wait = 0.1; next = true; }
            },
        }
        if next { self.i += 1; if self.i >= self.acts.len() { self.done = true; } }
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let mut app = Live::new(args)?;
    EventLoop::new().map_err(|e| e.to_string())?.run_app(&mut app).map_err(|e| e.to_string())
}
