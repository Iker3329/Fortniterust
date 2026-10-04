use crate::gen_tables::*;
use crate::materials::{gpu_material, MaterialMode, ShaderId, TexSrc, RADIAL_PROGRESS};
use crate::textures::*;
use bytemuck::{Pod, Zeroable};
use fn_ui::assets::UiAssets;
use fn_ui::draw::{Cmd, MaterialCmd, RenderList, TextureId};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::sync::Arc;

/// Maximum number of baked material textures kept (LRU eviction beyond this; entries used this frame are never evicted).
pub const BAKE_CACHE_CAP: usize = 400;
const MAT_SLOT: u64 = 4096; // bytes bound per material draw (256 x vec4)

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QuadInst { p01: [f32; 4], p23: [f32; 4], uv: [f32; 4], col: [f32; 4] }

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum TexRef { Store(u32), Baked((u64, u32, u32)) }

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum TexKey { Dummy, Embedded(u32), Store(u32, u32) }

enum Op {
    Scissor([u32; 4]),
    Quads { tex: TexRef, first: u32, count: u32 },
    Mat { shader: ShaderId, offset: u32, tex4: [TexKey; 4] },
}

/// Counters of the last `render` call (used by tests, handy for HUD overlays).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats { pub quads: u32, pub draw_calls: u32, pub gpu_materials: u32, pub baked_materials: u32, pub bake_cache_entries: u32, pub texture_uploads: u32 }

pub struct UiRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    max_dim: u32,
    adapter_info: Option<wgpu::AdapterInfo>,
    _instance: Option<wgpu::Instance>,
    frame_buf: wgpu::Buffer,
    frame_bg: wgpu::BindGroup,
    quad_tex_bgl: wgpu::BindGroupLayout,
    quad_pipeline: wgpu::RenderPipeline,
    samplers: Samplers,
    quad_tex: QuadTexMap,
    baked: HashMap<(u64, u32, u32), (GpuTex, u64)>,
    straight: HashMap<u32, StraightTex>,
    embedded: Vec<Option<StraightTex>>,
    dummy: StraightTex,
    inst_buf: wgpu::Buffer,
    inst_cap: u64,
    mat_buf: wgpu::Buffer,
    mat_cap: u64,
    mat_bgl: wgpu::BindGroupLayout,
    mat_bg: wgpu::BindGroup,
    tex4_bgl: wgpu::BindGroupLayout,
    mat_layout: wgpu::PipelineLayout,
    mat_pipes: HashMap<ShaderId, Option<wgpu::RenderPipeline>>,
    tex4_cache: HashMap<[TexKey; 4], wgpu::BindGroup>,
    mode: MaterialMode,
    frame: u64,
    stats: FrameStats,
}

fn shader(device: &wgpu::Device, label: &str, src: String) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) })
}

fn premul_blend() -> wgpu::BlendState {
    let c = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
    wgpu::BlendState { color: c, alpha: c }
}

fn make_straight(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, w: u32, h: u32, rgba: &[u8], version: u32) -> StraightTex {
    let texture = create_rgba8(device, label, w, h, 1);
    write_level(queue, &texture, 0, w, h, rgba);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    StraightTex { _texture: texture, view, version }
}

impl UiRenderer {
    /// Own Instance / Adapter (software fallback if no hardware adapter) / Device / Queue; renders offscreen into `Rgba8Unorm`.
    pub fn new_headless() -> Result<UiRenderer, String> {
        let backends = match std::env::var("FN_BACKEND").map(|s| s.to_ascii_lowercase()).as_deref() {
            Ok("vulkan") | Ok("vk") => wgpu::Backends::VULKAN,
            Ok("dx12") | Ok("d3d12") => wgpu::Backends::DX12,
            _ => wgpu::Backends::VULKAN | wgpu::Backends::DX12,
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let mut adapter = None;
        let mut last_err = String::new();
        // FN_UI_SOFTWARE=1 prefers the software adapter (WARP / lavapipe) first, e.g. to exercise a second implementation in tests
        let order = if std::env::var_os("FN_UI_SOFTWARE").is_some() { [true, false] } else { [false, true] };
        for fallback in order {
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, force_fallback_adapter: fallback, compatible_surface: None, apply_limit_buckets: false })) {
                Ok(a) => { adapter = Some(a); break; }
                Err(e) => last_err = format!("{e}"),
            }
        }
        let adapter = adapter.ok_or_else(|| format!("no wgpu adapter (hardware or software): {last_err}"))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("fn-ui-wgpu device"), required_features: wgpu::Features::empty(), required_limits: wgpu::Limits::default().using_resolution(adapter.limits()), ..Default::default() }))
            .map_err(|e| format!("request_device failed: {e}"))?;
        device.on_uncaptured_error(Arc::new(|e: wgpu::Error| panic!("wgpu error: {e}")));
        let mut r = UiRenderer::with_device(device, queue, wgpu::TextureFormat::Rgba8Unorm);
        r.adapter_info = Some(info);
        r._instance = Some(instance);
        Ok(r)
    }

    /// Integration entry: share fn-render's device/queue. `target_format` must be NON-sRGB (see [`crate::pick_surface_format`]).
    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue, target_format: wgpu::TextureFormat) -> UiRenderer {
        assert!(!target_format.is_srgb(), "fn-ui composites in gamma space: use a non-sRGB target format");
        let max_dim = device.limits().max_texture_dimension_2d;
        let frame_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ui frame bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }] });
        let frame_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui frame ubo"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ui frame bg"), layout: &frame_bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() }] });
        let quad_tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ui quad tex bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }] });
        let quad_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ui quad layout"), bind_group_layouts: &[Some(&frame_bgl), Some(&quad_tex_bgl)], immediate_size: 0 });
        let quad_sm = shader(&device, "ui quad", include_str!("shaders/quad.wgsl").to_owned());
        let attr = |location: u32, offset: u64| wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset, shader_location: location };
        let attrs = [attr(0, 0), attr(1, 16), attr(2, 32), attr(3, 48)];
        let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui quad pipeline"), layout: Some(&quad_layout),
            vertex: wgpu::VertexState { module: &quad_sm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 64, step_mode: wgpu::VertexStepMode::Instance, attributes: &attrs })] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: None, multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState { module: &quad_sm, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: target_format, blend: Some(premul_blend()), write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        let mat_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ui mat bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: NonZeroU64::new(MAT_SLOT) }, count: None }] });
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
        let tex4_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ui mat tex bgl"), entries: &[tex_entry(0), tex_entry(1), tex_entry(2), tex_entry(3)] });
        let mat_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ui mat layout"), bind_group_layouts: &[Some(&frame_bgl), Some(&mat_bgl), Some(&tex4_bgl)], immediate_size: 0 });
        let mat_cap = 64 * 1024;
        let mat_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui mat ubo"), size: mat_cap, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mat_bg = Self::make_mat_bg(&device, &mat_bgl, &mat_buf);
        let inst_cap = 4096u64;
        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui quad instances"), size: inst_cap * 64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let dummy = make_straight(&device, &queue, "ui dummy", 1, 1, &[0, 0, 0, 0], 0);
        let samplers = Samplers::new(&device);
        UiRenderer {
            device, queue, format: target_format, max_dim, adapter_info: None, _instance: None, frame_buf, frame_bg, quad_tex_bgl, quad_pipeline, samplers,
            quad_tex: HashMap::new(), baked: HashMap::new(), straight: HashMap::new(), embedded: (0..UIMAT_TEX.len()).map(|_| None).collect(), dummy,
            inst_buf, inst_cap, mat_buf, mat_cap, mat_bgl, mat_bg, tex4_bgl, mat_layout, mat_pipes: HashMap::new(), tex4_cache: HashMap::new(),
            mode: MaterialMode::Gpu, frame: 0, stats: FrameStats::default(),
        }
    }

    fn make_mat_bg(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout, buf: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ui mat bg"), layout: bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: buf, offset: 0, size: NonZeroU64::new(MAT_SLOT) }) }] })
    }

    pub fn device(&self) -> &wgpu::Device { &self.device }
    pub fn queue(&self) -> &wgpu::Queue { &self.queue }
    pub fn target_format(&self) -> wgpu::TextureFormat { self.format }
    /// Set when created by `new_headless`.
    pub fn adapter_info(&self) -> Option<&wgpu::AdapterInfo> { self.adapter_info.as_ref() }
    pub fn set_material_mode(&mut self, m: MaterialMode) { self.mode = m; }
    pub fn material_mode(&self) -> MaterialMode { self.mode }
    pub fn stats(&self) -> FrameStats { self.stats }
    /// Number of store textures currently resident on the GPU (quad textures).
    pub fn cached_texture_count(&self) -> usize { self.quad_tex.len() }
    /// Drop every cached baked material texture (e.g. after a DPI/scale change).
    pub fn clear_material_cache(&mut self) { self.baked.clear(); }
    /// Drop every cached texture (quad, straight and baked). Call when the `UiAssets`/`TextureStore` instance is replaced: caches are keyed by
    /// (TextureId, version) of ONE store.
    pub fn clear_caches(&mut self) { self.quad_tex.clear(); self.straight.clear(); self.baked.clear(); self.tex4_cache.clear(); }

    // ------------------------------------------------------------------ texture sync
    fn uploader(&self) -> TexUploader<'_> { TexUploader { device: &self.device, queue: &self.queue, bgl: &self.quad_tex_bgl, samplers: &self.samplers, max_dim: self.max_dim } }

    fn ensure_quad_tex(&mut self, id: TextureId, assets: &UiAssets) -> bool {
        let Some(t) = assets.store.get(id) else { return false };
        if let Some(g) = self.quad_tex.get(&id.0) { if g.version == t.version && g.key == sampler_flags(t) { return true; } }
        let existing = self.quad_tex.remove(&id.0);
        let (nearest, repeat, mips) = sampler_flags(t);
        let pm = t.premultiplied();
        let up = self.uploader().upload_premult(existing, t.w, t.h, &pm, mips, nearest, repeat, t.version, &format!("ui tex {}", id.0));
        self.stats.texture_uploads += 1;
        match up { Some(g) => { self.quad_tex.insert(id.0, g); true } None => false }
    }

    fn ensure_straight(&mut self, id: u32, assets: &UiAssets) -> Option<u32> {
        let t = assets.store.get(TextureId(id))?;
        if self.straight.get(&id).map_or(true, |s| s.version != t.version) {
            if t.w > self.max_dim || t.h > self.max_dim { return None; }
            let s = make_straight(&self.device, &self.queue, "ui straight tex", t.w, t.h, &t.rgba, t.version);
            self.stats.texture_uploads += 1;
            self.straight.insert(id, s);
        }
        Some(t.version)
    }

    fn ensure_embedded(&mut self, i: u32) {
        if self.embedded[i as usize].is_some() { return; }
        let (w, h, _, _, _, off) = UIMAT_TEX[i as usize];
        let n = (w * h * 4) as usize;
        self.embedded[i as usize] = Some(make_straight(&self.device, &self.queue, "ui embedded uimat tex", w, h, &UIMAT_TEX_BIN[off..off + n], 0));
    }

    fn tex4_bind_group(&mut self, keys: [TexKey; 4]) -> bool {
        if self.tex4_cache.contains_key(&keys) { return true; }
        if self.tex4_cache.len() > 512 { self.tex4_cache.clear(); }
        let mut views: Vec<&wgpu::TextureView> = Vec::with_capacity(4);
        for k in keys {
            views.push(match k {
                TexKey::Dummy => &self.dummy.view,
                TexKey::Embedded(i) => match &self.embedded[i as usize] { Some(t) => &t.view, None => return false },
                TexKey::Store(id, _) => match self.straight.get(&id) { Some(t) => &t.view, None => return false },
            });
        }
        let entries: Vec<wgpu::BindGroupEntry> = views.iter().enumerate().map(|(i, v)| wgpu::BindGroupEntry { binding: i as u32, resource: wgpu::BindingResource::TextureView(v) }).collect();
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ui mat tex bg"), layout: &self.tex4_bgl, entries: &entries });
        self.tex4_cache.insert(keys, bg);
        true
    }

    /// Lazily builds (and caches, also failures) the pipeline of a GPU material shader.
    fn ensure_mat_pipeline(&mut self, id: ShaderId) -> bool {
        if let Some(p) = self.mat_pipes.get(&id) { return p.is_some(); }
        let prelude = include_str!("shaders/mat_prelude.wgsl");
        let src = match id {
            ShaderId::Uimat(n) => format!("{prelude}\n{}\n{}", UIMAT_PS_WGSL[n as usize], include_str!("shaders/mat_fs_slate.wgsl")),
            ShaderId::Hud(RADIAL_PROGRESS) => format!("{prelude}\n{}", include_str!("shaders/hud_radial.wgsl")),
            ShaderId::Hud(n) => format!("{prelude}\n{}\n{}", HUD_WGSL[n as usize], include_str!("shaders/mat_fs_slate.wgsl")),
            ShaderId::Proc => format!("{prelude}\n{}", include_str!("shaders/proc_fs.wgsl")),
        };
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let sm = shader(&self.device, &format!("ui material {id:?}"), src);
        let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui material pipeline"), layout: Some(&self.mat_layout),
            vertex: wgpu::VertexState { module: &sm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: None, multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState { module: &sm, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: self.format, blend: Some(premul_blend()), write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        let err = pollster::block_on(scope.pop());
        let ok = match err { None => true, Some(e) => { eprintln!("fn-ui-wgpu: WGSL material {id:?} failed ({e}); falling back to CpuBake"); false } };
        self.mat_pipes.insert(id, if ok { Some(pipe) } else { None });
        ok
    }

    // ------------------------------------------------------------------ CpuBake
    fn baked_texture(&mut self, cmd: &MaterialCmd, assets: &mut UiAssets) -> Option<TexRef> {
        let (w, h) = (cmd.rect.w.round().max(1.0) as u32, cmd.rect.h.round().max(1.0) as u32);
        let key = (cmd.kind.cache_key(), w, h);
        if let Some(e) = self.baked.get_mut(&key) { e.1 = self.frame; return Some(TexRef::Baked(key)); }
        if w > self.max_dim || h > self.max_dim { return None; }
        let px = fn_ui::material::bake(&cmd.kind, w, h, assets);
        if px.len() != (w * h * 4) as usize { return None; }
        let g = self.uploader().upload_premult(None, w, h, &px, false, false, false, 0, "ui baked material")?;
        self.baked.insert(key, (g, self.frame));
        Some(TexRef::Baked(key))
    }

    // ------------------------------------------------------------------ render
    /// Draw `list` into `target` (format = the construction format, non-sRGB) at `w` x `h`; `clear` = straight RGBA to clear to first
    /// (stored premultiplied, as the framebuffer holds premultiplied colour).
    pub fn render(&mut self, list: &RenderList, ctx_assets: &mut UiAssets, target: &wgpu::TextureView, w: u32, h: u32, clear: Option<[f32; 4]>) {
        self.frame += 1;
        self.stats = FrameStats::default();
        let (wi, hi) = (w as i32, h as i32);
        let mut insts: Vec<QuadInst> = Vec::new();
        let mut ops: Vec<Op> = Vec::new();
        let mut mat_data: Vec<u8> = Vec::new();
        let align = (self.device.limits().min_uniform_buffer_offset_alignment as usize).max(16);
        let mut clips: Vec<[i32; 4]> = Vec::new();
        let full = [0i32, 0, wi, hi];
        let mut emitted: [i32; 4] = full; // the pass starts with a full-target scissor
        let mut checked: HashSet<u32> = HashSet::new();
        let mut failed: HashSet<u32> = HashSet::new();
        let cur_clip = |clips: &Vec<[i32; 4]>| clips.last().copied().unwrap_or(full);

        for cmd in &list.cmds {
            match cmd {
                Cmd::PushClip(r) => {
                    let n = [r.x.floor() as i32, r.y.floor() as i32, (r.x + r.w).ceil() as i32, (r.y + r.h).ceil() as i32];
                    let p = cur_clip(&clips);
                    clips.push([n[0].max(p[0]).max(0), n[1].max(p[1]).max(0), n[2].min(p[2]).min(wi), n[3].min(p[3]).min(hi)]);
                }
                Cmd::PopClip => { clips.pop(); }
                Cmd::Quad(q) => {
                    if q.tex == TextureId::NONE { continue; }
                    let c = cur_clip(&clips);
                    if c[2] <= c[0] || c[3] <= c[1] { continue; }
                    if failed.contains(&q.tex.0) { continue; }
                    if checked.insert(q.tex.0) && !self.ensure_quad_tex(q.tex, ctx_assets) { failed.insert(q.tex.0); continue; }
                    if !self.quad_tex.contains_key(&q.tex.0) { continue; }
                    let t = q.tint;
                    let inst = QuadInst {
                        p01: [q.pos[0][0], q.pos[0][1], q.pos[1][0], q.pos[1][1]], p23: [q.pos[2][0], q.pos[2][1], q.pos[3][0], q.pos[3][1]],
                        uv: q.uv, col: [t[0] * t[3], t[1] * t[3], t[2] * t[3], t[3]],
                    };
                    Self::push_quad(&mut ops, &mut insts, &mut emitted, c, TexRef::Store(q.tex.0), inst);
                    self.stats.quads += 1;
                }
                Cmd::Material(m) => {
                    let c = cur_clip(&clips);
                    if c[2] <= c[0] || c[3] <= c[1] { continue; }
                    let mut done = false;
                    if self.mode == MaterialMode::Gpu {
                        if let Some(draw) = gpu_material(m, ctx_assets) {
                            if self.ensure_mat_pipeline(draw.shader) {
                                // texture slots
                                let mut keys = [TexKey::Dummy; 4];
                                let mut ok = true;
                                for (i, s) in draw.tex.iter().enumerate() {
                                    keys[i] = match *s {
                                        TexSrc::Dummy => TexKey::Dummy,
                                        TexSrc::Embedded(e) => { self.ensure_embedded(e); TexKey::Embedded(e) }
                                        TexSrc::Store(id) => match self.ensure_straight(id, ctx_assets) { Some(v) => TexKey::Store(id, v), None => { ok = false; TexKey::Dummy } },
                                    };
                                }
                                if ok && self.tex4_bind_group(keys) {
                                    let off = (mat_data.len() + align - 1) / align * align;
                                    mat_data.resize(off, 0);
                                    mat_data.extend_from_slice(bytemuck::cast_slice(&draw.p));
                                    if c != emitted { ops.push(Op::Scissor([c[0] as u32, c[1] as u32, (c[2] - c[0]) as u32, (c[3] - c[1]) as u32])); emitted = c; }
                                    ops.push(Op::Mat { shader: draw.shader, offset: off as u32, tex4: keys });
                                    self.stats.gpu_materials += 1;
                                    done = true;
                                }
                            }
                        }
                    }
                    if !done {
                        if let Some(tr) = self.baked_texture(m, ctx_assets) {
                            let r = m.rect;
                            let uv = if m.flip_x { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] };
                            let t = m.tint;
                            let inst = QuadInst {
                                p01: [r.x, r.y, r.x + r.w, r.y], p23: [r.x + r.w, r.y + r.h, r.x, r.y + r.h],
                                uv, col: [t[0] * t[3], t[1] * t[3], t[2] * t[3], t[3]],
                            };
                            Self::push_quad(&mut ops, &mut insts, &mut emitted, c, tr, inst);
                            self.stats.baked_materials += 1;
                        }
                    }
                }
            }
        }

        // ---- upload
        self.queue.write_buffer(&self.frame_buf, 0, bytemuck::cast_slice(&[w as f32, h as f32, 0.0f32, 0.0]));
        if insts.len() as u64 > self.inst_cap {
            self.inst_cap = (insts.len() as u64).next_power_of_two();
            self.inst_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui quad instances"), size: self.inst_cap * 64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        if !insts.is_empty() { self.queue.write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(&insts)); }
        let need = mat_data.len() as u64 + MAT_SLOT;
        if need > self.mat_cap {
            self.mat_cap = need.next_power_of_two();
            self.mat_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui mat ubo"), size: self.mat_cap, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            self.mat_bg = Self::make_mat_bg(&self.device, &self.mat_bgl, &self.mat_buf);
        }
        if !mat_data.is_empty() { mat_data.resize((mat_data.len() + 3) / 4 * 4, 0); self.queue.write_buffer(&self.mat_buf, 0, &mat_data); }

        // ---- encode
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("fn-ui-wgpu") });
        {
            let load = match clear {
                Some(c) => wgpu::LoadOp::Clear(wgpu::Color { r: (c[0] * c[3]) as f64, g: (c[1] * c[3]) as f64, b: (c[2] * c[3]) as f64, a: c[3] as f64 }),
                None => wgpu::LoadOp::Load,
            };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fn-ui-wgpu pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bg, &[]);
            pass.set_scissor_rect(0, 0, w, h);
            let mut cur: Option<u8> = None; // 0 = quad pipeline, 1 = material pipeline (any)
            let mut cur_shader: Option<ShaderId> = None;
            for op in &ops {
                match op {
                    Op::Scissor(s) => pass.set_scissor_rect(s[0], s[1], s[2], s[3]),
                    Op::Quads { tex, first, count } => {
                        let bg = match tex { TexRef::Store(id) => self.quad_tex.get(id).map(|g| &g.bind_group), TexRef::Baked(k) => self.baked.get(k).map(|g| &g.0.bind_group) };
                        let Some(bg) = bg else { continue };
                        if cur != Some(0) { pass.set_pipeline(&self.quad_pipeline); pass.set_vertex_buffer(0, self.inst_buf.slice(..)); cur = Some(0); cur_shader = None; }
                        pass.set_bind_group(1, bg, &[]);
                        pass.draw(0..6, *first..*first + *count);
                        self.stats.draw_calls += 1;
                    }
                    Op::Mat { shader, offset, tex4 } => {
                        let (Some(Some(pipe)), Some(tbg)) = (self.mat_pipes.get(shader), self.tex4_cache.get(tex4)) else { continue };
                        if cur_shader != Some(*shader) { pass.set_pipeline(pipe); cur = Some(1); cur_shader = Some(*shader); }
                        pass.set_bind_group(1, &self.mat_bg, &[*offset]);
                        pass.set_bind_group(2, tbg, &[]);
                        pass.draw(0..6, 0..1);
                        self.stats.draw_calls += 1;
                    }
                }
            }
        }
        self.queue.submit([enc.finish()]);

        // ---- eviction of textures that left the store
        self.quad_tex.retain(|id, _| ctx_assets.store.get(TextureId(*id)).is_some());
        self.straight.retain(|id, _| ctx_assets.store.get(TextureId(*id)).is_some());
        // LRU cap of the baked-material cache, applied after submit so nothing recorded this frame can vanish (resources stay alive until the GPU is done)
        if self.baked.len() > BAKE_CACHE_CAP {
            let mut ages: Vec<((u64, u32, u32), u64)> = self.baked.iter().map(|(k, v)| (*k, v.1)).collect();
            ages.sort_by_key(|a| a.1);
            let target = BAKE_CACHE_CAP * 3 / 4;
            let excess = self.baked.len() - target;
            for (k, _) in ages.into_iter().take(excess) { self.baked.remove(&k); }
        }
        self.stats.bake_cache_entries = self.baked.len() as u32;
    }

    fn push_quad(ops: &mut Vec<Op>, insts: &mut Vec<QuadInst>, emitted: &mut [i32; 4], clip: [i32; 4], tex: TexRef, inst: QuadInst) {
        if clip != *emitted { ops.push(Op::Scissor([clip[0] as u32, clip[1] as u32, (clip[2] - clip[0]) as u32, (clip[3] - clip[1]) as u32])); *emitted = clip; }
        let idx = insts.len() as u32;
        insts.push(inst);
        if let Some(Op::Quads { tex: t, first, count }) = ops.last_mut() { if *t == tex && *first + *count == idx { *count += 1; return; } }
        ops.push(Op::Quads { tex, first: idx, count: 1 });
    }

    /// Offscreen convenience: returns PREMULTIPLIED RGBA8 (`w*h*4`, row 0 = top). Cleared to transparent black unless `clear` is given.
    pub fn render_to_image(&mut self, list: &RenderList, assets: &mut UiAssets, w: u32, h: u32, clear: Option<[f32; 4]>) -> Vec<u8> {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fn-ui-wgpu offscreen"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: self.format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.render(list, assets, &view, w, h, Some(clear.unwrap_or([0.0; 4])));
        let bpr_un = w * 4;
        let bpr = bpr_un.div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("fn-ui-wgpu readback"), size: (bpr * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        rx.recv().expect("map callback").expect("map failed");
        let data = slice.get_mapped_range().expect("map range");
        let mut out = Vec::with_capacity((bpr_un * h) as usize);
        for row in 0..h { let s = (row * bpr) as usize; out.extend_from_slice(&data[s..s + bpr_un as usize]); }
        drop(data);
        buffer.unmap();
        if matches!(self.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) { for px in out.chunks_exact_mut(4) { px.swap(0, 2); } }
        out
    }
}

/// First NON-sRGB format of the surface capabilities (fn-ui composites in gamma space).
pub fn pick_surface_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or_else(|| caps.formats.first().copied().unwrap_or(wgpu::TextureFormat::Bgra8Unorm))
}
