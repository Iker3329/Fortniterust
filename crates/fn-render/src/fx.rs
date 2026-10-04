//! The FX pass: translucent / unlit / vertex-animated (world position offset) / masked material draws that follow the SCENE pass and precede the HDR mip chain, driven by
//! shaders the application hands over as WGSL (the original materials' DXBC translated by `fn-dxbc`, assembled by `fn-fx`). The pass knows nothing about Unreal:
//! it owns GPU textures / samplers / uniform buffers / meshes / pipelines created from plain descriptions and draws the [`FxFrame`] the application submits for the
//! next frame with [`crate::Renderer::set_fx_frame`].
//!
//! Two render passes are recorded when the frame has draws of the kind:
//! * `fx opaque`      colour + the scene depth buffer writable (masked materials: the original base-pass `BLEND_Masked` draws);
//! * `fx translucent` colour + the scene depth buffer READ-ONLY (translucent / modulate draws; their pixel shaders may sample the scene depth, [`FxRes::SceneDepth`]).
//!
//! Blend state, depth test / write, culling and the topology come from the material flags the caller read from the package (see [`FxBlend`], [`FxDepth`], [`FxCull`]).
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct FxTexId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct FxBufId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct FxMeshId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct FxPipeId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct FxBindId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum FxTexDim { D2, D3, Cube }
/// One mip level of a texture (RGBA8; cube: the six faces concatenated, +X -X +Y -Y +Z -Z; 3D: depth slices concatenated).
#[derive(Clone, Debug)] pub struct FxLevel { pub width: u32, pub height: u32, pub depth: u32, pub rgba8: Vec<u8> }
/// A texture with the mip chain the package stores (nothing is generated).
#[derive(Clone, Debug)]
pub struct FxTextureDesc { pub label: String, pub dim: FxTexDim, pub srgb: bool, pub levels: Vec<FxLevel> }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub enum FxFilter { Nearest, Linear }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub enum FxAddress { Wrap, Clamp, Mirror }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FxSamplerDesc { pub mag: FxFilter, pub min: FxFilter, pub mip: FxFilter, pub address: FxAddress, pub aniso: u16 }
impl FxSamplerDesc {
    pub const LINEAR_WRAP: FxSamplerDesc = FxSamplerDesc { mag: FxFilter::Linear, min: FxFilter::Linear, mip: FxFilter::Linear, address: FxAddress::Wrap, aniso: 1 };
    pub const LINEAR_CLAMP: FxSamplerDesc = FxSamplerDesc { mag: FxFilter::Linear, min: FxFilter::Linear, mip: FxFilter::Linear, address: FxAddress::Clamp, aniso: 1 };
    pub const NEAREST_CLAMP: FxSamplerDesc = FxSamplerDesc { mag: FxFilter::Nearest, min: FxFilter::Nearest, mip: FxFilter::Nearest, address: FxAddress::Clamp, aniso: 1 };
    fn non_filtering(&self) -> bool { self.mag == FxFilter::Nearest && self.min == FxFilter::Nearest && self.mip == FxFilter::Nearest }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxFmt { Float32x2, Float32x3, Float32x4, Unorm8x4, Unorm8x4Bgra, Float16x2, Float16x4, Sint32x4, Uint32x4 }
impl FxFmt { fn wgpu(self) -> wgpu::VertexFormat { match self { FxFmt::Float32x2 => wgpu::VertexFormat::Float32x2, FxFmt::Float32x3 => wgpu::VertexFormat::Float32x3, FxFmt::Float32x4 => wgpu::VertexFormat::Float32x4, FxFmt::Unorm8x4 => wgpu::VertexFormat::Unorm8x4, FxFmt::Unorm8x4Bgra => wgpu::VertexFormat::Unorm8x4Bgra, FxFmt::Float16x2 => wgpu::VertexFormat::Float16x2, FxFmt::Float16x4 => wgpu::VertexFormat::Float16x4, FxFmt::Sint32x4 => wgpu::VertexFormat::Sint32x4, FxFmt::Uint32x4 => wgpu::VertexFormat::Uint32x4 } } }
#[derive(Clone, Copy, Debug)] pub struct FxAttr { pub location: u32, pub offset: u32, pub format: FxFmt }
#[derive(Clone, Debug)] pub struct FxVertexBuffer { pub stride: u32, pub instance: bool, pub attrs: Vec<FxAttr> }

/// A mesh: vertex bytes (one interleaved stream described by the pipeline's first vertex buffer) + u16 / u32 indices.
#[derive(Clone, Debug)] pub struct FxMeshDesc { pub label: String, pub vertices: Vec<u8>, pub indices: Vec<u32> }

/// Blend modes of the material's serialized `BlendMode` (UE 4.16 base-pass render states).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxBlend {
    /// `BLEND_Opaque` / `BLEND_Masked`: no blending.
    Opaque,
    /// `BLEND_Translucent`: colour `SrcAlpha, InvSrcAlpha`, alpha `Zero, InvSrcAlpha`.
    Translucent,
    /// `BLEND_Additive`: colour `One, One`, alpha `Zero, One`.
    Additive,
    /// `BLEND_Modulate`: colour `DestColor, Zero`, alpha `Zero, One`.
    Modulate,
}
/// Depth state: `test` = false is `bDisableDepthTest` (compare Always); `write` only exists in the opaque pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct FxDepth { pub test: bool, pub write: bool }
/// Face culling: `TwoSided` flag false -> `Back` with the Unreal front-face convention (clockwise in the D3D screen space, `front_cw`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum FxCull { None, BackCw, BackCcw }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxBindTy {
    Uniform,
    Tex2D { filterable: bool },
    Tex3D { filterable: bool },
    Cube { filterable: bool },
    /// The scene depth (Depth32Float) read as an unfilterable `texture_2d<f32>`.
    SceneDepth,
    Sampler { filtering: bool },
    /// A read-only storage buffer (the vertex / pixel shader's `Buffer<float4>` SRV).
    StorageRead,
}
#[derive(Clone, Copy, Debug)] pub struct FxLayoutEntry { pub binding: u32, pub ty: FxBindTy, pub vertex: bool, pub fragment: bool }

#[derive(Clone, Debug)]
pub struct FxPipelineDesc {
    pub label: String,
    pub wgsl: String,
    pub vs_entry: String,
    pub fs_entry: String,
    pub vertex_buffers: Vec<FxVertexBuffer>,
    /// Bind group layouts by group index.
    pub groups: Vec<Vec<FxLayoutEntry>>,
    pub blend: FxBlend,
    pub depth: FxDepth,
    pub cull: FxCull,
    /// Drawn in the `fx translucent` pass (depth read-only, may sample the scene depth) instead of `fx opaque`.
    pub translucent: bool,
}

/// What a bind group entry points at.
#[derive(Clone, Copy, Debug)]
pub enum FxRes { Buffer(FxBufId), Texture(FxTexId), Sampler(FxSamplerDesc), SceneDepth }
#[derive(Clone, Copy, Debug)] pub struct FxBindEntry { pub binding: u32, pub res: FxRes }

#[derive(Clone, Debug)]
pub enum FxGeom {
    Mesh(FxMeshId),
    /// A range of the mesh's index buffer (one section of a multi-material mesh).
    MeshRange { mesh: FxMeshId, first_index: u32, index_count: u32 },
    /// Per-frame vertices (e.g. camera facing sprites) + u32 indices, uploaded with the frame.
    Dynamic { vertices: Vec<u8>, indices: Vec<u32> },
}
/// Per-instance vertex data for vertex buffer slot 1 (the pipeline's second vertex buffer, `instance: true`).
#[derive(Clone, Debug)] pub struct FxInstances { pub bytes: Vec<u8>, pub count: u32 }

#[derive(Clone, Debug)]
pub struct FxDraw {
    pub pipe: FxPipeId,
    /// (group index, bind group); every group of the pipeline layout must be present.
    pub binds: Vec<(u32, FxBindId)>,
    pub geom: FxGeom,
    pub instances: Option<FxInstances>,
}
/// Everything the FX pass does in one frame.
#[derive(Clone, Debug, Default)]
pub struct FxFrame {
    /// Uniform buffer contents written before the passes (whole buffer from offset 0).
    pub buffer_writes: Vec<(FxBufId, Vec<u8>)>,
    /// Draws in submission order (translucent draws: sort back to front before submitting).
    pub draws: Vec<FxDraw>,
}

struct GpuTex { tex: wgpu::Texture, view: wgpu::TextureView, dim: FxTexDim }
struct GpuMesh { vb: wgpu::Buffer, ib: wgpu::Buffer, index_count: u32 }
struct GpuPipe { pipe: wgpu::RenderPipeline, layouts: Vec<wgpu::BindGroupLayout>, desc_groups: Vec<Vec<FxLayoutEntry>>, translucent: bool, instanced: bool }
struct BindGroupRec { pipe: FxPipeId, group: u32, entries: Vec<FxBindEntry>, built: Option<(u64, wgpu::BindGroup)>, uses_depth: bool }

pub struct FxRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    hdr_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    textures: Vec<GpuTex>,
    buffers: Vec<wgpu::Buffer>,
    meshes: Vec<GpuMesh>,
    pipes: Vec<GpuPipe>,
    binds: Vec<BindGroupRec>,
    samplers: HashMap<FxSamplerDesc, wgpu::Sampler>,
    depth_sampler: wgpu::Sampler,
    /// Frame data (set by `set_frame`, consumed by the passes).
    frame: FxFrame,
    dyn_vb: wgpu::Buffer,
    dyn_ib: wgpu::Buffer,
    inst_vb: wgpu::Buffer,
    /// Per draw: (dynamic vertex offset, dynamic index offset / count, instance offset)
    placed: Vec<Placed>,
    depth_gen: u64,
    /// Last error text of a failed pipeline / bind group creation (also returned by the create calls).
    pub last_error: Option<String>,
}
#[derive(Clone, Copy, Default)]
struct Placed { vb: u64, ib: u64, index_count: u32, inst: u64 }

const DYN_CAPACITY: u64 = 1 << 20;

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer { device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false }) }

impl FxRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, hdr_format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> FxRenderer {
        let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("fx scene depth sampler"), mag_filter: wgpu::FilterMode::Nearest, min_filter: wgpu::FilterMode::Nearest, mipmap_filter: wgpu::MipmapFilterMode::Nearest, address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        FxRenderer {
            device: device.clone(), queue: queue.clone(), hdr_format, depth_format,
            textures: vec![], buffers: vec![], meshes: vec![], pipes: vec![], binds: vec![], samplers: HashMap::new(), depth_sampler,
            frame: FxFrame::default(),
            dyn_vb: buffer(device, "fx dynamic vertices", DYN_CAPACITY, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST),
            dyn_ib: buffer(device, "fx dynamic indices", DYN_CAPACITY, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST),
            inst_vb: buffer(device, "fx instances", DYN_CAPACITY, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST),
            placed: vec![], depth_gen: 0, last_error: None,
        }
    }

    // ---- resources ---------------------------------------------------------------------------------------------------------
    pub fn create_texture(&mut self, d: &FxTextureDesc) -> FxTexId {
        let l0 = &d.levels[0];
        let (dimension, depth_or_layers, view_dim) = match d.dim { FxTexDim::D2 => (wgpu::TextureDimension::D2, 1, wgpu::TextureViewDimension::D2), FxTexDim::D3 => (wgpu::TextureDimension::D3, l0.depth.max(1), wgpu::TextureViewDimension::D3), FxTexDim::Cube => (wgpu::TextureDimension::D2, 6, wgpu::TextureViewDimension::Cube) };
        let format = if d.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor { label: Some(&d.label), size: wgpu::Extent3d { width: l0.width, height: l0.height, depth_or_array_layers: depth_or_layers }, mip_level_count: d.levels.len() as u32, sample_count: 1, dimension, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        for (i, l) in d.levels.iter().enumerate() {
            let layers = match d.dim { FxTexDim::D2 => 1, FxTexDim::D3 => l.depth.max(1), FxTexDim::Cube => 6 };
            self.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, &l.rgba8, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * l.width), rows_per_image: Some(l.height) }, wgpu::Extent3d { width: l.width, height: l.height, depth_or_array_layers: layers });
        }
        let view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(view_dim), ..Default::default() });
        self.textures.push(GpuTex { tex, view, dim: d.dim });
        FxTexId(self.textures.len() - 1)
    }
    /// A uniform buffer of `size` bytes (rounded up to 16; WGSL arrays of vec4 need no more).
    pub fn create_buffer(&mut self, label: &str, size: usize) -> FxBufId {
        self.buffers.push(buffer(&self.device, label, (size.max(16).div_ceil(16) * 16) as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST));
        FxBufId(self.buffers.len() - 1)
    }
    /// Writes a uniform buffer right away (static contents such as a material's constant buffer).
    pub fn write_buffer(&self, b: FxBufId, bytes: &[u8]) { if let Some(buf) = self.buffers.get(b.0) { let n = (bytes.len() as u64).min(buf.size()) as usize; self.queue.write_buffer(buf, 0, &bytes[..n]); } }
    pub fn create_mesh(&mut self, d: &FxMeshDesc) -> FxMeshId {
        let vb = buffer(&self.device, &d.label, d.vertices.len().max(4) as u64, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST);
        self.queue.write_buffer(&vb, 0, &d.vertices);
        let ib = buffer(&self.device, &d.label, (d.indices.len() * 4).max(4) as u64, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST);
        self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&d.indices));
        self.meshes.push(GpuMesh { vb, ib, index_count: d.indices.len() as u32 });
        FxMeshId(self.meshes.len() - 1)
    }
    /// Creates the pipeline; a WGSL / validation error is returned (and kept in `last_error`) instead of panicking.
    pub fn create_pipeline(&mut self, d: &FxPipelineDesc) -> Result<FxPipeId, String> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(&d.label), source: wgpu::ShaderSource::Wgsl(d.wgsl.as_str().into()) });
        let layouts: Vec<wgpu::BindGroupLayout> = d.groups.iter().map(|g| {
            let entries: Vec<wgpu::BindGroupLayoutEntry> = g.iter().map(|e| {
                let mut vis = wgpu::ShaderStages::empty();
                if e.vertex { vis |= wgpu::ShaderStages::VERTEX; }
                if e.fragment { vis |= wgpu::ShaderStages::FRAGMENT; }
                let ty = match e.ty {
                    FxBindTy::Uniform => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    FxBindTy::Tex2D { filterable } => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    FxBindTy::Tex3D { filterable } => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    FxBindTy::Cube { filterable } => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::Cube, multisampled: false },
                    FxBindTy::SceneDepth => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    FxBindTy::Sampler { filtering } => wgpu::BindingType::Sampler(if filtering { wgpu::SamplerBindingType::Filtering } else { wgpu::SamplerBindingType::NonFiltering }),
                    FxBindTy::StorageRead => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                };
                wgpu::BindGroupLayoutEntry { binding: e.binding, visibility: vis, ty, count: None }
            }).collect();
            self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(&d.label), entries: &entries })
        }).collect();
        let layout_refs: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Some).collect();
        let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(&d.label), bind_group_layouts: &layout_refs, immediate_size: 0 });
        let attrs: Vec<Vec<wgpu::VertexAttribute>> = d.vertex_buffers.iter().map(|b| b.attrs.iter().map(|a| wgpu::VertexAttribute { format: a.format.wgpu(), offset: a.offset as u64, shader_location: a.location }).collect()).collect();
        let vbs: Vec<Option<wgpu::VertexBufferLayout>> = d.vertex_buffers.iter().zip(&attrs).map(|(b, a)| Some(wgpu::VertexBufferLayout { array_stride: b.stride as u64, step_mode: if b.instance { wgpu::VertexStepMode::Instance } else { wgpu::VertexStepMode::Vertex }, attributes: a })).collect();
        let blend = match d.blend {
            FxBlend::Opaque => None,
            FxBlend::Translucent => Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add } }),
            FxBlend::Additive => Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } }),
            FxBlend::Modulate => Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Dst, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } }),
        };
        let write = d.depth.write && !d.translucent;
        let (cull_mode, front_face) = match d.cull { FxCull::None => (None, wgpu::FrontFace::Ccw), FxCull::BackCw => (Some(wgpu::Face::Back), wgpu::FrontFace::Cw), FxCull::BackCcw => (Some(wgpu::Face::Back), wgpu::FrontFace::Ccw) };
        let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&d.label), layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some(&d.vs_entry), compilation_options: Default::default(), buffers: &vbs },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode, front_face, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: self.depth_format, depth_write_enabled: Some(write), depth_compare: Some(if d.depth.test { wgpu::CompareFunction::LessEqual } else { wgpu::CompareFunction::Always }), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(&d.fs_entry), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: self.hdr_format, blend, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            let msg = format!("{}: {e}", d.label);
            self.last_error = Some(msg.clone());
            return Err(msg);
        }
        self.pipes.push(GpuPipe { pipe, layouts, desc_groups: d.groups.clone(), translucent: d.translucent, instanced: d.vertex_buffers.iter().any(|b| b.instance) });
        Ok(FxPipeId(self.pipes.len() - 1))
    }
    /// Bind group `group` of `pipe`. `SceneDepth` entries are re-created whenever the renderer's targets are (the id stays valid).
    pub fn create_bind_group(&mut self, pipe: FxPipeId, group: u32, entries: Vec<FxBindEntry>) -> FxBindId {
        let uses_depth = entries.iter().any(|e| matches!(e.res, FxRes::SceneDepth));
        self.binds.push(BindGroupRec { pipe, group, entries, built: None, uses_depth });
        FxBindId(self.binds.len() - 1)
    }
    fn sampler(&mut self, d: FxSamplerDesc) -> &wgpu::Sampler {
        let device = &self.device;
        self.samplers.entry(d).or_insert_with(|| {
            let f = |x: FxFilter| match x { FxFilter::Nearest => wgpu::FilterMode::Nearest, FxFilter::Linear => wgpu::FilterMode::Linear };
            let a = match d.address { FxAddress::Wrap => wgpu::AddressMode::Repeat, FxAddress::Clamp => wgpu::AddressMode::ClampToEdge, FxAddress::Mirror => wgpu::AddressMode::MirrorRepeat };
            device.create_sampler(&wgpu::SamplerDescriptor { label: Some("fx sampler"), address_mode_u: a, address_mode_v: a, address_mode_w: a, mag_filter: f(d.mag), min_filter: f(d.min), mipmap_filter: match d.mip { FxFilter::Nearest => wgpu::MipmapFilterMode::Nearest, FxFilter::Linear => wgpu::MipmapFilterMode::Linear }, anisotropy_clamp: if d.non_filtering() { 1 } else { d.aniso.max(1) }, ..Default::default() })
        })
    }

    // ---- per frame ----------------------------------------------------------------------------------------------------------
    pub fn set_frame(&mut self, frame: FxFrame) { self.frame = frame; }
    pub fn has_draws(&self, translucent: bool) -> bool { self.frame.draws.iter().any(|d| self.pipes.get(d.pipe.0).map_or(false, |p| p.translucent == translucent)) }
    pub fn any(&self) -> bool { !self.frame.draws.is_empty() }

    /// Uploads the frame's buffers / dynamic geometry and (re)builds the bind groups that reference the scene depth. Call once per rendered frame before the passes.
    pub fn prepare(&mut self, depth_view: &wgpu::TextureView, depth_gen: u64) {
        let frame = std::mem::take(&mut self.frame);
        for (b, bytes) in &frame.buffer_writes { if let Some(buf) = self.buffers.get(b.0) { let n = (bytes.len() as u64).min(buf.size()) as usize; self.queue.write_buffer(buf, 0, &bytes[..n]); } }
        // dynamic geometry / instances
        self.placed.clear();
        let (mut vb_at, mut ib_at, mut inst_at) = (0u64, 0u64, 0u64);
        for d in &frame.draws {
            let mut p = Placed::default();
            if let FxGeom::Dynamic { vertices, indices } = &d.geom {
                let vlen = vertices.len() as u64; let ilen = (indices.len() * 4) as u64;
                if vb_at + vlen <= DYN_CAPACITY && ib_at + ilen <= DYN_CAPACITY {
                    self.queue.write_buffer(&self.dyn_vb, vb_at, vertices); self.queue.write_buffer(&self.dyn_ib, ib_at, bytemuck::cast_slice(indices));
                    p.vb = vb_at; p.ib = ib_at; p.index_count = indices.len() as u32; vb_at += (vlen + 15) / 16 * 16; ib_at += (ilen + 15) / 16 * 16;
                }
            }
            if let Some(i) = &d.instances {
                let n = i.bytes.len() as u64;
                if inst_at + n <= DYN_CAPACITY { self.queue.write_buffer(&self.inst_vb, inst_at, &i.bytes); p.inst = inst_at; inst_at += (n + 15) / 16 * 16; } else { p.index_count = 0; }
            }
            self.placed.push(p);
        }
        self.frame = frame;
        // bind groups
        self.depth_gen = depth_gen;
        for i in 0..self.binds.len() {
            let needs = { let b = &self.binds[i]; match &b.built { None => true, Some((g, _)) => b.uses_depth && *g != depth_gen } };
            if !needs { continue; }
            let (pipe, group, entries) = { let b = &self.binds[i]; (b.pipe, b.group, b.entries.clone()) };
            if self.pipes.get(pipe.0).is_none() { continue; }
            // sampler objects must outlive the entry list: resolve them first
            for e in &entries { if let FxRes::Sampler(s) = e.res { let _ = self.sampler(s); } }
            let mut bge: Vec<wgpu::BindGroupEntry> = vec![];
            for e in &entries {
                let resource = match e.res {
                    FxRes::Buffer(b) => self.buffers[b.0].as_entire_binding(),
                    FxRes::Texture(t) => wgpu::BindingResource::TextureView(&self.textures[t.0].view),
                    FxRes::Sampler(s) => wgpu::BindingResource::Sampler(self.samplers.get(&s).unwrap()),
                    FxRes::SceneDepth => wgpu::BindingResource::TextureView(depth_view),
                };
                bge.push(wgpu::BindGroupEntry { binding: e.binding, resource });
            }
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("fx bind group"), layout: &self.pipes[pipe.0].layouts[group as usize], entries: &bge });
            self.binds[i].built = Some((depth_gen, bg));
        }
    }

    /// Records the draws of one pass (`translucent` selects the pipelines) into `pass`.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, translucent: bool) {
        for (n, d) in self.frame_draws().iter().enumerate() {
            let Some(p) = self.pipes.get(d.pipe.0) else { continue };
            if p.translucent != translucent { continue; }
            let placed = self.placed.get(n).copied().unwrap_or_default();
            pass.set_pipeline(&p.pipe);
            let mut ok = true;
            for (g, b) in &d.binds { match self.binds.get(b.0).and_then(|r| r.built.as_ref()) { Some((_, bg)) => pass.set_bind_group(*g, bg, &[]), None => ok = false } }
            if !ok { continue; }
            let instances = match &d.instances { Some(i) => { if p.instanced { pass.set_vertex_buffer(1, self.inst_vb.slice(placed.inst..placed.inst + i.bytes.len() as u64)); } i.count } None => 1 };
            match &d.geom {
                FxGeom::MeshRange { mesh, first_index, index_count } => { let Some(m) = self.meshes.get(mesh.0) else { continue }; pass.set_vertex_buffer(0, m.vb.slice(..)); pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32); pass.draw_indexed(*first_index..(first_index + index_count).min(m.index_count), 0, 0..instances); }
                FxGeom::Mesh(m) => { let Some(m) = self.meshes.get(m.0) else { continue }; pass.set_vertex_buffer(0, m.vb.slice(..)); pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32); pass.draw_indexed(0..m.index_count, 0, 0..instances); }
                FxGeom::Dynamic { vertices, indices } => { if placed.index_count == 0 { continue; } pass.set_vertex_buffer(0, self.dyn_vb.slice(placed.vb..placed.vb + vertices.len() as u64)); pass.set_index_buffer(self.dyn_ib.slice(placed.ib..placed.ib + (indices.len() * 4) as u64), wgpu::IndexFormat::Uint32); pass.draw_indexed(0..placed.index_count, 0, 0..instances); }
            }
        }
    }
    fn frame_draws(&self) -> &[FxDraw] { &self.frame.draws }
    pub fn depth_format(&self) -> wgpu::TextureFormat { self.depth_format }
    pub fn texture_dim(&self, t: FxTexId) -> FxTexDim { self.textures[t.0].dim }
    /// Keeps the unused-field lint quiet for the textures' own handle.
    pub fn texture_count(&self) -> usize { self.textures.iter().filter(|t| t.tex.width() > 0).count() }
}

/// 4 x f32 helper for callers that fill uniform buffers.
#[repr(C)] #[derive(Clone, Copy, Debug, Default, Pod, Zeroable)] pub struct Vec4f(pub [f32; 4]);



