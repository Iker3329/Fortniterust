//! DF SHADOW SCREEN PASS (2026-10-06): the screen-space pass of the sun's ray-traced distance-field cascade, UE4.16 `DistanceFieldShadowing.cpp`
//! `FProjectedShadowInfo::BeginRenderRayTracedDistanceFieldProjection` -> `RayTraceShadows` (`TDistanceFieldShadowingCS<DFS_DirectionalLightScatterTileCulling>`)
//! into the `RayTracedShadows` render target, which `RenderRayTracedDistanceFieldProjection` then upsamples into the light attenuation
//! (`TDistanceFieldShadowingUpsamplePS<true>`). Shader: `shaders/distance_field_shadow.wgsl`; the upsample runs per pixel in the lit shaders
//! (`lit_common.wgsl` `df_shadow_upsample`), which read this target at globals binding 16.
//!
//! # 1:1 with 1.8
//! * Resolution: `GetDFShadowDownsampleFactor()` = `GFullResolutionDFShadowing ? 1 : GAODownsampleFactor` = 2 (`r.DFFullResolution` default 0, not in the
//!   cooked ini; the 1.8 exe computes `2 - (DAT_1467bf8f0 != 0)` in `FProjectedShadowInfo::RenderRayTracedDistanceFieldProjection` @141803d90, its cvar global
//!   in .bss = 0). Target size `GetBufferSizeForDFShadows()` = `DivideAndRoundDown(BufferSize, factor)` (DistanceFieldShadowing.cpp:76): floor, also for
//!   odd sizes (2544 x 1353 -> 1272 x 676).
//! * Format `PF_G16R16F` (DistanceFieldShadowing.cpp:908; the 1.8 desc at @141803d90 stores format 0xf = PF_G16R16F, PixelFormat.h): `Rg16Float`, written as a
//!   render target (WebGPU core renders to rg16float; it is not a core storage format). The scatter-tile CS permutation uses no groupshared data, so one
//!   fragment per texel computes exactly the thread's texel.
//! * Order: after the full depth pre-pass and the backdrop's depth merge (the one scene depth UE's CS reads), before the base pass that consumes it.
//! * Not ported: `bUseSubsurfaceTransmission` for subsurface GBuffer pixels (the forward port has no GBuffer before its base pass; the per-fragment trace this
//!   replaces did not have it either). Debug only: `FN_DEBUG_DF_FULLRES=1` = `r.DFFullResolution 1` (factor 1, no bilateral upsample).
use bytemuck::{Pod, Zeroable};

/// `GAODownsampleFactor` (DistanceFieldAmbientOcclusion.h:25).
pub const DOWNSAMPLE_FACTOR: u32 = 2;
/// Format of `RayTracedShadows` (`PF_G16R16F`).
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// `GetDFShadowDownsampleFactor()`: 2, or 1 with the debug-only `FN_DEBUG_DF_FULLRES=1` (`r.DFFullResolution 1`).
pub fn downsample_factor() -> u32 {
    static FULL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *FULL.get_or_init(|| fn_live::var("FN_DEBUG_DF_FULLRES").map_or(false, |v| v == "1")) { 1 } else { DOWNSAMPLE_FACTOR }
}
/// `GetBufferSizeForDFShadows()` for a `w` x `h` view: `FIntPoint::DivideAndRoundDown(BufferSize, factor)`.
pub fn buffer_size(w: u32, h: u32, factor: u32) -> [u32; 2] { [(w / factor).max(1), (h / factor).max(1)] }

/// `struct DfsU` of distance_field_shadow.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct DfsUniform {
    pub view_inverse: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub proj: [f32; 4],
    pub size: [f32; 4],
    pub misc: [f32; 4],
}

/// The pass: pipeline, its own bind group (trace inputs + scene depth), the `RayTracedShadows` target and a 1x1 stand-in for frames without the pass.
pub(crate) struct DfShadowPass {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    pub(crate) uniform: wgpu::Buffer,
    target: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
    dummy_view: wgpu::TextureView,
    bind_group: Option<wgpu::BindGroup>,
    bg_key: Option<((u64, u64, u64), [u32; 2])>,
    /// The pass records its draw this frame.
    pub(crate) active: bool,
}

impl DfShadowPass {
    pub fn new(device: &wgpu::Device) -> DfShadowPass {
        let f = wgpu::ShaderStages::FRAGMENT;
        let e = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: f, ty, count: None };
        let buf = |ty: wgpu::BufferBindingType| wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("df shadow pass bgl"), entries: &[
            e(5, buf(wgpu::BufferBindingType::Uniform)),
            e(6, buf(wgpu::BufferBindingType::Storage { read_only: true })),
            e(7, buf(wgpu::BufferBindingType::Storage { read_only: true })),
            e(8, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
            e(9, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            e(17, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            e(18, buf(wgpu::BufferBindingType::Uniform)),
        ] });
        let src = [fn_live::wgsl!("../shaders/common.wgsl"), fn_live::wgsl!("../shaders/lit_common.wgsl"), fn_live::wgsl!("../shaders/distance_field_shadow.wgsl")].join("\n");
        let sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("df shadow pass"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("df shadow pass"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("df shadow pass"), layout: Some(&pl),
            vertex: wgpu::VertexState { module: &sm, entry_point: Some("vs_dfs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &sm, entry_point: Some("fs_dfs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("df shadow pass uniform"), size: std::mem::size_of::<DfsUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let dummy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RayTracedShadows (none)"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: FORMAT, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        DfShadowPass { pipeline, bgl, uniform, target: None, dummy_view: dummy.create_view(&Default::default()), bind_group: None, bg_key: None, active: false }
    }

    /// The view the globals bind group samples (binding 16): this frame's target, or the 1x1 stand-in before the first one exists.
    pub fn view(&self) -> &wgpu::TextureView { self.target.as_ref().map_or(&self.dummy_view, |t| &t.1) }

    /// (Re)creates the target for `size`; true when the view changed (the globals bind group must be rebuilt).
    pub fn ensure_target(&mut self, device: &wgpu::Device, size: [u32; 2]) -> bool {
        if matches!(&self.target, Some((_, _, s)) if *s == size) { return false; }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RayTracedShadows (PF_G16R16F)"), size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        self.target = Some((tex, view, size));
        self.bind_group = None;
        true
    }

    /// The pass's bind group over the current trace inputs and scene depth; rebuilt when `key` (resource generations) changes.
    #[allow(clippy::too_many_arguments)]
    pub fn ensure_bind_group(&mut self, device: &wgpu::Device, key: (u64, u64, u64), dfu: &wgpu::Buffer, objects: &wgpu::Buffer, tiles: &wgpu::Buffer, atlas: &wgpu::TextureView, sampler: &wgpu::Sampler, depth: &wgpu::TextureView) {
        let size = self.target.as_ref().map_or([0, 0], |t| t.2);
        let k = (key, size);
        if self.bind_group.is_some() && self.bg_key == Some(k) { return; }
        use wgpu::BindingResource as R;
        fn e(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> { wgpu::BindGroupEntry { binding, resource } }
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("df shadow pass bg"), layout: &self.bgl, entries: &[
            e(5, dfu.as_entire_binding()), e(6, objects.as_entire_binding()), e(7, tiles.as_entire_binding()), e(8, R::TextureView(atlas)), e(9, R::Sampler(sampler)),
            e(17, R::TextureView(depth)), e(18, self.uniform.as_entire_binding()),
        ] }));
        self.bg_key = Some(k);
    }

    /// Records the pass (one full-screen triangle over the target) when it is active this frame.
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>) {
        if !self.active { return; }
        let (Some((_, view, _)), Some(bg)) = (self.target.as_ref(), self.bind_group.as_ref()) else { return };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("df shadow (RayTraceShadows)"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: timestamps, occlusion_query_set: None, multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buffer_size_rounds_down_like_get_buffer_size_for_df_shadows() {
        // the user's maximised client: 2544 x 1353 -> 1272 x 676 (DivideAndRoundDown), the reference views 1600 x 900 -> 800 x 450
        assert_eq!(buffer_size(2544, 1353, DOWNSAMPLE_FACTOR), [1272, 676]);
        assert_eq!(buffer_size(1600, 900, DOWNSAMPLE_FACTOR), [800, 450]);
        assert_eq!(buffer_size(1, 1, DOWNSAMPLE_FACTOR), [1, 1]);
        assert_eq!(buffer_size(2544, 1353, 1), [2544, 1353]);
    }
    #[test]
    fn uniform_is_a_whole_number_of_vec4s() { assert_eq!(std::mem::size_of::<DfsUniform>(), 64 + 4 * 16); }
}
