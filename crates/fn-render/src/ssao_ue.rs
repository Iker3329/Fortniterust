//! UE 4.16 SCREEN-SPACE AMBIENT OCCLUSION (CompositionLighting.cpp `AddPostProcessingAmbientOcclusion`, PostProcessAmbientOcclusion.cpp / .usf),
//! replacing the port's earlier 16-sample hemisphere AO. Shaders: `shaders/ssao_ue.wgsl` (checked against the 1.8 global shader cache: setup
//! #496380, half-res AO #586389, full-res AO #600241, see the shader header).
//!
//! # Pass graph (CompositionLighting.cpp:170-260), `Levels` = [`pass_count`]
//! * Levels >= 2: `AmbientOcclusionSetup` mip 1 (half res, PF_FloatRGBA: world normal * 0.5 + 0.5, depth / 131072) from the scene depth + GBuffer;
//! * Levels >= 3: setup mip 2 (quarter res) and `AmbientOcclusion` mip 2 (setup as input, no upsample);
//! * Levels >= 2: `AmbientOcclusion` mip 1 (setup as input, upsampling mip 2 when it exists), PF_B8G8R8A8;
//! * always: `AmbientOcclusion` mip 0 at full resolution into `ScreenSpaceAO` (PF_G8), upsampling mip 1 when it exists, with the distance fade,
//!   `AmbientOcclusionPower` and `AmbientOcclusionIntensity` applied. The reflection / AO apply pass (ssao_apply.wgsl) reads it as
//!   `ScreenSpaceData.AmbientOcclusion` (sky light diffuse via min(GBufferAO, SSAO), specular occlusion via GBufferAO x SSAO).
//!
//! # Fortnite values (evidence)
//! * Post-process (`FPostProcessSettings`, exe ctor = Scene.h:1300-1316 defaults, TOD phase components override Intensity / Radius / Bias / Fade* /
//!   MipThreshold; Power / MipBlend / MipScale carry bOverride without a tagged value = the ctor value; Quality is not overridden):
//!   Radius 160 cm, Bias 2.4, Power 2, FadeDistance 550000, FadeRadius 200000, MipBlend 0.6, MipScale 1.7, MipThreshold 0.009, Quality 50,
//!   RadiusInWS false, Distance_DEPRECATED 80. Intensity per phase ([`crate::tonemap::PostSettings::ao_intensity`], stacked like UE).
//! * StaticFraction: `r.DefaultFeature.AmbientOcclusionStaticFraction=False` (CookedIniVersion.txt L66) forces it to 0 (SceneView.cpp:1590-1598),
//!   so there is no `BasePassAO` pass.
//! * Scalability `PostProcessQuality@0..3` (L5174-5241): `r.AmbientOcclusionMaxQuality` 0 / 60 / 100 / 100, `r.AmbientOcclusionLevels` 0 / -1 / -1 / -1,
//!   `r.AmbientOcclusionRadiusScale` 1.2 / 1.5 / 1.5 / 1.0, `r.AmbientOcclusionMipLevelFactor` 1.0 / 1.0 / 0.6 / 0.4 ([`crate::quality::PostProcessScalability`]).
//!   With Quality 50: shader quality 2 (3 points x 2 steps at full res, 6 points x 2 steps on the setup inputs, normal-aware quad blur) and 2 levels.
//! * `r.AmbientOcclusion.Compute` 0 (pixel shaders), `r.AmbientOcclusion.FadeRadiusScale` 1 (not cooked).
use crate::camera::CameraParams;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

/// The Athena post-process / engine values of the SSAO (module docs).
pub mod ue {
    /// `AmbientOcclusionPower` (FPostProcessSettings ctor, Scene.h:1312).
    pub const POWER: f32 = 2.0;
    /// `AmbientOcclusionBias` of the TOD phase components (cm-ish units, divided by 1000 in the shader parameters).
    pub const BIAS: f32 = 2.4;
    /// `AmbientOcclusionRadius` of the TOD phase components; scaled by `r.AmbientOcclusionRadiusScale`.
    pub const RADIUS: f32 = 160.0;
    /// `AmbientOcclusionRadiusInWS` (ctor false): the radius is in view space, 400 units.
    pub const RADIUS_IN_WS: bool = false;
    pub const FADE_DISTANCE: f32 = 550_000.0;
    pub const FADE_RADIUS: f32 = 200_000.0;
    pub const MIP_BLEND: f32 = 0.6;
    pub const MIP_SCALE: f32 = 1.7;
    pub const MIP_THRESHOLD: f32 = 0.009;
    /// `AmbientOcclusionQuality` (ctor 50, no Athena volume overrides it).
    pub const QUALITY: f32 = 50.0;
    /// `AmbientOcclusionDistance_DEPRECATED` (ctor 80) x `r.AmbientOcclusion.FadeRadiusScale` (1).
    pub const DISTANCE_DEPRECATED: f32 = 80.0;
    /// `AmbientOcclusionStaticFraction` after `r.DefaultFeature.AmbientOcclusionStaticFraction=False`.
    pub const STATIC_FRACTION: f32 = 0.0;
    /// `GSystemTextures.SSAORandomization`: 64 x 64 PF_R8G8 (SystemTextures.cpp:269).
    pub const RANDOM_SIZE: u32 = 64;
    /// `Constant_Float16F_Scale` of the setup's depth channel.
    pub const FLOAT16F_SCALE: f32 = 131_072.0;
}

/// `FSSAOHelper::GetAmbientOcclusionQualityRT`: `r.AmbientOcclusionMaxQuality` (< 0 enforces |value|) clamps the volume's quality.
pub fn quality_percent(max_quality_cvar: f32, pp_quality: f32) -> f32 {
    if max_quality_cvar < 0. { (-max_quality_cvar).clamp(0., 100.) } else { max_quality_cvar.min(pp_quality) }
}
/// `FSSAOHelper::GetAmbientOcclusionShaderLevel` (0..4).
pub fn shader_level(quality_percent: f32) -> u32 {
    (quality_percent > 75.) as u32 + (quality_percent > 55.) as u32 + (quality_percent > 25.) as u32 + (quality_percent > 5.) as u32
}
/// `ComputeAmbientOcclusionPassCount` (CompositionLighting.cpp:128): 0 = off, else 1..3 resolution levels. `enabled` = ShouldRenderScreenSpaceAmbientOcclusion.
pub fn pass_count(enabled: bool, quality_percent: f32, cvar_levels: i32) -> u32 {
    if !enabled { return 0; }
    let mut r = 1 + (quality_percent > 70.) as u32 + (quality_percent > 35.) as u32;
    if cvar_levels >= 0 { r = cvar_levels as u32; }
    r.min(3)
}
/// `ShouldRenderScreenSpaceAmbientOcclusion` (CompositionLighting.cpp:110) for the settings this port has (sky light active, deferred shading).
pub fn should_render(intensity: f32, radius: f32) -> bool { intensity > 0. && radius >= 0.1 }
/// The 16 rotation bases of `SSAORandomization` (SystemTextures.cpp:244-264): `FColor(Quantize8SignedByte(c), Quantize8SignedByte(s))`.
pub fn random_bases() -> [[u8; 2]; 16] {
    let (off2, off3) = (198f32, 23f32);
    let reorder: [i32; 16] = [0, 11, 7, 3, 10, 4, 15, 12, 6, 8, 1, 14, 13, 2, 9, 5];
    let q = |x: f32| ((x * 0.5 + 0.5) * 255.999) as i32 as u8;
    std::array::from_fn(|pos| {
        let w = reorder[pos] as f32;
        let ww = w / 16. * std::f32::consts::PI;
        let lenm = 1. - ((off2 * w * 0.01).sin() * 0.5 + 0.5) * off3 * 0.01;
        [q(ww.cos() * lenm), q(ww.sin() * lenm)]
    })
}
/// The 64 x 64 RG8 texel data (`Index = (x % 4) + (y % 4) * 4`).
pub fn random_texture() -> Vec<u8> {
    let b = random_bases();
    let n = ue::RANDOM_SIZE as usize;
    let mut v = Vec::with_capacity(n * n * 2);
    for y in 0..n { for x in 0..n { v.extend_from_slice(&b[(x % 4) + (y % 4) * 4]); } }
    v
}
/// `TemporalOffset` (PostProcessAmbientOcclusion.cpp:213): `(TemporalAASampleIndex % 8) * (2.48, 7.52) / 64`.
pub fn temporal_offset(taa_sample_index: u32) -> [f32; 2] {
    let i = (taa_sample_index % 8) as f32;
    [i * 2.48 / 64., i * 7.52 / 64.]
}
/// `AORadiusInShader` of a pass whose input is `scale_to_full_res` times smaller than the buffer (FScreenSpaceAOParameters::Set).
pub fn radius_in_shader(radius: f32, scale_to_full_res: u32) -> f32 {
    let r = if ue::RADIUS_IN_WS { radius } else { radius / 400. };
    r * ue::MIP_SCALE.powf((scale_to_full_res as f32).log2()) / 4.
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AoU {
    p: [[f32; 4]; 6],
    hzb_remap: [f32; 4],
    setup: [f32; 4],
    in0: [f32; 4],
    in1: [f32; 4],
    in2: [f32; 4],
    out: [f32; 4],
    buffer: [f32; 4],
    to_view: [[f32; 4]; 3],
    depth: [f32; 4],
}

fn size4(s: (u32, u32)) -> [f32; 4] { [s.0 as f32, s.1 as f32, 1. / s.0.max(1) as f32, 1. / s.1.max(1) as f32] }
fn half(s: (u32, u32)) -> (u32, u32) { (s.0.div_ceil(2).max(1), s.1.div_ceil(2).max(1)) }

/// One frame's SSAO inputs.
pub struct SsaoFrame<'a> {
    /// The (jittered) render camera of the view.
    pub cam: &'a CameraParams,
    pub depth_view: &'a wgpu::TextureView,
    /// The GBuffer world normal (reflection target 2).
    pub gbuf_view: &'a wgpu::TextureView,
    /// `View.HZB` and its mip-0 size.
    pub hzb: (&'a wgpu::TextureView, (u32, u32)),
    /// Generation of the scene targets (depth / GBuffer / HZB re-creation).
    pub gen: u64,
    /// `AmbientOcclusionIntensity` of the stacked TOD post-process.
    pub intensity: f32,
    pub scalability: crate::quality::PostProcessScalability,
    pub taa_sample_index: u32,
}

struct Targets {
    key: (u32, u32, u32, u64),
    setup: Vec<wgpu::TextureView>,   // setup mip 1, mip 2
    ao_low: Vec<wgpu::TextureView>,  // AO mip 1, mip 2
    ao: wgpu::TextureView,           // ScreenSpaceAO (full res)
    passes: Vec<Pass>,
}
struct Pass { label: &'static str, target: usize, kind: PassKind, buf: wgpu::Buffer, bg: wgpu::BindGroup }
#[derive(Clone, Copy)]
enum PassKind { Setup { initial: bool }, Ao { setup_as_input: bool, upsample: bool } }

/// The UE 4.16 SSAO passes (module docs).
pub struct UeSsao {
    device: wgpu::Device,
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    bgl: wgpu::BindGroupLayout,
    pipelines: HashMap<(u8, bool, bool, u32), wgpu::RenderPipeline>,
    random: wgpu::TextureView,
    white: wgpu::TextureView,
    point_clamp: wgpu::Sampler,
    point_wrap: wgpu::Sampler,
    targets: Option<Targets>,
    levels: u32,
    shader_level: u32,
    active: bool,
    gen: u64,
}

pub const SETUP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const AO_LOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

impl UeSsao {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> UeSsao {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ue ssao"), source: wgpu::ShaderSource::Wgsl(fn_live::wgsl!("../shaders/ssao_ue.wgsl").into()) });
        let f = wgpu::ShaderStages::FRAGMENT;
        let tex = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
        let samp = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: f, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ue ssao bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: f, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            tex(2), tex(3), tex(4), tex(5), tex(6), samp(7), samp(8), tex(9),
        ] });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ue ssao layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let mk_tex = |label: &str, format: wgpu::TextureFormat, size: u32, bytes: &[u8], bpp: u32| {
            let t = device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
            queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size * bpp), rows_per_image: Some(size) }, wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 });
            t.create_view(&Default::default())
        };
        let random = mk_tex("ssao randomization", wgpu::TextureFormat::Rg8Unorm, ue::RANDOM_SIZE, &random_texture(), 2);
        let white = mk_tex("ssao white", wgpu::TextureFormat::Rgba8Unorm, 1, &[255u8; 4], 4);
        let point_clamp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("ssao point clamp"), ..Default::default() });
        let point_wrap = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("ssao point wrap"), address_mode_u: wgpu::AddressMode::Repeat, address_mode_v: wgpu::AddressMode::Repeat, ..Default::default() });
        UeSsao { device: device.clone(), module, layout, bgl, pipelines: HashMap::new(), random, white, point_clamp, point_wrap, targets: None, levels: 0, shader_level: 0, active: false, gen: 0 }
    }

    /// SSAO is off this frame (PostProcessQuality Low, or no HZB).
    pub fn disable(&mut self) { self.active = false; }
    /// SSAO ran this frame (`ScreenSpaceAO` is valid).
    pub fn active(&self) -> bool { self.active }
    /// `ScreenSpaceAO` (full res, single channel); white while SSAO is off.
    pub fn ao_view(&self) -> &wgpu::TextureView { self.targets.as_ref().filter(|_| self.active).map_or(&self.white, |t| &t.ao) }
    /// Changes whenever [`Self::ao_view`] is a different texture.
    pub fn ao_key(&self) -> u64 { if self.active { self.gen } else { 0 } }

    fn pipeline_key(kind: PassKind, quality: u32) -> (u8, bool, bool, u32) {
        match kind {
            PassKind::Setup { initial } => (0u8, initial, false, 0u32),
            PassKind::Ao { setup_as_input, upsample } => (1u8, setup_as_input, upsample, quality),
        }
    }
    /// The permutation's pipeline (FPostProcessAmbientOcclusionSetupPS<bInitialPass> / FPostProcessAmbientOcclusionPS<SetupAsInput, Upsample, Quality>),
    /// created on first use (prepare) like UE's shader map lookups.
    pub fn prewarm(&mut self) {
        let q = shader_level(ue::QUALITY);
        for initial in [true, false] { self.ensure_pipeline(PassKind::Setup { initial }, 0); }
        for a in [false, true] { for b in [false, true] { self.ensure_pipeline(PassKind::Ao { setup_as_input: a, upsample: b }, q); } }
    }
    fn ensure_pipeline(&mut self, kind: PassKind, quality: u32) {
        let k = Self::pipeline_key(kind, quality);
        if self.pipelines.contains_key(&k) { return; }
        let format = match kind { PassKind::Setup { .. } => SETUP_FORMAT, PassKind::Ao { setup_as_input, .. } => if setup_as_input { AO_LOW_FORMAT } else { AO_FORMAT } };
        let constants: Vec<(&str, f64)> = match kind {
            PassKind::Setup { initial } => vec![("INITIAL_PASS", initial as u32 as f64)],
            PassKind::Ao { setup_as_input, upsample } => vec![("USE_AO_SETUP_AS_INPUT", setup_as_input as u32 as f64), ("USE_UPSAMPLE", upsample as u32 as f64), ("SHADER_QUALITY", quality as f64)],
        };
        let fs = if matches!(kind, PassKind::Setup { .. }) { "fs_setup" } else { "fs_ao" };
        let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ue ssao"), layout: Some(&self.layout),
            vertex: wgpu::VertexState { module: &self.module, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &self.module, entry_point: Some(fs), compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        self.pipelines.insert(k, p);
    }

    /// Targets, uniforms and bind groups of this frame; returns [`Self::active`].
    pub fn prepare(&mut self, queue: &wgpu::Queue, f: &SsaoFrame) -> bool {
        let s = &f.scalability;
        let radius = ue::RADIUS * s.ao_radius_scale.clamp(0.1, 15.);
        let q = quality_percent(s.ao_max_quality, ue::QUALITY);
        let levels = pass_count(should_render(f.intensity, radius), q, s.ao_levels);
        self.active = levels > 0;
        if !self.active { return false; }
        self.levels = levels;
        self.shader_level = shader_level(q);
        let buffer = (f.cam.width.max(1), f.cam.height.max(1));
        let key = (buffer.0, buffer.1, levels, f.gen);
        if self.targets.as_ref().map(|t| t.key) != Some(key) {
            self.gen = self.gen.wrapping_add(1);
            let mk = |label: &str, size: (u32, u32), format| self.device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] })
                .create_view(&Default::default());
            let s1 = half(buffer);
            let s2 = half(s1);
            let setup: Vec<_> = [(levels >= 2, s1, "ssao setup mip 1"), (levels >= 3, s2, "ssao setup mip 2")].iter().filter(|x| x.0).map(|x| mk(x.2, x.1, SETUP_FORMAT)).collect();
            let ao_low: Vec<_> = [(levels >= 2, s1, "ssao mip 1"), (levels >= 3, s2, "ssao mip 2")].iter().filter(|x| x.0).map(|x| mk(x.2, x.1, AO_LOW_FORMAT)).collect();
            let ao = mk("ScreenSpaceAO", buffer, AO_FORMAT);
            // pass list in UE's execution order; `target` indexes [setup1, setup2, ao1, ao2, ao]
            let mut list: Vec<(&'static str, usize, PassKind)> = Vec::new();
            if levels >= 2 { list.push(("ssao setup mip 1", 0, PassKind::Setup { initial: true })); }
            if levels >= 3 { list.push(("ssao setup mip 2", 1, PassKind::Setup { initial: false })); }
            if levels >= 3 { list.push(("ssao mip 2", 3, PassKind::Ao { setup_as_input: true, upsample: false })); }
            if levels >= 2 { list.push(("ssao mip 1", 2, PassKind::Ao { setup_as_input: true, upsample: levels >= 3 })); }
            list.push(("ssao full res", 4, PassKind::Ao { setup_as_input: false, upsample: levels >= 2 }));
            let white = &self.white;
            let passes = list.iter().map(|&(label, target, kind)| {
                // PostprocessInput0 (setup as input), PostprocessInput1 (the upsample's normal / depth), PostprocessInput2 (the lower AO)
                let (in_setup, in_setup_up, in_lower): (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView) = match (kind, target) {
                    (PassKind::Ao { .. }, 3) => (&setup[1], white, white),                                   // mip 2: setup mip 2
                    (PassKind::Ao { upsample, .. }, 2) => (&setup[0], &setup[0], if upsample { &ao_low[1] } else { white }), // mip 1
                    (PassKind::Ao { upsample, .. }, _) if upsample => (white, &setup[0], &ao_low[0]),      // full res
                    _ => (white, white, white),
                };
                let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("ue ssao params"), size: std::mem::size_of::<AoU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ue ssao bg"), layout: &self.bgl, entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(f.depth_view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(f.gbuf_view) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(in_setup) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(in_lower) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(f.hzb.0) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.random) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.point_clamp) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&self.point_wrap) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(in_setup_up) },
                ] });
                Pass { label, target, kind, buf, bg }
            }).collect();
            self.targets = Some(Targets { key, setup, ao_low, ao, passes });
        }
        let kinds: Vec<PassKind> = self.targets.as_ref().unwrap().passes.iter().map(|p| p.kind).collect();
        for k in kinds { self.ensure_pipeline(k, self.shader_level); }
        // ---- uniforms (FScreenSpaceAOParameters::Set / FPostProcessAmbientOcclusionSetupPS::SetParameters) ----
        let s1 = half(buffer);
        let s2 = half(s1);
        let view = f.cam.view_matrix();
        let (r0, r1, r2) = (view.row(0), view.row(1), view.row(2));
        let to_view = [[r0.x, r0.y, r0.z, 0.], [r1.x, r1.y, r1.z, 0.], [-r2.x, -r2.y, -r2.z, 0.]];
        let inv_tan_half_fov = f.cam.proj_matrix_no_aa().x_axis.x;
        let ratio = buffer.0 as f32 / buffer.1 as f32;
        let hzb_scale = [buffer.0 as f32 / (2 * f.hzb.1 .0) as f32, buffer.1 as f32 / (2 * f.hzb.1 .1) as f32];
        let hzb_remap = [0.5 * hzb_scale[0], -0.5 * hzb_scale[1], 0.5 * hzb_scale[0], 0.5 * hzb_scale[1]];
        let fade_radius = ue::FADE_RADIUS.max(1.);
        let inv_fade = 1. / fade_radius;
        let toff = temporal_offset(f.taa_sample_index);
        let t = self.targets.as_ref().unwrap();
        for pass in &t.passes {
            // the pass's output size and its PostprocessInput0 (the texture whose size drives ScaleToFullRes)
            let (out, in0, in1, in2) = match (pass.kind, pass.target) {
                (PassKind::Setup { .. }, 0) => (s1, buffer, buffer, buffer),
                (PassKind::Setup { .. }, _) => (s2, s1, s1, s1),
                (PassKind::Ao { .. }, 3) => (s2, s2, s2, s2),
                (PassKind::Ao { .. }, 2) => (s1, s1, s1, s2),
                _ => (buffer, buffer, s1, s1),
            };
            let scale_in = (buffer.0 / in0.0.max(1)).max(1);
            let scale_out = (buffer.0 / out.0.max(1)).max(1);
            let p = [
                [ue::POWER, ue::BIAS / 1000., 1. / ue::DISTANCE_DEPRECATED, f.intensity],
                [in0.0 as f32 / ue::RANDOM_SIZE as f32, in0.1 as f32 / ue::RANDOM_SIZE as f32, radius_in_shader(radius, scale_in), ratio],
                [scale_in as f32, ue::MIP_THRESHOLD / scale_in as f32, if ue::RADIUS_IN_WS { 1. } else { 0. }, ue::MIP_BLEND],
                [toff[0], toff[1], ue::STATIC_FRACTION, inv_tan_half_fov],
                [inv_fade, -(ue::FADE_DISTANCE - fade_radius) * inv_fade, s.ao_mip_level_factor.clamp(0., 100.), 0.],
                [buffer.0 as f32, buffer.1 as f32, 0., 0.],
            ];
            let u = AoU {
                p, hzb_remap,
                setup: [scale_out as f32, ue::MIP_THRESHOLD / scale_out as f32, buffer.0 as f32, buffer.1 as f32],
                in0: size4(in0), in1: size4(in1), in2: size4(in2), out: size4(out), buffer: size4(buffer),
                to_view, depth: [f.cam.near, f.cam.far, crate::reflection::UE_NEAR_M * 100., 0.],
            };
            queue.write_buffer(&pass.buf, 0, bytemuck::bytes_of(&u));
        }
        true
    }

    /// PERF (2026-10-06, debug only: `FN_SSAO_SPLIT=1`): the same passes as [`Self::encode`], each with the timestamps `ts(pass label)` gives.
    pub fn encode_split<'a>(&self, enc: &mut wgpu::CommandEncoder, ts: &dyn Fn(&str) -> Option<wgpu::RenderPassTimestampWrites<'a>>) {
        if !self.active { return; }
        let Some(t) = self.targets.as_ref() else { return };
        for pass in &t.passes {
            let Some(pipe) = self.pipelines.get(&Self::pipeline_key(pass.kind, self.shader_level)) else { continue };
            let view = match pass.target { 0 => &t.setup[0], 1 => &t.setup[1], 2 => &t.ao_low[0], 3 => &t.ao_low[1], _ => &t.ao };
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some(pass.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: ts(pass.label), occlusion_query_set: None, multiview_mask: None });
            p.set_pipeline(pipe);
            p.set_bind_group(0, &pass.bg, &[]);
            p.draw(0..3, 0..1);
        }
    }
    /// The SSAO passes (after the HZB build, before the AO / reflection apply). `ts` brackets all of them.
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, ts: Option<wgpu::RenderPassTimestampWrites<'_>>) {
        if !self.active { return; }
        let Some(t) = self.targets.as_ref() else { return };
        let (ts_begin, ts_end) = crate::reflection::split_timestamps(ts);
        let n = t.passes.len();
        for (i, pass) in t.passes.iter().enumerate() {
            let Some(pipe) = self.pipelines.get(&Self::pipeline_key(pass.kind, self.shader_level)) else { continue };
            let view = match pass.target { 0 => &t.setup[0], 1 => &t.setup[1], 2 => &t.ao_low[0], 3 => &t.ao_low[1], _ => &t.ao };
            let ts = match (i == 0, i + 1 == n) {
                (true, true) => crate::reflection::merge_timestamps(&ts_begin, &ts_end),
                (true, false) => ts_begin.as_ref().map(|t| wgpu::RenderPassTimestampWrites { query_set: t.query_set, beginning_of_pass_write_index: t.beginning_of_pass_write_index, end_of_pass_write_index: None }),
                (false, true) => ts_end.as_ref().map(|t| wgpu::RenderPassTimestampWrites { query_set: t.query_set, beginning_of_pass_write_index: None, end_of_pass_write_index: t.end_of_pass_write_index }),
                _ => None,
            };
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some(pass.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: ts, occlusion_query_set: None, multiview_mask: None });
            p.set_pipeline(pipe);
            p.set_bind_group(0, &pass.bg, &[]);
            p.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn athena_quality_gives_shader_level_2_and_two_levels() {
        // PostProcessQuality@2 / @3: r.AmbientOcclusionMaxQuality 100, Levels -1; the volume quality is the ctor 50
        let q = quality_percent(100., ue::QUALITY);
        assert_eq!(q, 50.);
        assert_eq!(shader_level(q), 2);
        assert_eq!(pass_count(true, q, -1), 2);
        // PostProcessQuality@1: max quality 60 -> still 50; @0: Levels 0 -> off
        assert_eq!(shader_level(quality_percent(60., ue::QUALITY)), 2);
        assert_eq!(pass_count(true, quality_percent(0., ue::QUALITY), 0), 0);
        assert_eq!(pass_count(false, 50., -1), 0);
        // the cvar wins and is clamped to 3
        assert_eq!(pass_count(true, 50., 5), 3);
        assert_eq!(shader_level(100.), 4);
    }
    #[test]
    fn radius_follows_the_mip_scale() {
        // full res: 160 / 400 / 4 = 0.1; half res: x 1.7
        assert!((radius_in_shader(160., 1) - 0.1).abs() < 1e-6);
        assert!((radius_in_shader(160., 2) - 0.17).abs() < 1e-6);
        assert!((radius_in_shader(160., 4) - 0.1 * 1.7 * 1.7).abs() < 1e-5);
    }
    #[test]
    fn randomization_matches_system_textures() {
        let b = random_bases();
        // Pos 0: w = 0 -> angle 0, lenm = 1 - (sin(0) * 0.5 + 0.5) * 0.23 = 0.885 -> (cos, sin) = (0.885, 0)
        let q = |x: f32| ((x * 0.5 + 0.5) * 255.999) as u8;
        assert_eq!(b[0], [q(0.885), q(0.)]);
        // every base vector is within the unit disc and the 4x4 pattern repeats over the 64x64 texture
        for v in b { let (x, y) = (v[0] as f32 / 255. * 2. - 1., v[1] as f32 / 255. * 2. - 1.); assert!(x * x + y * y <= 1.01, "base {v:?}"); }   // 8-bit quantisation can push the length ~0.8 % over 1 (bases 6 / 7)
        let t = random_texture();
        assert_eq!(t.len(), 64 * 64 * 2);
        assert_eq!(&t[0..2], &t[8..10]);
        assert_eq!(&t[0..2], &t[(4 * 64) * 2..(4 * 64) * 2 + 2]);
    }
    #[test]
    fn every_permutation_validates() {
        // naga validation of the shader under every override combination the pass graph can ask for (setup initial / not, AO setup-as-input x
        // upsample x shader quality 0..4); the derivatives of the quad blur must stay in uniform control flow
        let Ok(g) = crate::Gpu::headless() else { eprintln!("no GPU adapter: skipped"); return };
        let mut s = UeSsao::new(&g.device, &g.queue);
        s.ensure_pipeline(PassKind::Setup { initial: true }, 0);
        s.ensure_pipeline(PassKind::Setup { initial: false }, 0);
        for q in 0..5 { for a in [false, true] { for b in [false, true] { s.ensure_pipeline(PassKind::Ao { setup_as_input: a, upsample: b }, q); } } }
        assert_eq!(s.pipelines.len(), 2 + 20);
        g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
    #[test]
    fn temporal_offset_cycles_over_eight_samples() {
        assert_eq!(temporal_offset(0), [0., 0.]);
        assert_eq!(temporal_offset(8), [0., 0.]);
        assert!((temporal_offset(3)[0] - 3. * 2.48 / 64.).abs() < 1e-6);
    }
}
