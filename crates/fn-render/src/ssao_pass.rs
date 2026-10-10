//! AO ORDER (2026-10-05): the screen-space AO used to be applied by post.wgsl, i.e. AFTER `SceneColorHalfRes`, so bloom and the eye-adaptation
//! histogram saw the scene without AO. UE 4.16 applies SSAO inside the lighting (deferred `ReflectionEnvironmentAndSky` / `SkyLighting.usf` multiply the
//! sky light's diffuse by `ScreenSpaceAO`) - before fog, translucency, light shafts and every post-process pass. This pass applies `ScreenSpaceAO` (the UE 4.16
//! SSAO passes of ssao_ue.rs / ssao_ue.wgsl, bound at 11) to the HDR scene right after the opaque scene, so everything downstream sees the occluded scene.
//! DFAO is unaffected: it already acts in the lit shaders (lit_common.wgsl `df_sky_occlusion`).
//! The pass reads a copy of the HDR target (a render pass cannot sample its own attachment) and writes the result back.
//!
//! REFLECTION APPLY (docs/spec/reflection_gaps.md sections 1-2, reflection.rs): the same pass is UE's `ReflectionApplyPS` - it adds the deferred specular
//! indirect light (sky specular + SSR) with the specular occlusion of `GBufferAO * ScreenSpaceAO` (shaders/ssao_apply.wgsl). With SSAO off
//! (`r.AmbientOcclusionLevels 0`) only the reflections are added (One / One blend, no scene copy, ScreenSpaceAO = 1).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ApplyU { flags: [f32; 4] }

pub struct SsaoPass {
    pipeline: wgpu::RenderPipeline,
    reflect_pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    apply_buf: wgpu::Buffer,
    /// (key, scene copy, bind group); key = (w, h, target generation, SSR output size, UE SSAO output generation)
    copy: Option<((u32, u32, u64, Option<(u32, u32)>, u64), wgpu::Texture, wgpu::BindGroup)>,
}

impl SsaoPass {
    /// `common` = common.wgsl; the module is common + post.wgsl + ssao_apply.wgsl.
    pub fn new(device: &wgpu::Device, common: &str, hdr_format: wgpu::TextureFormat) -> SsaoPass {
        let src = [common, fn_live::wgsl!("../shaders/post.wgsl"), fn_live::wgsl!("../shaders/ssao_apply.wgsl")].join("\n");
        let sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ssao apply"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let f = wgpu::ShaderStages::FRAGMENT;
        let ub = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: f, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let tex = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ssao / reflection apply bgl"), entries: &[
            ub(0), tex(1),
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            tex(6), tex(7), tex(8), ub(9), tex(11),
        ] });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ssao / reflection apply layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipe = |label: &str, fs: &str, blend: Option<wgpu::BlendState>, write_mask: wgpu::ColorWrites| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &sm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &sm, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: hdr_format, blend, write_mask })] }),
            multiview_mask: None, cache: None,
        });
        let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let pipeline = pipe("ssao + reflection apply", "fs_ssao", None, wgpu::ColorWrites::ALL);
        let reflect_pipeline = pipe("reflection apply", "fs_reflect", Some(wgpu::BlendState { color: add, alpha: add }), wgpu::ColorWrites::COLOR);
        let apply_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("reflection apply params"), size: std::mem::size_of::<ApplyU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        SsaoPass { pipeline, reflect_pipeline, bgl, apply_buf, copy: None }
    }
    /// (Re)creates the scene copy and the bind group for targets of `w` x `h` (generation `gen`); `refl` = the lit passes' reflection targets 0 / 1,
    /// `ssr` = the SSR output (`ssr_key` its generation, `ssr_valid` = SSR ran this frame), `ao` = UE SSAO `ScreenSpaceAO` (`ao_key` its generation).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, post_buf: &wgpu::Buffer, depth: &wgpu::TextureView, refl: [&wgpu::TextureView; 2], ssr: &wgpu::TextureView,
                   ssr_key: Option<(u32, u32)>, ssr_valid: bool, ssao_on: bool, ao: &wgpu::TextureView, ao_key: u64, format: wgpu::TextureFormat, w: u32, h: u32, gen: u64) {
        queue.write_buffer(&self.apply_buf, 0, bytemuck::bytes_of(&ApplyU { flags: [if ssr_valid { 1. } else { 0. }, if ssao_on { 1. } else { 0. }, if crate::reflection::debug_view() { 1. } else { 0. }, 0.] }));
        let key = (w, h, gen, ssr_key, ao_key);
        if self.copy.as_ref().map(|c| c.0) == Some(key) { return; }
        let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("ssao scene copy"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let view = tex.create_view(&Default::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ssao / reflection apply bg"), layout: &self.bgl, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: post_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(refl[0]) },
            wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(refl[1]) },
            wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(ssr) },
            wgpu::BindGroupEntry { binding: 9, resource: self.apply_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(ao) },
        ] });
        self.copy = Some((key, tex, bg));
    }
    /// SSAO on: copies mip 0 of `hdr` and writes the AO-applied scene + the reflections back into `hdr_mip0`. SSAO off: adds the reflections only.
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, hdr: &wgpu::Texture, hdr_mip0: &wgpu::TextureView, ssao_on: bool, ts: Option<wgpu::RenderPassTimestampWrites<'_>>) {
        let Some((key, tex, bg)) = &self.copy else { return };
        if ssao_on {
            enc.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: hdr, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: key.0, height: key.1, depth_or_array_layers: 1 },
            );
        }
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(if ssao_on { "ssao + reflection apply" } else { "reflection apply" }),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: hdr_mip0, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: ts, occlusion_query_set: None, multiview_mask: None,
        });
        pass.set_pipeline(if ssao_on { &self.pipeline } else { &self.reflect_pipeline });
        pass.set_bind_group(0, bg, &[]);
        pass.draw(0..3, 0..1);
    }
}
