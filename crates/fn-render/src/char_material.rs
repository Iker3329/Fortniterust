//! BR character materials (head / hair / body skin, eyeshadow) on the GPU.
//!
//! The materials are NOT shaded by an approximation: the original compiled base-pass pixel shaders (translated from the cooked DXBC by `fn-dxbc`, generated into
//! `shaders/char_material_gen.wgsl` / `shaders/eyeshadow_gen.wgsl`) are evaluated per pixel. The CPU side hands over, per primitive, what the shader reads from its
//! material uniform buffer (the cooked uniform expression set evaluated with the material instance parameters; see `fn_pose::br_material`) and the fixed textures that are
//! not material parameters. The four material-parameter textures (`Diffuse`, `Normals`, `SpecularMasks`, `M`) are the primitive's [`crate::MaterialImages`] as before.
//!
//! * `UeMaterial::Char`: drawn in the scene pass (`char_scene.wgsl`): translated PS -> G-buffer values -> sun + sky ambient + the shader's own emissive output.
//! * `UeMaterial::Eyeshadow`: the plain `BLEND_Modulate` unlit material of the eye region, drawn after the scene (`char_overlay.wgsl`) with `DestColor, Zero` blending; its
//!   per-vertex colour (R = shadow mask) comes with the primitive.
use image::RgbaImage;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharPerm { Head = 0, Hair = 1, Body = 2 }

/// Sampler addressing of a fixed texture (the texture asset's own `AddressX/Y`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Addr { Wrap, Clamp, Mirror }
impl Addr { fn wgpu(self) -> wgpu::AddressMode { match self { Addr::Wrap => wgpu::AddressMode::Repeat, Addr::Clamp => wgpu::AddressMode::ClampToEdge, Addr::Mirror => wgpu::AddressMode::MirrorRepeat } } }

/// A texture that is not a material parameter (fixed in the material graph). `srgb`: hardware sRGB decode on sampling (the texture's SRGB flag, unless the
/// sampler type of the graph node is LinearColor).
#[derive(Clone, Debug)]
pub struct FixedTexture { pub image: RgbaImage, pub srgb: bool, pub address: [Addr; 2] }

/// Constants of one character material permutation.
#[derive(Clone, Debug)]
pub struct CharMaterial {
    pub perm: CharPerm,
    /// Registers of the material uniform buffer (`cb3` of the pixel shader, at most 32).
    pub cb: Vec<[f32; 4]>,
    /// Fixed texture slot 7 (`/Game/UI/UI_Faces/Noise`, the super-charged flicker noise).
    pub noise: FixedTexture,
}

/// The eye-region modulate overlay.
#[derive(Clone, Debug)]
pub struct EyeshadowOverlay {
    /// Registers of the material uniform buffer (`cb1` of the pixel shader, 10).
    pub cb: Vec<[f32; 4]>,
    /// Vertex colour (R, G, B, A bytes) of every vertex of the primitive.
    pub colors: Vec<[u8; 4]>,
    /// Fixed texture slots by texture index: 0 `T_NoisePack_01`, 1 `T_Slowfield_GridSquares_01`, 2 `ShieldPowerAnim`.
    pub tex0: FixedTexture,
    pub tex1: FixedTexture,
    pub tex2: FixedTexture,
}

#[derive(Clone, Debug)]
pub enum UeMaterial { Char(CharMaterial), Eyeshadow(EyeshadowOverlay) }

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CharMatUniform { cb: [[f32; 4]; 32], meta: [u32; 4] }

pub struct CharGpu {
    pub char_bgl: wgpu::BindGroupLayout,
    pub char_pipeline: wgpu::RenderPipeline,
    pub overlay_bgl: wgpu::BindGroupLayout,
    pub overlay_pipeline: wgpu::RenderPipeline,
}

pub struct GpuOverlay { pub bg: wgpu::BindGroup, pub color_vb: wgpu::Buffer }

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry { wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None } }
fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry { entry(binding, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }) }
fn samp_entry(binding: u32) -> wgpu::BindGroupLayoutEntry { entry(binding, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)) }
fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry { entry(binding, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }) }

fn upload_fixed(device: &wgpu::Device, queue: &wgpu::Queue, t: &FixedTexture, label: &str) -> (wgpu::TextureView, wgpu::Sampler) {
    let tex = crate::texture::upload_rgba8(device, queue, &t.image, t.srgb, crate::texture::MipFilter::Triangle, label);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label), address_mode_u: t.address[0].wgpu(), address_mode_v: t.address[1].wgpu(), address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, anisotropy_clamp: 8, ..Default::default()
    });
    (tex.create_view(&Default::default()), sampler)
}

const SKINNED_ATTRS: [wgpu::VertexAttribute; 5] = [
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint16x4, offset: 32, shader_location: 3 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 40, shader_location: 4 },
];
const COLOR_ATTR: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 0, shader_location: 5 }];

fn module(device: &wgpu::Device, label: &str, parts: &[&str]) -> wgpu::ShaderModule {
    // the translated shaders start with `diagnostic(off, derivative_uniformity);`, which must precede every declaration of the module
    let body = parts.join("\n").replace("diagnostic(off, derivative_uniformity);", "");
    device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(format!("diagnostic(off, derivative_uniformity);\n{body}").into()) })
}

impl CharGpu {
    #[allow(clippy::too_many_arguments)]
    pub fn new(device: &wgpu::Device, common: &str, lit_common: &str, bgl_globals: &wgpu::BindGroupLayout, bgl_material: &wgpu::BindGroupLayout, bgl_obj_skinned: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat, depth: wgpu::TextureFormat) -> CharGpu {
        let char_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("char material bgl"), entries: &[uniform_entry(0), tex_entry(1), samp_entry(2)] });
        let overlay_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("eyeshadow bgl"), entries: &[uniform_entry(0), tex_entry(1), samp_entry(2), tex_entry(3), samp_entry(4), tex_entry(5), samp_entry(6)] });
        let char_sm = module(device, "char scene", &[common, lit_common, include_str!("../shaders/char_material_gen.wgsl"), include_str!("../shaders/char_scene.wgsl")]);
        let overlay_sm = module(device, "char overlay", &[common, lit_common, include_str!("../shaders/eyeshadow_gen.wgsl"), include_str!("../shaders/char_overlay.wgsl")]);
        let layout = |label: &str, bgls: &[&wgpu::BindGroupLayout]| {
            let entries: Vec<Option<&wgpu::BindGroupLayout>> = bgls.iter().map(|b| Some(*b)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &entries, immediate_size: 0 })
        };
        let char_layout = layout("char scene layout", &[bgl_globals, bgl_material, bgl_obj_skinned, &char_bgl]);
        let stride = std::mem::size_of::<crate::mesh::SkinnedVertex>() as u64;
        let char_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("char scene"), layout: Some(&char_layout),
            vertex: wgpu::VertexState { module: &char_sm, entry_point: Some("vs_char"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &SKINNED_ATTRS })] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: depth, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &char_sm, entry_point: Some("fs_char"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: hdr, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        // overlay: globals, object (skinned), eyeshadow resources
        let overlay_layout = layout("eyeshadow layout", &[bgl_globals, bgl_obj_skinned, &overlay_bgl]);
        let modulate = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Dst, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("eyeshadow modulate"), layout: Some(&overlay_layout),
            vertex: wgpu::VertexState { module: &overlay_sm, entry_point: Some("vs_overlay"), compilation_options: Default::default(), buffers: &[
                Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: &SKINNED_ATTRS }),
                Some(wgpu::VertexBufferLayout { array_stride: 4, step_mode: wgpu::VertexStepMode::Vertex, attributes: &COLOR_ATTR }),
            ] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: depth, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &overlay_sm, entry_point: Some("fs_overlay"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: hdr, blend: Some(modulate), write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        CharGpu { char_bgl, char_pipeline, overlay_bgl, overlay_pipeline }
    }

    /// Bind group 3 of a character material primitive.
    pub fn char_bind_group(&self, device: &wgpu::Device, queue: &wgpu::Queue, m: &CharMaterial, label: &str) -> wgpu::BindGroup {
        let mut u = CharMatUniform { cb: [[0.0; 4]; 32], meta: [m.perm as u32, 0, 0, 0] };
        for (i, r) in m.cb.iter().take(32).enumerate() { u.cb[i] = *r; }
        let buf = crate::texture::buffer_init(device, &[u], wgpu::BufferUsages::UNIFORM, label);
        let (view, sampler) = upload_fixed(device, queue, &m.noise, label);
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &self.char_bgl, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
        ] })
    }

    /// Bind group 2 + the colour stream of an eyeshadow overlay primitive.
    pub fn overlay(&self, device: &wgpu::Device, queue: &wgpu::Queue, e: &EyeshadowOverlay, label: &str) -> GpuOverlay {
        let mut cb = [[0.0f32; 4]; 10];
        for (i, r) in e.cb.iter().take(10).enumerate() { cb[i] = *r; }
        let buf = crate::texture::buffer_init(device, &cb, wgpu::BufferUsages::UNIFORM, label);
        let color_vb = crate::texture::buffer_init(device, &e.colors, wgpu::BufferUsages::VERTEX, label);
        let (v0, s0) = upload_fixed(device, queue, &e.tex0, label);
        let (v1, s1) = upload_fixed(device, queue, &e.tex1, label);
        let (v2, s2) = upload_fixed(device, queue, &e.tex2, label);
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &self.overlay_bgl, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&v1) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&s1) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&v0) }, wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&s0) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&v2) }, wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&s2) },
        ] });
        GpuOverlay { bg, color_vb }
    }
}

/// Shared handle stored in `MaterialImages::ue`.
pub type UeMaterialRef = Arc<UeMaterial>;

#[cfg(test)]
mod tests {
    use super::*;
    /// Headless GPU: creates the character scene + eyeshadow overlay pipelines exactly as the renderer does (shader validation included;
    /// the device's uncaptured-error handler panics on any wgpu / naga error, e.g. a duplicated `@builtin(position)`).
    #[test]
    fn char_pipelines_create() {
        let gpu = match crate::gpu::Gpu::headless() { Ok(g) => g, Err(e) => { eprintln!("skipped: {e}"); return; } };
        let d = &gpu.device;
        let vf = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let ent = |binding: u32, vis: wgpu::ShaderStages, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty, count: None };
        let uni = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let tex = |dim| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: dim, multisampled: false };
        let f = wgpu::ShaderStages::FRAGMENT;
        let globals = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("globals"), entries: &[
            ent(0, vf, uni), ent(1, vf, uni),
            ent(2, f, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
            ent(3, f, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)), ent(4, f, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
        ] });
        let d2 = wgpu::TextureViewDimension::D2;
        let material = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("material"), entries: &[ent(0, f, uni), ent(1, f, tex(d2)), ent(2, f, tex(d2)), ent(3, f, tex(d2)), ent(4, f, tex(d2))] });
        let v = wgpu::ShaderStages::VERTEX;
        let skinned = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("skinned"), entries: &[
            ent(0, v, uni), ent(1, v, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
        ] });
        let _ = CharGpu::new(d, include_str!("../shaders/common.wgsl"), include_str!("../shaders/lit_common.wgsl"), &globals, &material, &skinned, crate::renderer::HDR_FORMAT, crate::renderer::DEPTH_FORMAT);
    }
}
