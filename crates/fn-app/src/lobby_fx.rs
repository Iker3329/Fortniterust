use crate::lobby_scene::{CameraFrame, LobbyTranslucent, NEAR_CM, FAR_CM};
use crate::lobby_world::{LobbySceneData, Xf};
use crate::pickups_dxbc::Dxbc;
use fn_fx::view::ViewParams;
use fn_particles::{Env, Instance, ParticleSystem, Transform};
use fn_render::fx::FxRenderer;
use glam::{Mat3, Vec3};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const PAD_SPARKLE: &str = "/Game/Effects/Fort_Effects/Effects/Characters/Outlander/InTheZone/P_Outlander_Teleport";
pub const PAD_SWIRL: &str = "/Game/UI/Foundation/Lobby/materials/P_LobbySpawnSpin";

pub struct FxSpawn { pub name: String, pub system: Arc<ParticleSystem>, pub xf: Transform, pub dilation: f32 }

pub struct LobbyFxAssets { pub spawns: Vec<FxSpawn>, pub notes: Vec<String> }

fn pkg(path: &str) -> &str { path.split('.').next().unwrap_or(path) }

fn to_transform(x: &Xf) -> Transform {
    let s = (x.s.x.abs() + x.s.y.abs() + x.s.z.abs()) / 3.0;
    Transform { pos: x.t, rot: Mat3::from_quat(x.q), scale: s }
}

pub fn load_assets(set: &fn_pak::PakSet, data: &LobbySceneData) -> LobbyFxAssets {
    let mut out = LobbyFxAssets { spawns: vec![], notes: vec![] };
    let mut cache: std::collections::HashMap<String, Option<Arc<ParticleSystem>>> = Default::default();
    let mut get = |path: &str, notes: &mut Vec<String>| -> Option<Arc<ParticleSystem>> {
        let p = pkg(path).to_owned();
        cache.entry(p.clone()).or_insert_with(|| match fn_particles::import::import_system(set, &p) {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => { notes.push(format!("{p}: {e}")); None }
        }).clone()
    };
    for p in data.particles.iter().filter(|p| p.visible && p.auto_activate) {
        let Some(t) = p.template.as_deref() else { continue };
        if pkg(t) == PAD_SPARKLE || pkg(t) == PAD_SWIRL { continue; }
        if let Some(sys) = get(t, &mut out.notes) { out.spawns.push(FxSpawn { name: format!("{}.{}", p.actor, p.component), system: sys, xf: to_transform(&p.xf), dilation: 1.0 }); }
    }
    let comps: [(&str, &str, [f32; 3], [f32; 3], [f32; 3], f32); 3] = [
        ("PS-NoPlayer-Sparkle", PAD_SPARKLE, [0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.4, 0.5, 1.0], 0.3),
        ("PS-NoPlayer-Swirl01", PAD_SWIRL, [0.0, 0.0, 49.0], [90.0, 0.0, 0.0], [5.0, 5.0, 5.0], 0.4),
        ("PS-NoPlayer-Swirl02", PAD_SWIRL, [0.0, 0.0, 51.0], [-90.0, 0.0, 0.0], [4.0, 4.0, 4.0], 1.0),
    ];
    let pad_fx = fn_live::var("FN_LOBBY_PAD_FX").map_or(false, |v| v == "1");
    for pl in data.placements.iter().filter(|p| pad_fx && !p.hidden && !p.player_on_pad) {
        for (name, tpl, loc, rot, scale, dil) in comps {
            let Some(sys) = get(tpl, &mut out.notes) else { continue };
            let world = Xf::from_pyr(loc, rot, scale).compose(&pl.root);
            out.spawns.push(FxSpawn { name: format!("{}.{name}", pl.actor), system: sys, xf: to_transform(&world), dilation: dil });
        }
    }
    out
}

pub struct LobbyFx {
    fx: FxRenderer,
    dx: Option<Result<Dxbc, String>>,
    install: PathBuf,
    live: Vec<(String, Instance, f32)>,
    last_time: Option<f32>,
    errors: std::collections::HashSet<String>,
}

impl LobbyFx {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, install: &Path, assets: LobbyFxAssets, eye: Vec3) -> LobbyFx {
        let fx = FxRenderer::new(device, queue, crate::lobby_scene::HDR_FORMAT, crate::lobby_scene::DEPTH_FORMAT);
        let live: Vec<(String, Instance, f32)> = assets.spawns.into_iter().enumerate().map(|(i, s)| {
            let d = (s.xf.pos - eye).length();
            (s.name, Instance::new(s.system, s.xf, 0x4c4f_4242 ^ i as u64, d), s.dilation)
        }).collect();
        for n in &assets.notes { eprintln!("[lobby fx] {n}"); }
        for (n, i, _) in &live { eprintln!("[lobby fx] {n}: {} at {:?}", i.system.name, i.xf.pos); }
        LobbyFx { fx, dx: None, install: install.to_path_buf(), live, last_time: None, errors: Default::default() }
    }
    pub fn count(&self) -> usize { self.live.len() }
}

fn view_params(cam: &CameraFrame, time: f32, dt: f32) -> ViewParams {
    let sw = |v: Vec3| Vec3::new(v.x, v.z, v.y);
    ViewParams { eye: sw(cam.eye) * 0.01, forward: sw(cam.fwd), up: Vec3::Y, tan_half: (cam.tan_x, cam.tan_y), near_m: NEAR_CM * 0.01, far_m: FAR_CM * 0.01, width: cam.width, height: cam.height, time, real_time: time, delta_world: dt }
}

impl LobbyTranslucent for LobbyFx {
    fn prepare(&mut self, cam: &CameraFrame, time: f32, depth_only: &wgpu::TextureView, depth_gen: u64, light: &[[f32; 4]; 10]) {
        let dt = match self.last_time { Some(t) if time >= t => (time - t).min(0.1), _ => 0.0 };
        self.last_time = Some(time);
        let env = Env::none();
        for (_, inst, dil) in self.live.iter_mut() { if dt > 0.0 { inst.tick(dt * *dil, &env); } }
        let view = view_params(cam, time, dt);
        if self.dx.is_none() {
            let r = Dxbc::load(&self.install, &mut self.fx);
            if let Err(e) = &r { eprintln!("[lobby fx] materials unavailable: {e}"); }
            self.dx = Some(r);
        }
        let Some(Ok(dx)) = self.dx.as_mut() else { return };
        dx.begin(&view);
        dx.override_light(light);
        let eye = [view.eye.x, view.eye.y, view.eye.z];
        for (name, inst, _) in &self.live {
            let mut draws = vec![];
            inst.draw(eye, &mut draws);
            let r = 50_000.0f32;
            let bounds = fn_render::primitive_ub::BoxSphereBounds { origin: inst.xf.pos.to_array(), extent: [r, r, r], radius: r * 1.732 };
            for e in &draws {
                if let Err(err) = dx.draw_emitter(&mut self.fx, e, &inst.xf, &bounds, dt) {
                    let k = format!("{name}/{}/{}", e.system, e.emitter);
                    if self.errors.insert(k.clone()) { eprintln!("[lobby fx] {k}: not drawn: {err}"); }
                }
            }
        }
        let frame = dx.take_frame();
        self.fx.set_view_size((cam.width, cam.height));
        self.fx.set_frame(frame);
        self.fx.prepare(depth_only, depth_gen);
    }
    fn draw_opaque<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) { self.fx.draw(pass, false); }
    fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.fx.draw(pass, true);
        self.fx.draw_separate(pass);
    }
}
