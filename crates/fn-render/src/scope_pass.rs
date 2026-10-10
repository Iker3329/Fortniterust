//! The scope pass: a screen-space post-process material pass over the finished (tone-mapped, sRGB-encoded) LDR image plus an optional opaque mesh drawn into that image
//! first. Like the gun pass the renderer knows nothing about Unreal here: it owns GPU textures / samplers / uniform buffers / pipelines created from plain descriptions
//! and draws the [`ScopeFrame`] the application submits for the next frame with `Renderer::set_scope_frame`.
//!
//! Order inside the frame graph (see `Renderer::render_scene`): `post` -> LDR image, then [`ScopeRenderer::encode_ring`] (the mesh, into the LDR image), then
//! [`ScopeRenderer::encode_post`] (a full-screen triangle whose fragment program samples the LDR image and writes the next target), then FXAA when it is on.
//! That is the UE 4.16 order for an `AfterTonemapping` post-process material followed by the AA pass.
//!
//! Resource model of the post program: every resource lives at the (group, binding) its description names; `ScopeBind::Scene` / `SceneSampler` are filled with the LDR
//! image of the frame. Bind groups are rebuilt every prepared frame (the LDR image is re-created on resize).
use crate::gun_material::{GunAddress, GunFilter, GunSamplerDesc, GunTextureDesc};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct ScopeTexId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct ScopeBufId(pub usize);

/// An HDR cube map: per mip level the face edge and the six faces (+X -X +Y -Y +Z -Z, the D3D / WebGPU order) as RGBA16F bit patterns, row 0 = top.
#[derive(Clone, Debug)]
pub struct ScopeCubeDesc { pub label: String, pub levels: Vec<(u32, [Vec<u16>; 6])> }

#[derive(Clone, Copy, Debug)]
pub enum ScopeBind { Uniform(ScopeBufId), Tex2D(ScopeTexId), Cube(ScopeTexId), Sampler(GunSamplerDesc), Scene, SceneSampler }
#[derive(Clone, Copy, Debug)]
pub struct ScopeBindEntry { pub group: u32, pub binding: u32, pub bind: ScopeBind }

/// The full-screen post program (`vs_entry` draws a 3-vertex triangle, no vertex buffers; `fs_entry` writes `@location(0)`).
#[derive(Clone, Debug)]
pub struct ScopePostDesc { pub label: String, pub wgsl: String, pub vs_entry: String, pub fs_entry: String, pub entries: Vec<ScopeBindEntry> }

/// The opaque mesh drawn into the LDR image: one `@location(0) vec3<f32>` position stream, a single uniform buffer at `@group(0) @binding(0)`, no depth, no culling.
#[derive(Clone, Debug)]
pub struct ScopeRingDesc { pub label: String, pub wgsl: String, pub vs_entry: String, pub fs_entry: String, pub positions: Vec<[f32; 3]>, pub indices: Vec<u32>, pub uniform: ScopeBufId }

/// What the scope pass does in one frame: buffer contents written before the pass, and which of the two draws run (`ring` only together with `post`).
#[derive(Clone, Debug, Default)]
pub struct ScopeFrame { pub buffer_writes: Vec<(ScopeBufId, Vec<u8>)>, pub post: bool, pub ring: bool }

struct PostPipe { desc: ScopePostDesc, module: wgpu::ShaderModule, group_layouts: Vec<wgpu::BindGroupLayout>, layout: wgpu::PipelineLayout, pipes: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>, groups: Vec<wgpu::BindGroup> }
struct RingPipe { label: String, vs: String, fs: String, module: wgpu::ShaderModule, bgl: wgpu::BindGroupLayout, layout: wgpu::PipelineLayout, pipes: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>, vb: wgpu::Buffer, ib: wgpu::Buffer, index_count: u32, uniform: ScopeBufId, bind: Option<wgpu::BindGroup> }

pub struct ScopeRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    textures: Vec<wgpu::TextureView>,
    buffers: Vec<wgpu::Buffer>,
    samplers: HashMap<GunSamplerDesc, wgpu::Sampler>,
    scene_sampler: wgpu::Sampler,
    post: Option<PostPipe>,
    ring: Option<RingPipe>,
    frame: ScopeFrame,
    /// Last error text of a failed pipeline creation.
    pub last_error: Option<String>,
}

fn sampler_for(device: &wgpu::Device, d: GunSamplerDesc) -> wgpu::Sampler {
    let f = |x: GunFilter| match x { GunFilter::Nearest => wgpu::FilterMode::Nearest, GunFilter::Linear => wgpu::FilterMode::Linear };
    let a = match d.address { GunAddress::Wrap => wgpu::AddressMode::Repeat, GunAddress::Clamp => wgpu::AddressMode::ClampToEdge, GunAddress::Mirror => wgpu::AddressMode::MirrorRepeat };
    let all_linear = d.mag == GunFilter::Linear && d.min == GunFilter::Linear && d.mip == GunFilter::Linear;
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("scope sampler"), address_mode_u: a, address_mode_v: a, address_mode_w: a, mag_filter: f(d.mag), min_filter: f(d.min),
        mipmap_filter: match d.mip { GunFilter::Nearest => wgpu::MipmapFilterMode::Nearest, GunFilter::Linear => wgpu::MipmapFilterMode::Linear },
        anisotropy_clamp: if all_linear { d.aniso.max(1) } else { 1 }, ..Default::default()
    })
}

impl ScopeRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> ScopeRenderer {
        // UE 4.16 binds PostprocessInput0 with `TStaticSamplerState<SF_Bilinear, AM_Clamp, AM_Clamp, AM_Clamp>`
        let scene_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("scope scene sampler"), address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, address_mode_w: wgpu::AddressMode::ClampToEdge, mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        ScopeRenderer { device: device.clone(), queue: queue.clone(), textures: vec![], buffers: vec![], samplers: HashMap::new(), scene_sampler, post: None, ring: None, frame: ScopeFrame::default(), last_error: None }
    }

    pub fn create_texture(&mut self, d: &GunTextureDesc) -> ScopeTexId {
        let (w0, h0, _) = &d.levels[0];
        let format = if d.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor { label: Some(&d.label), size: wgpu::Extent3d { width: *w0, height: *h0, depth_or_array_layers: 1 }, mip_level_count: d.levels.len() as u32, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        for (i, (w, h, rgba8)) in d.levels.iter().enumerate() {
            self.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, rgba8, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) }, wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 });
        }
        self.textures.push(tex.create_view(&Default::default()));
        ScopeTexId(self.textures.len() - 1)
    }

    pub fn create_cube(&mut self, d: &ScopeCubeDesc) -> ScopeTexId {
        let e0 = d.levels[0].0;
        let tex = self.device.create_texture(&wgpu::TextureDescriptor { label: Some(&d.label), size: wgpu::Extent3d { width: e0, height: e0, depth_or_array_layers: 6 }, mip_level_count: d.levels.len() as u32, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        for (m, (edge, faces)) in d.levels.iter().enumerate() {
            for (f, px) in faces.iter().enumerate() {
                self.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: m as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: f as u32 }, aspect: wgpu::TextureAspect::All }, bytemuck::cast_slice(px), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8 * edge), rows_per_image: Some(*edge) }, wgpu::Extent3d { width: *edge, height: *edge, depth_or_array_layers: 1 });
            }
        }
        self.textures.push(tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::Cube), ..Default::default() }));
        ScopeTexId(self.textures.len() - 1)
    }

    /// A uniform buffer of `size` bytes (rounded up to 16).
    pub fn create_buffer(&mut self, label: &str, size: usize) -> ScopeBufId {
        self.buffers.push(self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (size.max(16).div_ceil(16) * 16) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        ScopeBufId(self.buffers.len() - 1)
    }

    fn layout_entry(e: &ScopeBindEntry) -> wgpu::BindGroupLayoutEntry {
        let (vis, ty) = match e.bind {
            ScopeBind::Uniform(_) => (wgpu::ShaderStages::VERTEX_FRAGMENT, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
            ScopeBind::Tex2D(_) | ScopeBind::Scene => (wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            ScopeBind::Cube(_) => (wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::Cube, multisampled: false }),
            ScopeBind::Sampler(_) | ScopeBind::SceneSampler => (wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
        };
        wgpu::BindGroupLayoutEntry { binding: e.binding, visibility: vis, ty, count: None }
    }

    /// Creates the full-screen post program. A WGSL / validation error is returned (and kept in `last_error`) instead of panicking.
    pub fn create_post(&mut self, d: ScopePostDesc) -> Result<(), String> {
        for (i, e) in d.entries.iter().enumerate() { if d.entries.iter().skip(i + 1).any(|s| s.group == e.group && s.binding == e.binding) { return Err(format!("{}: group {} binding {} bound twice", d.label, e.group, e.binding)); } }
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(&d.label), source: wgpu::ShaderSource::Wgsl(d.wgsl.clone().into()) });
        let groups = d.entries.iter().map(|e| e.group).max().map_or(0, |g| g + 1);
        let group_layouts: Vec<wgpu::BindGroupLayout> = (0..groups).map(|g| {
            let entries: Vec<wgpu::BindGroupLayoutEntry> = d.entries.iter().filter(|e| e.group == g).map(Self::layout_entry).collect();
            self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(&d.label), entries: &entries })
        }).collect();
        let refs: Vec<Option<&wgpu::BindGroupLayout>> = group_layouts.iter().map(Some).collect();
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(&d.label), bind_group_layouts: &refs, immediate_size: 0 });
        for e in &d.entries { if let ScopeBind::Sampler(s) = e.bind { let dev = &self.device; self.samplers.entry(s).or_insert_with(|| sampler_for(dev, s)); } }
        if let Some(e) = crate::gpu::pop_error_scope(scope) { let m = format!("{}: {e}", d.label); self.last_error = Some(m.clone()); return Err(m); }
        self.post = Some(PostPipe { desc: d, module, group_layouts, layout, pipes: HashMap::new(), groups: vec![] });
        Ok(())
    }

    pub fn create_ring(&mut self, d: ScopeRingDesc) -> Result<(), String> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(&d.label), source: wgpu::ShaderSource::Wgsl(d.wgsl.clone().into()) });
        let bgl = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(&d.label), entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }] });
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(&d.label), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let vb = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(&d.label), size: (d.positions.len() * 12).max(4) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&d.positions));
        let ib = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(&d.label), size: (d.indices.len() * 4).max(4) as u64, usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&d.indices));
        if let Some(e) = crate::gpu::pop_error_scope(scope) { let m = format!("{}: {e}", d.label); self.last_error = Some(m.clone()); return Err(m); }
        self.ring = Some(RingPipe { label: d.label, vs: d.vs_entry, fs: d.fs_entry, module, bgl, layout, pipes: HashMap::new(), vb, ib, index_count: d.indices.len() as u32, uniform: d.uniform, bind: None });
        Ok(())
    }

    pub fn installed(&self) -> bool { self.post.is_some() }
    pub fn set_frame(&mut self, f: ScopeFrame) { self.frame = f; }
    /// True when this frame runs the post program (the LDR image is then needed).
    pub fn active(&self) -> bool { self.frame.post && self.post.is_some() }

    /// Uploads the frame's uniform buffers. Call once per rendered frame (also when the pass is not active, the writes are consumed).
    pub fn upload(&mut self) {
        for (b, bytes) in &self.frame.buffer_writes { if let Some(buf) = self.buffers.get(b.0) { let n = (bytes.len() as u64).min(buf.size()) as usize; self.queue.write_buffer(buf, 0, &bytes[..n]); } }
        self.frame.buffer_writes.clear();
    }

    /// Creates the pipelines for the target formats on first use and rebuilds the bind groups around `scene` (only while [`Self::active`]).
    /// `ring_format` = format of the image the mesh is drawn into (the LDR image), `post_format` = format of the image the post program writes.
    pub fn prepare(&mut self, scene: &wgpu::TextureView, ring_format: wgpu::TextureFormat, post_format: wgpu::TextureFormat) {
        if !self.active() { return; }
        let device = self.device.clone();
        if self.frame.ring {
            if let Some(r) = self.ring.as_mut() {
                if !r.pipes.contains_key(&ring_format) {
                    let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some(&r.label), layout: Some(&r.layout),
                        vertex: wgpu::VertexState { module: &r.module, entry_point: Some(&r.vs), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }] })] },
                        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                        depth_stencil: None, multisample: Default::default(),
                        fragment: Some(wgpu::FragmentState { module: &r.module, entry_point: Some(&r.fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: ring_format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
                        multiview_mask: None, cache: None,
                    });
                    r.pipes.insert(ring_format, p);
                }
                r.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(&r.label), layout: &r.bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.buffers[r.uniform.0].as_entire_binding() }] }));
            }
        }
        if let Some(p) = self.post.as_mut() {
            if !p.pipes.contains_key(&post_format) {
                let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&p.desc.label), layout: Some(&p.layout),
                    vertex: wgpu::VertexState { module: &p.module, entry_point: Some(&p.desc.vs_entry), compilation_options: Default::default(), buffers: &[] },
                    primitive: wgpu::PrimitiveState::default(), depth_stencil: None, multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState { module: &p.module, entry_point: Some(&p.desc.fs_entry), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: post_format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
                    multiview_mask: None, cache: None,
                });
                p.pipes.insert(post_format, pipe);
            }
            let (buffers, textures, samplers, scene_sampler) = (&self.buffers, &self.textures, &self.samplers, &self.scene_sampler);
            p.groups = p.group_layouts.iter().enumerate().map(|(g, layout)| {
                let entries: Vec<wgpu::BindGroupEntry> = p.desc.entries.iter().filter(|e| e.group == g as u32).map(|e| wgpu::BindGroupEntry { binding: e.binding, resource: match e.bind {
                    ScopeBind::Uniform(b) => buffers[b.0].as_entire_binding(),
                    ScopeBind::Tex2D(t) | ScopeBind::Cube(t) => wgpu::BindingResource::TextureView(&textures[t.0]),
                    ScopeBind::Sampler(s) => wgpu::BindingResource::Sampler(&samplers[&s]),
                    ScopeBind::Scene => wgpu::BindingResource::TextureView(scene),
                    ScopeBind::SceneSampler => wgpu::BindingResource::Sampler(scene_sampler),
                } }).collect();
                device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(&p.desc.label), layout, entries: &entries })
            }).collect();
        }
    }

    /// The mesh, drawn into `scene` (the LDR image; its contents are kept, `ring_format` must match [`Self::prepare`]).
    pub fn encode_ring(&self, enc: &mut wgpu::CommandEncoder, scene: &wgpu::TextureView, ring_format: wgpu::TextureFormat) {
        if !(self.active() && self.frame.ring) { return; }
        let Some(r) = self.ring.as_ref() else { return };
        let (Some(pipe), Some(bind)) = (r.pipes.get(&ring_format), r.bind.as_ref()) else { return };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scope ring"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: scene, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, bind, &[]);
        pass.set_vertex_buffer(0, r.vb.slice(..));
        pass.set_index_buffer(r.ib.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..r.index_count, 0, 0..1);
    }

    /// The post program: reads the LDR image (bound by [`Self::prepare`]), writes `target` (every pixel).
    pub fn encode_post(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, post_format: wgpu::TextureFormat) {
        if !self.active() { return; }
        let Some(p) = self.post.as_ref() else { return };
        let Some(pipe) = p.pipes.get(&post_format) else { return };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scope post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
        pass.set_pipeline(pipe);
        for (g, bg) in p.groups.iter().enumerate() { pass.set_bind_group(g as u32, bg, &[]); }
        pass.draw(0..3, 0..1);
    }
}
