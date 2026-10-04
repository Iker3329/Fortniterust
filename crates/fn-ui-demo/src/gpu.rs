//! Headless GPU backend wrapper (fn-ui-wgpu). Compiled only with the `gpu` feature; otherwise `new()` reports that.
use fn_ui::assets::UiAssets;
use fn_ui::draw::RenderList;

#[cfg(feature = "gpu")]
pub struct GpuBackend { r: fn_ui_wgpu::UiRenderer }
#[cfg(feature = "gpu")]
impl GpuBackend {
    pub fn new() -> Result<Self, String> { Ok(GpuBackend { r: fn_ui_wgpu::UiRenderer::new_headless()? }) }
    pub fn render(&mut self, list: &RenderList, assets: &mut UiAssets, w: u32, h: u32) -> Result<Vec<u8>, String> { Ok(self.r.render_to_image(list, assets, w, h, None)) }
}

#[cfg(not(feature = "gpu"))]
pub struct GpuBackend;
#[cfg(not(feature = "gpu"))]
impl GpuBackend {
    pub fn new() -> Result<Self, String> { Err("built without the `gpu` feature".into()) }
    pub fn render(&mut self, _list: &RenderList, _assets: &mut UiAssets, _w: u32, _h: u32) -> Result<Vec<u8>, String> { Err("built without the `gpu` feature".into()) }
}
