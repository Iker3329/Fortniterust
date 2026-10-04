//! Video-settings quality tables of the old runtime (`settings.rs`), expressed as renderer inputs. APPROXIMATIONS of UE sg.* scalability
//! groups exactly as the prototype defined them (the originals are in encrypted ini files). Levels are 0 (low) .. 3 (epic).
//!
//! fn-app maps `fn_settings` levels onto these helpers and passes the results through [`crate::RenderOptions`] and the camera far plane.

/// Shadows level -> shadow map edge in texels (`None` = shadows off).
pub fn shadow_map_size(level: u32) -> Option<u32> { match level { 0 => None, 1 => Some(512), 2 => Some(1024), _ => Some(2048) } }
/// View Distance level -> camera far plane in metres (set `CameraParams::far`; the sky pass keeps its own very far plane).
pub fn far_plane_m(level: u32) -> f32 { match level { 0 => 200., 1 => 400., 2 => 800., _ => 3000. } }
/// Anti-Aliasing level -> FXAA parameters; level 0 = off.
pub fn aa_params(level: u32) -> Option<FxaaParams> {
    match level {
        0 => None,
        1 => Some(FxaaParams { subpix: 0.5, edge_threshold: 0.25, edge_threshold_min: 0.0625, search_steps: 4 }),
        2 => Some(FxaaParams { subpix: 0.75, edge_threshold: 0.166, edge_threshold_min: 0.0833, search_steps: 8 }),
        _ => Some(FxaaParams { subpix: 1.0, edge_threshold: 0.125, edge_threshold_min: 0.0312, search_steps: 12 }),
    }
}
/// Textures level -> (top mip levels skipped, anisotropy). Skipped mips are emulated with the sampler's minimum LOD clamp.
pub fn texture_params(level: u32) -> (u32, u16) { match level { 0 => (2, 1), 1 => (1, 1), 2 => (0, 4), _ => (0, 16) } }
/// Post Processing level -> (ao, bloom, vignette).
pub fn post_flags(level: u32) -> (bool, bool, bool) { (level >= 2, level >= 1, level >= 3) }
/// Brightness slider 0..1 -> display exponent applied after the sRGB encode (gamma 1.7..2.7, 2.2 = identity).
pub fn display_exponent_for_brightness(brightness: f32) -> f32 { 2.2 / (1.7 + (2.7 - 1.7) * brightness.clamp(0., 1.)).max(0.1) }

/// FXAA parameters (old `settings::FxaaParams`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxaaParams {
    /// Sub-pixel aliasing removal (0..1).
    pub subpix: f32,
    /// Minimum local contrast that counts as an edge.
    pub edge_threshold: f32,
    /// Contrast floor for dark areas.
    pub edge_threshold_min: f32,
    /// Edge-end search steps in each direction (max 12).
    pub search_steps: i32,
}

/// Pause-menu backdrop: the 3D scene stays visible behind the menu, blurred and dimmed.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Backdrop {
    /// 0 = off, 1 = fully black.
    pub dim: f32,
    /// 0 = sharp .. 1 = strongest blur (HDR mip-chain blur radius).
    pub blur: f32,
}
impl Backdrop {
    pub const OFF: Backdrop = Backdrop { dim: 0., blur: 0. };
    /// A reasonable pause-menu look (scene clearly visible, soft and darker).
    pub const PAUSE_MENU: Backdrop = Backdrop { dim: 0.45, blur: 0.5 };
    pub fn active(&self) -> bool { self.dim > 0. || self.blur > 0. }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tables_match_the_old_runtime() {
        assert_eq!(shadow_map_size(0), None);
        assert_eq!(shadow_map_size(3), Some(2048));
        assert_eq!(far_plane_m(3), crate::camera::FAR);
        assert!(aa_params(0).is_none() && aa_params(3).unwrap().search_steps == 12);
        assert_eq!(texture_params(3), (0, 16));
        assert!((display_exponent_for_brightness(0.5) - 1.0).abs() < 1e-6);
    }
}
