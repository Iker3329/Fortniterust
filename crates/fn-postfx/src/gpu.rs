//! wgpu passes: [`PostFx`] encodes Gaussian DOF (far layer), tiled motion blur and the bloom chain.
//!
//! Chain position (UE 4.16 `PostProcessing.cpp` order): `scene colour (HDR, before exposure/tonemap) -> DOF -> motion blur -> bloom setup/chain
//! -> tonemapper (scene * tint + bloom * intensity, then exposure, vignette, LUT)`. See the crate README for the integration contract.
use crate::cpu::{gaussian_taps, half};
use crate::params::*;
use bytemuck::{Pod, Zeroable};

/// Render-target formats the passes use.
#[derive(Clone, Copy, Debug)]
pub struct PostFxFormats {
    /// Format of the HDR scene colour / outputs (`Rgba16Float` in fn-render).
    pub hdr: wgpu::TextureFormat,
}
impl Default for PostFxFormats {
    fn default() -> Self { PostFxFormats { hdr: wgpu::TextureFormat::Rgba16Float } }
}

/// Textures the passes read. All views must be 2D, mip 0, size = `params.view.{width,height}`.
pub struct PostFxInputs<'a> {
    /// Linear HDR scene colour, `TEXTURE_BINDING`, filterable.
    pub scene_color: &'a wgpu::TextureView,
    /// Depth-only view of the scene depth buffer (`TextureAspect::DepthOnly`, `TEXTURE_BINDING`), standard or reversed Z as described by
    /// `params.depth`.
    pub depth: &'a wgpu::TextureView,
    /// Optional dynamic-object velocity, `Rgba16Float`: rg = NDC displacement `cur - prev` (y up), a = 1 where written, 0 elsewhere.
    pub velocity: Option<&'a wgpu::TextureView>,
    pub params: &'a PostFxParams,
}

/// Targets the passes write (`RENDER_ATTACHMENT`, format = `PostFxFormats::hdr`).
pub struct PostFxOutputs<'a> {
    /// Receives the post-DOF/motion-blur colour (full size) when `PostFxResult::color_written`.
    pub color: &'a wgpu::TextureView,
    /// Receives the bloom image (`half(width) x half(height)`, premultiplied by the tints, NOT by `BloomIntensity`) when `Some`.
    pub bloom: Option<&'a wgpu::TextureView>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PostFxResult {
    /// False = neither DOF nor motion blur ran: keep using `scene_color`, `outputs.color` was not touched.
    pub color_written: bool,
    pub bloom_written: bool,
    pub passes: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DofU {
    depth_xf: [f32; 4],
    dist: [f32; 4],
    rows: [[f32; 4]; 3],
    clamp_rect: [f32; 4],
    inv_size: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlurU {
    taps: [[f32; 4]; 32],
    dir_texel: [f32; 4],
    tint: [f32; 4],
    flags: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MbU {
    size: [f32; 4],
    tile_inv: [f32; 4],
    rect: [u32; 4],
    edge: [f32; 4],
    p18: [f32; 4],
    p19: [f32; 4],
    depth_xf: [f32; 4],
    misc: [f32; 4],
    clip_to_prev: [[f32; 4]; 4],
    flags: [u32; 4],
    tile_grid: [u32; 4],
}

struct Tex {
    #[allow(dead_code)]
    tex: wgpu::Texture,
    view: wgpu::TextureView,
}

struct Targets {
    w: u32,
    h: u32,
    mid: Tex,
    far_a: Tex,
    far_b: Tex,
    flat: Tex,
    tiles_a: Tex,
    tiles_b: Tex,
    tile_grid: (u32, u32),
    dof_u: wgpu::Buffer,
    blur_u: Vec<wgpu::Buffer>,
    mb_u: wgpu::Buffer,
}

/// GPU time of one pass in milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct PassTime {
    pub name: &'static str,
    pub ms: f32,
}

struct Timer {
    qs: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    staging: wgpu::Buffer,
    names: Vec<&'static str>,
    period_ns: f32,
}
const MAX_TIMED: u32 = 48;

pub struct PostFx {
    device: wgpu::Device,
    formats: PostFxFormats,
    samp: wgpu::Sampler,
    dummy: Tex,
    // layouts
    bgl_dof: wgpu::BindGroupLayout,
    bgl_blur: wgpu::BindGroupLayout,
    bgl_flatten: wgpu::BindGroupLayout,
    bgl_gather: wgpu::BindGroupLayout,
    bgl_mb: wgpu::BindGroupLayout,
    // pipelines
    dof_setup: wgpu::RenderPipeline,
    dof_recombine: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    bloom_chain: crate::bloom::BloomChain,
    bloom_gen: u64,
    mb_flatten: wgpu::ComputePipeline,
    mb_gather: wgpu::ComputePipeline,
    mb_blur: wgpu::RenderPipeline,
    targets: Option<Targets>,
    timer: Option<Timer>,
}

fn shader(device: &wgpu::Device, label: &str, parts: &[&str]) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(parts.concat().into()) })
}

fn ent(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty, count: None }
}
fn uniform(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    ent(binding, vis, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None })
}
fn tex(binding: u32, vis: wgpu::ShaderStages, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    ent(binding, vis, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false })
}
fn depth_tex(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    ent(binding, vis, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false })
}
fn sampler(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    ent(binding, vis, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering))
}
fn storage(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    ent(binding, wgpu::ShaderStages::COMPUTE, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 })
}
fn be(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> { wgpu::BindGroupEntry { binding, resource } }

fn make_tex(device: &wgpu::Device, label: &str, w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> Tex {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    Tex { tex, view }
}

fn ubuf(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
}

impl PostFx {
    /// Compiles every pipeline. `device` must support `rgba32float` storage writes (all Vulkan/DX12 adapters do).
    pub fn new(device: &wgpu::Device, formats: PostFxFormats) -> PostFx {
        let common = fn_live::wgsl!("../shaders/common.wgsl");
        let frag = wgpu::ShaderStages::FRAGMENT;
        let comp = wgpu::ShaderStages::COMPUTE;
        let rt = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let bgl = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries });
        let bgl_dof = bgl("postfx dof", &[uniform(0, frag), depth_tex(1, frag), tex(2, frag, true), sampler(3, frag), tex(4, frag, true)]);
        let bgl_blur = bgl("postfx blur", &[uniform(0, frag), tex(1, frag, true), tex(2, frag, true), sampler(3, frag)]);
        let bgl_flatten = bgl("postfx flatten", &[uniform(0, comp), tex(1, comp, false), depth_tex(2, comp), storage(3, wgpu::TextureFormat::Rgba32Float), storage(4, wgpu::TextureFormat::Rgba32Float)]);
        let bgl_gather = bgl("postfx gather", &[uniform(0, comp), storage(4, wgpu::TextureFormat::Rgba32Float), tex(5, comp, false)]);
        let bgl_mb = bgl("postfx motion blur", &[uniform(0, frag), tex(5, frag, false), tex(6, frag, true), tex(7, frag, false), sampler(8, frag)]);
        let pl = |label: &str, b: &wgpu::BindGroupLayout| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(b)], immediate_size: 0 });
        let dof_sm = shader(device, "postfx dof", &[common, fn_live::wgsl!("../shaders/dof.wgsl")]);
        let blur_sm = shader(device, "postfx blur", &[common, fn_live::wgsl!("../shaders/blur.wgsl")]);
        let mb_sm = shader(device, "postfx motion blur", &[common, fn_live::wgsl!("../shaders/mb.wgsl")]);
        let fullscreen = |label: &str, sm: &wgpu::ShaderModule, fs: &str, layout: &wgpu::PipelineLayout, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: sm, entry_point: Some("vs_fullscreen"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState { module: sm, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let compute = |label: &str, sm: &wgpu::ShaderModule, entry: &str, layout: &wgpu::PipelineLayout| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(layout), module: sm, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
        };
        let (l_dof, l_blur, l_flat, l_gather, l_mb) = (pl("dof", &bgl_dof), pl("blur", &bgl_blur), pl("flatten", &bgl_flatten), pl("gather", &bgl_gather), pl("mb", &bgl_mb));
        let h = formats.hdr;
        let samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("postfx linear clamp"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let dummy = make_tex(device, "postfx dummy", 1, 1, formats.hdr, rt);
        PostFx {
            device: device.clone(),
            formats,
            samp,
            dummy,
            dof_setup: fullscreen("dof setup far", &dof_sm, "fs_setup_far", &l_dof, h),
            dof_recombine: fullscreen("dof recombine far", &dof_sm, "fs_recombine_far", &l_dof, h),
            blur: fullscreen("gaussian blur", &blur_sm, "fs_blur", &l_blur, h),
            bloom_chain: crate::bloom::BloomChain::new(device, formats.hdr),
            bloom_gen: 0,
            mb_flatten: compute("velocity flatten", &mb_sm, "cs_flatten", &l_flat),
            mb_gather: compute("velocity gather", &mb_sm, "cs_gather", &l_gather),
            mb_blur: fullscreen("motion blur", &mb_sm, "fs_motion_blur", &l_mb, h),
            bgl_dof,
            bgl_blur,
            bgl_flatten,
            bgl_gather,
            bgl_mb,
            targets: None,
            timer: None,
        }
    }

    /// Intermediate motion-blur textures of the last `encode` (flattened velocity full-res, tile min/max, dilated tiles; all `Rgba32Float`).
    #[doc(hidden)]
    pub fn debug_motion_textures(&self) -> Option<(&wgpu::Texture, &wgpu::Texture, &wgpu::Texture)> {
        self.targets.as_ref().map(|t| (&t.flat.tex, &t.tiles_a.tex, &t.tiles_b.tex))
    }

    /// Starts measuring GPU pass times (needs `Features::TIMESTAMP_QUERY`; returns false when unsupported).
    pub fn enable_timing(&mut self, queue: &wgpu::Queue) -> bool {
        if !self.device.features().contains(wgpu::Features::TIMESTAMP_QUERY) || queue.get_timestamp_period() <= 0.0 { return false; }
        let size = MAX_TIMED as u64 * 16;
        self.timer = Some(Timer {
            qs: self.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("postfx timestamps"), ty: wgpu::QueryType::Timestamp, count: MAX_TIMED * 2 }),
            resolve: self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("postfx ts resolve"), size, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
            staging: self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("postfx ts staging"), size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false }),
            names: vec![],
            period_ns: queue.get_timestamp_period(),
        });
        true
    }

    /// Blocks until the last submitted frame finished and returns the GPU time of each pass of the last `encode` (empty if timing is off).
    pub fn read_timings(&mut self) -> Vec<PassTime> {
        let Some(t) = &mut self.timer else { return vec![] };
        let n = t.names.len();
        if n == 0 { return vec![]; }
        let bytes = n as u64 * 16;
        let slice = t.staging.slice(..bytes);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        if rx.recv().ok().and_then(|r| r.ok()).is_none() { return vec![]; }
        let ticks: Vec<u64> = { let d = slice.get_mapped_range().expect("range"); bytemuck::cast_slice::<u8, u64>(&d).to_vec() };
        t.staging.unmap();
        t.names.iter().enumerate().map(|(i, name)| PassTime { name, ms: ticks[2 * i + 1].saturating_sub(ticks[2 * i]) as f32 * t.period_ns * 1e-6 }).collect()
    }

    fn ensure_targets(&mut self, w: u32, h: u32) {
        if self.targets.as_ref().is_some_and(|t| t.w == w && t.h == h) { return; }
        let d = &self.device;
        let rt = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let st = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC;
        let hdr = self.formats.hdr;
        let (hw, hh) = (half(w as usize) as u32, half(h as usize) as u32);
        let tile_grid = (w.div_ceil(16), h.div_ceil(16));
        self.targets = Some(Targets {
            w,
            h,
            mid: make_tex(d, "postfx mid", w, h, hdr, rt),
            far_a: make_tex(d, "dof far a", hw, hh, hdr, rt),
            far_b: make_tex(d, "dof far b", hw, hh, hdr, rt),
            flat: make_tex(d, "velocity flat", w, h, wgpu::TextureFormat::Rgba32Float, st),
            tiles_a: make_tex(d, "velocity tiles a", tile_grid.0, tile_grid.1, wgpu::TextureFormat::Rgba32Float, st),
            tiles_b: make_tex(d, "velocity tiles b", tile_grid.0, tile_grid.1, wgpu::TextureFormat::Rgba32Float, st),
            tile_grid,
            dof_u: ubuf(d, "dof u", std::mem::size_of::<DofU>()),
            blur_u: (0..16).map(|i| ubuf(d, &format!("blur u {i}"), std::mem::size_of::<BlurU>())).collect(),
            mb_u: ubuf(d, "mb u", std::mem::size_of::<MbU>()),
        });
    }

    fn slot(&mut self, name: &'static str) -> Option<u32> {
        let t = self.timer.as_mut()?;
        if t.names.len() as u32 >= MAX_TIMED { return None; }
        t.names.push(name);
        Some((t.names.len() as u32 - 1) * 2)
    }

    fn fullscreen_pass(&mut self, enc: &mut wgpu::CommandEncoder, name: &'static str, pipeline: &wgpu::RenderPipeline, bg: &wgpu::BindGroup, target: &wgpu::TextureView) {
        let slot = self.slot(name);
        let ts = slot.zip(self.timer.as_ref()).map(|(s, t)| wgpu::RenderPassTimestampWrites { query_set: &t.qs, beginning_of_pass_write_index: Some(s), end_of_pass_write_index: Some(s + 1) });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(name),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: ts,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bg, &[]);
        pass.draw(0..3, 0..1);
    }

    fn compute_pass(&mut self, enc: &mut wgpu::CommandEncoder, name: &'static str, pipeline: &wgpu::ComputePipeline, bg: &wgpu::BindGroup, groups: (u32, u32)) {
        let slot = self.slot(name);
        let ts = slot.zip(self.timer.as_ref()).map(|(s, t)| wgpu::ComputePassTimestampWrites { query_set: &t.qs, beginning_of_pass_write_index: Some(s), end_of_pass_write_index: Some(s + 1) });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(name), timestamp_writes: ts });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bg, &[]);
        pass.dispatch_workgroups(groups.0, groups.1, 1);
    }

    /// Records every active effect into `enc` (uniforms are written with `queue.write_buffer`, i.e. they land before the submit that
    /// carries `enc`; call once per frame per `PostFx`). See [`PostFxInputs`] / [`PostFxOutputs`] / [`PostFxResult`].
    pub fn encode(&mut self, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, inputs: &PostFxInputs, outputs: &PostFxOutputs) -> PostFxResult {
        let p = inputs.params;
        let (w, h) = (p.view.width, p.view.height);
        self.ensure_targets(w, h);
        if let Some(t) = &mut self.timer { t.names.clear(); }
        let mut res = PostFxResult::default();
        let dof_on = p.dof.enabled && p.dof.far_active();
        let mb_on = p.motion_blur.enabled && p.motion_blur.amount > 0.001 && p.motion_blur.max_percent > 0.001;
        let bloom_on = p.bloom.enabled && outputs.bloom.is_some();
        // The Targets are borrowed from self while passes also need &mut self for timing; take them out for the duration of the call.
        let t = self.targets.take().unwrap();
        let samp = self.samp.clone();
        let mut cur: &wgpu::TextureView = inputs.scene_color;

        // ---------------------------------------------------------------- depth of field
        if dof_on {
            let dev = self.device.clone();
            let (hw, hh) = (t.far_a.tex.width(), t.far_a.tex.height());
            let u = DofU {
                depth_xf: [p.depth.x, p.depth.y, p.depth.z, p.depth.w],
                dist: [p.dof.focal_distance + p.dof.focal_region, p.dof.far_transition.max(1.0e-4), if p.dof.sky_focus_distance > 0.0 { p.dof.sky_focus_distance } else { 1.0e30 }, p.cm_per_unit],
                rows: p.view.screen_to_world_rows,
                clamp_rect: [0.5 / hw as f32, 0.5 / hh as f32, 1.0 - 0.5 / hw as f32, 1.0 - 0.5 / hh as f32],
                inv_size: [1.0 / w as f32, 1.0 / h as f32, 1.0 / hw as f32, 1.0 / hh as f32],
            };
            queue.write_buffer(&t.dof_u, 0, bytemuck::bytes_of(&u));
            let bg_setup = dev.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("dof setup bg"), layout: &self.bgl_dof, entries: &[be(0, t.dof_u.as_entire_binding()), be(1, wgpu::BindingResource::TextureView(inputs.depth)), be(2, wgpu::BindingResource::TextureView(cur)), be(3, wgpu::BindingResource::Sampler(&samp)), be(4, wgpu::BindingResource::TextureView(&self.dummy.view))] });
            let radius = crate::cpu::dof_blur_radius(p.dof.far_blur_size, hw as usize);
            let taps = gaussian_taps(radius);
            let mk_blur = |idx: usize, dir: [f32; 2], inv: [f32; 2]| {
                let mut bu = BlurU { taps: [[0.0; 4]; 32], dir_texel: [dir[0] * inv[0], dir[1] * inv[1], inv[0], inv[1]], tint: [1.0; 4], flags: [0, taps.len() as u32, 0, 0] };
                for (i, tp) in taps.iter().enumerate() { bu.taps[i] = [tp[0], tp[1], 0.0, 0.0]; }
                queue.write_buffer(&t.blur_u[idx], 0, bytemuck::bytes_of(&bu));
            };
            let inv = [1.0 / hw as f32, 1.0 / hh as f32];
            mk_blur(0, [1.0, 0.0], inv);
            mk_blur(1, [0.0, 1.0], inv);
            let bg_h = dev.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("dof blur h"), layout: &self.bgl_blur, entries: &[be(0, t.blur_u[0].as_entire_binding()), be(1, wgpu::BindingResource::TextureView(&t.far_a.view)), be(2, wgpu::BindingResource::TextureView(&t.far_a.view)), be(3, wgpu::BindingResource::Sampler(&samp))] });
            let bg_v = dev.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("dof blur v"), layout: &self.bgl_blur, entries: &[be(0, t.blur_u[1].as_entire_binding()), be(1, wgpu::BindingResource::TextureView(&t.far_b.view)), be(2, wgpu::BindingResource::TextureView(&t.far_b.view)), be(3, wgpu::BindingResource::Sampler(&samp))] });
            let bg_rec = dev.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("dof recombine bg"), layout: &self.bgl_dof, entries: &[be(0, t.dof_u.as_entire_binding()), be(1, wgpu::BindingResource::TextureView(inputs.depth)), be(2, wgpu::BindingResource::TextureView(cur)), be(3, wgpu::BindingResource::Sampler(&samp)), be(4, wgpu::BindingResource::TextureView(&t.far_a.view))] });
            let (pl_setup, pl_blur, pl_rec) = (self.dof_setup.clone(), self.blur.clone(), self.dof_recombine.clone());
            self.fullscreen_pass(enc, "dof setup", &pl_setup, &bg_setup, &t.far_a.view);
            self.fullscreen_pass(enc, "dof blur h", &pl_blur, &bg_h, &t.far_b.view);
            self.fullscreen_pass(enc, "dof blur v", &pl_blur, &bg_v, &t.far_a.view);
            let dst = if mb_on { &t.mid.view } else { outputs.color };
            self.fullscreen_pass(enc, "dof recombine", &pl_rec, &bg_rec, dst);
            cur = dst;
            res.color_written = true;
            res.passes += 4;
        }

        // ---------------------------------------------------------------- motion blur
        if mb_on {
            let dev = self.device.clone();
            let mbp = MbU::new(p, &t, inputs.velocity.is_some());
            queue.write_buffer(&t.mb_u, 0, bytemuck::bytes_of(&mbp));
            let vel_view = inputs.velocity.unwrap_or(&self.dummy.view);
            let bg_flat = dev.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("flatten bg"), layout: &self.bgl_flatten, entries: &[be(0, t.mb_u.as_entire_binding()), be(1, wgpu::BindingResource::TextureView(vel_view)), be(2, wgpu::BindingResource::TextureView(inputs.depth)), be(3, wgpu::BindingResource::TextureView(&t.flat.view)), be(4, wgpu::BindingResource::TextureView(&t.tiles_a.view))] });
            let bg_gather = dev.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("gather bg"), layout: &self.bgl_gather, entries: &[be(0, t.mb_u.as_entire_binding()), be(4, wgpu::BindingResource::TextureView(&t.tiles_b.view)), be(5, wgpu::BindingResource::TextureView(&t.tiles_a.view))] });
            let bg_mb = dev.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("motion blur bg"), layout: &self.bgl_mb, entries: &[be(0, t.mb_u.as_entire_binding()), be(5, wgpu::BindingResource::TextureView(&t.tiles_b.view)), be(6, wgpu::BindingResource::TextureView(cur)), be(7, wgpu::BindingResource::TextureView(&t.flat.view)), be(8, wgpu::BindingResource::Sampler(&samp))] });
            let (pl_flat, pl_gather, pl_mb) = (self.mb_flatten.clone(), self.mb_gather.clone(), self.mb_blur.clone());
            self.compute_pass(enc, "velocity flatten", &pl_flat, &bg_flat, (t.tile_grid.0, t.tile_grid.1));
            self.compute_pass(enc, "velocity gather", &pl_gather, &bg_gather, (t.tile_grid.0.div_ceil(16), t.tile_grid.1.div_ceil(16)));
            self.fullscreen_pass(enc, "motion blur", &pl_mb, &bg_mb, outputs.color);
            cur = outputs.color;
            res.color_written = true;
            res.passes += 3;
        }

        // ---------------------------------------------------------------- bloom (bloom.rs: the exact 4.16 / 1.8 chain)
        if bloom_on {
            self.bloom_gen += 1; // `cur` may be a different view every call: rebuild the chain's bind groups
            self.bloom_chain.prepare(queue, cur, w, h, self.bloom_gen, &p.bloom, p.exposure);
            let n = self.bloom_chain.plan().map_or(0, |pl| 6 + pl.setup as u32 + 2 * pl.stages.len() as u32);
            // (not timed per pass: the chain records its own passes)
            res.bloom_written = self.bloom_chain.encode_with_final(enc, outputs.bloom);
            res.passes += n;
        }
        // ---------------------------------------------------------------- timestamp resolve
        if let Some(tm) = &self.timer {
            let n = tm.names.len() as u32;
            if n > 0 {
                enc.resolve_query_set(&tm.qs, 0..n * 2, &tm.resolve, 0);
                enc.copy_buffer_to_buffer(&tm.resolve, 0, &tm.staging, 0, n as u64 * 16);
            }
        }
        self.targets = Some(t);
        res
    }
}

impl MbU {
    fn new(p: &PostFxParams, t: &Targets, has_velocity: bool) -> MbU {
        let u = crate::cpu::MbUniforms::new(p);
        let (w, h) = (p.view.width, p.view.height);
        MbU {
            size: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
            tile_inv: [0.0, 0.0, 1.0 / t.tile_grid.0 as f32, 1.0 / t.tile_grid.1 as f32],
            rect: [0, 0, w, h],
            edge: [u.edge, 0.0, 0.0, 0.0],
            p18: [u.aspect, u.scale, u.scale / 16.0, u.max_px],
            p19: [0.0, u.scale, 0.0, u.max_px],
            depth_xf: [p.depth.x, p.depth.y, p.depth.z, p.depth.w],
            misc: [p.cm_per_unit, 0.0, 0.0, 0.0],
            clip_to_prev: u.clip_to_prev.to_cols_array_2d(),
            flags: [has_velocity as u32, 0, 0, 0],
            tile_grid: [t.tile_grid.0, t.tile_grid.1, 0, 0],
        }
    }
}

