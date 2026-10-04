//! The renderer-facing half: uploads the decoded textures / meshes of the original packages to `fn_render::fx`, builds one pipeline + bind groups per material from the
//! assembled original programs ([`crate::program::assemble`]) and the material flags, and describes the vertex streams of the two vertex factories used here.
use crate::material::{Material, MaterialFlags};
use crate::program::{assemble, Assembled, ResKind, Stage, PIXEL_BINDING_BASE};
use fn_dxbc::Opcode;
use fn_pak::PakSet;
use fn_render::fx::*;
use std::collections::HashMap;

/// The vertex factories the FX use (the FName of the type in the shader map).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vf { MeshParticle, Local, Sprite }
impl Vf {
    pub fn type_name(self) -> &'static str { match self { Vf::MeshParticle => "FMeshParticleVertexFactory", Vf::Local => "FLocalVertexFactory", Vf::Sprite => "FParticleSpriteVertexFactory" } }
}

/// Bytes per mesh-particle instance (`FMeshParticleInstanceVertex`): Transform[3] (3 x float4), SubUVParams (int4), SubUVLerpAndRelTime (float4, xy used),
/// DynamicParameter (float4), Color (float4), Velocity (float4).
pub const INSTANCE_STRIDE: u32 = 128;
/// Attribute indices of the mesh-particle instance stream (the `ATTRIBUTE n` of the vertex declaration the shaders were compiled against).
pub mod inst_attr { pub const TRANSFORM0: u32 = 8; pub const TRANSFORM1: u32 = 9; pub const TRANSFORM2: u32 = 10; pub const SUBUV: u32 = 11; pub const SUBUV_LERP_REL_TIME: u32 = 12; pub const DYNAMIC_PARAMETER: u32 = 13; pub const COLOR: u32 = 14; pub const VELOCITY: u32 = 15; }

/// Interleaved mesh vertex: position f32x3, TangentX / TangentZ / colour unorm8x4 as stored, then the UV sets.
pub fn mesh_stride(vf: Vf, num_uv: usize) -> u32 { if vf == Vf::Sprite { return SPRITE_VERTEX_STRIDE; } 24 + match vf { Vf::MeshParticle => 8 * num_uv as u32, _ => 16 * num_uv.div_ceil(2) as u32 } }
/// `FParticleSpriteVertex` + corner texture coordinate: Position+RelativeTime, OldPosition+ParticleId, Size.xy+Rotation+SubImageIndex, Color (4 x float4), TexCoord (float2).
pub const SPRITE_VERTEX_STRIDE: u32 = 72;

pub fn vertex_buffers(vf: Vf, num_uv: usize, instanced: bool) -> Vec<FxVertexBuffer> {
    if vf == Vf::Sprite {
        return vec![FxVertexBuffer { stride: SPRITE_VERTEX_STRIDE, instance: false, attrs: vec![FxAttr { location: 0, offset: 0, format: FxFmt::Float32x4 }, FxAttr { location: 1, offset: 16, format: FxFmt::Float32x4 }, FxAttr { location: 2, offset: 32, format: FxFmt::Float32x4 }, FxAttr { location: 3, offset: 48, format: FxFmt::Float32x4 }, FxAttr { location: 4, offset: 64, format: FxFmt::Float32x2 }] }];
    }
    let mut a = vec![FxAttr { location: 0, offset: 0, format: FxFmt::Float32x3 }, FxAttr { location: 1, offset: 12, format: FxFmt::Unorm8x4 }, FxAttr { location: 2, offset: 16, format: FxFmt::Unorm8x4 }, FxAttr { location: 3, offset: 20, format: FxFmt::Unorm8x4Bgra }];
    match vf {
        Vf::MeshParticle => for i in 0..num_uv { a.push(FxAttr { location: 4 + i as u32, offset: 24 + 8 * i as u32, format: FxFmt::Float32x2 }); },
        _ => for i in 0..num_uv.div_ceil(2) { a.push(FxAttr { location: 4 + i as u32, offset: 24 + 16 * i as u32, format: FxFmt::Float32x4 }); },
    }
    let mut v = vec![FxVertexBuffer { stride: mesh_stride(vf, num_uv), instance: false, attrs: a }];
    if instanced {
        v.push(FxVertexBuffer { stride: INSTANCE_STRIDE, instance: true, attrs: vec![
            FxAttr { location: inst_attr::TRANSFORM0, offset: 0, format: FxFmt::Float32x4 }, FxAttr { location: inst_attr::TRANSFORM1, offset: 16, format: FxFmt::Float32x4 }, FxAttr { location: inst_attr::TRANSFORM2, offset: 32, format: FxFmt::Float32x4 },
            FxAttr { location: inst_attr::SUBUV, offset: 48, format: FxFmt::Sint32x4 }, FxAttr { location: inst_attr::SUBUV_LERP_REL_TIME, offset: 64, format: FxFmt::Float32x4 },
            FxAttr { location: inst_attr::DYNAMIC_PARAMETER, offset: 80, format: FxFmt::Float32x4 }, FxAttr { location: inst_attr::COLOR, offset: 96, format: FxFmt::Float32x4 }, FxAttr { location: inst_attr::VELOCITY, offset: 112, format: FxFmt::Float32x4 },
        ] });
    }
    v
}

/// One uploaded static mesh (LOD 0, all sections: the FX meshes have one material slot) in the layout of [`vertex_buffers`].
pub struct MeshGpu {
    pub id: FxMeshId,
    pub num_uv: usize,
    pub vertex_count: usize,
    pub bounds_cm: ([f32; 3], [f32; 3]),
    pub name: String,
    pub material_paths: Vec<Option<String>>,
    /// (material slot, first index, index count) per section of LOD 0.
    pub sections: Vec<(usize, u32, u32)>,
}

pub fn load_mesh(set: &PakSet, fx: &mut FxRenderer, vf: Vf, object_path: &str) -> Result<MeshGpu, String> {
    let long = crate::data::long_package(object_path);
    let (a, x, b) = crate::data::package_files(set, long)?;
    let p = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| format!("{long}: {e}"))?;
    let m = fn_mesh::StaticMesh::parse(&p).map_err(|e| format!("{long}: {e}"))?;
    let lod = m.lods.first().ok_or_else(|| format!("{long}: no LOD"))?;
    let num_uv = lod.num_tex_coords;
    let stride = mesh_stride(vf, num_uv) as usize;
    let n = lod.positions.len();
    let mut bytes = vec![0u8; n * stride];
    for i in 0..n {
        let o = i * stride;
        for (k, f) in lod.positions[i].iter().enumerate() { bytes[o + 4 * k..o + 4 * k + 4].copy_from_slice(&f.to_le_bytes()); }
        bytes[o + 12..o + 16].copy_from_slice(&lod.tangent_x[i]);
        bytes[o + 16..o + 20].copy_from_slice(&lod.tangent_z[i]);
        // vertex colour: the stream is read as BGRA; meshes without colours use white (the engine's null colour stream)
        let c = lod.colors.get(i).copied().unwrap_or([255; 4]);
        bytes[o + 20..o + 24].copy_from_slice(&c);
        for u in 0..num_uv {
            let uv = lod.uvs.get(u).and_then(|s| s.get(i)).copied().unwrap_or([0.; 2]);
            let at = match vf { Vf::MeshParticle => o + 24 + 8 * u, _ => o + 24 + 16 * (u / 2) + 8 * (u % 2) };
            bytes[at..at + 4].copy_from_slice(&uv[0].to_le_bytes()); bytes[at + 4..at + 8].copy_from_slice(&uv[1].to_le_bytes());
        }
    }
    let id = fx.create_mesh(&FxMeshDesc { label: m.name.clone(), vertices: bytes, indices: lod.indices.clone() });
    let (mn, mx) = m.position_bounds(0).unwrap_or(([0.; 3], [0.; 3]));
    let sections = lod.sections.iter().map(|s| (s.material_index.max(0) as usize, s.first_index, s.num_triangles * 3)).collect();
    Ok(MeshGpu { id, num_uv, vertex_count: n, bounds_cm: (mn, mx), name: m.name.clone(), material_paths: m.materials.iter().map(|x| x.material.clone()).collect(), sections })
}

/// Textures of the original packages uploaded with their stored mip chain.
#[derive(Default)]
pub struct TextureCache { map: HashMap<String, (FxTexId, TexInfo)>, dummy_cube: Option<FxTexId>, dummy_3d: Option<FxTexId>, dummy_2d: Option<FxTexId> }
#[derive(Clone, Debug)]
pub struct TexInfo { pub filter: String, pub address_x: String, pub address_y: String, pub srgb: bool, pub width: usize, pub height: usize, pub levels: usize, pub compression: String }

fn strip_enum(s: &str) -> String { s.rsplit("::").next().unwrap_or(s).to_owned() }

impl TextureCache {
    pub fn load(&mut self, set: &PakSet, fx: &mut FxRenderer, object_path: &str) -> Result<(FxTexId, TexInfo), String> {
        if let Some(t) = self.map.get(object_path) { return Ok(t.clone()); }
        let long = crate::data::long_package(object_path);
        let (a, x, b) = crate::data::package_files(set, long)?;
        let p = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| format!("{long}: {e}"))?;
        let t = fn_texture::Texture2D::parse(&p).map_err(|e| format!("{long}: {e}"))?;
        if t.is_cube() { return Err(format!("{long}: cube map unsupported here")); }
        let mut levels = vec![];
        for i in t.available_mips() {
            let img = t.decode(i, fn_texture::DecodeOptions::default()).map_err(|e| format!("{long}: mip {i}: {e}"))?;
            levels.push(FxLevel { width: img.width as u32, height: img.height as u32, depth: 1, rgba8: img.to_rgba8() });
        }
        if levels.is_empty() { return Err(format!("{long}: no mip available")); }
        let info = TexInfo { filter: t.props.text("Filter").map(strip_enum).unwrap_or_else(|| "TF_Default".into()), address_x: t.address_x.clone(), address_y: t.address_y.clone(), srgb: t.srgb, width: levels[0].width as usize, height: levels[0].height as usize, levels: levels.len(), compression: t.compression_settings.clone() };
        let id = fx.create_texture(&FxTextureDesc { label: t.name.clone(), dim: FxTexDim::D2, srgb: t.srgb, levels });
        self.map.insert(object_path.to_owned(), (id, info.clone()));
        Ok((id, info))
    }
    pub fn dummy(&mut self, fx: &mut FxRenderer, dim: FxTexDim) -> FxTexId {
        let slot = match dim { FxTexDim::D2 => &mut self.dummy_2d, FxTexDim::D3 => &mut self.dummy_3d, FxTexDim::Cube => &mut self.dummy_cube };
        if let Some(t) = slot { return *t; }
        let layers = if dim == FxTexDim::Cube { 6 } else { 1 };
        let id = fx.create_texture(&FxTextureDesc { label: "fx dummy".into(), dim, srgb: false, levels: vec![FxLevel { width: 1, height: 1, depth: 1, rgba8: vec![0; 4 * layers] }] });
        *slot = Some(id);
        id
    }
}

/// The sampler state of a material texture slot: addressing from the texture asset (`sampler_source` 0) or the world group wrap / clamp (1 / 2); filtering from the
/// texture's `Filter` (`TF_Default` = the world texture group's anisotropic trilinear).
pub fn sampler_for(info: &TexInfo, sampler_source: i32) -> FxSamplerDesc {
    let addr = |s: &str| match s { "TA_Clamp" => FxAddress::Clamp, "TA_Mirror" => FxAddress::Mirror, _ => FxAddress::Wrap };
    let address = match sampler_source { 1 => FxAddress::Wrap, 2 => FxAddress::Clamp, _ => addr(&info.address_x) };
    match info.filter.as_str() {
        "TF_Nearest" => FxSamplerDesc { mag: FxFilter::Nearest, min: FxFilter::Nearest, mip: FxFilter::Nearest, address, aniso: 1 },
        "TF_Bilinear" => FxSamplerDesc { mag: FxFilter::Linear, min: FxFilter::Linear, mip: FxFilter::Nearest, address, aniso: 1 },
        "TF_Trilinear" => FxSamplerDesc { mag: FxFilter::Linear, min: FxFilter::Linear, mip: FxFilter::Linear, address, aniso: 1 },
        _ => FxSamplerDesc { mag: FxFilter::Linear, min: FxFilter::Linear, mip: FxFilter::Linear, address, aniso: 8 },
    }
}

/// Uniform buffers shared by every draw of a frame.
#[derive(Clone, Copy, Debug)]
pub struct SharedBuffers {
    pub view: FxBufId,
    pub primitive: FxBufId,
    /// `FParticleSpriteUniformParameters` ([`crate::view::sprite_vf_buffer`]), written once.
    pub sprite_vf: FxBufId,
    /// Lighting of lit FX materials (`FxLight`: to-sun direction, sun radiance, sky radiance; port world).
    pub light: FxBufId,
}

pub struct MaterialGpu {
    pub pipe: FxPipeId,
    pub binds: Vec<(u32, FxBindId)>,
    /// The `Material` uniform buffer of this material instance (rewrite it with [`Material::packed`] when a parameter changes).
    pub material_buf: FxBufId,
    pub material_registers: usize,
    pub assembled: Assembled,
    pub flags: MaterialFlags,
    pub translucent: bool,
    /// Bind group entries of the groups 0..3 and the (group, entry index) slots holding the `Primitive` buffer, to create one bind group set per primitive buffer.
    entries: Vec<Vec<FxBindEntry>>,
    primitive_slots: Vec<(usize, usize)>,
    material_slots: Vec<(usize, usize)>,
    group_count: usize,
}
impl MaterialGpu {
    /// Bind groups of this material that read `primitive` as their `Primitive` uniform buffer.
    pub fn bind_variant(&self, fx: &mut FxRenderer, primitive: FxBufId, material: Option<FxBufId>) -> Vec<(u32, FxBindId)> {
        let mut out = vec![];
        for (g, e) in self.entries.iter().enumerate().take(self.group_count) {
            let mut e = e.clone();
            for (sg, si) in &self.primitive_slots { if *sg == g { e[*si].res = FxRes::Buffer(primitive); } }
            if let Some(m) = material { for (sg, si) in &self.material_slots { if *sg == g { e[*si].res = FxRes::Buffer(m); } } }
            out.push((g as u32, fx.create_bind_group(self.pipe, g as u32, e)));
        }
        out
    }
}

fn blend_of(flags: &MaterialFlags) -> Result<FxBlend, String> {
    Ok(match flags.blend.as_str() { "BLEND_Opaque" | "BLEND_Masked" => FxBlend::Opaque, "BLEND_Translucent" => FxBlend::Translucent, "BLEND_Additive" => FxBlend::Additive, "BLEND_Modulate" => FxBlend::Modulate, o => return Err(format!("unsupported blend mode {o}")) })
}

/// Pipeline + bind groups of `mat` for the (vertex shader, pixel shader) type names of its `vf` permutation.
pub fn build_material(set: &PakSet, fx: &mut FxRenderer, cache: &mut TextureCache, mat: &Material, vf: Vf, vs_type: &str, ps_type: &str, num_uv: usize, instanced: bool, shared: SharedBuffers) -> Result<MaterialGpu, String> {
    // a lit (DefaultLit) material writes the deferred GBuffer: see `assemble`
    let lit = mat.flags.shading != "MSM_Unlit";
    let vs = mat.cooked.find(vf.type_name(), vs_type).ok_or_else(|| format!("{}: no {} / {vs_type} program", mat.base_path, vf.type_name()))?;
    let ps = mat.cooked.find(vf.type_name(), ps_type).ok_or_else(|| format!("{}: no {} / {ps_type} program", mat.base_path, vf.type_name()))?;
    // masked material whose base-pass pixel shader has no discard: the depth-only pixel shader clips (see `assemble`)
    let clip = if mat.flags.blend == "BLEND_Masked" && !ps.shader.body().any(|i| i.op == Opcode::Discard) { Some(mat.cooked.find(vf.type_name(), "FDepthOnlyPS").ok_or_else(|| format!("{}: masked material without a depth-only pixel shader", mat.base_path))?) } else { None };
    let asm = assemble(vs, ps, lit, clip)?;
    let flags = mat.flags.clone();
    let blend = blend_of(&flags)?;
    let packed = mat.packed(0.0);
    let declared_max = asm.resources.iter().filter(|r| r.kind == ResKind::Uniform).map(|r| r.size as usize).max().unwrap_or(0);
    let material_buf = fx.create_buffer("fx material cb", packed.registers.max(declared_max).max(1) * 16);
    let zero_buf = fx.create_buffer("fx zero cb", 512 * 16);
    let mut layouts: [Vec<FxLayoutEntry>; 4] = [vec![], vec![], vec![], vec![]];
    let mut entries: [Vec<FxBindEntry>; 4] = [vec![], vec![], vec![], vec![]];
    if lit {
        layouts[3].push(FxLayoutEntry { binding: 0, ty: FxBindTy::Uniform, vertex: false, fragment: true });
        entries[3].push(FxBindEntry { binding: 0, res: FxRes::Buffer(shared.light) });
    }
    let mut primitive_slots: Vec<(usize, usize)> = vec![];
    let mut material_slots: Vec<(usize, usize)> = vec![];
    // which unbound 2D textures (scene depth) exist per stage
    let unbound_2d = |prog: &crate::cooked::Program, slot: u32| !prog.bindings.iter().any(|b| b.kind == fn_dxbc::unreal_packet::ResourceKind::Texture && b.binding.register as u32 == slot);
    for r in &asm.resources {
        let prog = match r.stage { Stage::Vertex => vs, Stage::Pixel => ps, Stage::ClipPixel => clip.unwrap() };
        let (vertex, fragment) = (r.stage == Stage::Vertex, r.stage != Stage::Vertex);
        let g = r.group as usize;
        match r.kind {
            ResKind::Uniform => {
                let name = prog.uniform_buffer_names.as_ref().and_then(|n| n.get(r.slot as usize)).cloned().unwrap_or_default();
                let buf = match name.as_str() { "View" => shared.view, "Primitive" => shared.primitive, "Material" => material_buf, "SpriteVF" => shared.sprite_vf, _ => zero_buf };
                layouts[g].push(FxLayoutEntry { binding: r.binding, ty: FxBindTy::Uniform, vertex, fragment });
                if name == "Primitive" { primitive_slots.push((g, entries[g].len())); }
                if name == "Material" { material_slots.push((g, entries[g].len())); }
                entries[g].push(FxBindEntry { binding: r.binding, res: FxRes::Buffer(buf) });
            }
            ResKind::Tex2D => {
                let binding = prog.bindings.iter().find(|b| b.kind == fn_dxbc::unreal_packet::ResourceKind::Texture && b.binding.register as u32 == r.slot);
                match binding {
                    Some(b) => {
                        let k = (b.binding.member / 2) as usize;
                        let (path, _) = mat.texture_slot(k).ok_or_else(|| format!("{}: texture slot {k} unresolved", mat.path))?;
                        let (tex, _) = cache.load(set, fx, &path)?;
                        layouts[g].push(FxLayoutEntry { binding: r.binding, ty: FxBindTy::Tex2D { filterable: true }, vertex, fragment });
                        entries[g].push(FxBindEntry { binding: r.binding, res: FxRes::Texture(tex) });
                    }
                    None => {
                        if r.stage == Stage::Vertex { return Err(format!("{}: vertex shader texture t{} is not a material texture", mat.path, r.slot)); }
                        layouts[g].push(FxLayoutEntry { binding: r.binding, ty: FxBindTy::SceneDepth, vertex, fragment });
                        entries[g].push(FxBindEntry { binding: r.binding, res: FxRes::SceneDepth });
                    }
                }
            }
            ResKind::StorageBuf => {
                // `Buffer<float4>` SRVs of the programs (cut-out geometry tables) are not fed by these materials: an empty buffer keeps the layout valid
                layouts[g].push(FxLayoutEntry { binding: r.binding, ty: FxBindTy::StorageRead, vertex, fragment });
                entries[g].push(FxBindEntry { binding: r.binding, res: FxRes::Buffer(zero_buf) });
            }
            ResKind::Tex3D | ResKind::Cube => {
                let dim = if r.kind == ResKind::Cube { FxTexDim::Cube } else { FxTexDim::D3 };
                let ty = if dim == FxTexDim::Cube { FxBindTy::Cube { filterable: true } } else { FxBindTy::Tex3D { filterable: true } };
                let tex = cache.dummy(fx, dim);
                layouts[g].push(FxLayoutEntry { binding: r.binding, ty, vertex, fragment });
                entries[g].push(FxBindEntry { binding: r.binding, res: FxRes::Texture(tex) });
            }
            ResKind::Sampler => {
                let binding = prog.bindings.iter().find(|b| b.kind == fn_dxbc::unreal_packet::ResourceKind::Sampler && b.binding.register as u32 == r.slot);
                match binding {
                    Some(b) => {
                        let k = (b.binding.member / 2) as usize;
                        let (path, src) = mat.texture_slot(k).ok_or_else(|| format!("{}: sampler slot {k} unresolved", mat.path))?;
                        let (_, info) = cache.load(set, fx, &path)?;
                        layouts[g].push(FxLayoutEntry { binding: r.binding, ty: FxBindTy::Sampler { filtering: true }, vertex, fragment });
                        entries[g].push(FxBindEntry { binding: r.binding, res: FxRes::Sampler(sampler_for(&info, src)) });
                    }
                    None => {
                        let depth = r.stage != Stage::Vertex && unbound_2d(prog, r.slot) && prog.shader.decls.resources.iter().any(|d| d.slot == r.slot && d.dim == 3);
                        layouts[g].push(FxLayoutEntry { binding: r.binding, ty: FxBindTy::Sampler { filtering: !depth }, vertex, fragment });
                        entries[g].push(FxBindEntry { binding: r.binding, res: FxRes::Sampler(if depth { FxSamplerDesc::NEAREST_CLAMP } else { FxSamplerDesc::LINEAR_CLAMP }) });
                    }
                }
            }
        }
    }
    let _ = PIXEL_BINDING_BASE;
    let translucent = blend != FxBlend::Opaque;
    let desc = FxPipelineDesc {
        label: format!("fx {} {}", mat.base_path.rsplit('.').next().unwrap_or(""), vf.type_name()),
        wgsl: asm.wgsl.clone(), vs_entry: "vs_main".into(), fs_entry: "fs_main".into(),
        vertex_buffers: vertex_buffers(vf, num_uv, instanced),
        groups: layouts.iter().take(if lit { 4 } else { 3 }).cloned().collect(),
        blend, depth: FxDepth { test: !flags.disable_depth_test, write: !translucent },
        // UE4 D3D11: `FrontCounterClockwise = TRUE`, `CM_CW` -> cull back: front faces are the counter-clockwise ones (framebuffer space)
        cull: if flags.two_sided || std::env::var_os("FXDEBUG_NOCULL").is_some() { FxCull::None } else if std::env::var_os("FXDEBUG_CW").is_some() { FxCull::BackCw } else { FxCull::BackCcw },
        translucent,
    };
    let pipe = fx.create_pipeline(&desc)?;
    let mut binds = vec![];
    for (g, e) in entries.iter().enumerate().take(if lit { 4 } else { 3 }) { binds.push((g as u32, fx.create_bind_group(pipe, g as u32, e.clone()))); }
    fx.write_buffer(material_buf, &crate::view::bytes(&packed.cb));
    Ok(MaterialGpu { pipe, binds, material_buf, material_registers: packed.registers, assembled: asm, flags, translucent, entries: entries.into_iter().collect(), primitive_slots, material_slots, group_count: if lit { 4 } else { 3 } })
}









