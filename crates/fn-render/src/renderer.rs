//! The renderer: owns every pipeline and render target and records one frame as a [`FrameGraph`]:
//!
//! ```text
//! shadow cascade 0..2 (depth, skinned casters)        -> 3 x 2048^2 depth array
//! sky (dome, sun, clouds; colour only)                 -> HDR mip 0 (Rgba16Float)
//! scene (ground + skinned character parts, fog)        -> HDR mip 0 + depth (Depth32Float)
//! HDR mip chain (box downsample, bloom taps + metering)
//! exposure (compute: 1x1 mean -> adapted exposure)
//! post (AO, bloom, exposure, vignette, white balance, ACES film curve, sRGB) -> output view
//! ```
use crate::camera::{CameraParams, CameraUniform};
use crate::graph::{FrameGraph, PassTiming};
use crate::lighting::{self, Cascade, CASCADE_COUNT};
use crate::mesh::{MaterialKind, SkinnedPrimitive, SkinnedVertex};
use crate::profiler::{GpuProfiler, PassTimings};
use crate::props::{PropInstance, PropTemplates};
use crate::sky::{self, SkyAssets, SkyRenderer};
use crate::static_mesh::{StaticMaterialData, StaticMaterialHandle, StaticMeshData, StaticMeshHandle, StaticMeshInstance};
use crate::statics::{StaticRenderer, StaticStats};
use crate::texture::{self, MipFilter};
use crate::quality::{Backdrop, FxaaParams};
use crate::tonemap::{self, MeterParams, PostParams};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use image::RgbaImage;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const SHADOW_MAP_SIZE: u32 = 2048;
/// Format of the intermediate tone-mapped image FXAA reads.
pub const LDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const PROP_INITIAL_CAPACITY: usize = 4096;
/// Half edge of the 512 m baseplate and the extent of the visual ground ring (metres).
pub const GROUND_HALF: f32 = 256.0;
pub const GROUND_OUTER: f32 = 6000.0;

/// Runtime switches (the prototype's debug / settings toggles).
#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub shadows: bool,
    pub ao: bool,
    pub bloom: bool,
    pub vignette: bool,
    pub fog: bool,
    /// Sky layer mask (`sky::LAYER_*`).
    pub sky_layers: u32,
    /// Fixed exposure scale; `None` = auto exposure.
    pub fixed_exposure: Option<f32>,
    /// 0 final, 1 no film curve, 2 AO only, 3 bloom only, 4 exposure-scaled linear, 5 vignette only.
    pub debug_mode: i32,
    /// Material debug bits: 1 plain grass, 2 plain character, 4 fog off, 8 fog amount.
    pub material_debug: u32,
    /// Exponent applied after the sRGB encode (Brightness setting): 1.0 = unchanged.
    pub display_exponent: f32,
    /// FXAA resolve of the tone-mapped image (`quality::aa_params(level)`); `None` = off (the post pass writes straight to the target).
    pub fxaa: Option<FxaaParams>,
    /// Pause-menu backdrop (dim + blur of the finished scene); `Backdrop::OFF` normally.
    pub backdrop: Backdrop,
    /// Shadow map edge in texels (`quality::shadow_map_size`): 512 / 1024 / 2048 (any 256..=4096); the shadow texture is re-created when it changes.
    pub shadow_map_size: u32,
    /// Anisotropic filtering of the scene textures (`quality::texture_params().1`, 1 = off).
    pub anisotropy: u16,
    /// Top mip levels of the scene textures that are never sampled (`quality::texture_params().0`, sampler minimum LOD).
    pub skip_mips: u32,
}
impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            shadows: true, ao: true, bloom: true, vignette: true, fog: true, sky_layers: sky::LAYER_ALL, fixed_exposure: None, debug_mode: 0, material_debug: 0,
            display_exponent: 1.0, fxaa: None, backdrop: Backdrop::OFF, shadow_map_size: SHADOW_MAP_SIZE, anisotropy: 8, skip_mips: 0,
        }
    }
}

/// One vertex of the procedural props (trees, rocks): world-space position, flat normal, vertex colour (linear albedo).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct PropVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}

/// Hook for a 2D overlay drawn into the swapchain view AFTER the post chain (UI is composited in sRGB/gamma space over the tone-mapped
/// frame, as the old runtime did). Implemented by `fn_ui_wgpu::UiPass`.
pub trait UiOverlay {
    /// Draw into `target` (already holding the finished 3D frame; load, do not clear). May encode and submit its own command buffers.
    fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView, width: u32, height: u32);
}

/// Linear HDR scene colour read back from the HDR target (debug / reference comparison).
#[derive(Clone, Debug)]
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    /// Row-major, top row first; RGB linear scene radiance, A = ambient share.
    pub pixels: Vec<[f32; 4]>,
}
impl HdrImage {
    pub fn get(&self, x: u32, y: u32) -> [f32; 4] { self.pixels[(y * self.width + x) as usize] }
    /// Mean RGB over `[x0,x1) x [y0,y1)`.
    pub fn region_mean(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> [f64; 3] {
        let mut s = [0f64; 3];
        let mut n = 0f64;
        for y in y0..y1.min(self.height) { for x in x0..x1.min(self.width) { let p = self.get(x, y); for c in 0..3 { s[c] += p[c] as f64; } n += 1.; } }
        s.map(|v| v / n.max(1.))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelId(pub usize);

pub struct FrameParams<'a> {
    pub camera: &'a CameraParams,
    /// View.RealTime in seconds (drives sky / cloud animation).
    pub time: f32,
    /// Frame delta in seconds (eye adaptation).
    pub dt: f32,
    pub models: &'a [ModelId],
    pub options: &'a RenderOptions,
}

pub struct RendererAssets {
    pub sky: SkyAssets,
    /// T_Grasslands_D.png
    pub ground_texture: RgbaImage,
}

#[derive(Clone, Debug)]
pub struct FrameStats {
    pub cascades: Option<[Cascade; CASCADE_COUNT]>,
    pub passes: Vec<PassTiming>,
    /// Total CPU time recording + submitting the frame.
    pub cpu_ms: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ExposureInfo { pub ev: f32, pub target_ev: f32, pub exposure: f32, pub avg_luminance: f32 }

/// Light block (struct Light in scene.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LightUniform {
    pub sun_dir: [f32; 4],
    pub sun_radiance: [f32; 4],
    pub sky_radiance: [f32; 4],
    pub shadow_matrix: [[[f32; 4]; 4]; CASCADE_COUNT],
    pub cascade_far: [f32; 4],
    pub cascade_texel: [f32; 4],
    pub shadow_params: [f32; 4],
    pub misc: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GroundVertex { pos: [f32; 3], uv: [f32; 2] }
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectUniform { model: [[f32; 4]; 4] }
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform { kind: [u32; 4] }

struct GpuPrimitive {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    index_count: u32,
    material_bg: wgpu::BindGroup,
    /// CHAR MATERIAL HOOK: the original compiled character material of this primitive (bind group 3 of the character pipeline), see char_material.rs
    char_bg: Option<wgpu::BindGroup>,
    /// CHAR MATERIAL HOOK: an eye-region modulate overlay (not drawn in the scene / shadow passes, drawn after the scene with its own pipeline)
    overlay: Option<crate::char_material::GpuOverlay>,
}
struct Model {
    prims: Vec<GpuPrimitive>,
    obj: wgpu::Buffer,
    bones: wgpu::Buffer,
    bone_count: usize,
    obj_bg: wgpu::BindGroup,
}

struct Targets {
    width: u32,
    height: u32,
    hdr: wgpu::Texture,
    hdr_mip_views: Vec<wgpu::TextureView>,
    depth_view: wgpu::TextureView,
    post_bg: wgpu::BindGroup,
    downsample_bgs: Vec<wgpu::BindGroup>,
    meter_bg: wgpu::BindGroup,
    /// LDR (Rgba8Unorm) tone-mapped image + FXAA bind group, only while FXAA is on.
    ldr: Option<(wgpu::TextureView, wgpu::BindGroup)>,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    output_format: wgpu::TextureFormat,
    // layouts
    bgl_material: wgpu::BindGroupLayout,
    bgl_obj_skinned: wgpu::BindGroupLayout,
    bgl_post: wgpu::BindGroupLayout,
    bgl_downsample: wgpu::BindGroupLayout,
    bgl_meter: wgpu::BindGroupLayout,
    // pipelines
    ground_pipeline: wgpu::RenderPipeline,
    skinned_pipeline: wgpu::RenderPipeline,
    prop_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_prop_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    meter_pipeline: wgpu::ComputePipeline,
    post_pipeline: wgpu::RenderPipeline,
    post_ldr_pipeline: wgpu::RenderPipeline,
    fxaa_pipeline: wgpu::RenderPipeline,
    bgl_fxaa: wgpu::BindGroupLayout,
    fxaa_buf: wgpu::Buffer,
    bgl_globals: wgpu::BindGroupLayout,
    shadow_cmp: wgpu::Sampler,
    tex_sampler: wgpu::Sampler,
    shadow_size: u32,
    sampler_key: (u16, u32),
    prop_vb: wgpu::Buffer,
    prop_capacity: usize,
    prop_count: u32,
    sky: SkyRenderer,
    // shared resources
    camera_buf: wgpu::Buffer,
    light_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    #[allow(dead_code)]
    shadow_array_view: wgpu::TextureView,
    shadow_layer_views: Vec<wgpu::TextureView>,
    shadow_vp_bufs: Vec<wgpu::Buffer>,
    shadow_vp_bgs: Vec<wgpu::BindGroup>,
    clamp_sampler: wgpu::Sampler,
    neutral: [wgpu::Texture; 3],
    white: wgpu::Texture,
    post_buf: wgpu::Buffer,
    meter_buf: wgpu::Buffer,
    exposure_buf: wgpu::Buffer,
    // ground
    ground_vb: wgpu::Buffer,
    ground_count: u32,
    ground_material_bg: wgpu::BindGroup,
    ground_obj_bg: wgpu::BindGroup,
    // state
    models: Vec<Model>,
    targets: Option<Targets>,
    frame_index: u64,
    sky_radiance: [f32; 3],
    profiler: GpuProfiler,
    prop_templates: PropTemplates,
    prop_scratch: Vec<PropVertex>,
    statics: StaticRenderer,
    // RENDER FX HOOK: the translucent / unlit / WPO / masked material pass (fx.rs) and the generation of the targets its scene-depth bind groups were built for
    fx: crate::fx::FxRenderer,
    fx_gen: u64,
    // PARTICLE HOOK: the particle pass (particles.rs), drawn after the FX pass
    particles: crate::particles::ParticleRenderer,
    // GUN MATERIAL HOOK: the held firearms drawn with their original material programs (gun_material.rs), inside the scene pass
    gun: crate::gun_material::GunRenderer,
    // CHAR MATERIAL HOOK: the BR character materials' pipelines (head / hair / body skin, eyeshadow modulate overlay), see char_material.rs
    char_gpu: crate::char_material::CharGpu,
    // VOLUMETRIC FOG HOOK: the volumetric fog volume + passes (volfog.rs), composited right after the opaque scene
    volfog: crate::volfog::VolFog,
}

fn create_shadow_views(device: &wgpu::Device, size: u32) -> (wgpu::TextureView, Vec<wgpu::TextureView>) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow cascades"), size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: CASCADE_COUNT as u32 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
    });
    let array = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    let layers = (0..CASCADE_COUNT as u32).map(|i| tex.create_view(&wgpu::TextureViewDescriptor { label: Some("shadow layer"), dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: i, array_layer_count: Some(1), ..Default::default() })).collect();
    (array, layers)
}
fn wgsl(device: &wgpu::Device, label: &str, parts: &[&str]) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(parts.join("\n").into()) })
}
fn entry_uniform(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }
}
fn entry_storage(binding: u32, vis: wgpu::ShaderStages, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None }, count: None }
}
fn entry_texture(binding: u32, vis: wgpu::ShaderStages, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None }
}
fn entry_sampler(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::SamplerBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Sampler(ty), count: None }
}
fn bg_buf<'a>(binding: u32, b: &'a wgpu::Buffer) -> wgpu::BindGroupEntry<'a> { wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() } }
fn bg_tex<'a>(binding: u32, v: &'a wgpu::TextureView) -> wgpu::BindGroupEntry<'a> { wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(v) } }
fn bg_samp<'a>(binding: u32, s: &'a wgpu::Sampler) -> wgpu::BindGroupEntry<'a> { wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::Sampler(s) } }

const SKINNED_ATTRS: [wgpu::VertexAttribute; 5] = [
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint16x4, offset: 32, shader_location: 3 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 40, shader_location: 4 },
];
const PROP_ATTRS: [wgpu::VertexAttribute; 3] = [
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 24, shader_location: 2 },
];
const GROUND_ATTRS: [wgpu::VertexAttribute; 2] = [
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 12, shader_location: 1 },
];

/// Visual ground: the 512 m plate + the four ring rectangles out to `outer`, as a triangle list with the prototype's 2 m UV repeat.
pub fn ground_triangles(half: f32, outer: f32) -> Vec<([f32; 3], [f32; 2])> {
    let v = |x: f32, z: f32| ([x, 0., z], [(x + half) * 0.5, (z + half) * 0.5]);
    let mut quads = vec![[(-half, -half), (-half, half), (half, half), (half, -half)]];
    quads.extend(lighting::ground_ring(half, outer));
    let mut out = vec![];
    for q in quads {
        for k in [0, 1, 2, 0, 2, 3] { out.push(v(q[k].0, q[k].1)); }
    }
    out
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, output_format: wgpu::TextureFormat, assets: &RendererAssets) -> Result<Renderer, String> {
        Self::new_with_progress(device, queue, output_format, assets, &mut |_| {})
    }

    /// [Renderer::new] that announces every phase BEFORE running it (progress(label)), so a loading screen can show what is being built and time each phase.
    /// The phases are the shader-module, per-pipeline-group, target / buffer, texture and sky steps; each is one call into the driver and can take 100s of ms.
    pub fn new_with_progress(device: &wgpu::Device, queue: &wgpu::Queue, output_format: wgpu::TextureFormat, assets: &RendererAssets, progress: &mut dyn FnMut(&'static str)) -> Result<Renderer, String> {
        let (device, queue) = (device.clone(), queue.clone());
        progress("Creating bind group layouts");
        let vf = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let (v, f) = (wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT);
        // ---- bind group layouts ----
        let bgl_globals = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("globals bgl"), entries: &[
            entry_uniform(0, vf), entry_uniform(1, vf),
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
            entry_sampler(3, f, wgpu::SamplerBindingType::Comparison),
            entry_sampler(4, f, wgpu::SamplerBindingType::Filtering),
        ] });
        let bgl_material = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("material bgl"), entries: &[
            entry_uniform(0, f), entry_texture(1, f, true), entry_texture(2, f, true), entry_texture(3, f, true), entry_texture(4, f, true),
        ] });
        let bgl_obj_static = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("object (static) bgl"), entries: &[entry_uniform(0, v)] });
        let bgl_obj_skinned = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("object (skinned) bgl"), entries: &[entry_uniform(0, v), entry_storage(1, v, true)] });
        let bgl_shadow = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("shadow vp bgl"), entries: &[entry_uniform(0, v)] });
        let bgl_post = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("post bgl"), entries: &[
            entry_uniform(0, f), entry_texture(1, f, true),
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            entry_storage(3, f, true), entry_sampler(4, f, wgpu::SamplerBindingType::Filtering),
        ] });
        let bgl_downsample = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("downsample bgl"), entries: &[entry_texture(0, f, true), entry_sampler(1, f, wgpu::SamplerBindingType::Filtering)] });
        let c = wgpu::ShaderStages::COMPUTE;
        let bgl_meter = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("meter bgl"), entries: &[entry_uniform(0, c), entry_texture(1, c, false), entry_storage(2, c, false)] });

        // ---- shaders ----
        progress("Compiling shader modules");
        let common = include_str!("../shaders/common.wgsl");
        let lit_common = include_str!("../shaders/lit_common.wgsl");
        let scene_sm = wgsl(&device, "scene", &[common, lit_common, include_str!("../shaders/scene.wgsl")]);
        let shadow_sm = wgsl(&device, "shadow", &[include_str!("../shaders/shadow.wgsl")]);
        let post_sm = wgsl(&device, "post", &[common, include_str!("../shaders/post.wgsl")]);
        let down_sm = wgsl(&device, "downsample", &[include_str!("../shaders/downsample.wgsl")]);
        let meter_sm = wgsl(&device, "meter", &[include_str!("../shaders/meter.wgsl")]);
        let fxaa_sm = wgsl(&device, "fxaa", &[include_str!("../shaders/fxaa.wgsl")]);

        // ---- pipelines ----
        let pl = |label: &str, bgls: &[&wgpu::BindGroupLayout]| {
            let entries: Vec<Option<&wgpu::BindGroupLayout>> = bgls.iter().map(|b| Some(*b)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &entries, immediate_size: 0 })
        };
        let scene_depth = wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() };
        let scene_pipeline = |label: &str, vs: &str, fs: &str, layout: &wgpu::PipelineLayout, stride: u64, attrs: &[wgpu::VertexAttribute]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label), layout: Some(layout),
                vertex: wgpu::VertexState { module: &scene_sm, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: attrs })] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(scene_depth.clone()), multisample: Default::default(),
                fragment: Some(wgpu::FragmentState { module: &scene_sm, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
                multiview_mask: None, cache: None,
            })
        };
        progress("Compiling shaders: ground, character and prop pipelines");
        let ground_layout = pl("ground layout", &[&bgl_globals, &bgl_material, &bgl_obj_static]);
        let skinned_layout = pl("skinned layout", &[&bgl_globals, &bgl_material, &bgl_obj_skinned]);
        let ground_pipeline = scene_pipeline("ground", "vs_ground", "fs_main", &ground_layout, std::mem::size_of::<GroundVertex>() as u64, &GROUND_ATTRS);
        let skinned_pipeline = scene_pipeline("skinned", "vs_skinned", "fs_main", &skinned_layout, std::mem::size_of::<SkinnedVertex>() as u64, &SKINNED_ATTRS);
        let prop_layout = pl("prop layout", &[&bgl_globals]);
        let prop_pipeline = scene_pipeline("props", "vs_prop", "fs_prop", &prop_layout, std::mem::size_of::<PropVertex>() as u64, &PROP_ATTRS);
        progress("Compiling shaders: shadow pipelines");
        let shadow_layout = pl("shadow layout", &[&bgl_shadow, &bgl_obj_skinned]);
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"), layout: Some(&shadow_layout),
            vertex: wgpu::VertexState { module: &shadow_sm, entry_point: Some("vs_skinned"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<SkinnedVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &SKINNED_ATTRS })] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            // GL used polygon offset (2, 4); the receivers also apply a normal offset + constant bias in texels (scene.wgsl).
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 } }),
            multisample: Default::default(), fragment: None, multiview_mask: None, cache: None,
        });
        let shadow_prop_layout = pl("shadow prop layout", &[&bgl_shadow]);
        let shadow_prop_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow props"), layout: Some(&shadow_prop_layout),
            vertex: wgpu::VertexState { module: &shadow_sm, entry_point: Some("vs_prop"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<PropVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &PROP_ATTRS[..1] })] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 } }),
            multisample: Default::default(), fragment: None, multiview_mask: None, cache: None,
        });
        let fullscreen = |label: &str, sm: &wgpu::ShaderModule, fs: &str, layout: &wgpu::PipelineLayout, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label), layout: Some(layout),
                vertex: wgpu::VertexState { module: sm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
                fragment: Some(wgpu::FragmentState { module: sm, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
                multiview_mask: None, cache: None,
            })
        };
        progress("Compiling shaders: post-processing pipelines");
        let post_layout = pl("post layout", &[&bgl_post]);
        let post_pipeline = fullscreen("post", &post_sm, "fs_main", &post_layout, output_format);
        let post_ldr_pipeline = fullscreen("post (ldr, fxaa input)", &post_sm, "fs_main", &post_layout, LDR_FORMAT);
        let bgl_fxaa = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fxaa bgl"), entries: &[entry_uniform(0, f), entry_texture(1, f, true), entry_sampler(2, f, wgpu::SamplerBindingType::Filtering)] });
        let fxaa_layout = pl("fxaa layout", &[&bgl_fxaa]);
        let fxaa_pipeline = fullscreen("fxaa", &fxaa_sm, "fs_main", &fxaa_layout, output_format);
        let down_layout = pl("downsample layout", &[&bgl_downsample]);
        let downsample_pipeline = fullscreen("downsample", &down_sm, "fs_downsample", &down_layout, HDR_FORMAT);
        let meter_layout = pl("meter layout", &[&bgl_meter]);
        let meter_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("meter"), layout: Some(&meter_layout), module: &meter_sm, entry_point: Some("meter"), compilation_options: Default::default(), cache: None });

        // ---- shared buffers / textures ----
        progress("Creating render buffers and shadow maps");
        let ub = |label: &str, size: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let camera_buf = ub("camera", std::mem::size_of::<CameraUniform>());
        let light_buf = ub("light", std::mem::size_of::<LightUniform>());
        let post_buf = ub("post params", std::mem::size_of::<PostParams>());
        let fxaa_buf = ub("fxaa params", 32);
        let meter_buf = ub("meter params", std::mem::size_of::<MeterParams>());
        let exposure_init = [(tonemap::AUTO_EXPOSURE_MIN * tonemap::AUTO_EXPOSURE_GREY).log2(), 0f32, 1f32, 0f32];
        let exposure_buf = texture::buffer_init(&device, &exposure_init, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, "exposure state");
        let (shadow_array_view, shadow_layer_views) = create_shadow_views(&device, SHADOW_MAP_SIZE);
        let shadow_vp_bufs: Vec<_> = (0..CASCADE_COUNT).map(|_| ub("shadow vp", 64)).collect();
        let shadow_vp_bgs: Vec<_> = shadow_vp_bufs.iter().map(|b| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("shadow vp bg"), layout: &bgl_shadow, entries: &[bg_buf(0, b)] })).collect();
        let shadow_cmp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("shadow compare"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, compare: Some(wgpu::CompareFunction::LessEqual), ..Default::default() });
        let tex_sampler = texture::repeat_sampler_ex(&device, 8, 0, "scene sampler");
        let clamp_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("clamp linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, ..Default::default() });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("globals bg"), layout: &bgl_globals, entries: &[
            bg_buf(0, &camera_buf), bg_buf(1, &light_buf), bg_tex(2, &shadow_array_view), bg_samp(3, &shadow_cmp), bg_samp(4, &tex_sampler),
        ] });
        let neutral = [
            texture::solid_rgba8(&device, &queue, [128, 128, 255, 255], "neutral normal"),
            texture::solid_rgba8(&device, &queue, [255, 0, 128, 255], "neutral spec"),
            texture::solid_rgba8(&device, &queue, [255, 0, 0, 255], "neutral mask"),
        ];
        let white = texture::solid_rgba8(&device, &queue, [255, 255, 255, 255], "white");

        // ---- ground ----
        progress("Uploading the ground texture");
        let verts: Vec<GroundVertex> = ground_triangles(GROUND_HALF, GROUND_OUTER).into_iter().map(|(pos, uv)| GroundVertex { pos, uv }).collect();
        let ground_vb = texture::buffer_init(&device, &verts, wgpu::BufferUsages::VERTEX, "ground vb");
        let ground_tex = texture::upload_rgba8(&device, &queue, &assets.ground_texture, false, MipFilter::Triangle, "T_Grasslands_D");
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let kind_buf = |k: MaterialKind| texture::buffer_init(&device, &[MaterialUniform { kind: [k as u32, 0, 0, 0] }], wgpu::BufferUsages::UNIFORM, "material kind");
        let ground_kind = kind_buf(MaterialKind::Ground);
        let ground_material_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ground material"), layout: &bgl_material, entries: &[
            bg_buf(0, &ground_kind), bg_tex(1, &view(&ground_tex)), bg_tex(2, &view(&neutral[0])), bg_tex(3, &view(&neutral[1])), bg_tex(4, &view(&neutral[2])),
        ] });
        let ground_obj = texture::buffer_init(&device, &[ObjectUniform { model: Mat4::IDENTITY.to_cols_array_2d() }], wgpu::BufferUsages::UNIFORM, "ground object");
        let ground_obj_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ground object bg"), layout: &bgl_obj_static, entries: &[bg_buf(0, &ground_obj)] });
        let prop_vb = device.create_buffer(&wgpu::BufferDescriptor { label: Some("props vb"), size: (PROP_INITIAL_CAPACITY * std::mem::size_of::<PropVertex>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        progress("Compiling shaders: sky");
        let sky = SkyRenderer::new(&device, &queue, &assets.sky, HDR_FORMAT);
        progress("Starting the GPU profiler");
        let sky_radiance = lighting::sky_radiance();
        progress("Compiling shaders: textured static meshes");
        let statics = StaticRenderer::new(&device, &queue, common, lit_common, &bgl_globals, &bgl_shadow, HDR_FORMAT, DEPTH_FORMAT, texture::repeat_sampler_ex(&device, 8, 0, "static sampler"));
        let profiler = GpuProfiler::new(&device, &queue);
        let fx = crate::fx::FxRenderer::new(&device, &queue, HDR_FORMAT, DEPTH_FORMAT); // RENDER FX HOOK
        let particles = crate::particles::ParticleRenderer::new(&device, &queue, common, lit_common, &bgl_globals, HDR_FORMAT, DEPTH_FORMAT); // PARTICLE HOOK
        let gun = crate::gun_material::GunRenderer::new(&device, &queue, common, lit_common, &bgl_globals, HDR_FORMAT, DEPTH_FORMAT); // GUN MATERIAL HOOK
        let char_gpu = crate::char_material::CharGpu::new(&device, common, lit_common, &bgl_globals, &bgl_material, &bgl_obj_skinned, HDR_FORMAT, DEPTH_FORMAT); // CHAR MATERIAL HOOK
        let volfog = crate::volfog::VolFog::new(&device, &camera_buf, &light_buf, HDR_FORMAT); // VOLUMETRIC FOG HOOK
        Ok(Renderer {
            device, queue, output_format, bgl_material, bgl_obj_skinned, bgl_post, bgl_downsample, bgl_meter,
            ground_pipeline, skinned_pipeline, prop_pipeline, shadow_pipeline, shadow_prop_pipeline, downsample_pipeline, meter_pipeline, post_pipeline, post_ldr_pipeline,
            fxaa_pipeline, bgl_fxaa, fxaa_buf, bgl_globals, shadow_cmp, tex_sampler, shadow_size: SHADOW_MAP_SIZE, sampler_key: (8, 0), prop_vb,
            prop_capacity: PROP_INITIAL_CAPACITY, prop_count: 0, sky,
            camera_buf, light_buf, globals_bg, shadow_array_view, shadow_layer_views, shadow_vp_bufs, shadow_vp_bgs,
            clamp_sampler, neutral, white, post_buf, meter_buf, exposure_buf,
            ground_vb, ground_count: verts.len() as u32, ground_material_bg, ground_obj_bg,
            models: vec![], targets: None, frame_index: 0, sky_radiance,
            profiler, prop_templates: PropTemplates::default(), prop_scratch: Vec::new(), statics,
            fx, fx_gen: 0, // RENDER FX HOOK
            particles, // PARTICLE HOOK
            gun, // GUN MATERIAL HOOK
            char_gpu, // CHAR MATERIAL HOOK
            volfog, // VOLUMETRIC FOG HOOK
        })
    }

    pub fn output_format(&self) -> wgpu::TextureFormat { self.output_format }
    /// Forgets the adapted exposure so the next frame snaps to the metered value.
    pub fn reset_exposure(&mut self) { self.frame_index = 0; }
    pub fn model_count(&self) -> usize { self.models.len() }

    /// Uploads a skinned model (several primitives sharing one bone palette of `bone_count` matrices).
    pub fn create_model(&mut self, prims: &[SkinnedPrimitive], bone_count: usize) -> ModelId {
        let device = &self.device;
        let queue = &self.queue;
        let id = self.models.len();
        let mut gpu_prims = vec![];
        for (k, p) in prims.iter().enumerate() {
            let label = format!("model {id} prim {k}");
            let vb = texture::buffer_init(device, &p.vertices, wgpu::BufferUsages::VERTEX, &label);
            let ib = texture::buffer_init(device, &p.indices, wgpu::BufferUsages::INDEX, &label);
            let upload = |img: &Option<RgbaImage>, fallback: &wgpu::Texture| -> wgpu::TextureView {
                match img { Some(i) => texture::upload_rgba8(device, queue, i, false, MipFilter::Triangle, &label).create_view(&Default::default()), None => fallback.create_view(&Default::default()) }
            };
            // a primitive without a diffuse texture is drawn untextured (generic path), like the prototype
            let kind = if p.material.diffuse.is_none() { MaterialKind::Generic } else { p.kind };
            let kind_buf = texture::buffer_init(device, &[MaterialUniform { kind: [kind as u32, 0, 0, 0] }], wgpu::BufferUsages::UNIFORM, &label);
            let (d, n, s, m) = (upload(&p.material.diffuse, &self.white), upload(&p.material.normal, &self.neutral[0]), upload(&p.material.spec, &self.neutral[1]), upload(&p.material.mask, &self.neutral[2]));
            let material_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(&label), layout: &self.bgl_material, entries: &[bg_buf(0, &kind_buf), bg_tex(1, &d), bg_tex(2, &n), bg_tex(3, &s), bg_tex(4, &m)] });
            // CHAR MATERIAL HOOK: the original compiled character material (scene pass) or the eyeshadow modulate overlay (own pass)
            let (char_bg, overlay) = match p.material.ue.as_deref() {
                Some(crate::char_material::UeMaterial::Char(c)) => (Some(self.char_gpu.char_bind_group(device, queue, c, &label)), None),
                Some(crate::char_material::UeMaterial::Eyeshadow(e)) => (None, Some(self.char_gpu.overlay(device, queue, e, &label))),
                None => (None, None),
            };
            gpu_prims.push(GpuPrimitive { vb, ib, index_count: p.indices.len() as u32, material_bg, char_bg, overlay });
        }
        let obj = device.create_buffer(&wgpu::BufferDescriptor { label: Some("model object"), size: 64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let bone_count = bone_count.max(1);
        let identity: Vec<[[f32; 4]; 4]> = vec![Mat4::IDENTITY.to_cols_array_2d(); bone_count];
        let bones = texture::buffer_init(device, &identity, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, "model bones");
        queue.write_buffer(&obj, 0, bytemuck::bytes_of(&ObjectUniform { model: Mat4::IDENTITY.to_cols_array_2d() }));
        let obj_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("model object bg"), layout: &self.bgl_obj_skinned, entries: &[bg_buf(0, &obj), bg_buf(1, &bones)] });
        self.models.push(Model { prims: gpu_prims, obj, bones, bone_count, obj_bg });
        ModelId(id)
    }
    /// Sets the model-to-world matrix and the skin matrices (`globals[joint] * inverse_bind`, character space) of a model.
    pub fn update_model(&self, id: ModelId, model: Mat4, skin: &[Mat4]) {
        let m = &self.models[id.0];
        self.queue.write_buffer(&m.obj, 0, bytemuck::bytes_of(&ObjectUniform { model: model.to_cols_array_2d() }));
        let n = skin.len().min(m.bone_count);
        let data: Vec<[[f32; 4]; 4]> = skin[..n].iter().map(|x| x.to_cols_array_2d()).collect();
        self.queue.write_buffer(&m.bones, 0, bytemuck::cast_slice(&data));
    }

    /// Replaces the dynamic prop geometry (world-space triangle list: trees, rocks; rebuild each frame for shake sway). Props receive and cast shadows.
    pub fn set_props(&mut self, verts: &[PropVertex]) {
        if verts.len() > self.prop_capacity {
            self.prop_capacity = verts.len().next_power_of_two();
            self.prop_vb = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("props vb"), size: (self.prop_capacity * std::mem::size_of::<PropVertex>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        if !verts.is_empty() { self.queue.write_buffer(&self.prop_vb, 0, bytemuck::cast_slice(verts)); }
        self.prop_count = verts.len() as u32;
    }

    /// Replaces the prop geometry from a list of placed props (position, yaw, scale, shake) of trees and rocks (`fn_combat::prop_mesh`
    /// shapes). Equivalent to `set_props(PropTemplates::build(..))` without per-call allocations; call it each frame while something shakes.
    pub fn set_prop_instances(&mut self, instances: &[PropInstance]) {
        let mut scratch = std::mem::take(&mut self.prop_scratch);
        scratch.clear();
        for i in instances { self.prop_templates.append(i, &mut scratch); }
        self.set_props(&scratch);
        self.prop_scratch = scratch;
    }
    /// Uploads a textured static-mesh material (see static_mesh::StaticMaterialData).
    pub fn create_static_material(&mut self, data: &StaticMaterialData) -> StaticMaterialHandle { self.statics.create_material(data) }
    /// Uploads a static mesh (all LODs). materials[section.material] is the material of each section.
    pub fn create_static_mesh(&mut self, data: &StaticMeshData, materials: &[StaticMaterialHandle]) -> StaticMeshHandle { self.statics.create_mesh(data, materials) }
    /// Replaces the placed static meshes (real trees / rocks). Cheap to call every frame (a copy of the list); culling, LOD choice, fading and the
    /// batched instanced draws happen per frame inside `render`. LOD hysteresis state is kept by list index, so keep the order stable.
    pub fn set_static_instances(&mut self, instances: &[StaticMeshInstance]) { self.statics.set_instances(instances); }
    /// Culling / LOD / draw-call numbers of the last rendered frame.
    pub fn static_stats(&self) -> &StaticStats { self.statics.stats() }
    pub fn static_mesh_count(&self) -> usize { self.statics.mesh_count() }
    pub fn static_material_count(&self) -> usize { self.statics.material_count() }
    /// Number of placed static instances (set_static_instances).
    pub fn static_instance_count(&self) -> usize { self.statics.instance_count() }
    // RENDER FX HOOK: the FX pass (see fx.rs). `fx()` creates textures / buffers / meshes / pipelines / bind groups; `set_fx_frame` hands over the draws of the next frame.
    pub fn fx(&mut self) -> &mut crate::fx::FxRenderer { &mut self.fx }
    pub fn set_fx_frame(&mut self, frame: crate::fx::FxFrame) { self.fx.set_frame(frame); }
    // PARTICLE HOOK: textures / materials / meshes of the particle pass and the batches of the next frame (see particles.rs)
    pub fn create_particle_texture(&mut self, width: u32, height: u32, rgba8: &[u8], srgb: bool, label: &str) -> crate::particles::ParticleTexId { self.particles.create_texture(width, height, rgba8, srgb, label) }
    pub fn create_particle_material(&mut self, d: &crate::particles::ParticleMaterialDesc) -> crate::particles::ParticleMatId { self.particles.create_material(d) }
    pub fn create_particle_mesh(&mut self, d: &crate::particles::ParticleMeshDesc) -> crate::particles::ParticleMeshId { self.particles.create_mesh(d) }
    pub fn set_particle_frame(&mut self, frame: crate::particles::ParticleFrame) { self.particles.set_frame(frame); }
    pub fn particle_stats(&self) -> &crate::particles::ParticleStats { self.particles.stats() }
    // GUN MATERIAL HOOK: textures / buffers / meshes / pipelines of the gun pass (`gun()`) and the draws of the next frame (`set_gun_frame`), see gun_material.rs
    pub fn gun(&mut self) -> &mut crate::gun_material::GunRenderer { &mut self.gun }
    pub fn set_gun_frame(&mut self, frame: crate::gun_material::GunFrame) { self.gun.set_frame(frame); }

    /// Number of prop vertices currently uploaded.
    pub fn prop_vertex_count(&self) -> u32 { self.prop_count }

    fn rebuild_globals(&mut self) {
        let shadow_view = &self.shadow_array_view;
        self.globals_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("globals bg"), layout: &self.bgl_globals, entries: &[
            bg_buf(0, &self.camera_buf), bg_buf(1, &self.light_buf), bg_tex(2, shadow_view), bg_samp(3, &self.shadow_cmp), bg_samp(4, &self.tex_sampler),
        ] });
    }
    /// Applies the quality options that need new GPU objects (shadow map size, texture sampler).
    fn apply_quality(&mut self, opt: &RenderOptions) {
        let size = opt.shadow_map_size.clamp(256, 4096);
        let aniso = opt.anisotropy.clamp(1, 16);
        let mut dirty = false;
        if size != self.shadow_size {
            let (array, layers) = create_shadow_views(&self.device, size);
            self.shadow_array_view = array;
            self.shadow_layer_views = layers;
            self.shadow_size = size;
            dirty = true;
        }
        if (aniso, opt.skip_mips) != self.sampler_key {
            self.tex_sampler = texture::repeat_sampler_ex(&self.device, aniso, opt.skip_mips, "scene sampler");
            self.sampler_key = (aniso, opt.skip_mips);
            self.statics.set_sampler(texture::repeat_sampler_ex(&self.device, aniso, opt.skip_mips, "static sampler"));
            dirty = true;
        }
        if dirty { self.rebuild_globals(); }
    }

    /// Reads the linear HDR scene target (mip 0) of the last rendered frame back to the CPU (synchronous; reference comparison / debugging).
    pub fn read_hdr(&self) -> Option<HdrImage> {
        let t = self.targets.as_ref()?;
        let (w, h) = (t.width, t.height);
        let bpr = (w * 8).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("hdr readback"), size: (bpr * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &t.hdr, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            let row = &data[(y * bpr) as usize..(y * bpr + w * 8) as usize];
            for px in row.chunks_exact(8) {
                let c = |i: usize| half_to_f32(u16::from_le_bytes([px[i * 2], px[i * 2 + 1]]));
                pixels.push([c(0), c(1), c(2), c(3)]);
            }
        }
        drop(data);
        buffer.unmap();
        Some(HdrImage { width: w, height: h, pixels })
    }

    fn ensure_targets(&mut self, w: u32, h: u32) {
        if matches!(&self.targets, Some(t) if t.width == w && t.height == h) { return; }
        let device = &self.device;
        let mips = tonemap::mip_count(w, h);
        let hdr = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr scene"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: mips, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: HDR_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
        });
        let hdr_mip_views: Vec<_> = (0..mips).map(|i| hdr.create_view(&wgpu::TextureViewDescriptor { label: Some("hdr mip"), base_mip_level: i, mip_level_count: Some(1), ..Default::default() })).collect();
        let hdr_full = hdr.create_view(&Default::default());
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scene depth"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: DEPTH_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let post_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("post bg"), layout: &self.bgl_post, entries: &[
            bg_buf(0, &self.post_buf), bg_tex(1, &hdr_full), bg_tex(2, &depth_view), bg_buf(3, &self.exposure_buf), bg_samp(4, &self.clamp_sampler),
        ] });
        let downsample_bgs: Vec<_> = (1..mips as usize).map(|i| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("downsample bg"), layout: &self.bgl_downsample, entries: &[bg_tex(0, &hdr_mip_views[i - 1]), bg_samp(1, &self.clamp_sampler)] })).collect();
        let meter_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("meter bg"), layout: &self.bgl_meter, entries: &[bg_buf(0, &self.meter_buf), bg_tex(1, &hdr_mip_views[mips as usize - 1]), bg_buf(2, &self.exposure_buf)] });
        self.fx_gen += 1; // RENDER FX HOOK: the FX bind groups that sample the scene depth are rebuilt for the new depth texture
        self.targets = Some(Targets { width: w, height: h, hdr, hdr_mip_views, depth_view, post_bg, downsample_bgs, meter_bg, ldr: None });
    }

    /// Renders one frame into `target` (its size must equal the camera's width x height, and its format `output_format`).
    pub fn render(&mut self, frame: &FrameParams, target: &wgpu::TextureView) -> FrameStats { self.render_with_overlay(frame, target, None) }

    /// ONE call per frame for the application: 3D scene + post chain (+ FXAA) into `target`, then the optional 2D overlay (HUD / menus,
    /// `fn_ui_wgpu::UiPass`) composited over the finished image in sRGB space. Submits everything before returning.
    pub fn render_with_overlay(&mut self, frame: &FrameParams, target: &wgpu::TextureView, overlay: Option<&mut dyn UiOverlay>) -> FrameStats {
        let stats = self.render_scene(frame, target);
        if let Some(o) = overlay {
            // The overlay records and submits its own command buffers: bracket them with queue-ordered timestamps.
            let ui = if self.profiler.inside_encoders() { self.profiler.slot("ui overlay", "ui") } else { None };
            if let Some(s) = ui { self.profiler.write_marker(&self.device, &self.queue, s); }
            o.draw(&self.device, &self.queue, target, frame.camera.width, frame.camera.height);
            if let Some(s) = ui { self.profiler.write_marker(&self.device, &self.queue, s + 1); }
        }
        self.profiler.end_frame(&self.device, &self.queue);
        self.profiler.collect(&self.device, false);
        stats
    }

    /// True when the device has timestamp queries (`Gpu::new` requests them when the adapter supports them).
    pub fn gpu_timing_supported(&self) -> bool { self.profiler.supported() }
    /// GPU time per pass of the most recent frame whose timestamps have been read back (lags the CPU by 1-3 frames; empty until the first
    /// result arrives or when unsupported, see `PassTimings::supported`). Passes: `shadow cascade 0..2`, `sky`, `scene`, `hdr mip n`,
    /// `exposure`, `post`, `fxaa`, `ui overlay` (needs TIMESTAMP_QUERY_INSIDE_ENCODERS). Groups: shadow / sky / scene / post / ui.
    pub fn last_pass_timings(&self) -> &PassTimings { self.profiler.last() }
    /// Blocks until every in-flight frame finished and its timestamps were read, so `last_pass_timings` is the frame just rendered
    /// (benchmarks and tests; never call it in the frame loop).
    pub fn flush_gpu_timings(&mut self) { self.profiler.collect(&self.device, true); }
    /// Every frame whose timestamps arrived since the previous call (oldest first) - for tables / histograms; combine with lush_gpu_timings.
    /// Frames are only measured while one of the 8 staging buffers is free, so flush at least every 8 frames to measure all of them.
    pub fn take_pass_timings(&mut self) -> Vec<PassTimings> { self.profiler.take_completed() }

    fn render_scene(&mut self, frame: &FrameParams, target: &wgpu::TextureView) -> FrameStats {
        let t0 = std::time::Instant::now();
        let cam = frame.camera;
        let opt = frame.options;
        self.apply_quality(opt);
        self.profiler.begin_frame();
        self.ensure_targets(cam.width, cam.height);
        let fxaa = opt.fxaa.filter(|_| opt.debug_mode == 0);
        if let Some(t) = self.targets.as_mut() {
            if fxaa.is_none() { t.ldr = None; }
            else if t.ldr.is_none() {
                let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("ldr (fxaa input)"), size: wgpu::Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                    dimension: wgpu::TextureDimension::D2, format: LDR_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
                });
                let view = tex.create_view(&Default::default());
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("fxaa bg"), layout: &self.bgl_fxaa, entries: &[bg_buf(0, &self.fxaa_buf), bg_tex(1, &view), bg_samp(2, &self.clamp_sampler)] });
                t.ldr = Some((view, bg));
            }
        }
        let queue = &self.queue;

        // ---- uniforms ----
        let cu = cam.uniform(frame.time);
        queue.write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&cu));
        let sun = lighting::sun_direction();
        let sr = lighting::sun_radiance();
        let (tx, ty) = cam.tan_half();
        let casters: Vec<ModelId> = frame.models.to_vec();
        let prop_count = self.prop_count;
        let shadows_on = opt.shadows && (!casters.is_empty() || prop_count > 0 || self.statics.any());
        let shadow_size = self.shadow_size;
        let cascades = lighting::fit_cascades(cam.view_matrix().inverse(), tx, tx / ty, cam.near, sun, shadow_size);
        let penumbra = lighting::PENUMBRA_OCCLUDER_M * lighting::LIGHT_SOURCE_ANGLE_DEG.to_radians().tan();
        let light = LightUniform {
            sun_dir: [sun.x, sun.y, sun.z, 0.], sun_radiance: [sr[0], sr[1], sr[2], 0.], sky_radiance: [self.sky_radiance[0], self.sky_radiance[1], self.sky_radiance[2], 0.],
            shadow_matrix: std::array::from_fn(|i| lighting::texture_matrix(&cascades[i]).to_cols_array_2d()),
            cascade_far: [cascades[0].far, cascades[1].far, cascades[2].far, lighting::SHADOW_DISTANCE_M],
            cascade_texel: [cascades[0].texel, cascades[1].texel, cascades[2].texel, 0.],
            shadow_params: [if shadows_on { 1. } else { 0. }, penumbra, lighting::NORMAL_OFFSET_TEXELS, lighting::CONSTANT_BIAS_TEXELS],
            misc: [1., opt.material_debug as f32, shadow_size as f32, if opt.fog { 1. } else { 0. }],
        };
        queue.write_buffer(&self.light_buf, 0, bytemuck::bytes_of(&light));
        self.statics.prepare(cam, &cascades, shadows_on, frame.time);
        for (i, c) in cascades.iter().enumerate() { queue.write_buffer(&self.shadow_vp_bufs[i], 0, bytemuck::cast_slice(&c.light_view_proj.to_cols_array_2d())); }
        self.sky.update(queue, cam, frame.time, opt.sky_layers);
        let post = PostParams {
            wb: tonemap::white_balance_columns(),
            film_stw: [tonemap::FILM_SLOPE, tonemap::FILM_TOE, tonemap::FILM_SHOULDER, 0.],
            film_clip: [tonemap::FILM_BLACK_CLIP, tonemap::FILM_WHITE_CLIP, 0., 0.],
            bloom: [tonemap::BLOOM_INTENSITY, tonemap::BLOOM_THRESHOLD, opt.bloom as u32 as f32, 0.],
            vignette: [tonemap::VIGNETTE_INTENSITY, opt.vignette as u32 as f32, 0., 0.],
            ao: [tonemap::AO_INTENSITY, tonemap::AO_RADIUS_M, opt.ao as u32 as f32, tonemap::AO_BIAS_M],
            proj: [tx, ty, cam.near, cam.far],
            view: [cam.width as f32, cam.height as f32, opt.display_exponent, opt.debug_mode as f32],
            backdrop: [opt.backdrop.dim.clamp(0., 1.), opt.backdrop.blur.clamp(0., 1.), 0., 0.],
        };
        queue.write_buffer(&self.post_buf, 0, bytemuck::bytes_of(&post));
        if let Some(p) = fxaa {
            queue.write_buffer(&self.fxaa_buf, 0, bytemuck::cast_slice(&[1. / cam.width as f32, 1. / cam.height as f32, 0., 0., p.subpix, p.edge_threshold, p.edge_threshold_min, p.search_steps as f32]));
        }
        let meter = MeterParams {
            dt: frame.dt.clamp(0., 0.25), min_brightness: tonemap::AUTO_EXPOSURE_MIN, max_brightness: tonemap::AUTO_EXPOSURE_MAX, bias_ev: tonemap::AUTO_EXPOSURE_BIAS,
            first: if self.frame_index == 0 { 1. } else { 0. }, fixed_exposure: opt.fixed_exposure.unwrap_or(0.), adapt_up: tonemap::ADAPT_UP, adapt_down: tonemap::ADAPT_DOWN,
        };
        queue.write_buffer(&self.meter_buf, 0, bytemuck::bytes_of(&meter));

        // RENDER FX HOOK: upload this frame's FX buffers / dynamic geometry, (re)build the bind groups that read the scene depth
        { let t = self.targets.as_ref().unwrap(); self.fx.prepare(&t.depth_view, self.fx_gen); }
        self.particles.prepare(); // PARTICLE HOOK: upload this frame's particle instances
        self.gun.prepare(); // GUN MATERIAL HOOK: upload this frame's gun uniform buffers
        // VOLUMETRIC FOG HOOK: size the volume to the viewport, upload the frame's parameters (the passes below are added only while it is active)
        if opt.fog {
            let t = self.targets.as_ref().unwrap();
            self.volfog.prepare(&self.queue, &crate::volfog::VolFogFrame { width: t.width, height: t.height, camera: cam, frame_index: self.frame_index, shadow_view: &self.shadow_array_view, shadow_size: self.shadow_size, depth_view: &t.depth_view, depth_gen: self.fx_gen });
        } else { self.volfog.skip_frame(); }
        // ---- frame graph ----
        let this: &Renderer = &*self;
        let targets = this.targets.as_ref().unwrap();
        let mut graph = FrameGraph::new();
        for i in 0..CASCADE_COUNT {
            let casters = &casters;
            graph.add_if(shadows_on, format!("shadow cascade {i}"), move |enc| {
                let ts = this.profiler.slot(&format!("shadow cascade {i}"), "shadow");
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shadow cascade"), color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &this.shadow_layer_views[i], depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                    timestamp_writes: this.profiler.render_writes(ts), occlusion_query_set: None, multiview_mask: None,
                });
                pass.set_pipeline(&this.shadow_pipeline);
                pass.set_bind_group(0, &this.shadow_vp_bgs[i], &[]);
                for id in casters {
                    let m = &this.models[id.0];
                    pass.set_bind_group(1, &m.obj_bg, &[]);
                    for p in &m.prims {
                        if p.overlay.is_some() { continue; } // CHAR MATERIAL HOOK: a modulate overlay casts no shadow
                        pass.set_vertex_buffer(0, p.vb.slice(..));
                        pass.set_index_buffer(p.ib.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..p.index_count, 0, 0..1);
                    }
                }
                if prop_count > 0 {
                    pass.set_pipeline(&this.shadow_prop_pipeline);
                    pass.set_bind_group(0, &this.shadow_vp_bgs[i], &[]);
                    pass.set_vertex_buffer(0, this.prop_vb.slice(..));
                    pass.draw(0..prop_count, 0..1);
                }
                this.statics.draw_shadow(&mut pass, i, &this.shadow_vp_bgs[i]);
            });
        }
        let layers = opt.sky_layers;
        graph.add("sky", move |enc| {
            let ts = this.profiler.slot("sky", "sky");
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &targets.hdr_mip_views[0], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0., g: 0., b: 0., a: 1. }), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: this.profiler.render_writes(ts), occlusion_query_set: None, multiview_mask: None,
            });
            this.sky.draw(&mut pass, layers);
        });
        let models = frame.models;
        graph.add("scene", move |enc| {
            let ts = this.profiler.slot("scene", "scene");
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &targets.hdr_mip_views[0], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &targets.depth_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: this.profiler.render_writes(ts), occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_bind_group(0, &this.globals_bg, &[]);
            pass.set_pipeline(&this.ground_pipeline);
            pass.set_bind_group(1, &this.ground_material_bg, &[]);
            pass.set_bind_group(2, &this.ground_obj_bg, &[]);
            pass.set_vertex_buffer(0, this.ground_vb.slice(..));
            pass.draw(0..this.ground_count, 0..1);
            if prop_count > 0 {
                pass.set_pipeline(&this.prop_pipeline);
                pass.set_vertex_buffer(0, this.prop_vb.slice(..));
                pass.draw(0..prop_count, 0..1);
            }
            this.statics.draw_scene(&mut pass);
            this.gun.draw(&mut pass); // GUN MATERIAL HOOK: the held firearms (group 0 = the globals bound above)
            pass.set_pipeline(&this.skinned_pipeline);
            let mut on_char = false; // CHAR MATERIAL HOOK: which of the two skinned pipelines is bound
            for id in models {
                let m = &this.models[id.0];
                pass.set_bind_group(2, &m.obj_bg, &[]);
                for p in &m.prims {
                    if p.overlay.is_some() { continue; } // CHAR MATERIAL HOOK: drawn by the "char overlay" pass
                    if p.char_bg.is_some() != on_char {
                        on_char = p.char_bg.is_some();
                        pass.set_pipeline(if on_char { &this.char_gpu.char_pipeline } else { &this.skinned_pipeline });
                        pass.set_bind_group(0, &this.globals_bg, &[]);
                        pass.set_bind_group(2, &m.obj_bg, &[]);
                    }
                    if let Some(cbg) = &p.char_bg { pass.set_bind_group(3, cbg, &[]); }
                    pass.set_bind_group(1, &p.material_bg, &[]);
                    pass.set_vertex_buffer(0, p.vb.slice(..));
                    pass.set_index_buffer(p.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..p.index_count, 0, 0..1);
                }
            }
        });
        // CHAR MATERIAL HOOK: the eye-region modulate overlays (scene colour *= the original Eyeshadow shader's output), depth read-only, right after the scene pass
        let overlay_on = models.iter().any(|id| this.models[id.0].prims.iter().any(|p| p.overlay.is_some()));
        graph.add_if(overlay_on, "char overlay", move |enc| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("char overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &targets.hdr_mip_views[0], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &targets.depth_view, depth_ops: None, stencil_ops: None }),
                timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_pipeline(&this.char_gpu.overlay_pipeline);
            pass.set_bind_group(0, &this.globals_bg, &[]);
            for id in models {
                let m = &this.models[id.0];
                pass.set_bind_group(1, &m.obj_bg, &[]);
                for p in &m.prims {
                    let Some(o) = &p.overlay else { continue };
                    pass.set_bind_group(2, &o.bg, &[]);
                    pass.set_vertex_buffer(0, p.vb.slice(..));
                    pass.set_vertex_buffer(1, o.color_vb.slice(..));
                    pass.set_index_buffer(p.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..p.index_count, 0, 0..1);
                }
            }
        });
        // VOLUMETRIC FOG HOOK: material setup + light scattering + final integration (compute), then `dst * V.a + V.rgb` over the opaque scene (scene depth selects the slice)
        let vfog_on = this.volfog.active();
        graph.add_if(vfog_on, "vfog", move |enc| {
            let ts = this.profiler.slot("vfog", "scene");
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("vfog"), timestamp_writes: this.profiler.compute_writes(ts) });
            this.volfog.encode_compute(&mut pass);
        });
        graph.add_if(vfog_on, "vfog composite", move |enc| {
            let ts = this.profiler.slot("vfog composite", "scene");
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vfog composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &targets.hdr_mip_views[0], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: this.profiler.render_writes(ts), occlusion_query_set: None, multiview_mask: None,
            });
            this.volfog.encode_composite(&mut pass);
        });
        // RENDER FX HOOK: masked FX materials (depth writable), then translucent / modulate FX materials (depth read-only, may sample the scene depth), both over the finished scene
        graph.add_if(this.fx.has_draws(false), "fx opaque", move |enc| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx opaque"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &targets.hdr_mip_views[0], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &targets.depth_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            this.fx.draw(&mut pass, false);
        });
        graph.add_if(this.fx.has_draws(true), "fx translucent", move |enc| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx translucent"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &targets.hdr_mip_views[0], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &targets.depth_view, depth_ops: None, stencil_ops: None }),
                timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            this.fx.draw(&mut pass, true);
        });
        // PARTICLE HOOK: sprites + mesh particles over the finished scene (scene depth read-only)
        graph.add_if(this.particles.draw_count() > 0, "particles", move |enc| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("particles"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &targets.hdr_mip_views[0], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &targets.depth_view, depth_ops: None, stencil_ops: None }),
                timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            this.particles.draw(&mut pass, &this.globals_bg);
        });
        for i in 1..targets.hdr_mip_views.len() {
            graph.add(format!("hdr mip {i}"), move |enc| {
                let ts = this.profiler.slot(&format!("hdr mip {i}"), "post");
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("hdr downsample"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &targets.hdr_mip_views[i], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None, timestamp_writes: this.profiler.render_writes(ts), occlusion_query_set: None, multiview_mask: None,
                });
                pass.set_pipeline(&this.downsample_pipeline);
                pass.set_bind_group(0, &targets.downsample_bgs[i - 1], &[]);
                pass.draw(0..3, 0..1);
            });
        }
        graph.add("exposure", move |enc| {
            let ts = this.profiler.slot("exposure", "post");
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("exposure"), timestamp_writes: this.profiler.compute_writes(ts) });
            pass.set_pipeline(&this.meter_pipeline);
            pass.set_bind_group(0, &targets.meter_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        });
        let ldr = targets.ldr.as_ref().filter(|_| fxaa.is_some());
        graph.add("post", move |enc| {
            let view = ldr.map_or(target, |l| &l.0);
            let ts = this.profiler.slot("post", "post");
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: this.profiler.render_writes(ts), occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_pipeline(if ldr.is_some() { &this.post_ldr_pipeline } else { &this.post_pipeline });
            pass.set_bind_group(0, &targets.post_bg, &[]);
            pass.draw(0..3, 0..1);
        });
        if let Some(l) = ldr {
            graph.add("fxaa", move |enc| {
                let ts = this.profiler.slot("fxaa", "post");
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("fxaa"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None, timestamp_writes: this.profiler.render_writes(ts), occlusion_query_set: None, multiview_mask: None,
                });
                pass.set_pipeline(&this.fxaa_pipeline);
                pass.set_bind_group(0, &l.1, &[]);
                pass.draw(0..3, 0..1);
            });
        }
        let mut encoder = this.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let passes = graph.execute(&mut encoder);
        this.queue.submit([encoder.finish()]);
        self.frame_index += 1;
        FrameStats { cascades: if shadows_on { Some(cascades) } else { None }, passes, cpu_ms: t0.elapsed().as_secs_f32() * 1000. }
    }

    /// Blocks until the GPU is idle (benchmarking / screenshots).
    pub fn wait_idle(&self) { let _ = self.device.poll(wgpu::PollType::wait_indefinitely()); }

    /// Reads the adapted exposure state back (synchronous; for tests and reports, not for the frame loop).
    pub fn read_exposure(&self) -> ExposureInfo {
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("exposure readback"), size: 16, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.exposure_buf, 0, &staging, 0, 16);
        self.queue.submit([enc.finish()]);
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        rx.recv().unwrap().expect("map");
        let data = slice.get_mapped_range().expect("mapped range");
        let f: &[f32] = bytemuck::cast_slice(&data);
        ExposureInfo { ev: f[0], target_ev: f[1], exposure: f[2], avg_luminance: f[3] }
    }
}

/// IEEE binary16 to f32 (HDR read-back).
pub fn half_to_f32(h: u16) -> f32 {
    let (s, e, m) = (((h >> 15) & 1) as u32, ((h >> 10) & 0x1F) as u32, (h & 0x3FF) as u32);
    let bits = if e == 0 {
        if m == 0 { s << 31 } else { let p = 31 - m.leading_zeros(); (s << 31) | ((p + 103) << 23) | ((m & !(1 << p)) << (23 - p)) }
    } else if e == 31 { (s << 31) | 0x7F80_0000 | (m << 13) } else { (s << 31) | ((e + 112) << 23) | (m << 13) };
    f32::from_bits(bits)
}

/// Convenience: the camera-space frustum for tests / viewers that want the sun in view space.
pub fn sun_dir() -> Vec3 { lighting::sun_direction() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_blocks_are_vec4_aligned() {
        assert_eq!(std::mem::size_of::<LightUniform>() % 16, 0);
        assert_eq!(std::mem::size_of::<LightUniform>(), 3 * 16 + 3 * 64 + 4 * 16);
        assert_eq!(std::mem::size_of::<ObjectUniform>(), 64);
        assert_eq!(std::mem::size_of::<GroundVertex>(), 20);
    }
    #[test]
    fn ground_triangle_list_covers_plate_and_ring() {
        let t = ground_triangles(GROUND_HALF, GROUND_OUTER);
        assert_eq!(t.len(), 5 * 6);
        // UV repeat: 2 m per tile across the whole plate (256 tiles)
        let max_uv = t.iter().take(6).map(|v| v.1[0]).fold(0f32, f32::max);
        assert!((max_uv - 256.).abs() < 1e-3);
        assert!(t.iter().all(|v| v.0[1] == 0.));
    }
    #[test]
    fn wgsl_layouts_match_rust_blocks() {
        // the WGSL struct member order is the contract for the uniform uploads
        let s = include_str!("../shaders/lit_common.wgsl");
        let order = ["view_proj", "view:", "view_inverse", "position", "viewport", "proj:"];
        let mut at = s.find("struct Camera").unwrap();
        for m in order { let p = s[at..].find(m).unwrap_or_else(|| panic!("{m} missing")); at += p; }
        let l = s.find("struct Light").unwrap();
        for m in ["sun_dir", "sun_radiance", "sky_radiance", "shadow_matrix", "cascade_far", "cascade_texel", "shadow_params", "misc"] { assert!(s[l..].contains(m)); }
    }
}
