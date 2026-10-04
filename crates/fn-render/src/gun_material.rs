//! The gun pass: draws the held firearms with their ORIGINAL material, i.e. the base-pass vertex / pixel programs of the cooked `WeaponMat_Base` shader map
//! translated from DXBC to WGSL by `fn-dxbc` and assembled by `fn-matshader`. The renderer knows nothing about Unreal here either: it owns GPU textures / samplers /
//! uniform buffers / meshes / pipelines created from plain descriptions and draws the [`GunFrame`] the application submits for the next frame with
//! `Renderer::set_gun_frame`, inside the SCENE pass (colour = the HDR target, depth = the scene depth), so the original GBuffer outputs are resolved with the scene's
//! own sun / cascade shadows / sky / fog (`shaders/gun_resolve.wgsl`, appended after `common.wgsl` + `lit_common.wgsl`; group 0 is the renderer's globals bind group).
//!
//! Bind group 1 of every gun pipeline holds all the program resources (uniform buffers, 2D textures, samplers); the layout comes from the pipeline description.
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct GunTexId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct GunBufId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct GunMeshId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct GunPipeId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct GunBindId(pub usize);

/// A 2D texture with the mip chain the package stores (nothing is generated): `(width, height, rgba8)` per level.
#[derive(Clone, Debug)]
pub struct GunTextureDesc { pub label: String, pub srgb: bool, pub levels: Vec<(u32, u32, Vec<u8>)> }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub enum GunFilter { Nearest, Linear }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub enum GunAddress { Wrap, Clamp, Mirror }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GunSamplerDesc { pub mag: GunFilter, pub min: GunFilter, pub mip: GunFilter, pub address: GunAddress, pub aniso: u16 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum GunBindTy { Uniform, Tex2D, Sampler }
#[derive(Clone, Copy, Debug)] pub struct GunLayoutEntry { pub binding: u32, pub ty: GunBindTy, pub vertex: bool, pub fragment: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GunFmt { Float32x2, Float32x3, Float32x4, Unorm8x4, Unorm8x4Bgra, Uint8x4 }
impl GunFmt { fn wgpu(self) -> wgpu::VertexFormat { match self { GunFmt::Float32x2 => wgpu::VertexFormat::Float32x2, GunFmt::Float32x3 => wgpu::VertexFormat::Float32x3, GunFmt::Float32x4 => wgpu::VertexFormat::Float32x4, GunFmt::Unorm8x4 => wgpu::VertexFormat::Unorm8x4, GunFmt::Unorm8x4Bgra => wgpu::VertexFormat::Unorm8x4Bgra, GunFmt::Uint8x4 => wgpu::VertexFormat::Uint8x4 } } }
#[derive(Clone, Copy, Debug)] pub struct GunAttr { pub location: u32, pub offset: u32, pub format: GunFmt }

/// Face culling of the draw: UE `CM_CW` for one-sided materials (`BackCw`, front faces clockwise on screen) or none (two-sided).
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum GunCull { None, BackCw, BackCcw }

#[derive(Clone, Debug)]
pub struct GunPipelineDesc {
    pub label: String,
    /// Module text WITHOUT the renderer's common / lighting code (prepended here): translated programs + `vs_main` / `fs_main`, resources in `@group(1)`.
    pub wgsl: String,
    pub vs_entry: String,
    pub fs_entry: String,
    pub vertex_stride: u32,
    pub vertex_attrs: Vec<GunAttr>,
    pub layout: Vec<GunLayoutEntry>,
    pub cull: GunCull,
}

#[derive(Clone, Copy, Debug)]
pub enum GunRes { Buffer(GunBufId), Texture(GunTexId), Sampler(GunSamplerDesc) }
#[derive(Clone, Copy, Debug)] pub struct GunBindEntry { pub binding: u32, pub res: GunRes }

/// One indexed draw (a mesh section) of the frame.
#[derive(Clone, Debug)]
pub struct GunDraw { pub pipe: GunPipeId, pub bind: GunBindId, pub mesh: GunMeshId, pub first_index: u32, pub index_count: u32 }
/// Everything the gun pass does in one frame.
#[derive(Clone, Debug, Default)]
pub struct GunFrame {
    /// Uniform buffer contents written before the pass (whole buffer from offset 0).
    pub buffer_writes: Vec<(GunBufId, Vec<u8>)>,
    pub draws: Vec<GunDraw>,
}

struct GpuTex { view: wgpu::TextureView }
struct GpuMesh { vb: wgpu::Buffer, ib: wgpu::Buffer }
struct GpuPipe { pipe: wgpu::RenderPipeline, layout: wgpu::BindGroupLayout }

pub struct GunRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    hdr_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    common: String,
    bgl_globals: wgpu::BindGroupLayout,
    textures: Vec<GpuTex>,
    buffers: Vec<wgpu::Buffer>,
    meshes: Vec<GpuMesh>,
    pipes: Vec<GpuPipe>,
    binds: Vec<wgpu::BindGroup>,
    samplers: HashMap<GunSamplerDesc, wgpu::Sampler>,
    frame: GunFrame,
    /// Last error text of a failed pipeline creation.
    pub last_error: Option<String>,
}

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer { device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false }) }

impl GunRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, common: &str, lit_common: &str, bgl_globals: &wgpu::BindGroupLayout, hdr_format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> GunRenderer {
        let resolve = include_str!("../shaders/gun_resolve.wgsl");
        GunRenderer {
            device: device.clone(), queue: queue.clone(), hdr_format, depth_format,
            common: [common, lit_common, resolve].join("\n"), bgl_globals: bgl_globals.clone(),
            textures: vec![], buffers: vec![], meshes: vec![], pipes: vec![], binds: vec![], samplers: HashMap::new(), frame: GunFrame::default(), last_error: None,
        }
    }

    pub fn create_texture(&mut self, d: &GunTextureDesc) -> GunTexId {
        let (w0, h0, _) = &d.levels[0];
        let format = if d.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor { label: Some(&d.label), size: wgpu::Extent3d { width: *w0, height: *h0, depth_or_array_layers: 1 }, mip_level_count: d.levels.len() as u32, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        for (i, (w, h, rgba8)) in d.levels.iter().enumerate() {
            self.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, rgba8, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) }, wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 });
        }
        self.textures.push(GpuTex { view: tex.create_view(&Default::default()) });
        GunTexId(self.textures.len() - 1)
    }
    /// A uniform buffer of `size` bytes (rounded up to 16).
    pub fn create_buffer(&mut self, label: &str, size: usize) -> GunBufId {
        self.buffers.push(buffer(&self.device, label, (size.max(16).div_ceil(16) * 16) as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST));
        GunBufId(self.buffers.len() - 1)
    }
    /// Vertex bytes (the pipeline's single interleaved stream) + u32 indices.
    pub fn create_mesh(&mut self, label: &str, vertices: &[u8], indices: &[u32]) -> GunMeshId {
        let vb = buffer(&self.device, label, vertices.len().max(4) as u64, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST);
        self.queue.write_buffer(&vb, 0, vertices);
        let ib = buffer(&self.device, label, (indices.len() * 4).max(4) as u64, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST);
        self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(indices));
        self.meshes.push(GpuMesh { vb, ib });
        GunMeshId(self.meshes.len() - 1)
    }
    /// Creates the pipeline; a WGSL / validation error is returned (and kept in `last_error`) instead of panicking.
    pub fn create_pipeline(&mut self, d: &GunPipelineDesc) -> Result<GunPipeId, String> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        // the translated programs use screen-space derivatives / sampling in non-uniform control flow, as the HLSL they come from did
        let source = format!("diagnostic(off, derivative_uniformity);\n{}\n{}", self.common, d.wgsl);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(&d.label), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let entries: Vec<wgpu::BindGroupLayoutEntry> = d.layout.iter().map(|e| {
            let mut vis = wgpu::ShaderStages::empty();
            if e.vertex { vis |= wgpu::ShaderStages::VERTEX; }
            if e.fragment { vis |= wgpu::ShaderStages::FRAGMENT; }
            let ty = match e.ty {
                GunBindTy::Uniform => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                GunBindTy::Tex2D => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                GunBindTy::Sampler => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            };
            wgpu::BindGroupLayoutEntry { binding: e.binding, visibility: vis, ty, count: None }
        }).collect();
        let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(&d.label), entries: &entries });
        let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(&d.label), bind_group_layouts: &[Some(&self.bgl_globals), Some(&layout)], immediate_size: 0 });
        let attrs: Vec<wgpu::VertexAttribute> = d.vertex_attrs.iter().map(|a| wgpu::VertexAttribute { format: a.format.wgpu(), offset: a.offset as u64, shader_location: a.location }).collect();
        let (cull_mode, front_face) = match d.cull { GunCull::None => (None, wgpu::FrontFace::Ccw), GunCull::BackCw => (Some(wgpu::Face::Back), wgpu::FrontFace::Cw), GunCull::BackCcw => (Some(wgpu::Face::Back), wgpu::FrontFace::Ccw) };
        let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&d.label), layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some(&d.vs_entry), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: d.vertex_stride as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode, front_face, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: self.depth_format, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(&d.fs_entry), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: self.hdr_format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            let msg = format!("{}: {e}", d.label);
            self.last_error = Some(msg.clone());
            return Err(msg);
        }
        self.pipes.push(GpuPipe { pipe, layout });
        Ok(GunPipeId(self.pipes.len() - 1))
    }
    fn sampler(&mut self, d: GunSamplerDesc) {
        let device = &self.device;
        self.samplers.entry(d).or_insert_with(|| {
            let f = |x: GunFilter| match x { GunFilter::Nearest => wgpu::FilterMode::Nearest, GunFilter::Linear => wgpu::FilterMode::Linear };
            let a = match d.address { GunAddress::Wrap => wgpu::AddressMode::Repeat, GunAddress::Clamp => wgpu::AddressMode::ClampToEdge, GunAddress::Mirror => wgpu::AddressMode::MirrorRepeat };
            let all_linear = d.mag == GunFilter::Linear && d.min == GunFilter::Linear && d.mip == GunFilter::Linear;
            device.create_sampler(&wgpu::SamplerDescriptor { label: Some("gun sampler"), address_mode_u: a, address_mode_v: a, address_mode_w: a, mag_filter: f(d.mag), min_filter: f(d.min), mipmap_filter: match d.mip { GunFilter::Nearest => wgpu::MipmapFilterMode::Nearest, GunFilter::Linear => wgpu::MipmapFilterMode::Linear }, anisotropy_clamp: if all_linear { d.aniso.max(1) } else { 1 }, ..Default::default() })
        });
    }
    /// Bind group 1 of `pipe`.
    pub fn create_bind_group(&mut self, pipe: GunPipeId, entries: &[GunBindEntry]) -> GunBindId {
        for e in entries { if let GunRes::Sampler(s) = e.res { self.sampler(s); } }
        let bge: Vec<wgpu::BindGroupEntry> = entries.iter().map(|e| wgpu::BindGroupEntry { binding: e.binding, resource: match e.res {
            GunRes::Buffer(b) => self.buffers[b.0].as_entire_binding(),
            GunRes::Texture(t) => wgpu::BindingResource::TextureView(&self.textures[t.0].view),
            GunRes::Sampler(s) => wgpu::BindingResource::Sampler(self.samplers.get(&s).unwrap()),
        } }).collect();
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("gun bind group"), layout: &self.pipes[pipe.0].layout, entries: &bge });
        self.binds.push(bg);
        GunBindId(self.binds.len() - 1)
    }

    // ---- per frame -------------------------------------------------------------------------------------------------------------
    pub fn set_frame(&mut self, frame: GunFrame) { self.frame = frame; }
    pub fn any(&self) -> bool { !self.frame.draws.is_empty() }
    /// Uploads the frame's uniform buffers. Call once per rendered frame before the scene pass.
    pub fn prepare(&mut self) {
        for (b, bytes) in &self.frame.buffer_writes { if let Some(buf) = self.buffers.get(b.0) { let n = (bytes.len() as u64).min(buf.size()) as usize; self.queue.write_buffer(buf, 0, &bytes[..n]); } }
        self.frame.buffer_writes.clear();
    }
    /// Records the draws into the scene pass (group 0, the renderer's globals, is already set).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        for d in &self.frame.draws {
            let (Some(p), Some(m), Some(b)) = (self.pipes.get(d.pipe.0), self.meshes.get(d.mesh.0), self.binds.get(d.bind.0)) else { continue };
            pass.set_pipeline(&p.pipe);
            pass.set_bind_group(1, b, &[]);
            pass.set_vertex_buffer(0, m.vb.slice(..));
            pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(d.first_index..d.first_index + d.index_count, 0, 0..1);
        }
    }
}
