//! The particle pass: camera facing / velocity aligned sprites (sub-UV sheets) and instanced mesh particles, drawn after the scene (and after the FX pass) over the HDR
//! target with the scene depth read-only. Shader: `shaders/particles.wgsl`. The pass knows nothing about Unreal: the application creates textures / materials / meshes
//! from plain descriptions and submits a [`ParticleFrame`] per rendered frame with [`crate::Renderer::set_particle_frame`].
//!
//! Blend states are the UE 4.16 base-pass ones, identical to [`crate::fx::FxBlend`] (translucent `SrcAlpha, InvSrcAlpha` with alpha `Zero, InvSrcAlpha`, additive
//! `One, One`, modulate `DestColor, Zero`); masked and opaque materials write depth and are drawn before the blended ones.
use crate::fx::FxBlend;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct ParticleTexId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct ParticleMatId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct ParticleMeshId(pub usize);

/// Everything the shader needs from a material (see `particles.wgsl`).
#[derive(Clone, Debug)]
pub struct ParticleMaterialDesc {
    pub label: String,
    pub blend: FxBlend,
    /// Alpha-tested (opaque write): fragments with opacity below `clip` are discarded. Use `blend: Opaque`.
    pub masked: bool,
    pub clip: f32,
    pub unlit: bool,
    pub two_sided: bool,
    pub texture: Option<ParticleTexId>,
    /// Texture channel (0 R .. 3 A) of the opacity, `None` = particle alpha only.
    pub opacity_channel: Option<u8>,
    /// Channel that drives the ramp.
    pub value_channel: Option<u8>,
    /// 0 none, 2 two colours, 3 three colours (`ramp[0..n]`).
    pub ramp_mode: u8,
    pub ramp: [[f32; 3]; 3],
    pub opacity_exponent: f32,
    /// Added self-lit share: `rgb += base * emissive` (lit materials).
    pub emissive: f32,
    pub tint: [f32; 4],
}
impl Default for ParticleMaterialDesc {
    fn default() -> Self { ParticleMaterialDesc { label: String::new(), blend: FxBlend::Translucent, masked: false, clip: 0.33, unlit: false, two_sided: true, texture: None, opacity_channel: None, value_channel: None, ramp_mode: 0, ramp: [[1.0; 3]; 3], opacity_exponent: 1.0, emissive: 0.0, tint: [1.0; 4] } }
}

#[derive(Clone, Debug)]
pub struct ParticleMeshDesc { pub label: String, pub positions: Vec<[f32; 3]>, pub normals: Vec<[f32; 3]>, pub uvs: Vec<[f32; 2]>, pub indices: Vec<u32> }

/// One sprite, game space (metres).
#[derive(Clone, Copy, Debug)]
pub struct ParticleSprite {
    pub pos: [f32; 3],
    pub size: [f32; 2],
    /// Radians, about the view axis.
    pub rotation: f32,
    pub color: [f32; 4],
    /// Fractional sub-image index (row major, left to right, top to bottom).
    pub sub_image: f32,
    pub velocity: [f32; 3],
    pub dynamic: [f32; 4],
}
#[derive(Clone, Debug)]
pub struct SpriteBatch {
    pub material: ParticleMatId,
    pub columns: u32,
    pub rows: u32,
    /// Blend the two neighbouring frames (`PSUVIM_Linear_Blend`).
    pub blend_frames: bool,
    /// `PSA_Velocity`: the sprite's long axis follows the velocity.
    pub velocity_aligned: bool,
    pub pivot: [f32; 2],
    pub sprites: Vec<ParticleSprite>,
}
#[derive(Clone, Copy, Debug)]
pub struct ParticleMeshInstance {
    pub pos: [f32; 3],
    /// Orientation, columns of a rotation matrix (game space).
    pub orient: [[f32; 3]; 3],
    pub scale: [f32; 3],
    pub color: [f32; 4],
    pub dynamic: [f32; 4],
}
#[derive(Clone, Debug)]
pub struct MeshBatch { pub mesh: ParticleMeshId, pub first_index: u32, pub index_count: u32, pub material: ParticleMatId, pub instances: Vec<ParticleMeshInstance> }
#[derive(Clone, Debug, Default)]
pub struct ParticleFrame { pub sprites: Vec<SpriteBatch>, pub meshes: Vec<MeshBatch> }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParticleStats { pub sprite_batches: u32, pub sprites: u32, pub mesh_batches: u32, pub mesh_instances: u32, pub draw_calls: u32, pub skipped: u32 }

#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable)]
struct MatGpu { tint: [f32; 4], ramp0: [f32; 4], ramp1: [f32; 4], ramp2: [f32; 4], p0: [f32; 4], p1: [f32; 4] }
#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable)]
struct SpriteGpu { a: [f32; 4], b: [f32; 4], color: [f32; 4], c: [f32; 4], d: [f32; 4], e: [f32; 4] }
#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable)]
struct MeshInstGpu { r0: [f32; 4], r1: [f32; 4], r2: [f32; 4], color: [f32; 4], dynp: [f32; 4] }

struct GpuTex { view: wgpu::TextureView }
struct GpuMat { bg: wgpu::BindGroup, blend: FxBlend, masked: bool, two_sided: bool, translucent: bool }
struct GpuMesh { vb: wgpu::Buffer, ib: wgpu::Buffer }

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipeKey { mesh: bool, blend: u8, masked: bool, two_sided: bool }

#[derive(Clone, Copy)]
struct Cmd { mesh: Option<usize>, material: usize, first_instance: u32, count: u32, first_index: u32, index_count: u32 }

const SPRITE_ATTRS: [wgpu::VertexAttribute; 6] = [
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 0 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 1 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 2 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 48, shader_location: 3 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 64, shader_location: 4 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 80, shader_location: 5 },
];
const MESH_VERTEX_ATTRS: [wgpu::VertexAttribute; 3] = [
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
];
const MESH_INSTANCE_ATTRS: [wgpu::VertexAttribute; 5] = [
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 3 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 4 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 5 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 48, shader_location: 6 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 64, shader_location: 7 },
];

fn blend_state(b: FxBlend) -> Option<wgpu::BlendState> {
    use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation::Add};
    match b {
        FxBlend::Opaque => None,
        FxBlend::Translucent => Some(wgpu::BlendState { color: C { src_factor: F::SrcAlpha, dst_factor: F::OneMinusSrcAlpha, operation: Add }, alpha: C { src_factor: F::Zero, dst_factor: F::OneMinusSrcAlpha, operation: Add } }),
        FxBlend::Additive => Some(wgpu::BlendState { color: C { src_factor: F::One, dst_factor: F::One, operation: Add }, alpha: C { src_factor: F::Zero, dst_factor: F::One, operation: Add } }),
        FxBlend::Modulate => Some(wgpu::BlendState { color: C { src_factor: F::Dst, dst_factor: F::Zero, operation: Add }, alpha: C { src_factor: F::Zero, dst_factor: F::One, operation: Add } }),
    }
}
fn blend_id(b: FxBlend) -> u8 { match b { FxBlend::Opaque => 0, FxBlend::Translucent => 1, FxBlend::Additive => 2, FxBlend::Modulate => 3 } }

pub(crate) struct ParticleRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    hdr: wgpu::TextureFormat,
    depth: wgpu::TextureFormat,
    shader: wgpu::ShaderModule,
    bgl_globals: wgpu::BindGroupLayout,
    bgl_mat: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    textures: Vec<GpuTex>,
    materials: Vec<GpuMat>,
    meshes: Vec<GpuMesh>,
    pipes: HashMap<PipeKey, wgpu::RenderPipeline>,
    sprite_buf: wgpu::Buffer,
    sprite_cap: usize,
    mesh_inst_buf: wgpu::Buffer,
    mesh_cap: usize,
    frame: ParticleFrame,
    cmds: Vec<Cmd>,
    stats: ParticleStats,
}

fn buf(device: &wgpu::Device, label: &str, size: usize, usage: wgpu::BufferUsages) -> wgpu::Buffer { device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16) as u64, usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }) }

impl ParticleRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, common: &str, lit_common: &str, bgl_globals: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat, depth: wgpu::TextureFormat) -> ParticleRenderer {
        let f = wgpu::ShaderStages::FRAGMENT; let vf = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let bgl_mat = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("particle material bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: vf, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: f, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ] });
        let src = [common, lit_common, include_str!("../shaders/particles.wgsl")].join("\n");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("particles"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("particles"), bind_group_layouts: &[Some(bgl_globals), Some(&bgl_mat)], immediate_size: 0 });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("particle sampler"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        let white_tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("particle white"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &white_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, &[255; 4], wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) }, wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 });
        let white = white_tex.create_view(&Default::default());
        ParticleRenderer { device: device.clone(), queue: queue.clone(), hdr, depth, shader, bgl_globals: bgl_globals.clone(), bgl_mat, layout, sampler, white, textures: vec![], materials: vec![], meshes: vec![], pipes: HashMap::new(),
            sprite_buf: buf(device, "particle sprites", 1 << 20, wgpu::BufferUsages::VERTEX), sprite_cap: 1 << 20, mesh_inst_buf: buf(device, "particle mesh instances", 1 << 20, wgpu::BufferUsages::VERTEX), mesh_cap: 1 << 20,
            frame: ParticleFrame::default(), cmds: vec![], stats: ParticleStats::default() }
    }

    pub fn create_texture(&mut self, width: u32, height: u32, rgba8: &[u8], srgb: bool, label: &str) -> ParticleTexId {
        // CPU box-filtered mip chain (sub-UV sheets need mips to avoid shimmer at a distance)
        let mut levels: Vec<(u32, u32, Vec<u8>)> = vec![(width, height, rgba8.to_vec())];
        while levels.last().map_or(false, |l| l.0 > 1 || l.1 > 1) {
            let (w, h, src) = levels.last().unwrap().clone();
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let mut dst = vec![0u8; (nw * nh * 4) as usize];
            for y in 0..nh { for x in 0..nw { for c in 0..4 {
                let (x0, y0, x1, y1) = (x * 2, y * 2, (x * 2 + 1).min(w - 1), (y * 2 + 1).min(h - 1));
                let s = |xx: u32, yy: u32| src[((yy * w + xx) * 4) as usize + c] as u32;
                dst[((y * nw + x) * 4) as usize + c] = ((s(x0, y0) + s(x1, y0) + s(x0, y1) + s(x1, y1) + 2) / 4) as u8;
            } } }
            levels.push((nw, nh, dst));
        }
        let format = if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 }, mip_level_count: levels.len() as u32, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        for (i, (w, h, px)) in levels.iter().enumerate() {
            self.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, px, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) }, wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 });
        }
        self.textures.push(GpuTex { view: tex.create_view(&Default::default()) });
        ParticleTexId(self.textures.len() - 1)
    }

    pub fn create_mesh(&mut self, d: &ParticleMeshDesc) -> ParticleMeshId {
        let mut bytes: Vec<f32> = Vec::with_capacity(d.positions.len() * 8);
        for i in 0..d.positions.len() { bytes.extend_from_slice(&d.positions[i]); bytes.extend_from_slice(d.normals.get(i).unwrap_or(&[0.0, 1.0, 0.0])); bytes.extend_from_slice(d.uvs.get(i).unwrap_or(&[0.0, 0.0])); }
        let vb = buf(&self.device, &d.label, bytes.len() * 4, wgpu::BufferUsages::VERTEX); self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&bytes));
        let ib = buf(&self.device, &d.label, d.indices.len() * 4, wgpu::BufferUsages::INDEX); self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&d.indices));
        self.meshes.push(GpuMesh { vb, ib });
        ParticleMeshId(self.meshes.len() - 1)
    }

    pub fn create_material(&mut self, d: &ParticleMaterialDesc) -> ParticleMatId {
        let flags = (d.unlit as u32) | ((d.masked as u32) << 1) | ((d.texture.is_some() as u32) << 2) | (((d.blend == FxBlend::Additive) as u32) << 3) | (((d.blend == FxBlend::Modulate) as u32) << 4);
        let r = |i: usize| [d.ramp[i][0], d.ramp[i][1], d.ramp[i][2], 0.0];
        let g = MatGpu { tint: d.tint, ramp0: r(0), ramp1: r(1), ramp2: r(2), p0: [d.clip, d.opacity_exponent.max(0.01), flags as f32, d.ramp_mode as f32], p1: [d.opacity_channel.map_or(-1.0, |c| c as f32), d.value_channel.map_or(-1.0, |c| c as f32), d.emissive, 0.0] };
        let ub = buf(&self.device, &d.label, std::mem::size_of::<MatGpu>(), wgpu::BufferUsages::UNIFORM); self.queue.write_buffer(&ub, 0, bytemuck::bytes_of(&g));
        let view = d.texture.and_then(|t| self.textures.get(t.0)).map_or(&self.white, |t| &t.view);
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(&d.label), layout: &self.bgl_mat, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
        ] });
        self.materials.push(GpuMat { bg, blend: d.blend, masked: d.masked, two_sided: d.two_sided, translucent: d.blend != FxBlend::Opaque });
        ParticleMatId(self.materials.len() - 1)
    }

    fn pipeline(&mut self, key: PipeKey) {
        if self.pipes.contains_key(&key) { return; }
        let blend = match key.blend { 1 => FxBlend::Translucent, 2 => FxBlend::Additive, 3 => FxBlend::Modulate, _ => FxBlend::Opaque };
        let sprite_layouts = [Some(wgpu::VertexBufferLayout { array_stride: 96, step_mode: wgpu::VertexStepMode::Instance, attributes: &SPRITE_ATTRS })];
        let mesh_layouts = [Some(wgpu::VertexBufferLayout { array_stride: 32, step_mode: wgpu::VertexStepMode::Vertex, attributes: &MESH_VERTEX_ATTRS }), Some(wgpu::VertexBufferLayout { array_stride: 80, step_mode: wgpu::VertexStepMode::Instance, attributes: &MESH_INSTANCE_ATTRS })];
        // the pass binds the scene depth read-only (like the FX translucent pass): no depth writes, masked / opaque particles are drawn first
        let write = false;
        let p =self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles"), layout: Some(&self.layout),
            vertex: wgpu::VertexState { module: &self.shader, entry_point: Some(if key.mesh { "vs_mesh" } else { "vs_sprite" }), compilation_options: Default::default(), buffers: if key.mesh { &mesh_layouts } else { &sprite_layouts } },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: self.depth, depth_write_enabled: Some(write), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &self.shader, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: self.hdr, blend: blend_state(blend), write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        self.pipes.insert(key, p);
    }

    pub fn set_frame(&mut self, f: ParticleFrame) { self.frame = f; }
    pub fn stats(&self) -> &ParticleStats { &self.stats }

    /// Uploads the frame's instances and builds the draw list (opaque / masked batches first, then the blended ones in submission order).
    pub fn prepare(&mut self) {
        let frame = std::mem::take(&mut self.frame);
        self.cmds.clear(); self.stats = ParticleStats::default();
        let mut sprites: Vec<SpriteGpu> = vec![]; let mut meshes: Vec<MeshInstGpu> = vec![];
        let mut order: Vec<(bool, usize, bool)> = vec![]; // (translucent, index, is_mesh)
        for (i, b) in frame.sprites.iter().enumerate() { match self.materials.get(b.material.0) { Some(m) => order.push((m.translucent, i, false)), None => self.stats.skipped += 1 } }
        for (i, b) in frame.meshes.iter().enumerate() { match (self.materials.get(b.material.0), self.meshes.get(b.mesh.0)) { (Some(m), Some(_)) => order.push((m.translucent, i, true)), _ => self.stats.skipped += 1 } }
        order.sort_by_key(|o| o.0); // stable: opaque first, submission order inside each group
        for (_, i, is_mesh) in order {
            if !is_mesh {
                let b = &frame.sprites[i]; if b.sprites.is_empty() { continue; }
                let first = sprites.len() as u32;
                let flags = (b.blend_frames as u32 | ((b.velocity_aligned as u32) << 1)) as f32;
                for s in &b.sprites { sprites.push(SpriteGpu { a: [s.pos[0], s.pos[1], s.pos[2], s.rotation], b: [s.size[0], s.size[1], s.sub_image, s.dynamic[0]], color: s.color, c: [s.velocity[0], s.velocity[1], s.velocity[2], s.dynamic[1]], d: [b.columns.max(1) as f32, b.rows.max(1) as f32, flags, s.dynamic[2]], e: [b.pivot[0], b.pivot[1], s.dynamic[3], 0.0] }); }
                let m = &self.materials[b.material.0]; let key = PipeKey { mesh: false, blend: blend_id(m.blend), masked: m.masked, two_sided: m.two_sided };
                self.cmds.push(Cmd { mesh: None, material: b.material.0, first_instance: first, count: b.sprites.len() as u32, first_index: 0, index_count: 0 });
                self.stats.sprite_batches += 1; self.stats.sprites += b.sprites.len() as u32; let _ = key;
            } else {
                let b = &frame.meshes[i]; if b.instances.is_empty() { continue; }
                let first = meshes.len() as u32;
                for s in &b.instances {
                    let o = s.orient; let k = s.scale;
                    meshes.push(MeshInstGpu { r0: [o[0][0] * k[0], o[1][0] * k[1], o[2][0] * k[2], s.pos[0]], r1: [o[0][1] * k[0], o[1][1] * k[1], o[2][1] * k[2], s.pos[1]], r2: [o[0][2] * k[0], o[1][2] * k[1], o[2][2] * k[2], s.pos[2]], color: s.color, dynp: s.dynamic });
                }
                self.cmds.push(Cmd { mesh: Some(b.mesh.0), material: b.material.0, first_instance: first, count: b.instances.len() as u32, first_index: b.first_index, index_count: b.index_count });
                self.stats.mesh_batches += 1; self.stats.mesh_instances += b.instances.len() as u32;
            }
        }
        // pipelines
        let keys: Vec<PipeKey> = self.cmds.iter().map(|c| { let m = &self.materials[c.material]; PipeKey { mesh: c.mesh.is_some(), blend: blend_id(m.blend), masked: m.masked, two_sided: m.two_sided } }).collect();
        for k in keys { self.pipeline(k); }
        // instance buffers
        let sb: &[u8] = bytemuck::cast_slice(&sprites); let mb: &[u8] = bytemuck::cast_slice(&meshes);
        if sb.len() > self.sprite_cap { self.sprite_cap = sb.len().next_power_of_two(); self.sprite_buf = buf(&self.device, "particle sprites", self.sprite_cap, wgpu::BufferUsages::VERTEX); }
        if mb.len() > self.mesh_cap { self.mesh_cap = mb.len().next_power_of_two(); self.mesh_inst_buf = buf(&self.device, "particle mesh instances", self.mesh_cap, wgpu::BufferUsages::VERTEX); }
        if !sb.is_empty() { self.queue.write_buffer(&self.sprite_buf, 0, sb); }
        if !mb.is_empty() { self.queue.write_buffer(&self.mesh_inst_buf, 0, mb); }
        self.stats.draw_calls = self.cmds.len() as u32;
        self.frame = ParticleFrame::default();
    }

    pub fn draw_count(&self) -> usize { self.cmds.len() }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, globals: &'a wgpu::BindGroup) {
        pass.set_bind_group(0, globals, &[]);
        for c in &self.cmds {
            let m = &self.materials[c.material];
            let Some(p) = self.pipes.get(&PipeKey { mesh: c.mesh.is_some(), blend: blend_id(m.blend), masked: m.masked, two_sided: m.two_sided }) else { continue };
            pass.set_pipeline(p);
            pass.set_bind_group(1, &m.bg, &[]);
            match c.mesh {
                None => { pass.set_vertex_buffer(0, self.sprite_buf.slice(..)); pass.draw(0..6, c.first_instance..c.first_instance + c.count); }
                Some(mi) => {
                    let mesh = &self.meshes[mi];
                    pass.set_vertex_buffer(0, mesh.vb.slice(..)); pass.set_vertex_buffer(1, self.mesh_inst_buf.slice(..));
                    pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(c.first_index..c.first_index + c.index_count, 0, c.first_instance..c.first_instance + c.count);
                }
            }
        }
    }
    #[allow(dead_code)] pub fn globals_layout(&self) -> &wgpu::BindGroupLayout { &self.bgl_globals }
}
