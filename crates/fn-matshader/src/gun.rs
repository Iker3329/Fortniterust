//! The held firearm as a GPU model: per mesh section the original base-pass permutation of the section's material, its constant buffers, textures and samplers.
//!
//! Permutation (all RECOVERED / derived from the data, see the crate README of this task in `maps.rs`):
//! * shader map: `PCD3D_SM5` (D3D11 feature level SM5), quality High; the SM5 maps hold identical programs, so quality does not matter for these types;
//! * vertex factory: `TGPUSkinVertexFactoryfalse` (4 bone influences; the gun meshes have no extra influences, `fn_mesh` refuses an 8-influence LOD) - the pickaxe /
//!   guns are `bUsedWithSkeletalMesh` only, the material compiles no other skeletal factory;
//! * pass: the opaque deferred base pass: `TBasePassVSFNoLightMapPolicy` + `TBasePassPSFNoLightMapPolicySkylight`. A movable skeletal mesh has no lightmap
//!   (`FNoLightMapPolicy`); the pixel program is the one with the sky light enabled, because the Athena world has a sky light (its constants are the renderer's recovered
//!   ones). The `Simple*LightingPolicy` / `LightMapDensity` / `DebugView` / `ShadowDepth` / `HitProxy` / `Velocity` programs belong to other passes.
use crate::assemble::{assemble, layout_of, Assembled, ResKind, Stage};
use crate::frame::{self, bytes, fade_buffer, local_to_world_ue, primitive_buffer, vf_buffer};
use crate::maps::Program;
use crate::material::ResolvedMaterial;
use crate::mesh::{self, GunMeshData};
use crate::textures::{asset_sampler, sampler_member, world_group_sampler, SamplerMember, TextureCache};
use crate::view::{self, ViewInput, VIEW_FLOAT4S};
use fn_pak::PakSet;
use fn_render::gun_material::*;
use glam::Mat4;
use std::collections::HashMap;
use std::sync::Arc;

pub const VERTEX_FACTORY: &str = "TGPUSkinVertexFactoryfalse";
pub const VS_TYPE: &str = "TBasePassVSFNoLightMapPolicy";
pub const PS_TYPE: &str = "TBasePassPSFNoLightMapPolicySkylight";

/// Buffers shared by every draw of a frame.
#[derive(Clone, Copy)]
pub struct SharedBuffers { pub view: GunBufId, pub fade: GunBufId }

pub struct SectionDraw {
    pub pipe: GunPipeId,
    pub bind: GunBindId,
    pub first_index: u32,
    pub index_count: u32,
    pub bone_buf: GunBufId,
    /// Joint slot of the renderer's skin palette for every section-local bone.
    pub bone_slots: Vec<Option<usize>>,
    pub material_buf: GunBufId,
    pub material: Arc<ResolvedMaterial>,
    pub material_registers: usize,
}

pub struct GunModel {
    pub name: String,
    pub mesh: GunMeshId,
    pub sections: Vec<SectionDraw>,
    pub prim_buf: GunBufId,
    pub vf_buf: GunBufId,
    pub shared: SharedBuffers,
    pub sphere: ([f32; 3], f32),
    pub mesh_origin: [f32; 3],
    pub mesh_extension: [f32; 3],
}

pub struct FrameInput<'a> {
    pub view: ViewInput,
    /// The renderer's model matrix of the gun (port glTF mesh space -> port world).
    pub gun_to_world: Mat4,
    /// The renderer's skin matrices (glTF space, joint-slot order).
    pub skin: &'a [Mat4],
    /// `fn_render::lighting::sky_radiance()`.
    pub sky_radiance: [f32; 3],
    pub time: f32,
}

/// Everything shared between the guns: textures, resolved materials, pipelines and the shared buffers.
pub struct GunScene {
    pub shared: SharedBuffers,
    textures: TextureCache,
    materials: HashMap<String, Arc<ResolvedMaterial>>,
    pipelines: HashMap<(String, usize, usize), (GunPipeId, Arc<Assembled>)>,
    pub log: Vec<String>,
}

impl GunScene {
    pub fn new(gun: &mut GunRenderer) -> GunScene {
        let shared = SharedBuffers { view: gun.create_buffer("gun View", VIEW_FLOAT4S * 16), fade: gun.create_buffer("gun PrimitiveFade", 16) };
        GunScene { shared, textures: TextureCache::default(), materials: HashMap::new(), pipelines: HashMap::new(), log: vec![] }
    }

    fn material(&mut self, set: &PakSet, path: &str) -> Result<Arc<ResolvedMaterial>, String> {
        if let Some(m) = self.materials.get(path) { return Ok(m.clone()); }
        let m = Arc::new(ResolvedMaterial::load(set, path)?);
        self.materials.insert(path.to_owned(), m.clone());
        Ok(m)
    }

    /// Builds `mesh` with `materials[section]` (full object paths, section order). Errors name exactly what is not recoverable; nothing is guessed.
    pub fn build_gun(&mut self, set: &PakSet, gun: &mut GunRenderer, mesh: &GunMeshData, part: &fn_pose::Part, materials: &[Option<String>]) -> Result<GunModel, String> {
        if mesh.sections.len() != materials.len() { return Err(format!("{}: {} sections but {} materials", mesh.name, mesh.sections.len(), materials.len())); }
        let mesh_id = gun.create_mesh(&mesh.name, &mesh.vertices, &mesh.indices);
        let prim_buf = gun.create_buffer("gun Primitive", frame::PRIMITIVE_FLOAT4S * 16);
        let vf_buf = gun.create_buffer("gun vertex factory", frame::VF_FLOAT4S * 16);
        // reference-skeleton bone -> joint slot of the renderer's skin palette, by (lower-cased) bone name
        let slot_of: HashMap<&str, usize> = part.joints.iter().enumerate().map(|(slot, node)| (part.tree.names[*node].as_str(), slot)).collect();
        let mut sections = vec![];
        for (si, sec) in mesh.sections.iter().enumerate() {
            let path = materials[si].as_deref().ok_or_else(|| format!("{}: section {si} has no original material", mesh.name))?;
            let mat = self.material(set, path)?;
            if mat.flags.blend != "BLEND_Opaque" || mat.flags.shading != "MSM_DefaultLit" { return Err(format!("{path}: unsupported blend / shading ({} / {}): only opaque default-lit is wired", mat.flags.blend, mat.flags.shading)); }
            let map = mat.map();
            let vs = mat.maps.find(map.index, VERTEX_FACTORY, VS_TYPE).ok_or_else(|| format!("{}: no {VERTEX_FACTORY} / {VS_TYPE} program in map {}", mat.root_path, map.index))?;
            let ps = mat.maps.find(map.index, VERTEX_FACTORY, PS_TYPE).ok_or_else(|| format!("{}: no {VERTEX_FACTORY} / {PS_TYPE} program in map {}", mat.root_path, map.index))?;
            for (ty, p) in [(VS_TYPE, vs), (PS_TYPE, ps)] {
                if !mat.maps.programs_agree_across_maps(VERTEX_FACTORY, ty) { self.log.push(format!("[gun] {}: {ty} differs between the SM5 maps; map {} used", mat.root_path, map.index)); }
                let _ = p;
            }
            // pipeline (one per program pair)
            let key = (mat.root_path.clone(), vs.stream, ps.stream);
            let (pipe, asm) = match self.pipelines.get(&key) {
                Some(p) => p.clone(),
                None => {
                    let asm = Arc::new(assemble(vs, ps)?);
                    for a in &asm.vs_attributes { if !mesh::attributes().iter().any(|x| x.location == *a) { return Err(format!("{}: vertex program reads ATTRIBUTE{a}, which the GPU-skin stream does not provide", mat.root_path)); } }
                    let desc = GunPipelineDesc {
                        label: format!("gun {} {}", mat.root_path.rsplit('.').next().unwrap_or(""), VERTEX_FACTORY),
                        wgsl: asm.wgsl.clone(), vs_entry: "gun_vs_main".into(), fs_entry: "gun_fs_main".into(),
                        vertex_stride: mesh::STRIDE, vertex_attrs: mesh::attributes(), layout: layout_of(&asm.resources),
                        cull: if mat.flags.two_sided { GunCull::None } else { GunCull::BackCw },
                    };
                    let id = gun.create_pipeline(&desc)?;
                    self.pipelines.insert(key.clone(), (id, asm.clone()));
                    (id, asm)
                }
            };
            // per-section resources
            let packed = mat.packed(0.0);
            let declared = asm.resources.iter().filter(|r| r.kind == ResKind::Uniform && vs.ubo_names.get(r.slot as usize).map(|n| n == "Material").unwrap_or(false) || r.kind == ResKind::Uniform && r.stage == Stage::Pixel && ps.ubo_names.get(r.slot as usize).map(|n| n == "Material").unwrap_or(false)).map(|r| r.size as usize).max().unwrap_or(0);
            let material_registers = packed.registers.max(declared).max(1);
            let material_buf = gun.create_buffer("gun Material", material_registers * 16);
            let bone_buf = gun.create_buffer("gun bones", crate::assemble::BONE_VEC4S as usize * 16);
            let ntex = map.uniform.textures2d.len();
            let mut entries: Vec<GunBindEntry> = vec![];
            for r in &asm.resources {
                let prog: &Program = if r.stage == Stage::Vertex { vs } else { ps };
                let res = match r.kind {
                    ResKind::Uniform => {
                        let name = prog.ubo_names.get(r.slot as usize).map(|s| s.as_str()).unwrap_or("");
                        GunRes::Buffer(match (r.stage, name) {
                            (_, "View") => self.shared.view,
                            (_, "Primitive") => prim_buf,
                            (_, "PrimitiveFade") => self.shared.fade,
                            (_, "Material") => material_buf,
                            (Stage::Vertex, "") if r.slot == 0 => vf_buf,
                            _ => return Err(format!("{}: {:?} program reads uniform buffer slot {} ('{name}') which has no recovered source", mat.root_path, r.stage, r.slot)),
                        })
                    }
                    ResKind::BoneBuffer => GunRes::Buffer(bone_buf),
                    ResKind::Tex2D => {
                        if r.stage == Stage::Vertex { return Err(format!("{}: vertex program samples texture t{}", mat.root_path, r.slot)); }
                        let b = prog.bindings.iter().find(|b| b.kind == fn_dxbc::unreal_packet::ResourceKind::Texture && b.binding.register as u32 == r.slot).ok_or_else(|| format!("{}: pixel texture t{} is not a material texture (scene texture?)", mat.root_path, r.slot))?;
                        let k = (b.binding.member / 2) as usize;
                        let (tpath, _) = mat.texture_slot(k).ok_or_else(|| format!("{}: 2D texture expression {k} has no resolvable texture", mat.path))?;
                        let (id, _) = self.textures.load(set, gun, &tpath)?;
                        GunRes::Texture(id)
                    }
                    ResKind::Sampler => {
                        let b = prog.bindings.iter().find(|b| b.kind == fn_dxbc::unreal_packet::ResourceKind::Sampler && b.binding.register as u32 == r.slot).ok_or_else(|| format!("{}: sampler s{} is not a material sampler", mat.root_path, r.slot))?;
                        GunRes::Sampler(match sampler_member(b.binding.member as usize, ntex)? {
                            SamplerMember::Asset(k) => {
                                let (tpath, _) = mat.texture_slot(k).ok_or_else(|| format!("{}: sampler of texture {k} unresolved", mat.path))?;
                                let (_, info) = self.textures.load(set, gun, &tpath)?;
                                asset_sampler(&info)
                            }
                            SamplerMember::WrapWorld => world_group_sampler(false),
                            SamplerMember::ClampWorld => world_group_sampler(true),
                        })
                    }
                };
                entries.push(GunBindEntry { binding: r.binding, res });
            }
            let bind = gun.create_bind_group(pipe, &entries);
            let bone_slots = sec.bone_map.iter().map(|&b| mesh.bone_names.get(b as usize).and_then(|n| slot_of.get(n.as_str()).copied())).collect::<Vec<_>>();
            if bone_slots.len() > crate::assemble::BONE_VEC4S as usize / 3 { return Err(format!("{}: section {si} has {} bones, over the {} the uniform palette holds", mesh.name, bone_slots.len(), crate::assemble::BONE_VEC4S / 3)); }
            sections.push(SectionDraw { pipe, bind, first_index: sec.first_index, index_count: sec.index_count, bone_buf, bone_slots, material_buf, material: mat, material_registers });
        }
        Ok(GunModel { name: mesh.name.clone(), mesh: mesh_id, sections, prim_buf, vf_buf, shared: self.shared, sphere: mesh.sphere, mesh_origin: mesh.mesh_origin, mesh_extension: mesh.mesh_extension })
    }
}

impl GunModel {
    /// Appends this frame's uniform writes and draws.
    pub fn frame(&self, input: &FrameInput, out: &mut GunFrame) {
        let l2w = local_to_world_ue(input.gun_to_world);
        out.buffer_writes.push((self.shared.view, view::bytes(&input.view.view_buffer(input.sky_radiance))));
        out.buffer_writes.push((self.shared.fade, bytes(&fade_buffer())));
        out.buffer_writes.push((self.prim_buf, bytes(&primitive_buffer(&l2w, self.sphere))));
        out.buffer_writes.push((self.vf_buf, bytes(&vf_buffer(&l2w, self.mesh_origin, self.mesh_extension))));
        for s in &self.sections {
            out.buffer_writes.push((s.bone_buf, bytes(&frame::bone_buffer(input.skin, &s.bone_slots, crate::assemble::BONE_VEC4S as usize))));
            let mut cb = s.material.packed(input.time).cb;
            cb.resize(s.material_registers, [0.0; 4]);
            out.buffer_writes.push((s.material_buf, bytes(&cb)));
            out.draws.push(GunDraw { pipe: s.pipe, bind: s.bind, mesh: self.mesh, first_index: s.first_index, index_count: s.index_count });
        }
    }
}

