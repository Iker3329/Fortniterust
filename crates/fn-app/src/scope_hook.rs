//! SCOPE HOOK (fn-app): the scoped weapon's picture. While the held weapon's scope timeline (`B_Ranged_Generic.AnimateScopePostProcess`, driven by `ads::AdsDriver`) has a
//! non-zero `DownScopePostProcessAmount`, the renderer's scope pass runs the ORIGINAL `DownScopePostProcess_Athena` post-process material program over the finished
//! image, and once the timeline finished forward (`ScopeMesh1P` visible) it first draws the original `S_SniperScope_Proto` mesh (see `fn_matshader::scope` for what is
//! recovered, inferred and not done). The world is rendered through the weapon's scope camera (FOV 15, `Athena_PlayerCameraModeTargetingScope`) by the normal scene pass;
//! nothing here changes the camera.
//!
//! * [`install`] (from the scene build): decodes + uploads the material / mesh of the first ADS profile with a scope (the data names them; no path is hard-coded here).
//! * [`set`] (every frame, from the ADS step): the timeline weight and the mesh visibility.
//! * [`submit`] (from `Scene::draw`): the frame's draws for the renderer.
use fn_matshader::scope::{ScopeAssets, ScopeInput, ScopeScene};
use fn_matshader::view::ViewInput;
use fn_render::scope_pass::ScopeFrame;
use fn_render::{CameraParams, Renderer};
use fn_weapons::recovered_ads::ScopeData;
use std::path::Path;
use std::sync::Mutex;

struct State { scene: Option<ScopeScene>, amount: f32, mesh: bool, reported: bool }
static STATE: Mutex<State> = Mutex::new(State { scene: None, amount: 0., mesh: false, reported: false });

/// Builds the scope pass for `data` (the sniper's `ScopeData`). On error nothing is installed and a scoped weapon shows no scope picture (the error is printed).
pub fn install(renderer: &mut Renderer, install_dir: &Path, data: &ScopeData) {
    let built = fn_pak::PakSet::open_install(install_dir).map_err(|e| e.to_string())
        .and_then(|set| ScopeAssets::load(&set, &data.material, &data.scope_mesh, data.scope_camera_offset_amount))
        .and_then(|a| a.install(renderer.scope()));
    match built {
        Ok(s) => { eprintln!("[scope] ON: {} post-process program (SM5 FFPostProcessMaterialPS_HighEnd, DXBC -> WGSL) + {} mesh, camera offset {} cm", data.material.rsplit('.').next().unwrap_or(""), data.scope_mesh.rsplit('.').next().unwrap_or(""), data.scope_camera_offset_amount); STATE.lock().unwrap().scene = Some(s); }
        Err(e) => eprintln!("[scope] NOT available, the scoped view stays the plain scope-camera world: {e}"),
    }
}

/// This frame's scope timeline outputs (`ads::Frame::scope_amount` / `scope_mesh`).
pub fn set(amount: f32, mesh_visible: bool) { let mut s = STATE.lock().unwrap(); s.amount = if amount.is_finite() { amount.clamp(0., 1.) } else { 0. }; s.mesh = mesh_visible; }

/// POSTFX: this frame's `DownScopePostProcessAmount` = the BlendWeight of `DownScopePostProcess` (the timeline sets both, B_Ranged_Generic).
pub fn amount() -> f32 { STATE.lock().map(|s| s.amount).unwrap_or(0.) }

/// Hands the frame's scope draws to the renderer (call before `Renderer::render`).
pub fn submit(renderer: &mut Renderer, camera: &CameraParams, time: f32) {
    let mut s = STATE.lock().unwrap();
    let frame = match s.scene.as_ref() {
        Some(scene) if s.amount > 0. => {
            let (tx, ty) = camera.tan_half();
            let view = ViewInput { eye: camera.eye, forward: camera.forward, up: camera.up, tan_half: (tx, ty), near_m: camera.near, far_m: camera.far, width: camera.width, height: camera.height, time };
            scene.frame(&ScopeInput { view, amount: s.amount, mesh_visible: s.mesh })
        }
        _ => ScopeFrame::default(),
    };
    if frame.post && !s.reported { s.reported = true; eprintln!("[scope] first scoped frame: amount {:.2}, mesh {}", s.amount, s.mesh); }
    if !frame.post { s.reported = false; }
    drop(s);
    renderer.set_scope_frame(frame);
}
