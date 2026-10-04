//! HUD overlay (`--demo-ui`): the fn-ui HUD (quickbar / vitals / compass / minimap / counters) drawn by fn-ui-wgpu over the finished 3D frame
//! through `fn_render::UiOverlay` (same swapchain / offscreen view, sRGB space, after the post chain).
use fn_ui::draw::RenderList;
use fn_ui::scene::UiContext;
use fn_ui::screens::hud::Hud;
use fn_ui_wgpu::{UiPass, UiRenderer};

pub struct HudOverlay {
    ctx: UiContext,
    renderer: UiRenderer,
    hud: Hud,
    list: RenderList,
}
impl HudOverlay {
    pub fn new(device: &fn_render::wgpu::Device, queue: &fn_render::wgpu::Queue, format: fn_render::wgpu::TextureFormat) -> Result<Self, String> {
        let ctx = UiContext::from_default_assets().map_err(|e| format!("UI assets: {e} (set FN_UI_ASSETS to the FortniteRuntime assets folder)"))?;
        Ok(HudOverlay { ctx, renderer: UiRenderer::with_device(device.clone(), queue.clone(), format), hud: Hud::new(), list: RenderList::new() })
    }
    /// Advances the HUD animation and rebuilds the draw list for a `w` x `h` target; `yaw_deg` drives the compass.
    pub fn update(&mut self, w: u32, h: u32, yaw_deg: f32, dt: f32) {
        self.hud.tick(dt);
        self.list.clear();
        self.hud.build(&mut self.ctx, w as f32, h as f32, yaw_deg, &mut self.list);
    }
    pub fn pass(&mut self) -> UiPass<'_> { UiPass { renderer: &mut self.renderer, list: &self.list, assets: &mut self.ctx.assets } }
    pub fn missing_assets(&self) -> usize { self.ctx.assets.missing.len() }
}
