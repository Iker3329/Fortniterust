//! OUTLINE: the build preview's highlight of building actors (1.8: `AFortPlayerController` vfunc_672 @140849310). The highlighted actors draw into a custom depth
//! target with a custom stencil value ([`crate::statics`] `draw_custom`), then a `BL_BeforeTranslucency` post-process material
//! (`SkinAndMeshOutlinePostProcess`, the time-of-day manager's `BasePostProcessMaterial`) reads the scene depth, the custom depth, the scene colour and the custom
//! stencil and rewrites the scene colour: the stencil-marked pixels pulse between two scene modulation colours and get an outline where the custom depth has an edge.
//! The renderer knows nothing about Unreal here: the program, its bindings and its uniform buffers come from the caller ([`OutlineDesc`]); the caller writes the
//! buffers every frame ([`OutlineFrame`]).
//!
//! Order inside the frame graph (`Renderer::render_scene`): after the opaque scene and the volumetric fog composite, before the translucency (UE4.16
//! `BL_BeforeTranslucency`): custom depth pass -> copy of the scene colour -> the post program writing the scene colour. Nothing runs when no actor is highlighted
//! (the program is the identity on pixels without the stencil value, so skipping it is exact).

/// What a binding of the post program is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineBind {
    /// A uniform buffer (index into [`OutlineDesc::buffer_sizes`]).
    Uniform(usize),
    /// `SceneDepthTexture` (device Z, unfilterable).
    SceneDepth,
    /// `CustomDepthTexture` (device Z, unfilterable).
    CustomDepth,
    /// `PostprocessInput0` (the scene colour before this pass).
    SceneColor,
    /// `CustomStencilTexture` (unsigned, read with `textureLoad`).
    CustomStencil,
    /// A point sampler (the depth textures).
    PointSampler,
    /// A bilinear clamp sampler (the scene colour).
    LinearSampler,
}

#[derive(Clone, Debug)]
pub struct OutlineDesc {
    pub label: String,
    pub wgsl: String,
    pub vs_entry: String,
    pub fs_entry: String,
    /// (group, binding, what)
    pub entries: Vec<(u32, u32, OutlineBind)>,
    /// Bytes of each uniform buffer.
    pub buffer_sizes: Vec<usize>,
}

/// One frame: whether the pass runs, the custom stencil value the highlighted actors write, the uniform buffer contents.
#[derive(Clone, Debug, Default)]
pub struct OutlineFrame { pub active: bool, pub stencil: f32, pub buffer_writes: Vec<(usize, Vec<u8>)> }

struct Targets { w: u32, h: u32, depth: wgpu::TextureView, stencil: wgpu::TextureView, copy: wgpu::Texture, copy_view: wgpu::TextureView }

pub struct OutlineRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    desc: Option<OutlineDesc>,
    layouts: Vec<wgpu::BindGroupLayout>,
    pipe: Option<wgpu::RenderPipeline>,
    buffers: Vec<wgpu::Buffer>,
    groups: Vec<wgpu::BindGroup>,
    targets: Option<Targets>,
    point: wgpu::Sampler,
    linear: wgpu::Sampler,
    frame: OutlineFrame,
    pub last_error: Option<String>,
}

impl OutlineRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let s = |label: &str, f: wgpu::FilterMode| device.create_sampler(&wgpu::SamplerDescriptor { label: Some(label), address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, address_mode_w: wgpu::AddressMode::ClampToEdge, mag_filter: f, min_filter: f, ..Default::default() });
        // UE4.16 binds the scene textures with point samplers and PostprocessInput0 with `TStaticSamplerState<SF_Bilinear, AM_Clamp ...>`
        OutlineRenderer { device: device.clone(), queue: queue.clone(), desc: None, layouts: vec![], pipe: None, buffers: vec![], groups: vec![], targets: None,
            point: s("outline point sampler", wgpu::FilterMode::Nearest), linear: s("outline bilinear sampler", wgpu::FilterMode::Linear), frame: OutlineFrame::default(), last_error: None }
    }

    /// Creates the post program for an HDR target of `format`. A WGSL / validation error is returned (and kept in `last_error`).
    pub fn create(&mut self, d: OutlineDesc, format: wgpu::TextureFormat) -> Result<(), String> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(&d.label), source: wgpu::ShaderSource::Wgsl(d.wgsl.clone().into()) });
        let groups = d.entries.iter().map(|e| e.0).max().map_or(0, |g| g + 1);
        let layouts: Vec<wgpu::BindGroupLayout> = (0..groups).map(|g| {
            let entries: Vec<wgpu::BindGroupLayoutEntry> = d.entries.iter().filter(|e| e.0 == g).map(|&(_, binding, what)| {
                let tex = |sample_type| wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
                let ty = match what {
                    OutlineBind::Uniform(_) => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    OutlineBind::SceneDepth | OutlineBind::CustomDepth => tex(wgpu::TextureSampleType::Float { filterable: false }),
                    OutlineBind::SceneColor => tex(wgpu::TextureSampleType::Float { filterable: true }),
                    OutlineBind::CustomStencil => tex(wgpu::TextureSampleType::Uint),
                    OutlineBind::PointSampler => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    OutlineBind::LinearSampler => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                };
                wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty, count: None }
            }).collect();
            self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(&d.label), entries: &entries })
        }).collect();
        let refs: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Some).collect();
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(&d.label), bind_group_layouts: &refs, immediate_size: 0 });
        let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&d.label), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some(&d.vs_entry), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(&d.fs_entry), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        let buffers = d.buffer_sizes.iter().map(|&n| self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(&d.label), size: (n.max(16).div_ceil(16) * 16) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })).collect();
        if let Some(e) = crate::gpu::pop_error_scope(scope) { let m = format!("{}: {e}", d.label); self.last_error = Some(m.clone()); return Err(m); }
        self.desc = Some(d); self.layouts = layouts; self.pipe = Some(pipe); self.buffers = buffers; self.groups.clear();
        Ok(())
    }

    pub fn installed(&self) -> bool { self.pipe.is_some() }
    pub fn set_frame(&mut self, f: OutlineFrame) { self.frame = f; }
    /// The pass runs this frame (installed, asked for, and something to draw into the custom depth).
    pub fn active(&self, statics_have_custom: bool) -> bool { self.frame.active && self.pipe.is_some() && statics_have_custom }
    pub fn stencil(&self) -> f32 { self.frame.stencil }

    /// Uniform uploads, targets at the scene size, bind groups (around `scene_depth`). Call once per frame before encoding.
    pub fn prepare(&mut self, w: u32, h: u32, hdr_format: wgpu::TextureFormat, scene_depth: &wgpu::TextureView) {
        for (b, bytes) in std::mem::take(&mut self.frame.buffer_writes) { if let Some(buf) = self.buffers.get(b) { let n = (bytes.len() as u64).min(buf.size()) as usize; self.queue.write_buffer(buf, 0, &bytes[..n]); } }
        if !self.frame.active || self.pipe.is_none() { return; }
        if self.targets.as_ref().map_or(true, |t| t.w != w || t.h != h) {
            let tex = |label: &str, format: wgpu::TextureFormat, extra: wgpu::TextureUsages| self.device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | extra, view_formats: &[] });
            let depth = tex("custom depth", crate::statics::CUSTOM_DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
            let stencil = tex("custom stencil", crate::statics::CUSTOM_STENCIL_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
            let copy = tex("outline scene colour", hdr_format, wgpu::TextureUsages::COPY_DST);
            let copy_view = copy.create_view(&Default::default());
            self.targets = Some(Targets { w, h, depth, stencil, copy, copy_view });
        }
        let (Some(d), Some(t)) = (self.desc.as_ref(), self.targets.as_ref()) else { return };
        self.groups = self.layouts.iter().enumerate().map(|(g, layout)| {
            let entries: Vec<wgpu::BindGroupEntry> = d.entries.iter().filter(|e| e.0 == g as u32).map(|&(_, binding, what)| wgpu::BindGroupEntry { binding, resource: match what {
                OutlineBind::Uniform(i) => self.buffers[i].as_entire_binding(),
                OutlineBind::SceneDepth => wgpu::BindingResource::TextureView(scene_depth),
                OutlineBind::CustomDepth => wgpu::BindingResource::TextureView(&t.depth),
                OutlineBind::SceneColor => wgpu::BindingResource::TextureView(&t.copy_view),
                OutlineBind::CustomStencil => wgpu::BindingResource::TextureView(&t.stencil),
                OutlineBind::PointSampler => wgpu::BindingResource::Sampler(&self.point),
                OutlineBind::LinearSampler => wgpu::BindingResource::Sampler(&self.linear),
            } }).collect();
            self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("outline"), layout, entries: &entries })
        }).collect();
    }

    /// The custom depth pass: clears the targets (far depth, stencil 0) and lets `draw` issue the highlighted actors' draws.
    pub fn encode_custom_depth(&self, enc: &mut wgpu::CommandEncoder, draw: impl FnOnce(&mut wgpu::RenderPass<'_>)) {
        let Some(t) = self.targets.as_ref() else { return };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("custom depth"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.stencil, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
        draw(&mut pass);
    }

    /// Copies the scene colour (`hdr` mip 0) and runs the post program into `hdr_view`.
    pub fn encode_post(&self, enc: &mut wgpu::CommandEncoder, hdr: &wgpu::Texture, hdr_view: &wgpu::TextureView) {
        let (Some(t), Some(pipe)) = (self.targets.as_ref(), self.pipe.as_ref()) else { return };
        if self.groups.is_empty() { return; }
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: hdr, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &t.copy, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: t.w, height: t.h, depth_or_array_layers: 1 });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("outline post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: hdr_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
        pass.set_pipeline(pipe);
        for (g, bg) in self.groups.iter().enumerate() { pass.set_bind_group(g as u32, bg, &[]); }
        pass.draw(0..3, 0..1);
    }
}
