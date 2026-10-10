//! The UE 4.16 bloom chain of Fortnite 1.8 as a self-contained wgpu module ([`BloomChain`]) the renderer drives once per frame, plus the
//! host-side plan shared with the CPU reference (`cpu::bloom_chain`).
//!
//! Sources (every constant below is one of these):
//! * UE 4.16 `PostProcessing.cpp`: `SceneColorHalfRes` = `FRCPassPostProcessDownsample(PF_FloatRGB, r.Downsample.Quality = 3 -> clamp 1)` of the
//!   post-DOF/motion-blur/separate-translucency scene colour; `RenderHalfResBloomThreshold` returns that half-res image unchanged when
//!   `BloomThreshold <= -1` (no `FPostProcessBloomSetupPS` pass), `TBloomDownSampleArray<6>` (5 more `FRCPassPostProcessDownsample(.., 1, ..)`),
//!   `AddBloom`: stages Bloom6..Bloom1 on downsample 5..0, `BloomQualityStages = {3,3,4,5,6}` (r.BloomQuality 5 -> 6 stages),
//!   `TintScale = 1 / 6`, a stage runs when `BloomNSize > SMALL_NUMBER`, radius `BloomNSize * BloomSizeScale`, previous stage = additive input;
//!   all sizes 0 -> the bloom image is `PostProcessDownsamples[0]`.
//!   1.8 exe (decomp `FUN_1418adeb0` = AddBloom): TintScale `DAT_143feb1d4` = 0x3e2aaaab (1/6), stage table `DAT_144631368` = {3,3,4,5,6},
//!   SMALL_NUMBER `DAT_143ff9008` = 1e-8; `FUN_1418b5780`: the threshold test against `DAT_143feb1ec` = -1.0.
//! * UE 4.16 `PostProcessWeightedSampleSum.cpp` (`Compute1DGaussianFilterKernel`, `NormalDistributionUnscaled`, `Process`, `DoFastBlur`,
//!   `AdjustRectsForFastBlur`, `ComputeOutputDesc`) and `FilterPixelShader.usf`: weights `exp(-16.7 * (x / R)^2)` (r.Filter.NewMethod 1),
//!   bilinear tap pairs, `R = clamp(radius, DELTA, 31)` (r.Filter.LoopMode 0 -> 32 samples), integer radius `ceil(R * r.Filter.SizeScale)`,
//!   `radius = Size * (ViewWidth / SrcScaleFactor) / 2 / 100` texels, fast blur (half-width X pass, +-0.5 texel Y shifts) when
//!   `Size * SrcExtent.X * 2 / 100 >= r.FastBlurThreshold (7)`, sampler `SF_Bilinear, AM_Border, AM_Border` (outside = 0) for both the
//!   filtered and the additive texture. 1.8 exe: `FUN_1418ff170` = NormalDistributionUnscaled (constant `DAT_14463f128` = -16.7),
//!   `FUN_1418ef700` = Compute1DGaussianFilterKernel (SizeScale clamp 0.1..10, 32 samples), `FUN_141904d40` = Process (`* 0.005` =
//!   `DAT_1444efbf0`, doubled source size for the fast vertical pass). Cvar defaults from the exe (`fn-exe-defaults/reports/exe-defaults.json`):
//!   r.FastBlurThreshold 7, r.Filter.LoopMode 0, r.Filter.NewMethod 1, r.Filter.SizeScale 1, r.BloomQuality 5, r.Bloom.Cross 0.
//! * Tonemapper (`PostProcessTonemap.usf` + `.cpp`): `LinearColor = SceneColor * SceneColorTint + Bloom * (BloomIntensity + dirt mask)`,
//!   THEN `* ExposureScale`; the bloom image is never exposed (setup pass only, for the threshold).
use crate::params::BloomParams;
use bytemuck::{Pod, Zeroable};

/// `Compute1DGaussianFilterKernel` sample limit (`MAX_FILTER_COMPILE_TIME_SAMPLES`, r.Filter.LoopMode 0).
pub const MAX_SAMPLES: usize = 32;
/// `r.FastBlurThreshold` (exe default 7.0).
pub const FAST_BLUR_THRESHOLD: f32 = 7.0;
/// `r.Filter.SizeScale` (exe default 1.0, clamped 0.1..10 by the kernel).
pub const FILTER_SIZE_SCALE: f32 = 1.0;
/// `TintScale = 1.0f / NumBloomStages` (exe `DAT_143feb1d4` = 0x3e2aaaab).
pub const TINT_SCALE: f32 = 1.0 / 6.0;
/// `BloomQualityStages[r.BloomQuality - 1]` for r.BloomQuality 5 (exe default and Fortnite PostProcessQuality@2/@3, CookedIniVersion.txt L5218/5235;
/// @0/@1 use 4 -> 5 stages, see [`BloomScalability`]).
pub const STAGE_COUNT: usize = 6;
/// `SMALL_NUMBER` (exe `DAT_143ff9008`).
pub const SMALL_NUMBER: f32 = 1.0e-8;

/// SCALABILITY: the bloom cvars of Fortnite's `Scalability:PostProcessQuality@N` (CookedIniVersion.txt, extracted copy
/// `C:\FortniteRef\scratch\CookedIniVersion.txt`): `r.BloomQuality` 4/4/5/5 (L5184/5201/5218/5235), `r.FastBlurThreshold` 0/2/3/100
/// (L5185/5202/5219/5236), `r.Filter.SizeScale` 0.6/0.7/0.8/1 (L5189/5206/5223/5240). Epic (@3) never takes the fast blur (100 % of the view
/// width), the exe default 7 ([`FAST_BLUR_THRESHOLD`]) applies only when no scalability level is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomScalability {
    /// `r.BloomQuality` (`AddBloom`: `BloomQualityStages[q - 1]` = {3,3,4,5,6} stages, 0 = no bloom blur, PostProcessing.cpp:703-717).
    pub quality: i32,
    /// `r.FastBlurThreshold` (`DoFastBlur`, PostProcessWeightedSampleSum.cpp).
    pub fast_blur_threshold: f32,
    /// `r.Filter.SizeScale` (`Compute1DGaussianFilterKernel`, clamped 0.1..10).
    pub filter_size_scale: f32,
}
impl BloomScalability {
    /// Exe cvar defaults (no scalability group applied): BloomQuality 5, FastBlurThreshold 7, Filter.SizeScale 1 (exe-defaults.json).
    pub const EXE_DEFAULT: BloomScalability = BloomScalability { quality: 5, fast_blur_threshold: FAST_BLUR_THRESHOLD, filter_size_scale: FILTER_SIZE_SCALE };
    /// `PostProcessQuality@level` (levels above 3 = @3).
    pub const fn level(level: u32) -> BloomScalability {
        match level {
            0 => BloomScalability { quality: 4, fast_blur_threshold: 0., filter_size_scale: 0.6 },
            1 => BloomScalability { quality: 4, fast_blur_threshold: 2., filter_size_scale: 0.7 },
            2 => BloomScalability { quality: 5, fast_blur_threshold: 3., filter_size_scale: 0.8 },
            _ => BloomScalability { quality: 5, fast_blur_threshold: 100., filter_size_scale: 1. },
        }
    }
    /// `BloomQualityStages[clamp(r.BloomQuality, 0, 5) - 1]`; 0 = no Gaussian stage (the bloom image is downsample 0).
    pub fn stage_count(&self) -> usize { const T: [usize; 5] = [3, 3, 4, 5, 6]; let q = self.quality.clamp(0, 5); if q == 0 { 0 } else { T[q as usize - 1] } }
}
impl Default for BloomScalability { fn default() -> Self { BloomScalability::level(3) } }

/// `(offset in texels, weight)` pairs, exactly `Compute1DGaussianFilterKernel` (new method, no cross), at `r.Filter.SizeScale` 1.
pub fn ue_gaussian_taps(kernel_radius: f32) -> Vec<[f32; 2]> { ue_gaussian_taps_scaled(kernel_radius, FILTER_SIZE_SCALE) }
/// [`ue_gaussian_taps`] with an explicit `r.Filter.SizeScale` (the integer radius is `ceil(R * clamp(scale, 0.1, 10))`).
pub fn ue_gaussian_taps_scaled(kernel_radius: f32, size_scale: f32) -> Vec<[f32; 2]> {
    let max = (MAX_SAMPLES - 1) as f32;
    let clamped = kernel_radius.clamp(1.0e-5, max); // GetClampedKernelRadius: Clamp(KernelRadius, DELTA, MaxNumSamples - 1)
    let int_r = ((kernel_radius * size_scale.clamp(0.1, 10.0)).clamp(1.0e-5, max).ceil() as i32).min(MAX_SAMPLES as i32 - 1);
    let scale = 1.0 / clamped;
    let w = |x: i32| { let d = x.abs() as f32 * scale; (-16.7 * d * d).exp() };
    let mut taps = Vec::new();
    let mut sum = 0.0;
    let mut i = -int_r;
    while i <= int_r {
        let w0 = w(i);
        let w1 = if i != int_r { w(i + 1) } else { 0.0 };
        let tw = w0 + w1;
        // tw == 0 only for sub-texel radii (weights underflow): the engine divides 0/0 there; the tap carries no weight either way
        taps.push([i as f32 + if tw > 0.0 { w1 / tw } else { 0.0 }, tw]);
        sum += tw;
        i += 2;
    }
    for t in &mut taps { t[1] /= sum; }
    taps
}

/// One Gaussian stage (`RenderBloom` -> two `FRCPassPostProcessWeightedSampleSum`).
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    /// Downsample level that is blurred (0 = half res).
    pub level: usize,
    /// `BloomNSize * BloomSizeScale` (percent of the view width).
    pub size: f32,
    /// `BloomNTint * TintScale` (rgba).
    pub tint: [f32; 4],
    /// `DoFastBlur()` of the X pass (the Y pass follows it: half-width input detected).
    pub fast: bool,
    /// Width of the X-pass output (`DivideAndRoundUp(w, 2)` when fast).
    pub tmp_w: usize,
    pub taps_x: Vec<[f32; 2]>,
    pub taps_y: Vec<[f32; 2]>,
}

/// Host-side decisions of one frame's bloom chain.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Size of each of the 6 downsample levels (level 0 = half res).
    pub levels: Vec<(usize, usize)>,
    /// `BloomThreshold > -1`: the `FPostProcessBloomSetupPS` pass runs on the half-res image.
    pub setup: bool,
    /// Stages in execution order (Bloom6 first). Empty = the bloom image is level 0 itself.
    pub stages: Vec<Stage>,
}

pub fn half(n: usize) -> usize { n.div_ceil(2).max(1) }

pub fn plan(b: &BloomParams, view_w: usize, view_h: usize) -> Plan { plan_with(b, view_w, view_h, &BloomScalability::EXE_DEFAULT) }
/// [`plan`] for a `PostProcessQuality` level ([`BloomScalability`]): stage count, fast-blur threshold and kernel size scale.
pub fn plan_with(b: &BloomParams, view_w: usize, view_h: usize, sc: &BloomScalability) -> Plan {
    let mut levels = vec![(half(view_w), half(view_h))];
    for k in 1..6 { let (w, h) = levels[k - 1]; levels.push((half(w), half(h))); }
    let sizes = [b.sizes[5], b.sizes[4], b.sizes[3], b.sizes[2], b.sizes[1], b.sizes[0]];
    let tints = [b.tints[5], b.tints[4], b.tints[3], b.tints[2], b.tints[1], b.tints[0]];
    let buffer_ratio = view_w as f32 / view_h as f32;
    let mut stages = vec![];
    for i in 0..sc.stage_count().min(STAGE_COUNT) {
        let level = 5 - i;
        if !(sizes[i] > SMALL_NUMBER) { continue; }
        let size = sizes[i] * b.size_scale;
        let (lw, lh) = levels[level];
        // DoFastBlur (X): Size * SrcExtent.X * 2 / 100 >= r.FastBlurThreshold
        let fast = size * lw as f32 * 2.0 / 100.0 >= sc.fast_blur_threshold;
        let tmp_w = if fast { lw.div_ceil(2) } else { lw };
        // Process(): SrcSizeForThisAxis = ViewRect.Width / DivideAndRoundUp(BufferSize.X, SrcSize.X), radius = Size * that / 2 / 100
        let src_axis = |src_w: usize| view_w as f32 / view_w.div_ceil(src_w) as f32;
        let r_x = size * src_axis(lw) * 0.005;
        // the Y pass detects the half-width input (InputRatio < BufferRatio * 0.75) and doubles SrcSizeForThisAxis
        let y_fast = (tmp_w as f32 / lh as f32) < buffer_ratio * 0.75;
        let r_y = size * src_axis(tmp_w) * if y_fast { 2.0 } else { 1.0 } * 0.005;
        let t = tints[i];
        stages.push(Stage { level, size, tint: [t[0] * TINT_SCALE, t[1] * TINT_SCALE, t[2] * TINT_SCALE, TINT_SCALE], fast, tmp_w, taps_x: ue_gaussian_taps_scaled(r_x, sc.filter_size_scale), taps_y: ue_gaussian_taps_scaled(r_y, sc.filter_size_scale) });
    }
    Plan { levels, setup: b.threshold > -1.0, stages }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FilterU {
    taps: [[f32; 4]; 32],
    /// xy: pixel position -> source uv; zw: unused
    uv_scale: [f32; 4],
    /// xy: uv step per texel of tap offset, zw: constant uv shift (fast-blur half texel)
    step: [f32; 4],
    tint: [f32; 4],
    /// x: add the additive texture, y: tap count
    flags: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DownU {
    /// x = exposure, y = threshold (setup only)
    p: [f32; 4],
    /// xy: pixel position -> source uv
    uv_scale: [f32; 4],
    /// xy: one source texel in uv
    src_texel: [f32; 4],
}

struct Tex { #[allow(dead_code)] tex: wgpu::Texture, view: wgpu::TextureView }

fn make_tex(device: &wgpu::Device, label: &str, w: usize, h: usize, format: wgpu::TextureFormat) -> Tex {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label), size: wgpu::Extent3d { width: w.max(1) as u32, height: h.max(1) as u32, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    Tex { tex, view }
}

struct Pass { bg: wgpu::BindGroup, target: usize, down: bool }

struct Built {
    key: (usize, usize, u64),
    plan: Plan,
    /// Render targets: 0..6 = downsample levels, 6 = setup output, then per stage (tmp, out).
    texs: Vec<Tex>,
    passes: Vec<Pass>,
    ubufs: Vec<wgpu::Buffer>,
    out: usize,
}

/// The bloom chain. `prepare` once per frame (uniforms; textures / bind groups when the size, plan or source changed), `encode` records the passes.
pub struct BloomChain {
    device: wgpu::Device,
    format: wgpu::TextureFormat,
    bgl_down: wgpu::BindGroupLayout,
    bgl_filter: wgpu::BindGroupLayout,
    down: wgpu::RenderPipeline,
    setup: wgpu::RenderPipeline,
    filter: wgpu::RenderPipeline,
    clamp: wgpu::Sampler,
    built: Option<Built>,
}

impl BloomChain {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> BloomChain {
        let f = wgpu::ShaderStages::FRAGMENT;
        let ent = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: f, ty, count: None };
        let ub = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let tx = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let bgl_down = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("bloom down bgl"), entries: &[ent(0, ub), ent(1, tx), ent(2, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering))] });
        let bgl_filter = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("bloom filter bgl"), entries: &[ent(0, ub), ent(1, tx), ent(2, tx)] });
        let common = fn_live::wgsl!("../shaders/common.wgsl");
        let sm_down = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("bloom down"), source: wgpu::ShaderSource::Wgsl([common, fn_live::wgsl!("../shaders/bloom.wgsl")].concat().into()) });
        let sm_filter = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("bloom filter"), source: wgpu::ShaderSource::Wgsl([common, fn_live::wgsl!("../shaders/bloom_filter.wgsl")].concat().into()) });
        let pl = |b: &wgpu::BindGroupLayout| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("bloom layout"), bind_group_layouts: &[Some(b)], immediate_size: 0 });
        let (pl_down, pl_filter) = (pl(&bgl_down), pl(&bgl_filter));
        let pipe = |label: &str, sm: &wgpu::ShaderModule, fs: &str, layout: &wgpu::PipelineLayout| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label), layout: Some(layout),
            vertex: wgpu::VertexState { module: sm, entry_point: Some("vs_fullscreen"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: sm, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        BloomChain {
            down: pipe("bloom downsample", &sm_down, "fs_downsample", &pl_down),
            setup: pipe("bloom setup", &sm_down, "fs_setup", &pl_down),
            filter: pipe("bloom gaussian", &sm_filter, "fs_filter", &pl_filter),
            clamp: device.create_sampler(&wgpu::SamplerDescriptor { label: Some("bloom bilinear clamp"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }),
            device: device.clone(), format, bgl_down, bgl_filter, built: None,
        }
    }

    /// Per frame: `scene` = the full-resolution linear HDR scene colour (after every translucent pass), `source_gen` changes whenever `scene`
    /// is a different texture (the bind groups are rebuilt then), `exposure` = eye-adaptation scale (only the threshold pass reads it).
    pub fn prepare(&mut self, queue: &wgpu::Queue, scene: &wgpu::TextureView, width: u32, height: u32, source_gen: u64, b: &BloomParams, exposure: f32) {
        self.prepare_with(queue, scene, width, height, source_gen, b, exposure, &BloomScalability::EXE_DEFAULT)
    }
    /// [`BloomChain::prepare`] at a `PostProcessQuality` level (SCALABILITY, see [`BloomScalability`]).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_with(&mut self, queue: &wgpu::Queue, scene: &wgpu::TextureView, width: u32, height: u32, source_gen: u64, b: &BloomParams, exposure: f32, sc: &BloomScalability) {
        let (w, h) = (width as usize, height as usize);
        let plan = plan_with(b, w, h, sc);
        let stale = match &self.built { Some(x) => x.key != (w, h, source_gen) || x.plan != plan, None => true };
        if stale { self.built = Some(self.build(scene, w, h, source_gen, plan)); }
        let built = self.built.as_ref().unwrap();
        let mut u = 0;
        let mut put = |data: &[u8]| { queue.write_buffer(&built.ubufs[u], 0, data); u += 1; };
        let (hw, hh) = built.plan.levels[0];
        put(bytemuck::bytes_of(&DownU { p: [0.0; 4], uv_scale: [2.0 / w as f32, 2.0 / h as f32, 0.0, 0.0], src_texel: [1.0 / w as f32, 1.0 / h as f32, 0.0, 0.0] }));
        if built.plan.setup { put(bytemuck::bytes_of(&DownU { p: [exposure, b.threshold, 0.0, 0.0], uv_scale: [1.0 / hw as f32, 1.0 / hh as f32, 0.0, 0.0], src_texel: [0.0; 4] })); }
        for k in 1..6 {
            let (sw, sh) = built.plan.levels[k - 1];
            put(bytemuck::bytes_of(&DownU { p: [0.0; 4], uv_scale: [2.0 / sw as f32, 2.0 / sh as f32, 0.0, 0.0], src_texel: [1.0 / sw as f32, 1.0 / sh as f32, 0.0, 0.0] }));
        }
        for (si, s) in built.plan.stages.iter().enumerate() {
            let (lw, lh) = built.plan.levels[s.level];
            let fill = |taps: &[[f32; 2]]| { let mut t = [[0.0f32; 4]; 32]; for (i, x) in taps.iter().enumerate() { t[i] = [x[0], x[1], 0.0, 0.0]; } t };
            let (lwf, lhf, twf) = (lw as f32, lh as f32, s.tmp_w as f32);
            // X pass: source = level (lw), output tmp_w; fast: output texel x covers source texels 2x, 2x+1 (AdjustRectsForFastBlur), +0.5 texel in y
            let x = FilterU { taps: fill(&s.taps_x), uv_scale: [if s.fast { 2.0 / lwf } else { 1.0 / lwf }, 1.0 / lhf, 0.0, 0.0], step: [1.0 / lwf, 0.0, 0.0, if s.fast { 0.5 / lhf } else { 0.0 }], tint: [1.0; 4], flags: [0, s.taps_x.len() as u32, 0, 0] };
            // Y pass: source = tmp (tmp_w), output = level size; fast: the output is twice as wide as the source, -0.5 texel in y; previous stage added
            let y = FilterU { taps: fill(&s.taps_y), uv_scale: [if s.fast { 0.5 / twf } else { 1.0 / twf }, 1.0 / lhf, 0.0, 0.0], step: [0.0, 1.0 / lhf, 0.0, if s.fast { -0.5 / lhf } else { 0.0 }], tint: s.tint, flags: [(si > 0) as u32, s.taps_y.len() as u32, 0, 0] };
            put(bytemuck::bytes_of(&x));
            put(bytemuck::bytes_of(&y));
        }
    }

    fn build(&self, scene: &wgpu::TextureView, w: usize, h: usize, gen: u64, plan: Plan) -> Built {
        let d = &self.device;
        let mut texs: Vec<Tex> = plan.levels.iter().enumerate().map(|(k, &(lw, lh))| make_tex(d, &format!("bloom downsample {k}"), lw, lh, self.format)).collect();
        texs.push(make_tex(d, "bloom setup", plan.levels[0].0, plan.levels[0].1, self.format));
        let mut ubufs = vec![];
        let mut ub = |size: usize| { let b = d.create_buffer(&wgpu::BufferDescriptor { label: Some("bloom u"), size: size as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }); ubufs.push(b); ubufs.len() - 1 };
        let mut pending: Vec<(usize, Option<usize>, Option<usize>, usize, bool)> = vec![]; // (ubuf, src tex (None = scene), additive tex, target, downsample pipeline)
        pending.push((ub(std::mem::size_of::<DownU>()), None, None, 0, true));
        let level0 = if plan.setup { pending.push((ub(std::mem::size_of::<DownU>()), Some(0), Some(usize::MAX), 6, true)); 6 } else { 0 };
        for k in 1..6 { pending.push((ub(std::mem::size_of::<DownU>()), Some(if k == 1 { level0 } else { k - 1 }), None, k, true)); }
        let mut prev: Option<usize> = None;
        for (si, s) in plan.stages.iter().enumerate() {
            let (lw, lh) = plan.levels[s.level];
            let src = if s.level == 0 { level0 } else { s.level };
            texs.push(make_tex(d, &format!("bloom blur x {si}"), s.tmp_w, lh, self.format));
            let tmp = texs.len() - 1;
            texs.push(make_tex(d, &format!("bloom blur y {si}"), lw, lh, self.format));
            let out = texs.len() - 1;
            pending.push((ub(std::mem::size_of::<FilterU>()), Some(src), None, tmp, false));
            pending.push((ub(std::mem::size_of::<FilterU>()), Some(tmp), prev, out, false));
            prev = Some(out);
        }
        let out = prev.unwrap_or(level0);
        let passes = pending.iter().map(|&(u, src, add, target, down)| {
            let src_view = src.map(|i| &texs[i].view).unwrap_or(scene);
            let bg = if down {
                // the setup pass marks itself with add = usize::MAX (same layout, different pipeline)
                d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("bloom down bg"), layout: &self.bgl_down, entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ubufs[u].as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src_view) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.clamp) }] })
            } else {
                let add_view = add.map(|i| &texs[i].view).unwrap_or(src_view);
                d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("bloom filter bg"), layout: &self.bgl_filter, entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ubufs[u].as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src_view) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(add_view) }] })
            };
            Pass { bg, target, down: down && add != Some(usize::MAX) }
        }).collect();
        Built { key: (w, h, gen), plan, texs, passes, ubufs, out }
    }

    /// The bloom image (half resolution, tints applied, NOT multiplied by `BloomIntensity`, never exposed). `None` before the first `prepare`.
    pub fn output(&self) -> Option<&wgpu::TextureView> { self.built.as_ref().map(|b| &b.texs[b.out].view) }
    /// Changes whenever [`output`](Self::output) is a different texture (consumers rebuild their bind groups).
    pub fn output_id(&self) -> Option<(usize, usize, u64, usize)> { self.built.as_ref().map(|b| (b.key.0, b.key.1, b.key.2, b.texs.len())) }
    pub fn plan(&self) -> Option<&Plan> { self.built.as_ref().map(|b| &b.plan) }
    /// `SceneColorHalfRes` (the 4-tap downsample of the scene, before any threshold): the input of the 4.16 histogram (eye adaptation).
    pub fn half_res(&self) -> Option<&wgpu::TextureView> { self.built.as_ref().map(|b| &b.texs[0].view) }

    /// Records the downsample chain and the Gaussian stages (call after `prepare`, with the scene colour finished).
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder) { self.encode_with_final(enc, None); }

    /// [`encode`](Self::encode), writing the last Gaussian stage into `final_target` (same size as level 0) instead of the internal texture.
    /// Returns false when nothing was written there (no stage ran: the bloom image is level 0, see [`output`](Self::output)).
    pub fn encode_with_final(&self, enc: &mut wgpu::CommandEncoder, final_target: Option<&wgpu::TextureView>) -> bool {
        let Some(b) = &self.built else { return false };
        let n = b.passes.len();
        let redirect = final_target.filter(|_| !b.plan.stages.is_empty());
        for (i, p) in b.passes.iter().enumerate() {
            let view = match redirect { Some(v) if i + 1 == n => v, _ => &b.texs[p.target].view };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_pipeline(if p.down { &self.down } else if p.target == 6 { &self.setup } else { &self.filter });
            pass.set_bind_group(0, &p.bg, &[]);
            pass.draw(0..3, 0..1);
        }
        redirect.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn athena_plan_matches_the_ue_416_rules() {
        let (_, _, b) = crate::params::athena_day_cm();
        let p = plan(&b, 1920, 1080);
        assert!(!p.setup, "BloomThreshold -1 skips FPostProcessBloomSetupPS");
        assert_eq!(p.levels, vec![(960, 540), (480, 270), (240, 135), (120, 68), (60, 34), (30, 17)]);
        // Bloom6Size 0 -> 5 stages: Bloom5 on level 4 ... Bloom1 on level 0
        assert_eq!(p.stages.iter().map(|s| s.level).collect::<Vec<_>>(), vec![4, 3, 2, 1, 0]);
        // fast blur: Size * extent * 0.02 >= 7 -> Bloom5 30*60*.02 = 36, Bloom4 24, Bloom3 9.6, Bloom2 9.6, Bloom1 0.3*960*.02 = 5.76 (not fast)
        assert_eq!(p.stages.iter().map(|s| s.fast).collect::<Vec<_>>(), vec![true, true, true, true, false]);
        // Bloom1: radius 0.3 * 960 * 0.005 = 1.44 texels; Bloom3 tint 0.5 / 6
        let b1 = &p.stages[4];
        assert!((b1.tint[0] - 0.3465 / 6.0).abs() < 1e-7 && (p.stages[2].tint[0] - 0.5 / 6.0).abs() < 1e-7);
        // integer radius ceil(1.44) = 2 -> bilinear pairs starting at -2, 0 and the unpaired +2
        assert_eq!(b1.taps_x.len(), 3);
        assert!(b1.taps_x.iter().zip(&ue_gaussian_taps(1.44)).all(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6));
    }
    #[test]
    fn kernel_is_the_new_method_gaussian() {
        let t = ue_gaussian_taps(4.0);
        let s: f32 = t.iter().map(|x| x[1]).sum();
        assert!((s - 1.0).abs() < 1e-6);
        // integer radius 4 -> sample indices -4,-2,0,2,4 (5 pairs, the last unpaired)
        assert_eq!(t.len(), 5);
        assert!((t[4][0] - 4.0).abs() < 1e-6);
        // weight of the unpaired edge tap relative to the centre pair: exp(-16.7) / (1 + exp(-16.7 / 16))
        let w_c = 1.0 + (-16.7f32 / 16.0).exp();
        assert!((t[4][1] / t[2][1] - (-16.7f32).exp() / w_c).abs() < 1e-9);
    }
    #[test]
    fn scalability_levels_follow_fortnite_post_process_quality() {
        // CookedIniVersion.txt L5184-5240: BloomQuality 4/4/5/5, FastBlurThreshold 0/2/3/100, Filter.SizeScale 0.6/0.7/0.8/1
        let l: Vec<_> = (0..4).map(BloomScalability::level).collect();
        assert_eq!(l.iter().map(|x| x.stage_count()).collect::<Vec<_>>(), vec![5, 5, 6, 6]);
        assert_eq!(l.iter().map(|x| x.fast_blur_threshold).collect::<Vec<_>>(), vec![0., 2., 3., 100.]);
        let b = crate::params::athena_day_cm().2;
        // Epic: no stage takes the fast blur at 1920x1080; Low: every stage does (threshold 0) and Bloom1 (the last, smallest stage) is dropped
        let epic = plan_with(&b, 1920, 1080, &l[3]);
        assert!(epic.stages.iter().all(|s| !s.fast));
        let low = plan_with(&b, 1920, 1080, &l[0]);
        assert!(low.stages.iter().all(|s| s.fast) && low.stages.iter().all(|s| s.level >= 1));
        // SizeScale 0.6 shortens the kernel
        assert!(ue_gaussian_taps_scaled(10.0, 0.6).len() < ue_gaussian_taps(10.0).len());
    }
}
