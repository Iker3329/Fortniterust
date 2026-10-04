//! GPU side of the textured static-mesh path: pipelines (scene + alpha-tested shadow variants), mesh / material storage, per-frame instance
//! culling (frustum, distance fade / cull, LOD with hysteresis, per-cascade light-box culling) and the batched instanced draws.
//!
//! Frame flow (`Renderer::render_scene`): `prepare` (CPU) writes ONE instance buffer holding, back to back, the instance lists of the main
//! view and of the three shadow cascades, grouped per (mesh, LOD); `draw_scene` / `draw_shadow` then issue one `draw_indexed` per
//! (mesh, LOD, section) with `first_instance..first_instance + count`, so draw calls scale with the number of distinct meshes, not with
//! the number of props.
use crate::camera::CameraParams;
use crate::lighting::{Cascade, CASCADE_COUNT, SHADOW_DISTANCE_M};
use crate::static_mesh::*;
use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use image::RgbaImage;

/// Back-face culling of one-sided materials (the exported glTF winding is counter-clockwise outward).
const CULL_ONE_SIDED: Option<wgpu::Face> = Some(wgpu::Face::Back);
/// Growth margin of the bounding sphere for wind / shake displacement (share of the mesh height).
const SWAY_MARGIN: f32 = 0.06;
const INITIAL_INSTANCE_CAPACITY: usize = 4096;

/// What the last `prepare` did (for tools, tests and the viewer HUD).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StaticStats {
    /// Instances submitted with `set_static_instances`.
    pub instances: u32,
    /// Instances drawn in the main view.
    pub visible: u32,
    pub culled_distance: u32,
    pub culled_frustum: u32,
    /// Instances in the main view per LOD.
    pub lod_counts: [u32; MAX_LODS],
    /// Instance draws summed over the three shadow cascades.
    pub shadow_instances: u32,
    /// `draw_indexed` calls of the main view / of the three cascades together.
    pub draw_calls: u32,
    pub shadow_draw_calls: u32,
    /// Triangles submitted in the main view (instances x triangles of the chosen LOD).
    pub triangles: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialGpu { tint: [f32; 4], p0: [f32; 4], p1: [f32; 4], p2: [f32; 4] }

struct GpuMaterial {
    buf: wgpu::Buffer,
    base: wgpu::TextureView,
    normal: wgpu::TextureView,
    bg: wgpu::BindGroup,
    masked: bool,
    two_sided: bool,
}
struct GpuSection { first: u32, count: u32, material: usize }
struct GpuLod { vb: wgpu::Buffer, ib: wgpu::Buffer, sections: Vec<GpuSection>, tris: u32 }
struct GpuMesh { lods: Vec<GpuLod>, limits: Vec<f32>, center: Vec3, radius: f32, height: f32, cull: f32, fade: f32, cast_shadow: bool }
#[derive(Clone, Copy)]
struct Group { mesh: u32, lod: u32, first: u32, count: u32 }

const VERTEX_ATTRS: [wgpu::VertexAttribute; 4] = [
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 32, shader_location: 3 },
];
const INSTANCE_ATTRS: [wgpu::VertexAttribute; 3] = [
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 4 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 5 },
    wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 6 },
];

fn upload_levels(device: &wgpu::Device, queue: &wgpu::Queue, levels: &[RgbaImage], format: wgpu::TextureFormat, label: &str) -> wgpu::TextureView {
    let base = &levels[0];
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label), size: wgpu::Extent3d { width: base.width(), height: base.height(), depth_or_array_layers: 1 }, mip_level_count: levels.len() as u32,
        sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
    });
    for (i, l) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            l.as_raw(), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * l.width()), rows_per_image: Some(l.height()) },
            wgpu::Extent3d { width: l.width(), height: l.height(), depth_or_array_layers: 1 },
        );
    }
    tex.create_view(&Default::default())
}

pub(crate) struct StaticRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    bgl_material: wgpu::BindGroupLayout,
    /// [cull back, two-sided]
    scene_pipes: [wgpu::RenderPipeline; 2],
    /// [opaque (vertex only), masked (alpha tested)]
    shadow_pipes: [wgpu::RenderPipeline; 2],
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    flat_normal: wgpu::TextureView,
    materials: Vec<GpuMaterial>,
    meshes: Vec<GpuMesh>,
    instances: Vec<StaticMeshInstance>,
    lod_state: Vec<u8>,
    inst_buf: wgpu::Buffer,
    inst_capacity: usize,
    all: Vec<InstanceGpu>,
    buckets: Vec<Vec<InstanceGpu>>,
    /// [main, cascade 0, 1, 2]
    groups: [Vec<Group>; 1 + CASCADE_COUNT],
    stats: StaticStats,
}

impl StaticRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, common: &str, lit_common: &str, bgl_globals: &wgpu::BindGroupLayout, bgl_shadow: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat, depth: wgpu::TextureFormat, sampler: wgpu::Sampler) -> StaticRenderer {
        let (v, f, vf) = (wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT);
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
        let bgl_material = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("static material bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: vf, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            tex_entry(1), tex_entry(2),
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: f, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ] });
        let _ = v;
        let static_common = include_str!("../shaders/static_common.wgsl");
        let scene_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("static scene"), source: wgpu::ShaderSource::Wgsl([common, lit_common, static_common, include_str!("../shaders/static_scene.wgsl")].join("\n").into()) });
        let shadow_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("static shadow"), source: wgpu::ShaderSource::Wgsl([static_common, include_str!("../shaders/static_shadow.wgsl")].join("\n").into()) });
        let layout = |label: &str, g0: &wgpu::BindGroupLayout| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(g0), Some(&bgl_material)], immediate_size: 0 });
        let scene_layout = layout("static scene layout", bgl_globals);
        let shadow_layout = layout("static shadow layout", bgl_shadow);
        let buffers = [
            Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<StaticVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &VERTEX_ATTRS }),
            Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<InstanceGpu>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &INSTANCE_ATTRS }),
        ];
        let scene_pipe = |label: &str, cull: Option<wgpu::Face>| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label), layout: Some(&scene_layout),
            vertex: wgpu::VertexState { module: &scene_sm, entry_point: Some("vs_static"), compilation_options: Default::default(), buffers: &buffers },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: cull, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: depth, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &scene_sm, entry_point: Some("fs_static"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: hdr, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        let shadow_pipe = |label: &str, masked: bool| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label), layout: Some(&shadow_layout),
            vertex: wgpu::VertexState { module: &shadow_sm, entry_point: Some("vs_static_shadow"), compilation_options: Default::default(), buffers: &buffers },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: depth, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 } }),
            multisample: Default::default(),
            fragment: if masked { Some(wgpu::FragmentState { module: &shadow_sm, entry_point: Some("fs_static_shadow"), compilation_options: Default::default(), targets: &[] }) } else { None },
            multiview_mask: None, cache: None,
        });
        let scene_pipes = [scene_pipe("static (one-sided)", CULL_ONE_SIDED), scene_pipe("static (two-sided)", None)];
        let shadow_pipes = [shadow_pipe("static shadow (opaque)", false), shadow_pipe("static shadow (masked)", true)];
        let one = |px: [u8; 4], srgb: bool, label: &str| upload_levels(device, queue, &[RgbaImage::from_pixel(1, 1, image::Rgba(px))], if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm }, label);
        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("static instances"), size: (INITIAL_INSTANCE_CAPACITY * std::mem::size_of::<InstanceGpu>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        StaticRenderer {
            device: device.clone(), queue: queue.clone(), bgl_material, scene_pipes, shadow_pipes, sampler,
            white: one([255; 4], true, "static white"), flat_normal: one([128, 128, 255, 255], false, "static flat normal"),
            materials: vec![], meshes: vec![], instances: vec![], lod_state: vec![], inst_buf, inst_capacity: INITIAL_INSTANCE_CAPACITY,
            all: vec![], buckets: vec![], groups: Default::default(), stats: StaticStats::default(),
        }
    }

    fn material_bg(&self, buf: &wgpu::Buffer, base: &wgpu::TextureView, normal: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("static material"), layout: &self.bgl_material, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(base) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(normal) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
        ] })
    }
    /// Replaces the texture sampler (anisotropy / minimum mip changed) and rebuilds every material bind group.
    pub fn set_sampler(&mut self, sampler: wgpu::Sampler) {
        self.sampler = sampler;
        let bgs: Vec<_> = self.materials.iter().map(|m| self.material_bg(&m.buf, &m.base, &m.normal)).collect();
        for (m, bg) in self.materials.iter_mut().zip(bgs) { m.bg = bg; }
    }

    pub fn create_material(&mut self, d: &StaticMaterialData) -> StaticMaterialHandle {
        let masked = d.alpha_mode == AlphaMode::Masked;
        let base = match &d.base_color {
            Some(img) => upload_levels(&self.device, &self.queue, &build_mips_srgb(img, masked.then_some(d.alpha_cutoff)), wgpu::TextureFormat::Rgba8UnormSrgb, &d.name),
            None => self.white.clone(),
        };
        let normal = match &d.normal {
            Some(img) => upload_levels(&self.device, &self.queue, &crate::texture::build_mips(img, crate::texture::MipFilter::Triangle), wgpu::TextureFormat::Rgba8Unorm, &d.name),
            None => self.flat_normal.clone(),
        };
        let u = MaterialGpu {
            tint: [d.tint[0], d.tint[1], d.tint[2], d.translucency],
            p0: [d.alpha_cutoff, masked as u32 as f32, d.two_sided as u32 as f32, d.normal.is_some() as u32 as f32],
            p1: [d.roughness, d.specular, d.sway, d.flutter],
            p2: [d.wind_from_vertex_color as u32 as f32, d.vertex_color_albedo as u32 as f32, 0., 0.],
        };
        let buf = crate::texture::buffer_init(&self.device, &[u], wgpu::BufferUsages::UNIFORM, "static material uniform");
        let bg = self.material_bg(&buf, &base, &normal);
        self.materials.push(GpuMaterial { buf, base, normal, bg, masked, two_sided: d.two_sided });
        StaticMaterialHandle(self.materials.len() - 1)
    }

    /// Uploads a mesh; `materials[section.material]` is the material of every section (missing indices fall back to the first handle).
    pub fn create_mesh(&mut self, d: &StaticMeshData, materials: &[StaticMaterialHandle]) -> StaticMeshHandle {
        assert!(!materials.is_empty(), "a static mesh needs at least one material");
        let (center, radius, height) = mesh_bounds(d);
        let lods: Vec<&StaticLod> = d.lods.iter().take(MAX_LODS).collect();
        let mut gpu_lods = vec![];
        for (i, l) in lods.iter().enumerate() {
            let label = format!("{} lod {i}", d.name);
            let vb = crate::texture::buffer_init(&self.device, &l.vertices, wgpu::BufferUsages::VERTEX, &label);
            let ib = crate::texture::buffer_init(&self.device, &l.indices, wgpu::BufferUsages::INDEX, &label);
            let sections = l.sections.iter().filter(|s| s.index_count > 0).map(|s| GpuSection { first: s.first_index, count: s.index_count, material: materials.get(s.material).copied().unwrap_or(materials[0]).0 }).collect();
            gpu_lods.push(GpuLod { vb, ib, sections, tris: (l.indices.len() / 3) as u32 });
        }
        let limits = lods.iter().take(lods.len().saturating_sub(1)).map(|l| l.max_distance).collect();
        self.meshes.push(GpuMesh { lods: gpu_lods, limits, center, radius, height, cull: d.cull_distance, fade: d.fade_range, cast_shadow: d.cast_shadow });
        StaticMeshHandle(self.meshes.len() - 1)
    }

    pub fn mesh_count(&self) -> usize { self.meshes.len() }
    pub fn material_count(&self) -> usize { self.materials.len() }
    pub fn stats(&self) -> &StaticStats { &self.stats }
    pub fn instance_count(&self) -> usize { self.instances.len() }

    pub fn set_instances(&mut self, instances: &[StaticMeshInstance]) {
        // keep the LOD state for the common case of an unchanged list; reset when the list shrinks / grows
        if self.lod_state.len() != instances.len() { self.lod_state = vec![0; instances.len()]; }
        self.instances.clear();
        self.instances.extend_from_slice(instances);
    }

    /// True when anything will be drawn (instances exist).
    pub fn any(&self) -> bool { !self.instances.is_empty() && !self.meshes.is_empty() }

    /// CPU culling + instance upload for this frame (see the module docs).
    pub fn prepare(&mut self, cam: &CameraParams, cascades: &[Cascade; CASCADE_COUNT], shadows_on: bool, time: f32) {
        self.stats = StaticStats { instances: self.instances.len() as u32, ..Default::default() };
        self.all.clear();
        for g in &mut self.groups { g.clear(); }
        if !self.any() { return; }
        let nm = self.meshes.len();
        let set_len = nm * MAX_LODS;
        if self.buckets.len() != (1 + CASCADE_COUNT) * set_len { self.buckets = vec![Vec::new(); (1 + CASCADE_COUNT) * set_len]; }
        for b in &mut self.buckets { b.clear(); }
        let planes = frustum_planes(&(cam.proj_matrix() * cam.view_matrix()));
        let light_planes: Vec<_> = cascades.iter().map(|c| frustum_planes(&c.light_view_proj)).collect();
        for (i, inst) in self.instances.iter().enumerate() {
            let Some(mesh) = self.meshes.get(inst.mesh.0) else { continue };
            if mesh.lods.is_empty() { continue; }
            let rot = Quat::from_rotation_y(inst.yaw);
            let center = inst.pos + rot * (mesh.center * inst.scale);
            let r = (mesh.radius + mesh.height * SWAY_MARGIN) * inst.scale;
            let d = (center - cam.eye).length();
            if mesh.cull > 0. && d - r >= mesh.cull { self.stats.culled_distance += 1; continue; }
            let fade = distance_fade(d, mesh.cull, mesh.fade);
            if fade <= 0. { self.stats.culled_distance += 1; continue; }
            let lod = select_lod(&mesh.limits, d, self.lod_state[i] as usize, LOD_HYSTERESIS);
            self.lod_state[i] = lod as u8;
            let mut g = InstanceGpu {
                pos_scale: [inst.pos.x, inst.pos.y, inst.pos.z, inst.scale],
                rot_fade: [inst.yaw.cos(), inst.yaw.sin(), fade, shake_amplitude(inst.shake) * inst.scale],
                wind: [wind_phase(inst.pos), time, inst.sway, 1. / mesh.height.max(0.1)],
            };
            let slot = inst.mesh.0 * MAX_LODS + lod;
            if sphere_in_planes(&planes, 6, center, r) { self.buckets[slot].push(g); } else { self.stats.culled_frustum += 1; }
            if shadows_on && mesh.cast_shadow && d - r < SHADOW_DISTANCE_M {
                g.rot_fade[2] = 1.;
                for c in 0..CASCADE_COUNT {
                    if sphere_in_planes(&light_planes[c], 4, center, r) { self.buckets[(1 + c) * set_len + slot].push(g); }
                }
            }
        }
        for s in 0..(1 + CASCADE_COUNT) {
            for slot in 0..set_len {
                let b = &self.buckets[s * set_len + slot];
                if b.is_empty() { continue; }
                let (mesh, lod) = (slot / MAX_LODS, slot % MAX_LODS);
                let gm = &self.meshes[mesh];
                self.groups[s].push(Group { mesh: mesh as u32, lod: lod as u32, first: self.all.len() as u32, count: b.len() as u32 });
                let calls = gm.lods[lod].sections.len() as u32;
                if s == 0 {
                    self.stats.visible += b.len() as u32;
                    self.stats.lod_counts[lod] += b.len() as u32;
                    self.stats.draw_calls += calls;
                    self.stats.triangles += gm.lods[lod].tris as u64 * b.len() as u64;
                } else {
                    self.stats.shadow_instances += b.len() as u32;
                    self.stats.shadow_draw_calls += calls;
                }
                self.all.extend_from_slice(b);
            }
        }
        if self.all.len() > self.inst_capacity {
            self.inst_capacity = self.all.len().next_power_of_two();
            self.inst_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("static instances"), size: (self.inst_capacity * std::mem::size_of::<InstanceGpu>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        if !self.all.is_empty() { self.queue.write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(&self.all)); }
    }

    /// Main-view draws; bind group 0 (globals) must already be set.
    pub fn draw_scene(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.groups[0].is_empty() { return; }
        pass.set_vertex_buffer(1, self.inst_buf.slice(..));
        let (mut cur_pipe, mut cur_mat) = (usize::MAX, usize::MAX);
        for g in &self.groups[0] {
            let lod = &self.meshes[g.mesh as usize].lods[g.lod as usize];
            pass.set_vertex_buffer(0, lod.vb.slice(..));
            pass.set_index_buffer(lod.ib.slice(..), wgpu::IndexFormat::Uint32);
            for s in &lod.sections {
                let m = &self.materials[s.material];
                let pi = m.two_sided as usize;
                if pi != cur_pipe { pass.set_pipeline(&self.scene_pipes[pi]); cur_pipe = pi; }
                if s.material != cur_mat { pass.set_bind_group(1, &m.bg, &[]); cur_mat = s.material; }
                pass.draw_indexed(s.first..s.first + s.count, 0, g.first..g.first + g.count);
            }
        }
    }
    /// Shadow-cascade draws (bind group 0 = the cascade's light view-projection is set here).
    pub fn draw_shadow(&self, pass: &mut wgpu::RenderPass<'_>, cascade: usize, vp_bg: &wgpu::BindGroup) {
        let groups = &self.groups[1 + cascade];
        if groups.is_empty() { return; }
        pass.set_bind_group(0, vp_bg, &[]);
        pass.set_vertex_buffer(1, self.inst_buf.slice(..));
        let (mut cur_pipe, mut cur_mat) = (usize::MAX, usize::MAX);
        for g in groups {
            let lod = &self.meshes[g.mesh as usize].lods[g.lod as usize];
            pass.set_vertex_buffer(0, lod.vb.slice(..));
            pass.set_index_buffer(lod.ib.slice(..), wgpu::IndexFormat::Uint32);
            for s in &lod.sections {
                let m = &self.materials[s.material];
                let pi = m.masked as usize;
                if pi != cur_pipe { pass.set_pipeline(&self.shadow_pipes[pi]); cur_pipe = pi; }
                if s.material != cur_mat { pass.set_bind_group(1, &m.bg, &[]); cur_mat = s.material; }
                pass.draw_indexed(s.first..s.first + s.count, 0, g.first..g.first + g.count);
            }
        }
    }
}
