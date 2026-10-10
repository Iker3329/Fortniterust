//! COLOR LUT (GLOBAL LOOK 2026-10-05): the 32^3 colour-grading volume LUT of the UE 4.16 tonemapper (`FRCPassPostProcessCombineLUTs`), rebuilt every
//! frame from the post-process uniform block (white balance, colour grading, Film* curve, display gamma) and sampled by post.wgsl.
//!
//! UE 4.16 does not evaluate the colour transform per pixel: `PostProcessCombineLUTs.usf` bakes it into a 32^3 `PF_A2B10G10R10` volume texture
//! (`GLUTSize` 32, PostProcessCombineLUTs.cpp:37 / :1022, `r.LUT.Size` not in the cooked ini) indexed by `LinToLog`, and `PostProcessTonemap.usf`
//! `ColorLookupTable` samples it trilinearly. The port used to evaluate the transform directly, which (a) missed the `/ 1.05` the combine pass stores
//! (1.8 DXBC GlobalShaderCache#1182809: `mul o0.xyz, r0.xyzx, l(0.952381)`), so with the tonemapper's `* 1.05` every pixel came out 5 % brighter in
//! display space and highlights clipped at 0.952, and (b) skipped the LUT's 0.4375-stop trilinear interpolation and 10-bit quantisation.
//! Shader: `shaders/color_lut.wgsl` (+ `lut_device_color` in post.wgsl).

pub const LUT_SIZE: u32 = 32;
/// The LUT texture format: rgba16float stands in for PF_A2B10G10R10 (the 10-bit rounding is done in the shader before the store).
pub const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub struct ColorLut {
    pipeline: wgpu::ComputePipeline,
    params_bg: wgpu::BindGroup,
    out_bg: wgpu::BindGroup,
    view: wgpu::TextureView,
}

impl ColorLut {
    /// `common` = common.wgsl; `post_params` = the post pass uniform buffer (`tonemap::PostParams`), read at dispatch time.
    pub fn new(device: &wgpu::Device, common: &str, post_params: &wgpu::Buffer) -> ColorLut {
        let src = [common, fn_live::wgsl!("../shaders/post.wgsl"), fn_live::wgsl!("../shaders/color_lut.wgsl")].join("\n");
        let sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("combine luts"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let c = wgpu::ShaderStages::COMPUTE;
        let bgl_params = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("combine luts params bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: c, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ] });
        let bgl_out = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("combine luts out bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: c, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: LUT_FORMAT, view_dimension: wgpu::TextureViewDimension::D3 }, count: None },
        ] });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("combine luts layout"), bind_group_layouts: &[Some(&bgl_params), Some(&bgl_out)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("combine luts"), layout: Some(&layout), module: &sm, entry_point: Some("combine_luts"), compilation_options: Default::default(), cache: None });
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("color grading lut"), size: wgpu::Extent3d { width: LUT_SIZE, height: LUT_SIZE, depth_or_array_layers: LUT_SIZE }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D3, format: LUT_FORMAT, usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let params_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("combine luts params"), layout: &bgl_params, entries: &[wgpu::BindGroupEntry { binding: 0, resource: post_params.as_entire_binding() }] });
        let out_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("combine luts out"), layout: &bgl_out, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }] });
        ColorLut { pipeline, params_bg, out_bg, view }
    }
    /// The LUT view the tonemapper samples (post.wgsl binding 10).
    pub fn view(&self) -> &wgpu::TextureView { &self.view }
    /// Rebuilds the LUT from the current contents of the post uniform buffer (run after it is written, before the post pass).
    pub fn dispatch(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("combine luts"), timestamp_writes: None });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.params_bg, &[]);
        pass.set_bind_group(1, &self.out_bg, &[]);
        pass.dispatch_workgroups(LUT_SIZE / 4, LUT_SIZE / 4, LUT_SIZE / 4);
    }
}
