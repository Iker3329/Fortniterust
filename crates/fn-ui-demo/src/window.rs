//! Interactive winit window using fn-ui-wgpu. Left/Right (or PageUp/PageDown) cycle screens, Esc quits.
//! Mouse movement/clicks are forwarded to a live `MenuSystem` when the menu scenes are shown only through the scripted scenes (static state).
use fn_ui::scene::{all_scenes, SceneDef, UiContext};
use std::sync::Arc;
use winit::{application::ApplicationHandler, event::*, event_loop::{ActiveEventLoop, EventLoop}, keyboard::{Key, NamedKey}, window::{Window, WindowId}};

pub struct Gfx { pub window: Arc<Window>, pub surface: wgpu::Surface<'static>, pub config: wgpu::SurfaceConfiguration, pub ui: fn_ui_wgpu::UiRenderer, pub device: wgpu::Device }
struct App { gfx: Option<Gfx>, ctx: UiContext, scenes: Vec<SceneDef>, idx: usize, size: (u32, u32) }

/// Create the window, the wgpu surface and the UI renderer (shared by the scene viewer and the live mode).
pub fn create_gfx(el: &ActiveEventLoop, size: (u32, u32), title: &str) -> Result<Gfx, String> {
    let window = Arc::new(el.create_window(Window::default_attributes().with_title(title).with_inner_size(winit::dpi::PhysicalSize::new(size.0, size.1))).map_err(|e| e.to_string())?);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() })).map_err(|e| e.to_string())?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).map_err(|e| e.to_string())?;
    let caps = surface.get_capabilities(&adapter);
    let format = fn_ui_wgpu::pick_surface_format(&caps);
    let sz = window.inner_size();
    let config = wgpu::SurfaceConfiguration { usage: wgpu::TextureUsages::RENDER_ATTACHMENT, format, width: sz.width.max(1), height: sz.height.max(1), present_mode: wgpu::PresentMode::AutoVsync, desired_maximum_frame_latency: 2, alpha_mode: caps.alpha_modes[0], view_formats: vec![], color_space: Default::default() };
    surface.configure(&device, &config);
    let ui = fn_ui_wgpu::UiRenderer::with_device(device.clone(), queue, format);
    Ok(Gfx { window, surface, config, ui, device })
}

impl App {
    fn init(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        self.gfx = Some(create_gfx(el, self.size, "fn-ui-demo")?);
        Ok(())
    }
    fn draw(&mut self) {
        let Some(g) = self.gfx.as_mut() else { return };
        let (w, h) = (g.config.width, g.config.height);
        let list = (self.scenes[self.idx].build)(&mut self.ctx, w as f32, h as f32);
        g.window.set_title(&format!("fn-ui-demo - {} ({}/{}) {}x{}", self.scenes[self.idx].name, self.idx + 1, self.scenes.len(), w, h));
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => { g.surface.configure(&g.device, &g.config); return; }
        };
        let view = frame.texture.create_view(&Default::default());
        // Stand-in "world" behind HUD scenes: a flat green-grey so translucent UI is visible.
        let bg = if self.scenes[self.idx].over_world { [0.25, 0.35, 0.25, 1.] } else { [0., 0., 0., 1.] };
        g.ui.render(&list, &mut self.ctx.assets, &view, w, h, Some(bg));
        g.ui.queue().present(frame);
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) { if self.gfx.is_none() { if let Err(e) = self.init(el) { eprintln!("error: {e}"); el.exit(); } } }
    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => { if let Some(g) = self.gfx.as_mut() { g.config.width = s.width.max(1); g.config.height = s.height.max(1); g.surface.configure(&g.device, &g.config); g.window.request_redraw(); } }
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key, state: ElementState::Pressed, .. }, .. } => {
                let n = self.scenes.len();
                match logical_key { Key::Named(NamedKey::Escape) => el.exit(), Key::Named(NamedKey::ArrowRight | NamedKey::PageDown) => self.idx = (self.idx + 1) % n, Key::Named(NamedKey::ArrowLeft | NamedKey::PageUp) => self.idx = (self.idx + n - 1) % n, _ => {} }
                if let Some(g) = &self.gfx { g.window.request_redraw(); }
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let scenes = all_scenes();
    let idx = args.iter().position(|a| a == "--screen").and_then(|i| args.get(i + 1)).and_then(|n| scenes.iter().position(|s| s.name == *n)).unwrap_or(0);
    let size = args.iter().position(|a| a == "--size").and_then(|i| args.get(i + 1)).and_then(|s| s.split_once('x')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).unwrap_or((1280, 720));
    let mut app = App { gfx: None, ctx: crate::make_context(args)?, scenes, idx, size };
    EventLoop::new().map_err(|e| e.to_string())?.run_app(&mut app).map_err(|e| e.to_string())
}

