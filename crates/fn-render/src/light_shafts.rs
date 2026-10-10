//! LIGHT SHAFT HOOK: light-shaft bloom of the Athena sun (UE 4.16 `FDeferredShadingSceneRenderer::RenderLightShaftBloom`,
//! `Engine/Source/Runtime/Renderer/Private/LightShaftRendering.cpp` + `Engine/Shaders/LightShaftShader.usf`; shaders/light_shafts.wgsl).
//!
//! # 1.8 data (read with fn-uasset from the paks / the 1.8 exe)
//! * TODM_A `DayNightDirectionalLightComponent` serializes `bEnableLightShaftBloom = true`, `BloomScale = 0.1` and `LightShaftOverrideDirection =
//!   (0.101203, -0.344474, -0.933325)`; TODM_A_Child_Athena's component overrides none of these. Not serialized, so the exe class constructors apply:
//!   `BloomThreshold = 0` and `BloomScale` ctor 0.2 (`ULightComponent` ctor 0x1420a4c90: 8-byte store 0x3e4ccccd at +0x324/+0x328), `BloomTint = FColor(255,255,255,255)`
//!   (+0x32c from 0x145198860 = 0xffffffff), `bEnableLightShaftOcclusion = false`, `OcclusionMaskDarkness = 0.05`, `OcclusionDepthRange = 100000` cm
//!   (`UDirectionalLightComponent` ctor 0x1420a4750: +0x3c0 bit 0 cleared, +0x3c4 0x3d4ccccd, +0x3c8 0x47c35000). Same values as the UE 4.16 ctors.
//!   `BloomMaxBrightness` does not exist in this engine (not in the 1.8 SDK `ULightComponent` layout, not in UE 4.16), so it is not applied.
//! * The light-shaft direction is NOT the light's: `AFortTimeOfDayManager::PositionTheSunAndMoonModels` (0x140641e30) calls
//!   `SetLightShaftOverrideDirection(-SunlightDirectionalVector)` (the constant at 0x143feb1ec is -1.0) every update, `SunlightDirectionalVector` being the
//!   direction towards the sun/moon MODEL. `GetLightPositionForLightShafts` = `ViewOrigin - OverrideDirection * WORLD_MAX` therefore lies towards the sun
//!   model = [`crate::time_of_day::sun_direction`] (the shared function; the light itself uses `lighting::sun_direction`). The serialized override is replaced at runtime.
//! * `r.LightShaftQuality` = 1 only at PostProcessQuality@3 (Epic) in the cooked scalability ini; `r.LightShaftDownSampleFactor` 2, `r.LightShaftBlurPasses` 3,
//!   `r.LightShaftFirstPassDistance` 0.1, `r.LightShaftNumSamples` 12 are the 4.16 defaults (no ini override). `r.LightShaftRenderToSeparateTranslucency` = 1
//!   (RendererSettings): the shafts are added to the separate-translucency buffer, which the recombine adds as `scene * sep.a + sep.rgb`; since the blend is
//!   additive on RGB only the result equals adding them to the scene colour AFTER the recombine, which is what the renderer does (renderer.rs). The
//!   downsample reads the scene colour before the recombine (UE: the separate translucency lives in its own buffer at that point).
//!
//! # Temporal AA (LIGHT SHAFT TAA, 2026-10-05)
//! * `ApplyTemporalAA` (LightShaftRendering.cpp:562-626) runs between the downsample and the radial blur when the view uses `AAM_TemporalAA` (the port's
//!   High / Epic AA, taa.rs): with a history (`LightShaftBloomHistoryRTs[light]`, kept in the view state) and no camera cut, `LightShaftTemporalAAPS`
//!   (shaders/light_shaft_temporal.wgsl) blends the new source into it and the result becomes the history AND the first blur pass's input; without one the
//!   source itself becomes the history (no shader). Other AA methods: pass-through, the history is left as it is.
//!
//! # Not reproduced
//! * Nothing of the buffer setup any more: the filter buffers are R11G11B10F like UE's `PF_FloatRGB` when the device can render to it ([`filter_format`]).
use glam::{Mat4, Vec3, Vec4};

/// `WORLD_MAX` (EngineDefines.h) in cm. The projected blur origin does not depend on it (the view translation cancels), kept for the UE formula.
pub const WORLD_MAX_CM: f32 = 2_097_152.0;
/// `r.LightShaftDownSampleFactor` default.
pub const DOWNSAMPLE_FACTOR: u32 = 2;
/// `r.LightShaftBlurPasses` default.
pub const BLUR_PASSES: u32 = 3;
/// `r.LightShaftFirstPassDistance` default.
pub const FIRST_PASS_DISTANCE: f32 = 0.1;
/// `r.LightShaftNumSamples` default (compiled into the blur shader as NUM_SAMPLES).
pub const NUM_SAMPLES: u32 = 12;
/// Sun light (TODM_A DayNightDirectionalLightComponent) light-shaft properties, see the module docs.
pub const BLOOM_SCALE: f32 = 0.1;
pub const BLOOM_THRESHOLD: f32 = 0.0;
pub const BLOOM_TINT_SRGB: [u8; 3] = [255, 255, 255];
pub const OCCLUSION_DEPTH_RANGE_CM: f32 = 100_000.0;
pub const OCCLUSION_MASK_DARKNESS: f32 = 0.05;
pub const ENABLE_LIGHT_SHAFT_BLOOM: bool = true;
pub const ENABLE_LIGHT_SHAFT_OCCLUSION: bool = false;

/// SCENE COLOR FORMAT (2026-10-05): `AllocateOrReuseLightShaftRenderTarget` (LightShaftRendering.cpp:470) allocates every light-shaft buffer (LightShafts0/1 and the
/// TAA history) as `PF_FloatRGB` = `DXGI_FORMAT_R11G11B10_FLOAT` on D3D11 (D3D11Device.cpp:147) at every quality level: the same format here when the device can
/// render to it (`RG11B10UFLOAT_RENDERABLE`, requested by gpu.rs), else Rgba16Float. Alpha is not stored (reads 1), as in UE.
pub fn filter_format(device: &wgpu::Device) -> wgpu::TextureFormat {
    if device.features().contains(wgpu::Features::RG11B10UFLOAT_RENDERABLE) { wgpu::TextureFormat::Rg11b10Ufloat } else { wgpu::TextureFormat::Rgba16Float }
}

/// GPU block (struct LsParams in light_shafts.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable, PartialEq)]
pub struct LsParams {
    pub blur_origin: [f32; 4],
    pub aspect: [f32; 4],
    pub uv_min_max: [f32; 4],
    pub params: [f32; 4],
    pub tint_threshold: [f32; 4],
    pub depth: [f32; 4],
}

fn srgb_to_linear(v: u8) -> f32 { let c = v as f32 / 255.; if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) } }

/// `FLightShaftPixelShaderParameters::SetParameters` for a directional light whose shaft origin lies towards `to_light` (renderer world, unit) and a
/// view of `w` x `h` pixels filling the scene buffer. `view_proj` = projection * view of the main pass. `None` = `ShouldRenderLightShaftsForLight`
/// is false (origin behind the view) or the view is too small for a downsampled rect.
pub fn frame_params(view_proj: Mat4, to_light: Vec3, w: u32, h: u32, near_m: f32, far_m: f32) -> Option<LsParams> {
    let f = DOWNSAMPLE_FACTOR;
    let (dsx, dsy) = (w / f, h / f); // DownSampledViewSize (FloorToInt(ViewRect / factor))
    let (fx, fy) = ((w / f).max(1) as f32, (h / f).max(1) as f32); // FilterBufferSize = BufferSize / factor (AllocateOrReuse: Max(.., 1))
    if dsx == 0 || dsy == 0 { return None; }
    // ViewOrigin - Direction * WORLD_MAX with Direction = -to_light; View.WorldToScreen: the view translation cancels after the divide by W
    let p = view_proj * Vec4::new(to_light.x, to_light.y, to_light.z, 0.) * (WORLD_MAX_CM * 0.01);
    if !(p.w > 0.) { return None; }
    let ratio = (dsx as f32 / fx, dsy as f32 / fy);
    let aspect = [ratio.0, fx * ratio.1 / fy, 1. / ratio.0, fy / (fx * ratio.1)];
    // ScreenSpaceBlurOrigin (GProjectionSignY > 0: D3D), / BufferSize, * InvAspect
    let sx = 0.5 + p.x * 0.5 / p.w;
    let sy = 0.5 - p.y * 0.5 / p.w;
    let tint = BLOOM_TINT_SRGB.map(srgb_to_linear); // FLinearColor(FColor): sRGB table
    Some(LsParams {
        blur_origin: [sx * aspect[2], sy * aspect[3], 0., 0.],
        aspect,
        uv_min_max: [0., 0., (dsx as f32 - 1.) / fx, (dsy as f32 - 1.) / fy],
        params: [1. / OCCLUSION_DEPTH_RANGE_CM, BLOOM_SCALE, 1., OCCLUSION_MASK_DARKNESS],
        tint_threshold: [tint[0], tint[1], tint[2], BLOOM_THRESHOLD],
        depth: [near_m, far_m, w as f32, h as f32],
    })
}

/// `RadialBlurParameters` of blur pass `i`: (NumSamples, FirstPassDistance, PassIndex).
pub fn radial_params(i: u32) -> [f32; 4] { [NUM_SAMPLES as f32, FIRST_PASS_DISTANCE, i as f32, 0.] }
/// Fraction of the distance to the origin one blur pass spans: `min(FirstPassDistance * pow(0.4 * NUM_SAMPLES, PassIndex), 1)`.
pub fn pass_blur_fraction(i: u32) -> f32 { (FIRST_PASS_DISTANCE * (0.4 * NUM_SAMPLES as f32).powi(i as i32)).min(1.) }

struct Targets { key: (u32, u32, u64), bg_down: wgpu::BindGroup, bg_blur: Vec<wgpu::BindGroup>, bg_apply: wgpu::BindGroup, views: [wgpu::TextureView; 2],
    /// LIGHT SHAFT TAA: `LSBloomHistory` ping-pong (written in frame parity p = history[p]), the TAA bind group writing history[p] (reads LightShafts0 + history[1 - p])
    /// and the first blur pass reading history[p]
    history: [wgpu::TextureView; 2], bg_taa: [wgpu::BindGroup; 2], bg_blur0_hist: [wgpu::BindGroup; 2] }

/// LIGHT SHAFT TAA: what [`LightShafts::prepare`] needs from a temporal-AA view (`None` = AAM is not temporal AA: pass-through).
pub struct LsTaaFrame<'a> {
    /// The jittered render camera of this frame (view size).
    pub cam: &'a crate::camera::CameraParams,
    /// `View.TemporalJitterPixels` (taa.rs).
    pub jitter_px: [f32; 2],
}

/// GPU block `LsTaaU` of light_shaft_temporal.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LsTaaU { clip_to_prev: [[f32; 4]; 4], weights0: [f32; 4], weights1: [f32; 4], weights2: [f32; 4], viewport: [f32; 4] }

/// The light-shaft bloom passes. `prepare` per frame, then `encode_downsample_blur` (before the separate-translucency recombine) and `encode_apply`.
pub struct LightShafts {
    bgl: wgpu::BindGroupLayout,
    down: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    apply: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    radial: Vec<wgpu::Buffer>,
    sampler: wgpu::Sampler,
    /// SCENE COLOR FORMAT: the filter buffers' format ([`filter_format`])
    fmt: wgpu::TextureFormat,
    targets: Option<Targets>,
    active: bool,
    // LIGHT SHAFT TAA (see the module docs)
    taa_pipe: wgpu::RenderPipeline,
    taa_bgl: wgpu::BindGroupLayout,
    taa_buf: wgpu::Buffer,
    /// this frame runs the TAA branch (`AAM_TemporalAA`)
    taa_on: bool,
    /// a history exists (written by an earlier temporal-AA frame, targets unchanged)
    has_history: bool,
    /// the history parity written this frame
    parity: usize,
    /// previous frame's unjittered camera (ClipToPrevClip)
    prev_cam: Option<crate::camera::CameraParams>,
}

impl LightShafts {
    pub fn new(device: &wgpu::Device, hdr_format: wgpu::TextureFormat) -> LightShafts {
        let fmt = filter_format(device);
        let f = wgpu::ShaderStages::FRAGMENT;
        let u = |b| wgpu::BindGroupLayoutEntry { binding: b, visibility: f, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("light shafts bgl"), entries: &[
            u(0),
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: f, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            u(4),
        ] });
        let sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("light shafts"), source: wgpu::ShaderSource::Wgsl(fn_live::wgsl!("../shaders/light_shafts.wgsl").into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("light shafts layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipe = |label: &str, fs: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>, mask: wgpu::ColorWrites| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &sm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &sm, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: mask })] }),
            multiview_mask: None, cache: None,
        });
        // ApplyLightShaftBloom: TStaticBlendState<CW_RGB, BO_Add, BF_One, BF_One>
        let add = wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent::REPLACE };
        let down = pipe("light shafts downsample", "fs_downsample", fmt, None, wgpu::ColorWrites::ALL);
        let blur = pipe("light shafts radial blur", "fs_blur", fmt, None, wgpu::ColorWrites::ALL);
        let apply = pipe("light shafts apply", "fs_apply", hdr_format, Some(add), wgpu::ColorWrites::COLOR);
        let ub = |label: &str, size: u64| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let params = ub("light shaft params", std::mem::size_of::<LsParams>() as u64);
        let radial = (0..BLUR_PASSES).map(|_| ub("light shaft radial", 16)).collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("light shaft bilinear clamp"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        // LIGHT SHAFT TAA: LightShaftTemporalAAPS (light_shaft_temporal.wgsl): 0 params, 1 source (point loads), 2 history (bilinear), 3 sampler, 4 scene depth, 5 exposure
        let taa_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("light shaft taa bgl"), entries: &[
            u(0),
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: f, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            wgpu::BindGroupLayoutEntry { binding: 4, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 5, visibility: f, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ] });
        let taa_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("light shaft taa"), source: wgpu::ShaderSource::Wgsl(fn_live::wgsl!("../shaders/light_shaft_temporal.wgsl").into()) });
        let taa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("light shaft taa layout"), bind_group_layouts: &[Some(&taa_bgl)], immediate_size: 0 });
        // FRCPassPostProcessLightShaftTemporalAA::Process: TStaticBlendState<> (no blending), no depth test
        let taa_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("light shaft taa"), layout: Some(&taa_layout),
            vertex: wgpu::VertexState { module: &taa_sm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &taa_sm, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: fmt, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        let taa_buf = ub("light shaft taa params", std::mem::size_of::<LsTaaU>() as u64);
        LightShafts { bgl, down, blur, apply, params, radial, sampler, fmt, targets: None, active: false, taa_pipe, taa_bgl, taa_buf, taa_on: false, has_history: false, parity: 0, prev_cam: None }
    }

    /// This frame's parameters; `enabled` = r.LightShaftQuality && show flags. (Re)creates the filter buffers when the scene targets change (`gen`).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enabled: bool, view_proj: Mat4, to_light: Vec3, near_m: f32, far_m: f32,
                   hdr: &wgpu::TextureView, depth: &wgpu::TextureView, w: u32, h: u32, gen: u64, taa: Option<LsTaaFrame>, exposure_buf: &wgpu::Buffer) {
        let p = if enabled && ENABLE_LIGHT_SHAFT_BLOOM { frame_params(view_proj, to_light, w, h, near_m, far_m) } else { None };
        self.active = p.is_some();
        self.taa_on = false;
        let Some(p) = p else { return };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        for (i, b) in self.radial.iter().enumerate() { queue.write_buffer(b, 0, bytemuck::cast_slice(&radial_params(i as u32))); }
        if self.targets.as_ref().map(|t| t.key) != Some((w, h, gen)) { self.create_targets(device, hdr, depth, w, h, gen, exposure_buf); }
        // LIGHT SHAFT TAA (ApplyTemporalAA): only for AAM_TemporalAA views; this frame writes history[parity]
        if let Some(f) = taa {
            self.taa_on = true;
            self.parity ^= 1;
            let (dw, dh) = ((w / DOWNSAMPLE_FACTOR).max(1), (h / DOWNSAMPLE_FACTOR).max(1));
            // View.ClipToPrevClip with the no-AA matrices (as reflection.rs / taa.rs); bPrevTransformsReset (first frame, large move) = the current matrices
            let mut prev = match &self.prev_cam { Some(pc) if !crate::taa::is_large_camera_movement(f.cam, pc) => *pc, _ => *f.cam };
            prev.width = f.cam.width; prev.height = f.cam.height;
            let vp = f.cam.proj_matrix_no_aa() * f.cam.view_matrix();
            let clip_to_prev = (prev.proj_matrix_no_aa() * prev.view_matrix()) * vp.inverse();
            let mut c = *f.cam; c.jitter = [0.; 2];
            self.prev_cam = Some(c);
            // SampleWeights of View.TemporalJitterPixels (PostProcessTemporalAA.cpp:130-182; the same pixel jitter on the half-res source, as UE)
            let wts = crate::reflection::sample_weights(f.jitter_px);
            queue.write_buffer(&self.taa_buf, 0, bytemuck::bytes_of(&LsTaaU {
                clip_to_prev: clip_to_prev.to_cols_array_2d(),
                weights0: [wts[0], wts[1], wts[2], wts[3]], weights1: [wts[4], wts[5], wts[6], wts[7]], weights2: [wts[8], 0., 0., 0.],
                viewport: [dw as f32, dh as f32, 1. / dw as f32, 1. / dh as f32],
            }));
        }
    }
    /// LIGHT SHAFT TAA: forget the history (camera cut / `Renderer::reset_exposure`).
    pub fn reset(&mut self) { self.has_history = false; self.prev_cam = None; }
    /// The TAA shader runs this frame (temporal AA view with a history); otherwise the source becomes the history.
    fn taa_shader(&self) -> bool { self.taa_on && self.has_history }
    /// Call once the frame's passes were recorded: a temporal-AA frame leaves a history for the next one.
    pub fn end_frame(&mut self) { if self.active && self.taa_on && self.targets.is_some() { self.has_history = true; } }

    #[allow(clippy::too_many_arguments)]
    fn create_targets(&mut self, device: &wgpu::Device, hdr: &wgpu::TextureView, depth: &wgpu::TextureView, w: u32, h: u32, gen: u64, exposure_buf: &wgpu::Buffer) {
        let size = wgpu::Extent3d { width: (w / DOWNSAMPLE_FACTOR).max(1), height: (h / DOWNSAMPLE_FACTOR).max(1), depth_or_array_layers: 1 };
        let mk = |label: &str| device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: self.fmt, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }).create_view(&Default::default());
        let views = [mk("LightShafts0"), mk("LightShafts1")];
        let history = [mk("LSBloomHistory a"), mk("LSBloomHistory b")];
        self.has_history = false;
        let bg = |src: &wgpu::TextureView, radial: &wgpu::Buffer| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("light shafts bg"), layout: &self.bgl, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            wgpu::BindGroupEntry { binding: 4, resource: radial.as_entire_binding() },
        ] });
        // downsample -> 0; blur i reads (i even ? 0 : 1), writes the other; the result of the last pass is read by apply
        let bg_down = bg(hdr, &self.radial[0]);
        let bg_blur = (0..BLUR_PASSES as usize).map(|i| bg(&views[i % 2], &self.radial[i])).collect();
        let bg_apply = bg(&views[BLUR_PASSES as usize % 2], &self.radial[0]);
        // LIGHT SHAFT TAA: the first blur pass reads the history written this frame (HistoryOutput = FirstPassSource of ApplyRadialBlurPasses)
        let bg_blur0_hist = [bg(&history[0], &self.radial[0]), bg(&history[1], &self.radial[0])];
        let taa_bg = |p: usize| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("light shaft taa bg"), layout: &self.taa_bgl, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: self.taa_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[0]) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&history[1 - p]) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(depth) },
            wgpu::BindGroupEntry { binding: 5, resource: exposure_buf.as_entire_binding() },
        ] });
        let bg_taa = [taa_bg(0), taa_bg(1)];
        self.targets = Some(Targets { key: (w, h, gen), bg_down, bg_blur, bg_apply, views, history, bg_taa, bg_blur0_hist });
    }
    pub fn active(&self) -> bool { self.active && self.targets.is_some() }

    fn pass<'e>(enc: &'e mut wgpu::CommandEncoder, view: &wgpu::TextureView, label: &str) -> wgpu::RenderPass<'e> {
        enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            // AllocateOrReuseLightShaftRenderTarget clears to black; every pass then covers the whole downsampled rect
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        })
    }
    /// DownsamplePass<false> + ApplyTemporalAA + ApplyRadialBlurPasses (LightShaftRendering.cpp:1034-1062).
    pub fn encode_downsample_blur(&self, enc: &mut wgpu::CommandEncoder) {
        let Some(t) = self.targets.as_ref().filter(|_| self.active) else { return };
        let cur = self.parity;
        // ApplyTemporalAA without a history: the source itself becomes the history (downsampled straight into it); with one: LightShafts0 -> TAA -> history
        let down_target = if self.taa_on && !self.taa_shader() { &t.history[cur] } else { &t.views[0] };
        {
            let mut p = Self::pass(enc, down_target, "light shafts downsample");
            p.set_pipeline(&self.down);
            p.set_bind_group(0, &t.bg_down, &[]);
            p.draw(0..3, 0..1);
        }
        if self.taa_shader() {
            let mut p = Self::pass(enc, &t.history[cur], "light shaft temporal aa");
            p.set_pipeline(&self.taa_pipe);
            p.set_bind_group(0, &t.bg_taa[cur], &[]);
            p.draw(0..3, 0..1);
        }
        for i in 0..BLUR_PASSES as usize {
            let mut p = Self::pass(enc, &t.views[(i + 1) % 2], "light shafts radial blur");
            p.set_pipeline(&self.blur);
            p.set_bind_group(0, if i == 0 && self.taa_on { &t.bg_blur0_hist[cur] } else { &t.bg_blur[i] }, &[]);
            p.draw(0..3, 0..1);
        }
    }
    /// ApplyLightShaftBloom: full-res additive into `target` (the HDR scene colour), colour channels only.
    pub fn encode_apply(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(t) = self.targets.as_ref().filter(|_| self.active) else { return };
        let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("light shafts apply"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
        p.set_pipeline(&self.apply);
        p.set_bind_group(0, &t.bg_apply, &[]);
        p.draw(0..3, 0..1);
    }
}

/// LIGHT SHAFT HOOK: depth mask for the SEPARATE-TRANSLUCENCY sun (sky.rs `draw_sun_separate`). In UE the sun disc (M_Sun_Arid: BLEND_Translucent,
/// `bEnableSeparateTranslucency` not serialized -> UMaterial ctor true, exe 0x14222d130 sets bit 0 of +0x7e0; M_AridSky / M_AridClouds serialize it
/// false) is drawn into the separate-translucency buffer, so it is NOT in the scene colour the light-shaft downsample reads, and it composites over
/// the standard-translucent clouds. The port draws it after the shaft blur; this mask stands in for UE's depth test of the 36 km sun against the scene
/// depth: 1 where the scene depth is clear (sky), 0 elsewhere (SSR BACKDROP: the scene depth holds the backdrop too, backdrop.rs encode_depth_merge).
pub struct SunMask {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    target: Option<((u32, u32, u64), wgpu::TextureView, wgpu::BindGroup)>,
}
const SUN_MASK_WGSL: &str = "
@group(0) @binding(0) var src_depth: texture_depth_2d;
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i32(i & 1u) * 4 - 1), f32(i32((i >> 1u) & 1u) * 4 - 1), 0.0, 1.0);
}
@fragment fn fs_main(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    return select(1.0, 0.0, textureLoad(src_depth, vec2<i32>(p.xy), 0) < 1.0);
}";
impl SunMask {
    pub fn new(device: &wgpu::Device) -> SunMask {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("sun mask bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
        ] });
        let sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sun mask"), source: wgpu::ShaderSource::Wgsl(SUN_MASK_WGSL.into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sun mask layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sun mask"), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &sm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            // min(mask, coverage): Less + write keeps a 0 once written
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &sm, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[] }),
            multiview_mask: None, cache: None,
        });
        SunMask { pipeline, bgl, target: None }
    }
    pub fn prepare(&mut self, device: &wgpu::Device, scene_depth: &wgpu::TextureView, w: u32, h: u32, gen: u64) {
        if self.target.as_ref().map(|t| t.0) == Some((w, h, gen)) { return; }
        let view = device.create_texture(&wgpu::TextureDescriptor { label: Some("sun mask depth"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Depth32Float, usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] }).create_view(&Default::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sun mask bg"), layout: &self.bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(scene_depth) }] });
        self.target = Some(((w, h, gen), view, bg));
    }
    /// Folds the current scene depth into the mask (`clear` = start a new frame's mask at 1).
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, clear: bool) {
        let Some((_, view, bg)) = &self.target else { return };
        let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sun mask"), color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view, depth_ops: Some(wgpu::Operations { load: if clear { wgpu::LoadOp::Clear(1.0) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
        p.set_pipeline(&self.pipeline);
        p.set_bind_group(0, bg, &[]);
        p.draw(0..3, 0..1);
    }
    pub fn view(&self) -> Option<&wgpu::TextureView> { self.target.as_ref().map(|t| &t.1) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::CameraParams;
    #[test]
    fn origin_projects_to_the_screen_centre_when_looking_at_the_light() {
        let c = CameraParams::from_yaw_pitch(Vec3::new(3., 2., 1.), 30., 10., 1280, 720);
        let p = frame_params(c.proj_matrix() * c.view_matrix(), c.forward, 1280, 720, c.near, c.far).unwrap();
        // centre of the screen: (0.5, 0.5) in buffer UV, times InvAspect (1, H/W)
        assert!((p.blur_origin[0] - 0.5).abs() < 1e-4 && (p.blur_origin[1] - 0.5 * 360. / 640.).abs() < 1e-4, "{p:?}");
        assert_eq!(p.aspect, [1., 640. / 360., 1., 360. / 640.]);
        assert_eq!(p.uv_min_max, [0., 0., 639. / 640., 359. / 360.]);
        assert_eq!(p.params, [1e-5, 0.1, 1., 0.05]);
        assert_eq!(p.tint_threshold, [1., 1., 1., 0.]);
        // behind the camera: ShouldRenderLightShaftsForLight is false
        assert!(frame_params(c.proj_matrix() * c.view_matrix(), -c.forward, 1280, 720, c.near, c.far).is_none());
        // up-right of the view centre -> uv x > 0.5, y < 0.5 (texture v points down)
        let d = (c.forward + c.up * 0.2 + c.forward.cross(c.up) * 0.2).normalize();
        let q = frame_params(c.proj_matrix() * c.view_matrix(), d, 1280, 720, c.near, c.far).unwrap();
        assert!(q.blur_origin[0] > 0.5 && q.blur_origin[1] < 0.5 * 360. / 640.);
    }
    #[test]
    fn blur_passes_grow_by_4_8_and_stop_at_the_origin() {
        assert_eq!(pass_blur_fraction(0), 0.1);
        assert!((pass_blur_fraction(1) - 0.48).abs() < 1e-6);
        assert_eq!(pass_blur_fraction(2), 1.0);
        assert_eq!(radial_params(2), [12., 0.1, 2., 0.]);
    }
    /// LIGHT SHAFT TAA: LightShaftTemporalAAPS defines (AA_LERP 64, no dynamic velocity, HdrWeight4 tone, AABB clamp)
    #[test]
    fn temporal_shader_keeps_the_light_shaft_defines() {
        let s = fn_live::wgsl!("../shaders/light_shaft_temporal.wgsl");
        for n in ["1.0 / 64.0", "fn hdr_weight4", "history_clamp(", "1.0 / (1.0 + luma_contrast * 32.0)"] { assert!(s.contains(n), "{n}"); }
        assert!(!s.contains("t_velocity"), "AA_DYNAMIC 0: camera motion only");
    }
    #[test]
    fn wgsl_keeps_the_usf_constants() {
        let s = fn_live::wgsl!("../shaders/light_shafts.wgsl");
        for n in ["vec3<f32>(0.3, 0.59, 0.11)", "6.10352e-5", "* 2.0", "0.4 * f32(NUM_SAMPLES)", "NUM_SAMPLES: i32 = 12", "* 8.0"] { assert!(s.contains(n), "{n}"); }
    }
}
