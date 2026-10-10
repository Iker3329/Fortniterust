//! REFLECTION APPLY (docs/spec/reflection_gaps.md sections 1 and 2): the deferred specular indirect light of UE 4.16 in this forward renderer.
//!
//! UE applies the reflections of every opaque GBuffer pixel in `ReflectionApplyPS` (ReflectionEnvironmentShaders.usf:537; Athena has no reflection captures
//! and `r.AllowStaticLighting=0`, so it is SSR + sky light only), AFTER the screen-space AO, with the specular occlusion
//! `saturate(Square(NoV + AO) - 1 + AO)` of `AO = GBufferAO * ScreenSpaceAO`, which is not linear in AO. This port therefore splits it:
//! * every opaque lit pipeline of the "backdrop" and "scene" passes writes three extra targets ([`lit_targets`], `lit_common.wgsl` `LitOut`):
//!   0 rgb = sky specular without the SO (x fog transmittance), a = NoV; 1 rgb = EnvBRDF (x fog transmittance), a = GBufferAO; 2 xyz = GBuffer world normal
//!   (0 = ShadingModelID 0, nothing to reflect), w = roughness;
//! * [`Ssr`] (this module) traces the screen space reflections (HZB, trace, and the SSR temporal filter of views without temporal AA);
//! * `ssao_pass.rs` / `ssao_apply.wgsl` applies the screen-space AO and composites `SO * (SSR.rgb * EnvBRDF + (1 - SSR.a) * Sky)` into the scene colour
//!   (ReflectionApplyPS:585-665), before fog / translucency / light shafts / bloom like UE (the fog transmittance is premultiplied).
//!
//! # SSR (ScreenSpaceReflections.cpp / .usf, ScreenSpaceRayCast.usf, HZBOcclusion.usf)
//! * Gate `ShouldRenderScreenSpaceReflections` (:58): `r.SSR.Quality > 0` (cooked 0 / 0 / 2 / 3 / 4 for Effects Low..Cine, [`crate::quality::EffectsScalability`])
//!   and `ScreenSpaceReflectionIntensity >= 1` (exe ctor 100; no TOD phase overrides it, [`crate::quality::SSR_PP_INTENSITY`]).
//! * Shader quality `min(ComputeSSRQuality(50) = 2, r.SSR.Quality)` = 2 at High, Epic and Cine: 16 steps, 1 ray, `reflect(-V, N)`, IGN step offset.
//! * `SSRParams` = (clamp(Intensity / 100, 0, 1) = 1, RoughnessMaskScale -6.667 (MaxRoughness 0.6), 0, TAA sample index * 1551 (unused below quality 3)).
//! * HZB (`BuildHZB`, SceneOcclusion.cpp:1076): `2^(max(ceil(log2(view)) - 1, 1))` per axis, R16F, the furthest UE device Z of each 2x2 block, mips down to 1.
//! * Colour source: the previous frame's temporal AA history when there is one and no camera cut (`bPrevFrame`, :512), else the current scene colour.
//! * Temporal filter only without temporal AA (`IsSSRTemporalPassRequired`, :91, `r.SSR.Temporal 0`): `SSRTemporalAAPS` into SSRHistoryRT.
//!
//! # Port limitations (documented, not approximations of a formula)
//! * The backdrop pass (landscape beyond the plate, ocean, far skybox) draws with its own projection; its depth is merged into the scene depth (renderer.rs
//!   "backdrop depth", SSR BACKDROP), so it is in the HZB / SSR, but only out to the main far plane (3 km; the backdrop is clamped to about 2995 m).
//! * Velocity is only rendered while temporal AA runs (taa.rs); UE also renders it for the SSR temporal pass (VelocityRendering.cpp:871), so without TAA the
//!   SSR reprojection is camera-only (moving characters' reflections reproject like static geometry).
//! * SSR qualities 3 / 4 (multi-ray, glossy) are unreachable with the Athena post-process quality 50 and are not ported (quality 3 / 4 fall back to quality 2).
use crate::camera::CameraParams;
use bytemuck::{Pod, Zeroable};

/// Format of the three reflection-input targets (fp16 like UE's PF_FloatRGBA scene targets).
pub const REFL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The SSR output (`FRCPassPostProcessScreenSpaceReflections::ComputeOutputDesc`: PF_FloatRGBA).
pub const SSR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// `BuildHZB`: PF_R16F.
pub const HZB_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
/// UE's `GNearClippingPlane` (10 cm) in metres: the HZB / ray maths use UE's device Z = near / view depth.
pub const UE_NEAR_M: f32 = 0.1;

/// Verification aid (`FN_SSR_DEBUG=1`, SSAO on): the trace treats every lit pixel as roughness 0 and the apply pass shows the raw SSR over the dimmed scene, so the trace / HZB /
/// reprojection can be checked on any surface (BR has few pixels below the 0.3 roughness cut-off). `FN_SSR=0` switches SSR off.
pub fn debug_view() -> bool { fn_live::var("FN_SSR_DEBUG").map_or(false, |v| v == "1") }

/// The colour targets of every opaque lit pipeline drawn in the "backdrop" / "scene" passes: the HDR scene colour + the three reflection inputs.
pub fn lit_targets(hdr: wgpu::TextureFormat) -> [Option<wgpu::ColorTargetState>; 4] {
    let t = |format| Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL });
    [t(hdr), t(REFL_FORMAT), t(REFL_FORMAT), t(REFL_FORMAT)]
}

/// `BuildHZB` mip-0 size and mip count for a view (SceneOcclusion.cpp:1082-1088).
pub fn hzb_size(view_w: u32, view_h: u32) -> ((u32, u32), u32) {
    let mips = |n: u32| ((n.max(1) as f32).log2().ceil() as i32 - 1).max(1) as u32;
    let (mx, my) = (mips(view_w), mips(view_h));
    ((1 << mx, 1 << my), mx.max(my))
}
/// `HZBUvFactorAndInvFactor` (ScreenSpaceReflections.cpp:268-278).
pub fn hzb_uv_factor(view_w: u32, view_h: u32) -> [f32; 4] {
    let ((hw, hh), _) = hzb_size(view_w, view_h);
    let f = [view_w as f32 / (2 * hw) as f32, view_h as f32 / (2 * hh) as f32];
    [f[0], f[1], 1. / f[0], 1. / f[1]]
}
/// `SampleWeights[9]` of the temporal AA shaders for a pixel jitter (PostProcessTemporalAA.cpp:130-182, `r.TemporalAACatmullRom` 0 = Gaussian).
pub fn sample_weights(jitter_px: [f32; 2]) -> [f32; 9] {
    let mut w = [0f32; 9];
    for (i, wi) in w.iter_mut().enumerate() {
        let (ox, oy) = ((i % 3) as f32 - 1. - jitter_px[0], (i / 3) as f32 - 1. - jitter_px[1]);
        let (x, y) = (ox / crate::taa::ue::TEMPORAL_AA_FILTER_SIZE, oy / crate::taa::ue::TEMPORAL_AA_FILTER_SIZE);
        *wi = (-2.29 * (x * x + y * y)).exp();
    }
    let t: f32 = w.iter().sum();
    w.map(|x| x / t)
}

/// The three reflection-input targets of the lit passes (view size).
pub struct ReflTargets {
    pub size: (u32, u32),
    pub views: [wgpu::TextureView; 3],
}
impl ReflTargets {
    pub fn new(device: &wgpu::Device, w: u32, h: u32) -> ReflTargets {
        let mk = |label: &str| device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: REFL_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }).create_view(&Default::default());
        ReflTargets { size: (w, h), views: [mk("reflection sky specular"), mk("reflection env brdf"), mk("reflection gbuffer")] }
    }
    /// The extra colour attachments of the "backdrop" / "scene" passes (cleared to 0 = nothing to reflect by the first of them).
    pub fn attachments(&self, clear: bool) -> [Option<wgpu::RenderPassColorAttachment<'_>>; 3] {
        let load = if clear { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load };
        let a = |v| Some(wgpu::RenderPassColorAttachment { view: v, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } });
        [a(&self.views[0]), a(&self.views[1]), a(&self.views[2])]
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HzbU { depth: [f32; 4] }
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SsrU { view: [[f32; 4]; 4], clip_to_prev: [[f32; 4]; 4], proj: [f32; 4], params: [f32; 4], hzb: [f32; 4], viewport: [f32; 4], depth: [f32; 4] }
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TempU { clip_to_prev: [[f32; 4]; 4], weights0: [f32; 4], weights1: [f32; 4], weights2: [f32; 4], viewport: [f32; 4] }

struct Hzb { view_size: (u32, u32), /** OCCLUSION: the texture itself (the readback copy) */ tex: wgpu::Texture, all: wgpu::TextureView, mips: Vec<wgpu::TextureView>, bgs: Vec<wgpu::BindGroup>, depth_gen: u64 }
struct Out { size: (u32, u32), raw: (wgpu::Texture, wgpu::TextureView), final_: wgpu::TextureView, history: [wgpu::TextureView; 2], scene_copy: (wgpu::Texture, wgpu::TextureView) }

/// One frame's SSR inputs.
pub struct SsrFrame<'a> {
    /// The jittered render camera of the view.
    pub cam: &'a CameraParams,
    /// `r.SSR.Quality` of the Effects level.
    pub cvar_quality: i32,
    pub depth_view: &'a wgpu::TextureView,
    pub depth_gen: u64,
    pub gbuf_view: &'a wgpu::TextureView,
    /// The previous frame's temporal AA history (None = TAA off / no history / camera cut).
    pub taa_history: Option<&'a wgpu::TextureView>,
    /// Temporal AA is the view's AA method (no SSR temporal pass).
    pub taa_active: bool,
    pub velocity: Option<&'a wgpu::TextureView>,
    pub jitter_px: [f32; 2],
    pub frame_index: u64,
    pub exposure_buf: &'a wgpu::Buffer,
}

/// The screen space reflections: HZB, trace, SSR temporal filter (module docs).
pub struct Ssr {
    device: wgpu::Device,
    hzb0: wgpu::RenderPipeline,
    hzb_down: wgpu::RenderPipeline,
    hzb_buf: wgpu::Buffer,
    trace: wgpu::RenderPipeline,
    trace_buf: wgpu::Buffer,
    temporal: wgpu::RenderPipeline,
    temporal_buf: wgpu::Buffer,
    point: wgpu::Sampler,
    lin: wgpu::Sampler,
    dummy_velocity: wgpu::TextureView,
    black: wgpu::TextureView,
    hzb: Option<Hzb>,
    out: Option<Out>,
    // this frame
    /// UE SSAO: the HZB is built this frame (for the SSR and / or the screen-space AO, which also samples `View.HZB`)
    hzb_this_frame: bool,
    active: bool,
    temporal_on: bool,
    /// the trace reads the scene colour copy (no TAA history this frame)
    color_from_copy: bool,
    num_steps: u32,
    trace_bg: Option<wgpu::BindGroup>,
    temporal_bg: Option<wgpu::BindGroup>,
    parity: usize,
    history_valid: bool,
    prev_cam: Option<CameraParams>,
}

impl Ssr {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Ssr {
        let sm = |label: &str, src: &str| device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let hzb_sm = sm("hzb", fn_live::wgsl!("../shaders/hzb.wgsl"));
        let trace_sm = sm("ssr trace", fn_live::wgsl!("../shaders/ssr.wgsl"));
        let temporal_sm = sm("ssr temporal", fn_live::wgsl!("../shaders/ssr_temporal.wgsl"));
        let pipe = |label: &str, sm: &wgpu::ShaderModule, fs: &str, targets: &[Option<wgpu::ColorTargetState>]| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label), layout: None,
            vertex: wgpu::VertexState { module: sm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: sm, entry_point: Some(fs), compilation_options: Default::default(), targets }),
            multiview_mask: None, cache: None,
        });
        let t = |format| Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL });
        let buf = |label: &str, size: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        // `TStaticSamplerState<SF_Point, AM_Clamp, AM_Clamp, AM_Clamp>` (the HZB / scene colour inputs of the trace) and the bilinear clamp history sampler
        let point = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("ssr point clamp"), ..Default::default() });
        let lin = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("ssr bilinear clamp"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let tex1 = |label: &str, format: wgpu::TextureFormat, bytes: &[u8]| {
            let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
            queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes.len() as u32), rows_per_image: Some(1) }, wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 });
            tex.create_view(&Default::default())
        };
        Ssr {
            device: device.clone(),
            hzb0: pipe("hzb mip 0", &hzb_sm, "fs_mip0", &[t(HZB_FORMAT)]),
            hzb_down: pipe("hzb downsample", &hzb_sm, "fs_down", &[t(HZB_FORMAT)]),
            hzb_buf: buf("hzb params", std::mem::size_of::<HzbU>()),
            trace: pipe("ssr trace", &trace_sm, "fs_trace", &[t(SSR_FORMAT)]),
            trace_buf: buf("ssr params", std::mem::size_of::<SsrU>()),
            temporal: pipe("ssr temporal", &temporal_sm, "fs_temporal", &[t(SSR_FORMAT), t(SSR_FORMAT)]),
            temporal_buf: buf("ssr temporal params", std::mem::size_of::<TempU>()),
            point, lin,
            dummy_velocity: tex1("no velocity", crate::taa::VELOCITY_FORMAT, &[0u8; 8]),
            black: tex1("ssr black", SSR_FORMAT, &[0u8; 8]),
            hzb: None, out: None, hzb_this_frame: false, active: false, temporal_on: false, color_from_copy: false, num_steps: 16, trace_bg: None, temporal_bg: None, parity: 0, history_valid: false, prev_cam: None,
        }
    }

    /// SSR runs this frame.
    pub fn active(&self) -> bool { self.active }
    /// The SSR result the reflection apply reads (`SSROutput`); a black 1x1 texture while SSR is off (the apply's flag skips it then).
    pub fn output(&self) -> &wgpu::TextureView { self.out.as_ref().map_or(&self.black, |o| &o.final_) }
    /// Generation of [`Self::output`] (changes when the SSR targets are re-created).
    pub fn output_key(&self) -> Option<(u32, u32)> { self.out.as_ref().map(|o| o.size) }
    /// Forget the SSR history (camera cut).
    pub fn reset(&mut self) { self.history_valid = false; self.prev_cam = None; }
    /// UE SSAO: the HZB is built this frame (`encode_hzb` has work).
    pub fn hzb_this_frame(&self) -> bool { self.hzb_this_frame }
    /// OCCLUSION (2026-10-07): the HZB texture of this frame and its view size (the statics' occlusion readback, occlusion.rs).
    pub fn hzb_texture(&self) -> Option<(&wgpu::Texture, (u32, u32))> { self.hzb.as_ref().filter(|_| self.hzb_this_frame).map(|z| (&z.tex, z.view_size)) }
    /// UE SSAO: `View.HZB` (all mips) and its mip-0 size (`HZBMipmap0Size`), once [`Self::prepare_hzb`] ran this frame.
    pub fn hzb(&self) -> Option<(&wgpu::TextureView, (u32, u32))> {
        self.hzb.as_ref().filter(|_| self.hzb_this_frame).map(|z| (&z.all, hzb_size(z.view_size.0, z.view_size.1).0))
    }

    /// The HZB of this frame (`BuildHZB`, SceneOcclusion.cpp:1076): targets / bind groups (re-created when the view size or the depth target changes) and
    /// its uniform. Needed by the SSR trace and by the screen-space AO (`PostprocessInput3` = `View.HZB`, CompositionLighting.cpp:179).
    pub fn prepare_hzb(&mut self, queue: &wgpu::Queue, cam: &CameraParams, depth_view: &wgpu::TextureView, depth_gen: u64) {
        let (w, h) = (cam.width.max(1), cam.height.max(1));
        let d = self.device.clone();
        let (hsize, hmips) = hzb_size(w, h);
        if self.hzb.as_ref().map(|z| (z.view_size, z.depth_gen)) != Some(((w, h), depth_gen)) {
            let tex = d.create_texture(&wgpu::TextureDescriptor { label: Some("hzb"), size: wgpu::Extent3d { width: hsize.0, height: hsize.1, depth_or_array_layers: 1 }, mip_level_count: hmips, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: HZB_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC /* OCCLUSION readback */, view_formats: &[] });
            let mips: Vec<_> = (0..hmips).map(|m| tex.create_view(&wgpu::TextureViewDescriptor { label: Some("hzb mip"), base_mip_level: m, mip_level_count: Some(1), ..Default::default() })).collect();
            let mut bgs = vec![d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("hzb mip 0 bg"), layout: &self.hzb0.get_bind_group_layout(0), entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.hzb_buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth_view) },
            ] })];
            for m in 1..hmips as usize {
                bgs.push(d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("hzb down bg"), layout: &self.hzb_down.get_bind_group_layout(0), entries: &[
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&mips[m - 1]) },
                ] }));
            }
            self.hzb = Some(Hzb { view_size: (w, h), all: tex.create_view(&Default::default()), tex, mips, bgs, depth_gen });
        }
        queue.write_buffer(&self.hzb_buf, 0, bytemuck::bytes_of(&HzbU { depth: [cam.near, cam.far, UE_NEAR_M, 0.] }));
        self.hzb_this_frame = true;
    }

    /// Targets, uniforms and bind groups of this frame. `f.cvar_quality <= 0` (Effects Low / Medium) switches SSR off.
    pub fn prepare(&mut self, queue: &wgpu::Queue, f: &SsrFrame) {
        let q = crate::quality::ssr_shader_quality(f.cvar_quality, crate::quality::SSR_PP_QUALITY, crate::quality::SSR_PP_INTENSITY);
        self.active = q.is_some() && fn_live::var("FN_SSR").map_or(true, |v| v != "0");
        self.trace_bg = None;
        self.temporal_bg = None;
        self.hzb_this_frame = false;
        if !self.active { self.history_valid = false; self.prev_cam = None; return; }
        let quality = q.unwrap();
        // SSR_QUALITY 1: 8 steps, 2: 16 steps (ScreenSpaceReflections.usf:165-170); 3 / 4 are not ported (module docs)
        self.num_steps = if quality == 1 { 8 } else { 16 };
        let (w, h) = (f.cam.width.max(1), f.cam.height.max(1));
        let d = self.device.clone();
        // ---- HZB ----
        self.prepare_hzb(queue, f.cam, f.depth_view, f.depth_gen);
        // ---- SSR targets ----
        if self.out.as_ref().map(|o| o.size) != Some((w, h)) {
            let mk = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| d.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
            let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let raw = mk("ssr trace", SSR_FORMAT, rt);
            let raw_view = raw.create_view(&Default::default());
            let copy = mk("ssr scene colour copy", crate::renderer::HDR_FORMAT, wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING);
            let copy_view = copy.create_view(&Default::default());
            self.out = Some(Out { size: (w, h), raw: (raw, raw_view), final_: mk("ssr output", SSR_FORMAT, rt).create_view(&Default::default()),
                history: [mk("ssr history a", SSR_FORMAT, rt).create_view(&Default::default()), mk("ssr history b", SSR_FORMAT, rt).create_view(&Default::default())], scene_copy: (copy, copy_view) });
            self.history_valid = false;
        }
        // ---- uniforms ----
        // ClipToPrevClip with the no-AA matrices, previous matrices reset on a large camera move (as taa.rs)
        let mut prev = match &self.prev_cam { Some(p) if !crate::taa::is_large_camera_movement(f.cam, p) => *p, _ => { self.history_valid = false; *f.cam } };
        prev.width = f.cam.width; prev.height = f.cam.height;
        let vp = f.cam.proj_matrix_no_aa() * f.cam.view_matrix();
        let clip_to_prev = (prev.proj_matrix_no_aa() * prev.view_matrix()) * vp.inverse();
        let mut c = *f.cam; c.jitter = [0.; 2];
        self.prev_cam = Some(c);
        let p = f.cam.proj_matrix_no_aa();
        let velocity = f.velocity.is_some();
        queue.write_buffer(&self.trace_buf, 0, bytemuck::bytes_of(&SsrU {
            view: f.cam.view_matrix().to_cols_array_2d(), clip_to_prev: clip_to_prev.to_cols_array_2d(),
            proj: [p.x_axis.x, p.y_axis.y, f.cam.jitter[0], f.cam.jitter[1]],
            // SSRParams: Intensity * 0.01 clamped, RoughnessMaskScale, View.StateFrameIndexMod8, NumSteps
            params: [(crate::quality::SSR_PP_INTENSITY * 0.01).clamp(0., 1.), crate::quality::ssr_roughness_mask_scale(crate::quality::SSR_PP_MAX_ROUGHNESS, quality), (f.frame_index % 8) as f32, self.num_steps as f32],
            hzb: hzb_uv_factor(w, h), viewport: [w as f32, h as f32, 1. / w as f32, 1. / h as f32],
            depth: [f.cam.near, f.cam.far, UE_NEAR_M, (velocity as u32 | if debug_view() { 2 } else { 0 }) as f32],
        }));
        self.temporal_on = !f.taa_active;
        let o = self.out.as_ref().unwrap();
        let z = self.hzb.as_ref().unwrap();
        // colour source: the TAA history (PrevFrame) or the current scene colour (copied before the trace)
        let color = f.taa_history.unwrap_or(&o.scene_copy.1);
        let vel = f.velocity.unwrap_or(&self.dummy_velocity);
        self.trace_bg = Some(d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ssr trace bg"), layout: &self.trace.get_bind_group_layout(0), entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: self.trace_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(f.depth_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(f.gbuf_view) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&z.all) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.point) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(color) },
            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(vel) },
        ] }));
        self.color_from_copy = f.taa_history.is_none();
        if self.temporal_on {
            let wts = sample_weights(f.jitter_px);
            queue.write_buffer(&self.temporal_buf, 0, bytemuck::bytes_of(&TempU {
                clip_to_prev: clip_to_prev.to_cols_array_2d(),
                weights0: [wts[0], wts[1], wts[2], wts[3]], weights1: [wts[4], wts[5], wts[6], wts[7]],
                weights2: [wts[8], if self.history_valid { 1. } else { 0. }, if velocity { 1. } else { 0. }, 1.],
                viewport: [w as f32, h as f32, 1. / w as f32, 1. / h as f32],
            }));
            self.temporal_bg = Some(d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ssr temporal bg"), layout: &self.temporal.get_bind_group_layout(0), entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.temporal_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&o.raw.1) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&o.history[self.parity]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.lin) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(f.depth_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(vel) },
                wgpu::BindGroupEntry { binding: 6, resource: f.exposure_buf.as_entire_binding() },
            ] }));
        } else {
            self.history_valid = false;
        }
    }

    /// The HZB build of this frame (before the SSR trace and the screen-space AO). `ts` brackets every mip.
    pub fn encode_hzb(&self, enc: &mut wgpu::CommandEncoder, ts: Option<wgpu::RenderPassTimestampWrites<'_>>) {
        let Some(z) = self.hzb.as_ref().filter(|_| self.hzb_this_frame) else { return };
        let (ts_begin, ts_end) = split_timestamps(ts);
        let n = z.mips.len();
        for m in 0..n {
            let ts = match (m == 0, m + 1 == n) {
                (true, true) => merge_timestamps(&ts_begin, &ts_end),
                (true, false) => ts_begin.as_ref().map(|t| wgpu::RenderPassTimestampWrites { query_set: t.query_set, beginning_of_pass_write_index: t.beginning_of_pass_write_index, end_of_pass_write_index: None }),
                (false, true) => ts_end.as_ref().map(|t| wgpu::RenderPassTimestampWrites { query_set: t.query_set, beginning_of_pass_write_index: None, end_of_pass_write_index: t.end_of_pass_write_index }),
                _ => None,
            };
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some(if m == 0 { "hzb mip 0" } else { "hzb downsample" }),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &z.mips[m], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: ts, occlusion_query_set: None, multiview_mask: None });
            p.set_pipeline(if m == 0 { &self.hzb0 } else { &self.hzb_down });
            p.set_bind_group(0, &z.bgs[m], &[]);
            p.draw(0..3, 0..1);
        }
    }

    /// (Scene colour copy) + trace + (temporal filter); the HZB is built before by [`Self::encode_hzb`]. `hdr` = the scene colour texture (mip 0 is
    /// copied when there is no TAA history).
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, hdr: &wgpu::Texture, ts: Option<wgpu::RenderPassTimestampWrites<'_>>) {
        let (Some(o), Some(trace_bg)) = (self.out.as_ref(), self.trace_bg.as_ref()) else { return };
        if !self.active { return; }
        let pass = |enc: &mut wgpu::CommandEncoder, label: &str, views: &[&wgpu::TextureView], pipe: &wgpu::RenderPipeline, bg: &wgpu::BindGroup, ts: Option<wgpu::RenderPassTimestampWrites<'_>>| {
            let att: Vec<_> = views.iter().map(|v| Some(wgpu::RenderPassColorAttachment { view: v, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })).collect();
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some(label), color_attachments: &att, depth_stencil_attachment: None, timestamp_writes: ts, occlusion_query_set: None, multiview_mask: None });
            p.set_pipeline(pipe);
            p.set_bind_group(0, bg, &[]);
            p.draw(0..3, 0..1);
        };
        // timestamps bracket the trace (+ temporal filter): the first pass starts it, the last one ends it
        let (ts_begin, ts_end) = split_timestamps(ts);
        if self.color_from_copy {
            enc.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: hdr, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo { texture: &o.scene_copy.0, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: o.size.0, height: o.size.1, depth_or_array_layers: 1 },
            );
        }
        match (&self.temporal_bg, self.temporal_on) {
            (Some(tbg), true) => {
                pass(enc, "ssr trace", &[&o.raw.1], &self.trace, trace_bg, ts_begin);
                pass(enc, "ssr temporal", &[&o.history[1 - self.parity], &o.final_], &self.temporal, tbg, ts_end);
            }
            _ => pass(enc, "ssr trace", &[&o.final_], &self.trace, trace_bg, merge_timestamps(&ts_begin, &ts_end)),
        }
    }

    /// After the frame was submitted: the written SSR history becomes the previous one.
    pub fn end_frame(&mut self) {
        if self.active && self.temporal_on && self.temporal_bg.is_some() { self.parity = 1 - self.parity; self.history_valid = true; }
    }
}

/// Splits one pass's timestamp writes into a "begin only" and an "end only" half (a node made of several render passes).
pub(crate) fn split_timestamps<'a>(ts: Option<wgpu::RenderPassTimestampWrites<'a>>) -> (Option<wgpu::RenderPassTimestampWrites<'a>>, Option<wgpu::RenderPassTimestampWrites<'a>>) {
    match ts {
        Some(t) => (Some(wgpu::RenderPassTimestampWrites { query_set: t.query_set, beginning_of_pass_write_index: t.beginning_of_pass_write_index, end_of_pass_write_index: None }),
            Some(wgpu::RenderPassTimestampWrites { query_set: t.query_set, beginning_of_pass_write_index: None, end_of_pass_write_index: t.end_of_pass_write_index })),
        None => (None, None),
    }
}
/// The begin of `a` and the end of `b` on one pass (a node that turned out to be a single render pass).
pub(crate) fn merge_timestamps<'a>(a: &Option<wgpu::RenderPassTimestampWrites<'a>>, b: &Option<wgpu::RenderPassTimestampWrites<'a>>) -> Option<wgpu::RenderPassTimestampWrites<'a>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(wgpu::RenderPassTimestampWrites { query_set: a.query_set, beginning_of_pass_write_index: a.beginning_of_pass_write_index, end_of_pass_write_index: b.end_of_pass_write_index }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hzb_size_follows_build_hzb() {
        // 1920x1080: ceil(log2) - 1 = 10 / 10 -> 1024 x 1024, 10 mips; 1280x720: 1024 x 512, 10 mips
        assert_eq!(hzb_size(1920, 1080), ((1024, 1024), 10));
        assert_eq!(hzb_size(1280, 720), ((1024, 512), 10));
        assert_eq!(hzb_size(2, 2), ((2, 2), 1));
        let f = hzb_uv_factor(1280, 720);
        assert!((f[0] - 0.625).abs() < 1e-6 && (f[1] - 0.703125).abs() < 1e-6 && (f[2] * f[0] - 1.).abs() < 1e-6);
    }
    #[test]
    fn sample_weights_are_normalised_and_centred_without_jitter() {
        let w = sample_weights([0., 0.]);
        assert!((w.iter().sum::<f32>() - 1.).abs() < 1e-6);
        assert!(w[4] > w[1] && (w[1] - w[3]).abs() < 1e-7 && (w[0] - w[8]).abs() < 1e-7);
        let j = sample_weights([0.4, 0.]);
        assert!(j[5] > j[3]);
    }
    #[test]
    fn lit_targets_fit_the_attachment_budget() {
        // wgpu's default max_color_attachment_bytes_per_sample is 32: HDR + 3 x Rgba16Float = 4 x 8 bytes
        let t = lit_targets(crate::renderer::HDR_FORMAT);
        let bytes: u32 = t.iter().map(|c| c.as_ref().unwrap().format.target_pixel_byte_cost().unwrap()).sum();
        assert!(bytes <= wgpu::Limits::default().max_color_attachment_bytes_per_sample, "{bytes}");
    }
    /// Every pipeline of the reflection apply validates (the uncaptured-error handler of `Gpu::headless` panics on an invalid module / pipeline): HZB, trace,
    /// SSR temporal, the AO + reflection apply, and the lit modules that now return `LitOut`.
    #[test]
    fn reflection_pipelines_validate() {
        let Ok(g) = crate::Gpu::headless() else { eprintln!("no GPU adapter: skipped"); return };
        let _ssr = Ssr::new(&g.device, &g.queue);
        let _apply = crate::ssao_pass::SsaoPass::new(&g.device, fn_live::wgsl!("../shaders/common.wgsl"), crate::renderer::HDR_FORMAT);
        let (common, lit) = (fn_live::wgsl!("../shaders/common.wgsl"), fn_live::wgsl!("../shaders/lit_common.wgsl"));
        // (char_scene.wgsl: char_material.rs's own test builds CharGpu; the gun programs: fn-matshader)
        let tc = fn_live::wgsl!("../shaders/terrain_common.wgsl"); // TERRAIN MAIN PASS: the shared landscape shading of the backdrop and the main-pass terrain
        for (label, parts) in [("backdrop", vec![common, lit, tc, fn_live::wgsl!("../shaders/backdrop.wgsl")]), ("terrain", vec![common, lit, tc, fn_live::wgsl!("../shaders/terrain.wgsl")]), ("gun resolve", vec![common, lit, fn_live::wgsl!("../shaders/gun_resolve.wgsl")])] {
            let _ = g.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(parts.join("\n").into()) });
        }
        g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
    #[test]
    fn camera_view_matrix_maps_forward_to_minus_z() {
        // the trace converts the renderer's view space (-z forward) to UE's (z = view depth): N_ue = (x, y, -z)
        let c = CameraParams::from_yaw_pitch(glam::Vec3::new(1., 2., 3.), 40., -10., 1280, 720);
        let f = c.view_matrix().transform_vector3(c.forward);
        assert!((f - glam::Vec3::NEG_Z).length() < 1e-5);
    }
}
