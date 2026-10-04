//! Interactive window (winit 0.30): Athena third-person / orbit / fly cameras over the viewer scene.
//!
//! Controls: left-drag = look (Athena: control yaw/pitch, Orbit: orbit the character), wheel = zoom (Orbit) / speed (Fly),
//! V = cycle camera mode, W/A/S/D/Q/E = fly (Fly mode, hold right mouse to look), 1 shadows, 2 AO, 3 bloom, 4 fog, 5 vignette,
//! 6 sky, 7 rest pose / clip, P prints frame time, F12 saves screenshot.png (swapchain format), Esc quits.
use crate::{athena_cam, assets::AssetIndex, scene::{Scene, SceneConfig}, Cli, CameraMode};
use fn_render::{CameraParams, FrameParams, Gpu, RenderOptions};
use glam::Vec3;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

struct Gfx {
    window: Arc<Window>,
    surface: fn_render::wgpu::Surface<'static>,
    config: fn_render::wgpu::SurfaceConfiguration,
    gpu: Gpu,
    scene: Scene,
    hud: Option<crate::ui_overlay::HudOverlay>,
}
struct Viewer {
    cli: Cli,
    gfx: Option<Gfx>,
    options: RenderOptions,
    mode: CameraMode,
    yaw: f32,
    pitch: f32,
    orbit_dist: f32,
    eye: Vec3,
    fly_speed: f32,
    keys: [bool; 6],
    left_down: bool,
    right_down: bool,
    last_cursor: Option<(f64, f64)>,
    start: Instant,
    last_frame: Instant,
    frame_times: Vec<f32>,
    error: Option<String>,
}

pub fn run(cli: Cli) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let options = cli.options.clone();
    let mut viewer = Viewer {
        mode: cli.camera, yaw: cli.yaw, pitch: cli.pitch, orbit_dist: 3.2, eye: Vec3::new(0., 1.8, 16.), fly_speed: 4., keys: [false; 6],
        left_down: false, right_down: false, last_cursor: None, start: Instant::now(), last_frame: Instant::now(), frame_times: vec![], error: None,
        cli, gfx: None, options,
    };
    event_loop.run_app(&mut viewer).map_err(|e| e.to_string())?;
    match viewer.error { Some(e) => Err(e), None => Ok(()) }
}

impl Viewer {
    fn init(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        let (w, h) = self.cli.size;
        let window = Arc::new(el.create_window(Window::default_attributes().with_title("fn-viewer (wgpu)").with_inner_size(PhysicalSize::new(w, h))).map_err(|e| e.to_string())?);
        let instance = Gpu::create_instance();
        let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let gpu = Gpu::new(instance, Some(&surface))?;
        eprintln!("[gpu] {}", gpu.describe());
        let size = window.inner_size();
        let caps = surface.get_capabilities(&gpu.adapter);
        // the post pass writes sRGB-encoded values itself: use a non-sRGB swapchain format
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let mut config = surface.get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1)).ok_or("surface not supported by the adapter")?;
        config.format = format;
        config.present_mode = if self.cli.vsync { fn_render::wgpu::PresentMode::AutoVsync } else { fn_render::wgpu::PresentMode::AutoNoVsync };
        surface.configure(&gpu.device, &config);
        let index = AssetIndex::scan(&self.cli.assets)?;
        let scene = Scene::load(&gpu, &index, format, &SceneConfig { use_clip: self.cli.use_clip, use_noise: self.cli.use_noise, props: self.cli.props, extra_props: self.cli.prop_count, static_dir: self.cli.static_dir.clone(), prop_grid: self.cli.prop_grid, prop_seed: self.cli.prop_seed, prop_kind: self.cli.prop_kind.clone() })?;
        self.eye = athena_cam::settled_eye(scene.feet, false, self.yaw, self.pitch);
        let hud = if self.cli.demo_ui { Some(crate::ui_overlay::HudOverlay::new(&gpu.device, &gpu.queue, format)?) } else { None };
        self.gfx = Some(Gfx { window, surface, config, gpu, scene, hud });
        Ok(())
    }

    fn camera(&mut self, dt: f32, w: u32, h: u32) -> CameraParams {
        let g = self.gfx.as_ref().unwrap();
        match self.mode {
            CameraMode::Athena => {
                let eye = athena_cam::settled_eye(g.scene.feet, false, self.yaw, self.pitch);
                CameraParams::from_yaw_pitch(eye, self.yaw, self.pitch, w, h)
            }
            CameraMode::Orbit => {
                let (f, _, _) = athena_cam::view_basis(self.yaw, self.pitch);
                let eye = g.scene.focus() - f * self.orbit_dist;
                CameraParams::from_yaw_pitch(Vec3::new(eye.x, eye.y.max(0.15), eye.z), self.yaw, self.pitch, w, h)
            }
            CameraMode::Fly => {
                let (f, r, _) = athena_cam::view_basis(self.yaw, self.pitch);
                let k = |i: usize| self.keys[i] as i32 as f32;
                let delta = f * (k(0) - k(1)) + r * (k(3) - k(2)) + Vec3::Y * (k(4) - k(5));
                self.eye += delta * self.fly_speed * dt;
                CameraParams::from_yaw_pitch(self.eye, self.yaw, self.pitch, w, h)
            }
        }
    }

    fn frame(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.25);
        self.last_frame = now;
        let (w, h) = { let g = self.gfx.as_ref().unwrap(); (g.config.width, g.config.height) };
        let hh = h;
        let cam = self.camera(dt, w, h);
        let time = self.start.elapsed().as_secs_f32();
        let g = self.gfx.as_mut().unwrap();
        g.scene.animate(time);
        use fn_render::wgpu::CurrentSurfaceTexture as C;
        let frame = match g.surface.get_current_texture() {
            C::Success(t) | C::Suboptimal(t) => t,
            C::Outdated | C::Lost => { g.surface.configure(&g.gpu.device, &g.config); return Ok(()); }
            C::Timeout | C::Occluded => return Ok(()),
            C::Validation => return Err("surface validation error".into()),
        };
        let view = frame.texture.create_view(&Default::default());
        let ids = g.scene.model_ids();
        let fp = FrameParams { camera: &cam, time, dt, models: &ids, options: &self.options };
        match g.hud.as_mut() {
            Some(h) => { h.update(w, hh, self.yaw, dt); g.scene.renderer.render_with_overlay(&fp, &view, Some(&mut h.pass())); }
            None => { g.scene.renderer.render(&fp, &view); }
        }
        g.window.pre_present_notify();
        g.gpu.queue.present(frame);
        self.frame_times.push(dt * 1000.);
        if self.frame_times.len() >= 120 {
            let mean = self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32;
            g.window.set_title(&format!("fn-viewer (wgpu) {:?}  {:.2} ms/frame ({:.0} fps)  {w}x{h}", self.mode, mean, 1000. / mean));
            self.frame_times.clear();
        }
        Ok(())
    }

    fn key(&mut self, code: KeyCode, pressed: bool, el: &ActiveEventLoop) {
        let fly = match code { KeyCode::KeyW => Some(0), KeyCode::KeyS => Some(1), KeyCode::KeyA => Some(2), KeyCode::KeyD => Some(3), KeyCode::KeyE => Some(4), KeyCode::KeyQ => Some(5), _ => None };
        if let Some(i) = fly { self.keys[i] = pressed; }
        if !pressed { return; }
        match code {
            KeyCode::Escape => el.exit(),
            KeyCode::KeyV => {
                self.mode = match self.mode { CameraMode::Athena => CameraMode::Orbit, CameraMode::Orbit => CameraMode::Fly, CameraMode::Fly => CameraMode::Athena };
                if self.mode == CameraMode::Fly {
                    if let Some(g) = &self.gfx { self.eye = athena_cam::settled_eye(g.scene.feet, false, self.yaw, self.pitch); }
                }
            }
            KeyCode::Digit1 => self.options.shadows = !self.options.shadows,
            KeyCode::Digit2 => self.options.ao = !self.options.ao,
            KeyCode::Digit3 => self.options.bloom = !self.options.bloom,
            KeyCode::Digit4 => self.options.fog = !self.options.fog,
            KeyCode::Digit5 => self.options.vignette = !self.options.vignette,
            KeyCode::Digit6 => self.options.sky_layers = if self.options.sky_layers == 0 { fn_render::sky::LAYER_ALL } else { 0 },
            KeyCode::KeyT => if let Some(g) = &self.gfx {
                let t = g.scene.renderer.last_pass_timings();
                if !t.supported { eprintln!("[gpu timings] unsupported on this device"); }
                else { eprintln!("[gpu timings] sum {:.2} ms: {}", t.sum_ms(), t.groups().iter().map(|(n, ms)| format!("{n} {ms:.2}")).collect::<Vec<_>>().join(", ")); }
            },
            KeyCode::Digit7 => if let Some(g) = &mut self.gfx { g.scene.use_clip = !g.scene.use_clip; },
            _ => {}
        }
    }
}

impl ApplicationHandler for Viewer {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_none() {
            if let Err(e) = self.init(el) { self.error = Some(e); el.exit(); }
        }
    }
    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gfx {
                    if size.width > 0 && size.height > 0 {
                        g.config.width = size.width;
                        g.config.height = size.height;
                        g.surface.configure(&g.gpu.device, &g.config);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key { self.key(code, event.state == ElementState::Pressed, el); }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                match button { MouseButton::Left => self.left_down = down, MouseButton::Right => self.right_down = down, _ => {} }
                if !down { self.last_cursor = None; }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.left_down || self.right_down {
                    if let Some((lx, ly)) = self.last_cursor {
                        self.yaw = (self.yaw + (position.x - lx) as f32 * 0.15).rem_euclid(360.);
                        self.pitch = (self.pitch - (position.y - ly) as f32 * 0.15).clamp(-athena_cam::PITCH_LIMIT, athena_cam::PITCH_LIMIT);
                    }
                    self.last_cursor = Some((position.x, position.y));
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta { MouseScrollDelta::LineDelta(_, y) => y, MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40. };
                if self.mode == CameraMode::Fly { self.fly_speed = (self.fly_speed * 1.2f32.powf(d)).clamp(0.2, 200.); }
                else { self.orbit_dist = (self.orbit_dist * 0.9f32.powf(d)).clamp(0.5, 60.); }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.frame() { self.error = Some(e); el.exit(); }
                if let Some(g) = &self.gfx { g.window.request_redraw(); }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(g) = &self.gfx { g.window.request_redraw(); }
    }
}
