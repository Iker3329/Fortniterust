//! VOLUMETRIC FOG HOOK: the GPU passes of UE 4.16's volumetric fog (Athena Day phase), ported from the disassembled 1.8 global shaders
//! (`shaders/volfog.wgsl`, constants and CPU ports in [`crate::fog`]).
//!
//! Per frame (all inside `Renderer::render_scene`, only when `RenderOptions::fog` is on):
//! 1. `vfog` compute pass: `material_setup` (VBufferA = global height-fog density split into scattering / absorption), `scatter` (directional light with the
//!    renderer's sun cascades + the flat sky-light term, Henyey-Greenstein phase, temporal history reprojection with weight 0.9), `integrate` (front-to-back
//!    integration to (light rgb, transmittance) per slice, `FVolumetricFogFinalIntegrationCS`).
//! 2. `vfog composite` render pass right after the opaque scene: the scene depth selects the slice and the HDR target is blended with
//!    `dst * V.a + V.rgb` (fixed-function blend; the HDR alpha, the ambient share used by the AO pass, is scaled by V.a too).
//!
//! The grid is 16 px x 16 px x 64 slices (`r.VolumetricFog.GridPixelSize` / `GridSizeZ`), exponentially distributed over the near plane .. 4500 cm
//! (`VolumetricFogDistance`), all stored as `Rgba16Float` exactly like the engine's PF_FloatRGBA volumes.
//!
//! What the recovered values do (measured by the tests): the density written by the material setup is `FogDensity / 1000 * 0.5 = 1.1e-7 /cm` AT the fog plane
//! (650 m up) and 57.7x that at ground level (`2^(65000 * 9e-5)`), i.e. ~6.4e-6 /cm: the volume dims the scene by ~3-4 % over its 45 m. The scattering
//! (`* ScatteringScale 0.1`, ~6e-7 /cm at ground level) times the in-scattered light lands in the half-float subnormal range, so the in-scattered
//! colour of the volume is ~1e-4 of the scene (quantisation noise): visually it is the small transmittance dim only. The passes are complete and
//! parameter-driven ([`VolFogParams`]).
use crate::camera::CameraParams;
use crate::fog;
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

pub const VOLUME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Runtime parameters of the volume (defaults = the recovered Athena Day values; tests override them).
#[derive(Clone, Copy, Debug)]
pub struct VolFogParams {
    /// Density / falloff after the engine's /1000 remap.
    pub density: f32,
    pub falloff: f32,
    pub fog_height_cm: f32,
    pub scattering_distribution: f32,
    pub scattering_scale: f32,
    pub absorption_scale: f32,
    pub distance_cm: f32,
    /// Directional light colour after the override rule (luma(light) * DirectionalInscatteringColor), and its `VolumetricScatteringIntensity`.
    pub dir_color: [f32; 3],
    pub dir_intensity: f32,
    pub dir_enabled: bool,
    /// Flat sky term (FogInscatteringColor * Y00) and `SkyLightVolumetricScatteringIntensity`.
    pub sky_color: [f32; 3],
    pub sky_intensity: f32,
    pub history_weight: f32,
    pub jitter: bool,
}
impl VolFogParams {
    pub fn athena_day() -> Self {
        // UE LightColor = colour * Brightness (no 1/pi); the renderer's sun radiance is Brightness / pi
        let sun = crate::lighting::sun_radiance().map(|c| c * std::f32::consts::PI);
        let l = fog::ue_luma(sun);
        VolFogParams {
            density: fog::day::FOG_DENSITY * fog::engine::DENSITY_REMAP, falloff: fog::day::FOG_HEIGHT_FALLOFF * fog::engine::DENSITY_REMAP,
            fog_height_cm: fog::day::HEIGHT_FOG_Z_OFFSET, scattering_distribution: fog::day::VOLUMETRIC_FOG_SCATTERING_DISTRIBUTION,
            scattering_scale: fog::day::VOLUMETRIC_FOG_SCATTERING_SCALE, absorption_scale: fog::day::VOLUMETRIC_FOG_ABSORPTION_SCALE,
            distance_cm: fog::day::VOLUMETRIC_FOG_DISTANCE,
            dir_color: fog::day::DIRECTIONAL_INSCATTERING_COLOR.map(|c| l * c), dir_intensity: fog::day::SUN_VOLUMETRIC_SCATTERING_INTENSITY, dir_enabled: true,
            sky_color: fog::sky_scatter_override(fog::day::FOG_INSCATTERING_COLOR, 1.0), sky_intensity: fog::day::SKY_VOLUMETRIC_SCATTERING_INTENSITY,
            history_weight: fog::engine::HISTORY_WEIGHT, jitter: fog::engine::JITTER,
        }
    }
}

/// `VolumetricFogTemporalRandom` (recalled engine C++, the shader only adds the value to the integer grid coordinate): Halton(2, 3, 5), or the voxel centre without jitter.
pub fn jitter_offsets(frame: u64, jitter: bool) -> [f32; 3] {
    fn halton(mut i: u64, base: u64) -> f32 {
        let (mut r, inv, mut f) = (0f64, 1.0 / base as f64, 1.0 / base as f64);
        while i > 0 { r += (i % base) as f64 * f; i /= base; f *= inv; }
        r as f32
    }
    if !jitter { return [0.5; 3]; }
    let i = frame & 1023;
    [halton(i, 2), halton(i, 3), halton(i, 5)]
}

/// GPU block (struct VFog in volfog.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct VFogUniform {
    pub grid: [f32; 4],
    pub z_params: [f32; 4],
    pub jitter: [f32; 4],
    pub fog: [f32; 4],
    pub scale: [f32; 4],
    pub dir_color: [f32; 4],
    pub sky_color: [f32; 4],
    pub prev_view_proj: [[f32; 4]; 4],
}

/// Everything `prepare` needs from the frame.
pub struct VolFogFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub camera: &'a CameraParams,
    pub frame_index: u64,
    pub shadow_view: &'a wgpu::TextureView,
    pub shadow_size: u32,
    pub depth_view: &'a wgpu::TextureView,
    /// Changes whenever the scene depth texture is recreated.
    pub depth_gen: u64,
}

struct Volume {
    grid: [u32; 3],
    vbuf_a: wgpu::Texture,
    light: [wgpu::Texture; 2],
    integrated: wgpu::Texture,
    setup_bg: wgpu::BindGroup,
    scatter_bg: [Option<wgpu::BindGroup>; 2],
    integrate_bg: [wgpu::BindGroup; 2],
    composite_bg: Option<wgpu::BindGroup>,
    key: (u32, u64),
}

pub struct VolFog {
    device: wgpu::Device,
    pub params: VolFogParams,
    setup_pipe: wgpu::ComputePipeline,
    scatter_pipe: wgpu::ComputePipeline,
    integrate_pipe: wgpu::ComputePipeline,
    composite_pipe: wgpu::RenderPipeline,
    bgl_setup: wgpu::BindGroupLayout,
    bgl_scatter: wgpu::BindGroupLayout,
    bgl_integrate: wgpu::BindGroupLayout,
    bgl_composite: wgpu::BindGroupLayout,
    uniform_buf: wgpu::Buffer,
    camera_buf: wgpu::Buffer,
    light_buf: wgpu::Buffer,
    cmp_sampler: wgpu::Sampler,
    lin_sampler: wgpu::Sampler,
    volume: Option<Volume>,
    prev_view_proj: Option<Mat4>,
    last_frame: Option<u64>,
    parity: usize,
    active: bool,
}

const C: wgpu::ShaderStages = wgpu::ShaderStages::COMPUTE;
const F: wgpu::ShaderStages = wgpu::ShaderStages::FRAGMENT;
fn e_uniform(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }
}
fn e_tex3d(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None }
}
fn e_store3d(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: C, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: VOLUME_FORMAT, view_dimension: wgpu::TextureViewDimension::D3 }, count: None }
}
fn e_depth(binding: u32, vis: wgpu::ShaderStages, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: dim, multisampled: false }, count: None }
}
fn e_sampler(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::SamplerBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Sampler(ty), count: None }
}
fn b_buf(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> { wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() } }
fn b_tex(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> { wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(v) } }
fn b_samp(binding: u32, s: &wgpu::Sampler) -> wgpu::BindGroupEntry<'_> { wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::Sampler(s) } }

impl VolFog {
    pub fn new(device: &wgpu::Device, camera_buf: &wgpu::Buffer, light_buf: &wgpu::Buffer, hdr_format: wgpu::TextureFormat) -> VolFog {
        let sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("volumetric fog"), source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!("../shaders/volfog.wgsl"))) });
        let layout = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries });
        let bgl_setup = layout("vfog setup bgl", &[e_uniform(0, C), e_uniform(1, C), e_store3d(9)]);
        let bgl_scatter = layout("vfog scatter bgl", &[
            e_uniform(0, C), e_uniform(1, C), e_uniform(2, C), e_depth(3, C, wgpu::TextureViewDimension::D2Array), e_sampler(4, C, wgpu::SamplerBindingType::Comparison),
            e_sampler(5, C, wgpu::SamplerBindingType::Filtering), e_tex3d(6, C), e_tex3d(7, C), e_store3d(8),
        ]);
        let bgl_integrate = layout("vfog integrate bgl", &[e_uniform(0, C), e_uniform(1, C), e_store3d(10), e_tex3d(11, C)]);
        let bgl_composite = layout("vfog composite bgl", &[e_uniform(0, F), e_uniform(1, F), e_sampler(5, F, wgpu::SamplerBindingType::Filtering), e_tex3d(12, F), e_depth(13, F, wgpu::TextureViewDimension::D2)]);
        let pl = |label: &str, b: &wgpu::BindGroupLayout| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(b)], immediate_size: 0 });
        let cp = |label: &str, entry: &str, b: &wgpu::BindGroupLayout| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(&pl(label, b)), module: &sm, entry_point: Some(entry), compilation_options: Default::default(), cache: None });
        let setup_pipe = cp("vfog material setup", "material_setup", &bgl_setup);
        let scatter_pipe = cp("vfog light scattering", "scatter", &bgl_scatter);
        let integrate_pipe = cp("vfog final integration", "integrate", &bgl_integrate);
        let one_src_alpha = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::SrcAlpha, operation: wgpu::BlendOperation::Add };
        let alpha_scale = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::SrcAlpha, operation: wgpu::BlendOperation::Add };
        let composite_layout = pl("vfog composite", &bgl_composite);
        let composite_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vfog composite"), layout: Some(&composite_layout),
            vertex: wgpu::VertexState { module: &sm, entry_point: Some("vs_composite"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &sm, entry_point: Some("fs_composite"), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: hdr_format, blend: Some(wgpu::BlendState { color: one_src_alpha, alpha: alpha_scale }), write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("vfog params"), size: std::mem::size_of::<VFogUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let cmp_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("vfog shadow compare"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, compare: Some(wgpu::CompareFunction::LessEqual), ..Default::default() });
        let lin_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("vfog volume"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        VolFog {
            device: device.clone(), params: VolFogParams::athena_day(), setup_pipe, scatter_pipe, integrate_pipe, composite_pipe, bgl_setup, bgl_scatter, bgl_integrate, bgl_composite,
            uniform_buf, camera_buf: camera_buf.clone(), light_buf: light_buf.clone(), cmp_sampler, lin_sampler, volume: None, prev_view_proj: None, last_frame: None, parity: 0, active: false,
        }
    }

    /// True once `prepare` ran for the frame being recorded (the renderer adds the passes only then).
    pub fn active(&self) -> bool { self.active }
    /// Called instead of `prepare` for a frame without fog: the next frame must not reproject the stale history.
    pub fn skip_frame(&mut self) { self.active = false; self.last_frame = None; }
    pub fn grid(&self) -> Option<[u32; 3]> { self.volume.as_ref().map(|v| v.grid) }

    fn create_volume(&self, grid: [u32; 3], key: (u32, u64)) -> Volume {
        let d = &self.device;
        let mk = |label: &str| d.create_texture(&wgpu::TextureDescriptor {
            label: Some(label), size: wgpu::Extent3d { width: grid[0], height: grid[1], depth_or_array_layers: grid[2] }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D3, format: VOLUME_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
        });
        let (vbuf_a, l0, l1, integrated) = (mk("vfog VBufferA"), mk("vfog light scattering 0"), mk("vfog light scattering 1"), mk("vfog integrated light scattering"));
        let v = |t: &wgpu::Texture| t.create_view(&Default::default());
        let setup_bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("vfog setup bg"), layout: &self.bgl_setup, entries: &[b_buf(0, &self.uniform_buf), b_buf(1, &self.camera_buf), b_tex(9, &v(&vbuf_a))] });
        let integrate_bg = [&l0, &l1].map(|l| d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("vfog integrate bg"), layout: &self.bgl_integrate, entries: &[
            b_buf(0, &self.uniform_buf), b_buf(1, &self.camera_buf), b_tex(10, &v(&integrated)), b_tex(11, &v(l)),
        ] }));
        Volume { grid, vbuf_a, light: [l0, l1], integrated, setup_bg, scatter_bg: [None, None], integrate_bg, composite_bg: None, key }
    }

    /// Uploads the frame's parameters and (re)builds the volume / bind groups when the viewport, the shadow map or the scene depth changed.
    pub fn prepare(&mut self, queue: &wgpu::Queue, f: &VolFogFrame) {
        let grid = fog::grid_size(f.width, f.height);
        let key = (f.shadow_size, f.depth_gen);
        let fresh = !matches!(&self.volume, Some(v) if v.grid == grid);
        if fresh { self.volume = Some(self.create_volume(grid, key)); self.last_frame = None; }
        let consecutive = self.last_frame.map_or(false, |l| l + 1 == f.frame_index);
        self.parity = (f.frame_index & 1) as usize;
        let vol = self.volume.as_mut().unwrap();
        if fresh || vol.key != key || vol.scatter_bg[0].is_none() || vol.composite_bg.is_none() {
            let d = &self.device;
            let v = |t: &wgpu::Texture| t.create_view(&Default::default());
            let (va, l0, l1, ig) = (v(&vol.vbuf_a), v(&vol.light[0]), v(&vol.light[1]), v(&vol.integrated));
            let lights = [&l0, &l1];
            vol.scatter_bg = [0usize, 1].map(|c| Some(d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("vfog scatter bg"), layout: &self.bgl_scatter, entries: &[
                b_buf(0, &self.uniform_buf), b_buf(1, &self.camera_buf), b_buf(2, &self.light_buf), b_tex(3, f.shadow_view), b_samp(4, &self.cmp_sampler), b_samp(5, &self.lin_sampler),
                b_tex(6, &va), b_tex(7, lights[1 - c]), b_tex(8, lights[c]),
            ] })));
            vol.composite_bg = Some(d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("vfog composite bg"), layout: &self.bgl_composite, entries: &[
                b_buf(0, &self.uniform_buf), b_buf(1, &self.camera_buf), b_samp(5, &self.lin_sampler), b_tex(12, &ig), b_tex(13, f.depth_view),
            ] }));
            vol.key = key;
        }
        let p = &self.params;
        let near_cm = f.camera.near * 100.0;
        let z = fog::grid_z_params(near_cm, p.distance_cm, grid[2], fog::engine::DEPTH_DISTRIBUTION_SCALE);
        let j = jitter_offsets(f.frame_index, p.jitter);
        let u = VFogUniform {
            grid: [grid[0] as f32, grid[1] as f32, grid[2] as f32, if consecutive && self.prev_view_proj.is_some() { 1.0 } else { 0.0 }],
            z_params: [z[0], z[1], z[2], near_cm],
            jitter: [j[0], j[1], j[2], p.history_weight],
            fog: [p.density, p.falloff, p.fog_height_cm, p.scattering_distribution],
            scale: [p.scattering_scale, p.absorption_scale, if p.dir_enabled { 1.0 } else { 0.0 }, 0.0],
            dir_color: [p.dir_color[0], p.dir_color[1], p.dir_color[2], p.dir_intensity],
            sky_color: [p.sky_color[0], p.sky_color[1], p.sky_color[2], p.sky_intensity],
            prev_view_proj: self.prev_view_proj.unwrap_or(Mat4::IDENTITY).to_cols_array_2d(),
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&u));
        self.prev_view_proj = Some(f.camera.proj_matrix() * f.camera.view_matrix());
        self.last_frame = Some(f.frame_index);
        self.active = true;
    }

    /// The three compute dispatches (record them inside ONE compute pass: wgpu scopes storage usage per dispatch).
    pub fn encode_compute(&self, pass: &mut wgpu::ComputePass<'_>) {
        let Some(v) = &self.volume else { return };
        let g = v.grid;
        pass.set_pipeline(&self.setup_pipe);
        pass.set_bind_group(0, &v.setup_bg, &[]);
        pass.dispatch_workgroups(g[0].div_ceil(4), g[1].div_ceil(4), g[2].div_ceil(4));
        pass.set_pipeline(&self.scatter_pipe);
        pass.set_bind_group(0, v.scatter_bg[self.parity].as_ref(), &[]);
        pass.dispatch_workgroups(g[0].div_ceil(4), g[1].div_ceil(4), g[2].div_ceil(4));
        pass.set_pipeline(&self.integrate_pipe);
        pass.set_bind_group(0, &v.integrate_bg[self.parity], &[]);
        pass.dispatch_workgroups(g[0].div_ceil(8), g[1].div_ceil(8), 1);
    }

    /// `dst * V.a + V.rgb` over the whole viewport (the pass must have no depth attachment: the scene depth is sampled).
    pub fn encode_composite(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(v) = &self.volume else { return };
        pass.set_pipeline(&self.composite_pipe);
        pass.set_bind_group(0, v.composite_bg.as_ref(), &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::CameraUniform;
    use crate::gpu::Gpu;
    use glam::Vec3;

    fn f16_round(x: f32) -> f32 {
        if x == 0.0 || !x.is_finite() { return x; }
        let a = x.abs();
        if a >= 65520.0 { return f32::INFINITY.copysign(x); }
        let e = (a.log2().floor() as i32).max(-14);
        let q = 2f32.powi(e - 10);
        ((a / q).round_ties_even() * q).copysign(x)
    }
    fn f16_to_f32(h: u16) -> f32 {
        let (s, e, m) = ((h >> 15) as u32, ((h >> 10) & 31) as i32, (h & 1023) as f32);
        let v = if e == 0 { m * 2f32.powi(-24) } else if e == 31 { if m == 0.0 { f32::INFINITY } else { f32::NAN } } else { (1.0 + m / 1024.0) * 2f32.powi(e - 15) };
        if s == 1 { -v } else { v }
    }

    #[test]
    fn jitter_is_halton_and_centre_without_jitter() {
        assert_eq!(jitter_offsets(5, false), [0.5; 3]);
        let j = jitter_offsets(1, true);
        assert!((j[0] - 0.5).abs() < 1e-6 && (j[1] - 1.0 / 3.0).abs() < 1e-6 && (j[2] - 0.2).abs() < 1e-6, "{j:?}");
        assert_eq!(jitter_offsets(0, true), [0.0; 3]);
    }
    #[test]
    fn uniform_block_is_vec4_aligned_and_wgsl_mirrors_it() {
        assert_eq!(std::mem::size_of::<VFogUniform>(), 7 * 16 + 64);
        let src = include_str!("../shaders/volfog.wgsl");
        let s = &src[src.find("struct VFog").unwrap()..];
        let s = &s[..s.find("}\n").unwrap()];
        let order = ["grid", "z_params", "jitter", "fog", "scale", "dir_color", "sky_color", "prev_view_proj"];
        let mut at = 0;
        for n in order { let i = s[at..].find(&format!("{n}:")).unwrap_or_else(|| panic!("{n} missing or out of order")); at += i; }
    }
    #[test]
    fn athena_day_params_are_the_recovered_values() {
        let p = VolFogParams::athena_day();
        assert!((p.density - 2.2e-7).abs() < 1e-12 && (p.falloff - 9e-5).abs() < 1e-10 && p.fog_height_cm == 65000.0);
        assert_eq!((p.scattering_distribution, p.scattering_scale, p.absorption_scale, p.distance_cm), (0.05, 0.1, 1.0, 4500.0));
        // override rule: luma(sun colour * brightness 3) * DirectionalInscatteringColor
        assert!(p.dir_color[2] > p.dir_color[1] && p.dir_color[0] > 0.0);
        assert!((p.sky_color[1] - 0.277_685 * 0.282_095).abs() < 1e-6 && p.sky_intensity == 0.65);
        // the fog plane (HeightFogZOffset 650 m) is far above the play space: at ground level the density is 2^(65000 * 9e-5) = 57.7x the plane value
        let (s, a) = fog::material_setup(0.0, p.density, p.falloff, p.fog_height_cm, p.scattering_scale, p.absorption_scale);
        assert!((a - 6.35e-6).abs() < 1e-7 && (s[0] - a * 0.1).abs() < 1e-9, "{a} {s:?}");
        // the half-float volumes flush the scattering (1.1e-8 at the plane) but keep the absorption (see the module docs)
        assert_eq!(f16_round(p.density * 0.5 * p.scattering_scale), 0.0);
        assert!(f16_round(a) > 0.0);
    }

    fn gpu() -> Option<Gpu> { Gpu::headless().ok() }
    fn storage(device: &wgpu::Device, label: &str, data: &[f32]) -> wgpu::Buffer {
        crate::texture::buffer_init(device, data, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, label)
    }
    fn read_buf(g: &Gpu, buf: &wgpu::Buffer, bytes: u64) -> Vec<u8> {
        let staging = g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: bytes, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = g.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(buf, 0, &staging, 0, bytes);
        g.queue.submit([enc.finish()]);
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let out = slice.get_mapped_range().unwrap().to_vec();
        staging.unmap();
        out
    }

    /// GL (Y up, metres) -> Unreal (Z up) axes for the CPU ports; the horizontal handedness is irrelevant to the fog.
    fn ue(v: Vec3, s: f32) -> [f32; 3] { [v.x * s, v.z * s, v.y * s] }

    #[test]
    fn height_fog_wgsl_matches_the_cpu_port() {
        let Some(g) = gpu() else { eprintln!("no GPU adapter: skipped"); return };
        let src = format!("{}\n{}", include_str!("../shaders/common.wgsl"), r#"
struct In { world: vec4<f32>, cam: vec4<f32>, fwd: vec4<f32>, sun: vec4<f32> }
@group(0) @binding(0) var<storage, read> ins: array<In>;
@group(0) @binding(1) var<storage, read_write> outs: array<vec4<f32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) { let i = ins[id.x]; outs[id.x] = height_fog(i.world.xyz, i.cam.xyz, i.sun.xyz, i.fwd.xyz); }
"#);
        let sm = g.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("fog test"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let sun = Vec3::new(0.3, 0.8, -0.52).normalize();
        let mut cases: Vec<(Vec3, Vec3, Vec3)> = vec![]; // camera, forward, receiver
        for cam_y in [1.75f32, 120.0, 650.0, 900.0] {
            let cam = Vec3::new(10.0, cam_y, -4.0);
            for fwd in [Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.6, 0.2, -0.77).normalize(), Vec3::new(0.0, -0.5, -0.87).normalize(), Vec3::new(-0.3, 0.7, 0.2).normalize()] {
                for dist in [0.2f32, 3.0, 30.0, 44.0, 46.0, 90.0, 400.0, 3000.0, 60000.0, 2.0e6] {
                    cases.push((cam, fwd, cam + fwd * dist));
                    // off-axis receivers (the exclusion distance depends on the angle to the view axis)
                    let side = fwd.cross(Vec3::Y).normalize_or_zero() * 0.4 * dist;
                    cases.push((cam, fwd, cam + fwd * dist + side));
                }
            }
        }
        let mut data = vec![];
        for (cam, fwd, world) in &cases { data.extend_from_slice(&[world.x, world.y, world.z, 0., cam.x, cam.y, cam.z, 0., fwd.x, fwd.y, fwd.z, 0., sun.x, sun.y, sun.z, 0.]); }
        let n = cases.len();
        let ins = storage(&g.device, "ins", &data);
        let outs = storage(&g.device, "outs", &vec![0f32; n * 4]);
        let bgl = g.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: C, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: C, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ] });
        let pl = g.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipe = g.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: Some(&pl), module: &sm, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let bg = g.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl, entries: &[b_buf(0, &ins), b_buf(1, &outs)] });
        let mut enc = g.device.create_command_encoder(&Default::default());
        { let mut p = enc.begin_compute_pass(&Default::default()); p.set_pipeline(&pipe); p.set_bind_group(0, &bg, &[]); p.dispatch_workgroups(n as u32, 1, 1); }
        g.queue.submit([enc.finish()]);
        let out: Vec<f32> = bytemuck::cast_slice(&read_buf(&g, &outs, (n * 16) as u64)).to_vec();
        let ps = fog::HeightFogPs::athena_day(true);
        // The line integral (1 - 2^-x) / x cancels catastrophically for tiny x = falloff * dz (near-horizontal rays): an f32 2^-x has a ~6e-8 spacing, i.e. up to
        // ~6e-8 / x relative error in the integral, and the GPU's exp2 and the CPU's differ in the last bits (the original shader has exactly this property on
        // real hardware). The tolerance therefore widens with that conditioning, and is a hard 2e-5 everywhere else.
        let (mut max_t, mut max_c, mut worst_ratio, mut strict) = (0f32, 0f32, 0f32, 0usize);
        for (i, (cam, fwd, world)) in cases.iter().enumerate() {
            let c = ue(*world - *cam, 100.0);
            let (rgb, t) = ps.eval(c, ue(*fwd, 1.0), ue(sun, 1.0), cam.y * 100.0);
            let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
            let along = c[0] * fwd.x + c[1] * fwd.z + c[2] * fwd.y;
            let ex = (len / along * fog::day::VOLUMETRIC_FOG_DISTANCE).max(0.0);
            let dz = c[2] * (1.0 - (ex / len).min(1.0));
            let x = (if dz.abs() > 0.01 { dz.abs() } else { 0.01 }) * ps.falloff;
            let budget = 2e-5 + (1.0 - t) * 3e-7 / x.max(1e-9);
            let dt = (out[i * 4 + 3] - t).abs();
            let dc = (0..3).map(|k| (out[i * 4 + k] - rgb[k]).abs()).fold(0f32, f32::max);
            if budget < 1e-4 { strict += 1; max_t = max_t.max(dt); max_c = max_c.max(dc); }
            worst_ratio = worst_ratio.max(dt / budget).max(dc / budget);
        }
        eprintln!("height fog WGSL vs fog.rs over {n} samples ({strict} well-conditioned): max |dT| = {max_t:.2e}, max |d rgb| = {max_c:.2e}, worst error / conditioning budget = {worst_ratio:.2}");
        assert!(strict > n / 4, "too few well-conditioned samples");
        assert!(max_t < 2e-5 && max_c < 2e-5, "well-conditioned samples: T {max_t} rgb {max_c}");
        assert!(worst_ratio < 1.0, "an ill-conditioned sample exceeded its budget: {worst_ratio}");
    }

    /// Mirrors volfog.wgsl for one frame without history on the CPU, using the fog.rs ports, with half-float rounding at every texture store.
    struct CpuVolume { grid: [u32; 3], integrated: Vec<[f32; 4]> }
    #[allow(clippy::too_many_arguments)]
    fn cpu_volume(cam: &CameraParams, p: &VolFogParams, grid: [u32; 3], j: [f32; 3], to_sun: Vec3, cascade_vis: &dyn Fn(f32) -> f32) -> CpuVolume {
        let (tx, ty) = cam.tan_half();
        let vi = cam.view_matrix().inverse();
        let zp = fog::grid_z_params(cam.near * 100.0, p.distance_cm, grid[2], fog::engine::DEPTH_DISTRIBUTION_SCALE);
        let world = |uv: [f32; 2], depth_cm: f32| {
            let ndc = [(uv[0] * 2.0 - 1.0), -(uv[1] * 2.0 - 1.0)];
            let d = depth_cm * 0.01;
            vi.transform_point3(Vec3::new(ndc[0] * tx * d, ndc[1] * ty * d, -d))
        };
        let n = (grid[0] * grid[1] * grid[2]) as usize;
        let idx = |x: u32, y: u32, z: u32| ((z * grid[1] + y) * grid[0] + x) as usize;
        let mut lightv = vec![[0f32; 4]; n];
        for z in 0..grid[2] { for y in 0..grid[1] { for x in 0..grid[0] {
            // material setup (cell centre)
            let c = [x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5];
            let wc = world([c[0] / grid[0] as f32, c[1] / grid[1] as f32], fog::depth_from_slice(zp, c[2]));
            let (s, a) = fog::material_setup(wc.y * 100.0, p.density, p.falloff, p.fog_height_cm, p.scattering_scale, p.absorption_scale);
            let (s, a) = (s.map(f16_round), f16_round(a));
            // light scattering (jittered cell)
            let cj = [x as f32 + j[0], y as f32 + j[1], z as f32 + j[2]];
            let depth_cm = fog::depth_from_slice(zp, cj[2]);
            let w = world([cj[0] / grid[0] as f32, cj[1] / grid[1] as f32], depth_cm);
            let ray = (w - cam.eye).normalize();
            let mut l = [0f32; 3];
            if p.dir_enabled {
                let vis = cascade_vis(depth_cm * 0.01);
                let k = vis * p.dir_intensity * fog::hg_phase(p.scattering_distribution, to_sun.dot(-ray));
                for i in 0..3 { l[i] += p.dir_color[i] * k; }
            }
            if p.sky_intensity > 0.0 { for i in 0..3 { l[i] += p.sky_intensity * p.sky_color[i].max(0.0); } }
            let ext = fog::ue_luma(s) + a;
            lightv[idx(x, y, z)] = [l[0] * s[0], l[1] * s[1], l[2] * s[2], ext].map(f16_round);
        } } }
        let mut integrated = vec![[0f32; 4]; n];
        for y in 0..grid[1] { for x in 0..grid[0] {
            let uv = [(x as f32 + 0.5) / grid[0] as f32, (y as f32 + 0.5) / grid[1] as f32];
            let mut prev = cam.eye;
            let (mut slices, mut steps, mut pos_all) = (vec![], vec![], vec![]);
            for z in 0..grid[2] {
                let pos = world(uv, fog::depth_from_slice(zp, z as f32 + 0.5));
                let v = lightv[idx(x, y, z)];
                slices.push(([v[0], v[1], v[2]], v[3]));
                steps.push((pos - prev).length() * 100.0);
                prev = pos;
                pos_all.push(pos);
            }
            for (z, (l, t)) in fog::final_integration(&slices, &steps).into_iter().enumerate() {
                integrated[idx(x, y, z as u32)] = [l[0], l[1], l[2], t].map(f16_round);
            }
        } }
        CpuVolume { grid, integrated }
    }

    fn read_volume(g: &Gpu, tex: &wgpu::Texture, grid: [u32; 3]) -> Vec<[f32; 4]> {
        let bpr = (grid[0] * 8).div_ceil(256) * 256;
        let size = (bpr * grid[1] * grid[2]) as u64;
        let buf = g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("vol readback"), size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = g.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(grid[1]) } },
            wgpu::Extent3d { width: grid[0], height: grid[1], depth_or_array_layers: grid[2] },
        );
        g.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range().unwrap();
        let mut out = vec![];
        for z in 0..grid[2] { for y in 0..grid[1] { for x in 0..grid[0] {
            let o = (((z * grid[1] + y) * bpr) + x * 8) as usize;
            let c = |i: usize| f16_to_f32(u16::from_le_bytes([data[o + i * 2], data[o + i * 2 + 1]]));
            out.push([c(0), c(1), c(2), c(3)]);
        } } }
        out
    }

    fn run_volume_case(g: &Gpu, params: VolFogParams, shadows: bool, label: &str) -> (f32, f32, f32) {
        let (w, h) = (192u32, 108u32);
        let cam = CameraParams::from_yaw_pitch(Vec3::new(3.0, 1.75, -2.0), 25.0, -8.0, w, h);
        let to_sun = Vec3::new(0.3, 0.8, -0.52).normalize();
        // camera + light blocks (zero light block with only what the volume reads)
        let cu: CameraUniform = cam.uniform(0.0);
        let camera_buf = g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("camera"), size: std::mem::size_of::<CameraUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        g.queue.write_buffer(&camera_buf, 0, bytemuck::bytes_of(&cu));
        let mut lb = vec![0f32; 76];
        lb[0..3].copy_from_slice(&to_sun.to_array());
        // every cascade matrix maps any point to uv (0.5, 0.5), depth 0.5
        for c in 0..3 { let m = Mat4::from_translation(Vec3::splat(0.5)) * Mat4::from_scale(Vec3::ZERO); lb[12 + c * 16..12 + c * 16 + 16].copy_from_slice(&m.to_cols_array()); }
        lb[60..64].copy_from_slice(&[10.0, 20.0, 30.0, 45.0]);
        lb[64..68].copy_from_slice(&[0.02, 0.04, 0.08, 0.0]);
        lb[68..72].copy_from_slice(&[if shadows { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0]);
        let light_buf = g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("light"), size: 304, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        g.queue.write_buffer(&light_buf, 0, bytemuck::cast_slice(&lb));
        // shadow array: layer 0 and 2 hold depth 0.25 (the receiver at 0.5 is shadowed), layer 1 holds 1.0 (lit)
        let sh = g.device.create_texture(&wgpu::TextureDescriptor { label: Some("shadow"), size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 3 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Depth32Float, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let mut enc = g.device.create_command_encoder(&Default::default());
        for (layer, depth) in [(0u32, 0.25f32), (1, 1.0), (2, 0.25)] {
            let lv = sh.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: layer, array_layer_count: Some(1), ..Default::default() });
            let _p = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &lv, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(depth), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
        }
        g.queue.submit([enc.finish()]);
        let shadow_view = sh.create_view(&Default::default());
        let depth = g.device.create_texture(&wgpu::TextureDescriptor { label: Some("depth"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Depth32Float, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let depth_view = depth.create_view(&Default::default());

        let mut vf = VolFog::new(&g.device, &camera_buf, &light_buf, wgpu::TextureFormat::Rgba16Float);
        vf.params = params;
        vf.prepare(&g.queue, &VolFogFrame { width: w, height: h, camera: &cam, frame_index: 3, shadow_view: &shadow_view, shadow_size: 4, depth_view: &depth_view, depth_gen: 1 });
        let mut enc = g.device.create_command_encoder(&Default::default());
        { let mut pass = enc.begin_compute_pass(&Default::default()); vf.encode_compute(&mut pass); }
        g.queue.submit([enc.finish()]);
        let grid = vf.grid().unwrap();
        let gpu_vol = read_volume(g, &vf.volume.as_ref().unwrap().integrated, grid);

        let j = jitter_offsets(3, params.jitter);
        // the shadow test texture: layer 0 (< 10 m) and 2 (>= 20 m, < 45 m) shadowed, layer 1 lit, >= 45 m lit
        let vis = move |d_m: f32| if !shadows || d_m >= 45.0 { 1.0 } else if d_m < 10.0 || d_m >= 20.0 { 0.0 } else { 1.0 };
        let cpu = cpu_volume(&cam, &params, grid, j, to_sun, &vis);
        assert_eq!(cpu.grid, grid);
        let (mut max_t, mut max_l, mut max_rel) = (0f32, 0f32, 0f32);
        let mut lmax = 0f32;
        for (a, b) in gpu_vol.iter().zip(&cpu.integrated) {
            max_t = max_t.max((a[3] - b[3]).abs());
            for c in 0..3 { max_l = max_l.max((a[c] - b[c]).abs()); lmax = lmax.max(b[c]); if b[c] > 1e-4 { max_rel = max_rel.max((a[c] - b[c]).abs() / b[c]); } }
        }
        let last = gpu_vol.last().unwrap();
        eprintln!("[{label}] grid {grid:?}: max |dT| {max_t:.2e}, max |d light| {max_l:.2e} (peak light {lmax:.3e}, max rel above 1e-4: {max_rel:.2e}); far corner voxel (light {:.3e} {:.3e} {:.3e}, T {:.5})", last[0], last[1], last[2], last[3]);
        (max_t, max_l / lmax.max(1e-9), lmax)
    }

    /// Every shader that calls `height_fog` / `output_hdr` still validates with the new 4-argument fog (the uncaptured-error handler of `Gpu::new` panics on an invalid module).
    #[test]
    fn every_height_fog_caller_compiles() {
        let Some(g) = gpu() else { eprintln!("no GPU adapter: skipped"); return };
        let (common, lit, sky_common) = (include_str!("../shaders/common.wgsl"), include_str!("../shaders/lit_common.wgsl"), include_str!("../shaders/sky_common.wgsl"));
        let sets: [(&str, Vec<&str>); 6] = [
            ("scene", vec![common, lit, include_str!("../shaders/scene.wgsl")]),
            ("particles", vec![common, lit, include_str!("../shaders/particles.wgsl")]),
            ("static scene", vec![common, lit, include_str!("../shaders/static_common.wgsl"), include_str!("../shaders/static_scene.wgsl")]),
            ("sky dome", vec![common, sky_common, include_str!("../shaders/sky_dome.wgsl")]),
            ("sky sun", vec![common, sky_common, include_str!("../shaders/sky_sun.wgsl")]),
            ("sky clouds", vec![common, sky_common, include_str!("../shaders/sky_clouds.wgsl")]),
        ];
        for (label, parts) in sets {
            let _ = g.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(parts.join("\n").into()) });
        }
        g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }

    /// Frame cost of the volumetric fog at 1920x1080 (grid 120 x 68 x 64), real 3 x 2048^2 shadow array: submit + wait, averaged (upper bound: includes
    /// the CPU submit latency; the in-renderer GPU timestamps are the `vfog` / `vfog composite` passes of `last_pass_timings`).
    #[test]
    fn frame_cost_at_1080p() {
        let Some(g) = gpu() else { eprintln!("no GPU adapter: skipped"); return };
        let (w, h) = (1920u32, 1080u32);
        let cam = CameraParams::from_yaw_pitch(Vec3::new(3.0, 1.75, -2.0), 25.0, -8.0, w, h);
        let camera_buf = g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("camera"), size: std::mem::size_of::<CameraUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        g.queue.write_buffer(&camera_buf, 0, bytemuck::bytes_of(&cam.uniform(0.0)));
        let mut lb = vec![0f32; 76];
        lb[0..3].copy_from_slice(&Vec3::new(0.3, 0.8, -0.52).normalize().to_array());
        for c in 0..3 { let m = Mat4::from_translation(Vec3::splat(0.5)) * Mat4::from_scale(Vec3::ZERO); lb[12 + c * 16..12 + c * 16 + 16].copy_from_slice(&m.to_cols_array()); }
        lb[60..64].copy_from_slice(&[10.0, 20.0, 30.0, 45.0]);
        lb[68] = 1.0;
        let light_buf = g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("light"), size: 304, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        g.queue.write_buffer(&light_buf, 0, bytemuck::cast_slice(&lb));
        let tex2d = |label: &str, w: u32, h: u32, layers: u32, format: wgpu::TextureFormat| g.device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: layers }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let shadow_view = tex2d("shadow", 2048, 2048, 3, wgpu::TextureFormat::Depth32Float).create_view(&Default::default());
        let depth_view = tex2d("depth", w, h, 1, wgpu::TextureFormat::Depth32Float).create_view(&Default::default());
        let hdr = tex2d("hdr", w, h, 1, wgpu::TextureFormat::Rgba16Float);
        let hdr_view = hdr.create_view(&Default::default());
        let mut vf = VolFog::new(&g.device, &camera_buf, &light_buf, wgpu::TextureFormat::Rgba16Float);
        let (mut total, mut n) = (0f64, 0u32);
        for i in 0..70u64 {
            vf.prepare(&g.queue, &VolFogFrame { width: w, height: h, camera: &cam, frame_index: i, shadow_view: &shadow_view, shadow_size: 2048, depth_view: &depth_view, depth_gen: 1 });
            let t0 = std::time::Instant::now();
            let mut enc = g.device.create_command_encoder(&Default::default());
            { let mut pass = enc.begin_compute_pass(&Default::default()); vf.encode_compute(&mut pass); }
            { let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &hdr_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })] });
              vf.encode_composite(&mut pass); }
            g.queue.submit([enc.finish()]);
            g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            if i >= 10 { total += t0.elapsed().as_secs_f64() * 1000.0; n += 1; }
        }
        eprintln!("volumetric fog 1920x1080, grid {:?}: {:.3} ms per frame (submit + wait, 3 compute dispatches + composite)", vf.grid().unwrap(), total / n as f64);
    }

    #[test]
    fn volume_passes_match_the_cpu_ports() {
        let Some(g) = gpu() else { eprintln!("no GPU adapter: skipped"); return };
        // a dense, bright test medium so the half-float volumes carry signal (the recovered Athena density flushes to zero scattering)
        let mut dense = VolFogParams::athena_day();
        dense.density = 6.0e-3; dense.falloff = 1.0e-4; dense.fog_height_cm = 0.0;
        dense.scattering_scale = 1.0; dense.history_weight = 0.0;
        dense.dir_color = [0.5, 0.45, 0.4]; dense.sky_color = [0.3, 0.4, 0.6];
        let (dt, dl, peak) = run_volume_case(&g, dense, false, "dense, no shadows");
        assert!(peak > 1e-3, "test medium must scatter, peak {peak}");
        assert!(dt < 4e-3 && dl < 3e-2, "dT {dt} relative light {dl}");
        let (dt, dl, _) = run_volume_case(&g, dense, true, "dense, cascade shadows");
        assert!(dt < 4e-3 && dl < 3e-2, "dT {dt} relative light {dl}");
        // Recovered Athena values: the transmittance must agree; the in-scattered light is ~1e-4 of the scene, made of half-float subnormal quanta, where the
        // GPU and the CPU round borderline values differently, so only its magnitude is asserted.
        let (dt, _, peak) = run_volume_case(&g, VolFogParams::athena_day(), true, "recovered Athena Day values");
        assert!(dt < 4e-3, "dT {dt}");
        assert!(peak < 1e-2, "peak in-scattered light {peak}");
    }
}
