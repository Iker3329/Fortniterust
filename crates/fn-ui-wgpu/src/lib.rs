//! wgpu backend for the fn-ui low-level draw list ([`fn_ui::draw::RenderList`]).
//!
//! # Architecture
//! * [`UiRenderer`] consumes a `RenderList` (quads, procedural materials, clip push/pop) and draws it with one render pass into a caller supplied
//!   target view (or offscreen via [`UiRenderer::render_to_image`]). Commands are replayed strictly in order; consecutive quads that share a texture
//!   and scissor become ONE instanced draw call (an instance = 4 corner positions + uv rect + premultiplied tint, 64 bytes, drawn as 6 vertices).
//!   Instance data lives in one persistent growing vertex buffer, per-draw material uniforms in one growing dynamic-offset uniform buffer; no
//!   pipeline is created per frame (material pipelines are built lazily on first use and cached).
//! * Textures come from [`fn_ui::assets::TextureStore`]. Quads upload `Texture::premultiplied()` as `Rgba8Unorm` (with a CPU box-filtered mip chain
//!   when `Sampler::mips`, trilinear sampling, clamp-to-edge unless `wrap`), keyed by (id, version): when the store bumps a version (text atlases,
//!   baked textures) the pixels are re-uploaded (same size -> `write_texture` in place). Ids removed from the store are evicted after each frame.
//! * `PushClip`/`PopClip` are scissor rects: nested clips are intersected, floored/ceiled outwards to whole pixels (as the old runtime did) and
//!   clamped to the target; an empty clip skips drawing.
//!
//! # Blend / colour-space rules (do not change)
//! fn-ui colours are gamma (sRGB-encoded) values and Slate composites in gamma space on a **non-sRGB** back buffer. Therefore the target format must
//! be non-sRGB ([`pick_surface_format`] picks one; `Rgba8Unorm` offscreen) and nothing is ever converted to/from sRGB by the hardware. Tints are
//! premultiplied on the CPU (`rgb * a`), texels are premultiplied at upload, and the blend state is `ONE, ONE_MINUS_SRC_ALPHA` (colour and alpha).
//! The framebuffer therefore holds PREMULTIPLIED colour; `clear` colours are given straight and stored premultiplied. Material shaders output
//! the straight shader colour, which `finish()` premultiplies and multiplies with the widget tint (identical to bake -> quad with tint).
//!
//! # Materials ([`MaterialMode`])
//! * `CpuBake`: [`fn_ui::material::bake`] once per (`MaterialKind::cache_key`, round(w), round(h)); the premultiplied result is uploaded and drawn as
//!   a (flip_x / tint honouring) quad. LRU cache capped at [`BAKE_CACHE_CAP`] entries.
//! * `Gpu` (default): WGSL per-pixel evaluation, falling back to `CpuBake` per material when no translation exists, a texture is missing, or the
//!   shader fails validation. The translations:
//!   - **generated** by `examples/gen_wgsl.rs` (`cargo run -p fn-ui-wgpu --example gen_wgsl`) (checked in under `src/shaders/gen/`, tables in `src/gen_tables.rs`): the 30 Settings-screen
//!     pixel shaders `uimat::ps_0..ps_29` and the HUD shaders `ui_shaders::{item_card, compass, hud_bar, panel_line, panel_vgrad}_ps`. The tool is a
//!     tiny parser for the straight-line Rust subset the DXBC translator emits (register arrays, vec4 temporaries, `m()/nz()/sat()`, bit masks as
//!     `bitcast<f32>`, `tex()` -> bilinear `textureLoad` with sRGB decode + wrap/clamp from per-draw meta) and throws on anything unknown.
//!     The cb constant buffers stay on the CPU (`uimat::material_uniforms`, `ui_shaders::*_cb`) and are uploaded as a uniform array.
//!   - **hand translated** (`src/shaders/proc_fs.wgsl`): `Procedural` (Outline family, Primary-M) and `Panel` (ShellUberBG backdrop, modal body
//!     and header), from the OLD `src/menu/outline.rs` maths (look tables are duplicated in `materials.rs`).
//!   Slate shader inputs: v1 = packed linear brush tint, v6 = brush uv (nine-patch mapped in the fragment shader), v7.zw = pixel size,
//!   v8 = material uv 0..1, `cb0[2]` = (1, 1/2.2).
//!
//! # Integrating with fn-render
//! Create the renderer from fn-render's objects: `UiRenderer::with_device(device.clone(), queue.clone(), format)` where `format` is the swapchain
//! format chosen with [`pick_surface_format`] (wgpu `Device`/`Queue` are cheap handles). Each `render` call encodes its own pass and submits it, so
//! call it after the 3D scene's submit and pass the swapchain view with `clear = None` to composite the UI over the scene. The renderer sets no
//! global state and installs no error handler on a shared device (only `new_headless` does); WGSL material pipelines are validated under an error
//! scope so a bad shader degrades to `CpuBake` instead of poisoning the device.
//!
//! The vertex/uniform buffers are written with `queue.write_buffer`, so two `render` calls may not be recorded into one command buffer; keep the
//! one-submit-per-render contract (the type owns the encoder).

mod gen_tables;
mod materials;
mod renderer;
mod textures;

pub use materials::MaterialMode;
pub use renderer::{pick_surface_format, FrameStats, UiRenderer, BAKE_CACHE_CAP};

/// A UI draw list bound to its renderer and assets for ONE frame; implements [`fn_render::UiOverlay`] so the application can pass it to
/// `fn_render::Renderer::render_with_overlay(frame, swapchain_view, Some(&mut UiPass { .. }))` and draw 3D scene + post chain + UI with a
/// single call. The UI is composited in sRGB/gamma space over the tone-mapped frame (target format must be non-sRGB, see [`pick_surface_format`]).
pub struct UiPass<'a> {
    pub renderer: &'a mut UiRenderer,
    pub list: &'a fn_ui::draw::RenderList,
    pub assets: &'a mut fn_ui::assets::UiAssets,
}
impl fn_render::UiOverlay for UiPass<'_> {
    fn draw(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue, target: &wgpu::TextureView, width: u32, height: u32) {
        self.renderer.render(self.list, self.assets, target, width, height, None);
    }
}

#[cfg(test)]
mod tests;
