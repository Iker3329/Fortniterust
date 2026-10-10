//! FRONT END BACKDROP: the 3D front-end level behind the login screen and the game picker, drawn by the front-end scene renderer (`lobby_scene`) from the
//! 1.8 data for the `EFrontEndCamera::Login` view.
//!
//! * `LoginScreen` / `SubgameSelect` enter their state with `FortFrontEndContext.SetFrontEndCamera(7)` (`EFrontEndCamera::Login`); `UFortFrontEndContext::
//!   SetFrontEndCamera` (exe 0x142de5d20) -> `AFortFrontEndCameraManager::SetCamera`. `/Game/Maps/Frontend`'s `FrontEndSettingsBP_31.Cameras` pairs `Login`
//!   with `FrontendCamera_Main_646` (root (32, 40960, 36) cm, pitch -1.2 / yaw -180, `CameraComponent.FieldOfView` 93).
//! * In front of it, the always-loaded `/Game/Maps/UI/Frontend_BG_Main` (LevelTransform (0, 40960, 0)): black `M_UnlitBackground` domes, the curved
//!   `S_UI_Backplate_noLOD` (`M_UI_Backplate_C`, unlit gradient) and the translucent `S_UI_Sphere_noLOD` shell (`M_UI_Shell_Sphere_C`, whose slow turn is
//!   the material's own world-position offset on Time). The level has no Blueprint script.
//! * The camera sits inside `PostProcessVolume-Lobby2` ((5, 40964, 128) x2, priority 100): WhiteTemp 5000, Film slope / toe / shoulder 0.95 / 0.4 /
//!   0.22, ColorSaturationShadows 1.25, ColorSaturationMidtones (1.15, 0.92, 0.92), the `T_LUT_Vibrant02` grading LUT, AutoExposure min = max = 1,
//!   vignette 0 - stacked by `lobby_world::stack_post_settings` and applied by `lobby_scene`'s post pass.
//! `LobbyAssets::load_backdrop(install, "Login")` = `lobby_world::read_view_scene_from` for that camera (every static / light / fog / post volume of the
//! persistent and always-loaded levels within 50 m of it).
use crate::lobby_scene::{LobbyAssets, LobbyScene};
use std::path::Path;

/// The `EFrontEndCamera` the login screen and the picker request.
pub const CAMERA: &str = "Login";

pub struct FrontendBackdrop { scene: LobbyScene, pub notes: Vec<String> }

impl FrontendBackdrop {
    /// CPU half (paks, scene data, textures): safe on a worker thread.
    pub fn load_assets(install: &Path) -> Result<LobbyAssets, String> { LobbyAssets::load_backdrop(install, CAMERA, None) }
    /// GPU half on the render thread. `format` = the swapchain format (non-sRGB: the post pass writes sRGB-encoded values).
    pub fn from_assets(assets: LobbyAssets, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Result<FrontendBackdrop, String> {
        let mut notes = assets.report.lines.clone();
        let d = &assets.data;
        if let Some(c) = &d.camera { notes.push(format!("camera {} at {:?} fov {}", c.actor, c.xf.t, c.fov_deg)); }
        for v in &d.post_volumes { notes.push(format!("post volume {} prio {} unbound {} bounds {:?}", v.name, v.priority, v.unbound, v.bounds)); }
        notes.push(format!("post at the camera: LUT {:?}, WhiteTemp {:?}, Film {:?}/{:?}/{:?}, overridden {:?}", d.post.color_grading_lut, d.post.get("WhiteTemp"), d.post.get("FilmSlope"), d.post.get("FilmToe"), d.post.get("FilmShoulder"), d.post.overridden));
        Ok(FrontendBackdrop { scene: LobbyScene::from_assets(assets, device, queue, format)?, notes })
    }
    pub fn load(install: &Path, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Result<FrontendBackdrop, String> {
        Self::from_assets(Self::load_assets(install)?, device, queue, format)
    }
    /// Renders the backdrop into `target` (before the UI); `t` = seconds since the front end appeared (material time).
    pub fn render(&mut self, target: &wgpu::TextureView, w: u32, h: u32, t: f32, _dt: f32) { self.scene.render(target, w, h, t); }
}
