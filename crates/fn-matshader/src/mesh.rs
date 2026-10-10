//! The raw GPU-skin vertex streams of a skeletal mesh LOD in the layout of Unreal's `TGPUSkinVertexFactory<false>` declaration, plus the per-section bone palettes.
//!
//! Vertex layout (one interleaved stream, 40 bytes), the attributes the recovered base-pass vertex program declares in its input signature:
//! `ATTRIBUTE0` position f32x3, `1` TangentX (4 x u8, as stored), `2` TangentZ (4 x u8, as stored, w = binormal sign), `3` bone indices (4 x u8 -> uint, SECTION LOCAL),
//! `4` bone weights (4 x u8 normalised), `5` UV0 f32x2, `13` vertex colour (the engine's null colour stream = white when the mesh has none, memory order B G R A).
//! Nothing is converted: the bytes are the cooked ones (`fn_mesh::SkeletalLod`), so the original program does the dequantisation, skinning and tangent frame itself.
use fn_mesh::SkeletalMesh;
use fn_render::gun_material::{GunAttr, GunFmt};

pub const STRIDE: u32 = 40;

pub fn attributes() -> Vec<GunAttr> {
    vec![
        GunAttr { location: 0, offset: 0, format: GunFmt::Float32x3 },
        GunAttr { location: 1, offset: 12, format: GunFmt::Unorm8x4 },
        GunAttr { location: 2, offset: 16, format: GunFmt::Unorm8x4 },
        GunAttr { location: 3, offset: 20, format: GunFmt::Uint8x4 },
        GunAttr { location: 4, offset: 24, format: GunFmt::Unorm8x4 },
        GunAttr { location: 5, offset: 28, format: GunFmt::Float32x2 },
        GunAttr { location: 13, offset: 36, format: GunFmt::Unorm8x4Bgra },
    ]
}

/// The skeletal mesh a weapon item definition names (`WeaponMeshOverride` of its `*ItemDefinition` export), decoded to GPU-skin streams.
pub fn load_weapon_mesh(set: &fn_pak::PakSet, wid_package: &str) -> Result<GunMeshData, String> {
    let pkg = fn_fx::data::PackageJson::load(set, wid_package)?;
    let def = pkg.exports.iter().find(|e| e.class.ends_with("ItemDefinition")).ok_or_else(|| format!("{wid_package}: no item definition export"))?;
    let path = def.props.get("WeaponMeshOverride").and_then(|v| v.as_str()).filter(|p| p.starts_with("/Game/")).ok_or_else(|| format!("{wid_package}: item definition has no WeaponMeshOverride"))?;
    let long = fn_fx::data::long_package(path);
    let (a, x, b) = fn_fx::data::package_files(set, long)?;
    let p = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| format!("{long}: {e}"))?;
    let mesh = SkeletalMesh::parse(&p).map_err(|e| format!("{long}: {e}"))?;
    GunMeshData::from_skeletal(&mesh)
}

pub struct SectionData {
    pub first_index: u32,
    pub index_count: u32,
    pub material_index: usize,
    /// Section-local bone index -> mesh (reference skeleton) bone index.
    pub bone_map: Vec<u16>,
    pub max_bone_influences: i32,
}

pub struct GunMeshData {
    pub name: String,
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub sections: Vec<SectionData>,
    /// Lower-cased reference-skeleton bone names (index = mesh bone index).
    pub bone_names: Vec<String>,
    pub mesh_origin: [f32; 3],
    pub mesh_extension: [f32; 3],
    /// Cooked `ImportedBounds` (UE cm): origin, box extent, sphere radius (`USkeletalMesh::GetBounds`).
    pub bounds: fn_render::primitive_ub::BoxSphereBounds,
    pub extra_bone_influences: bool,
}

impl GunMeshData {
    /// LOD 0 of `mesh`. Refuses what the 4-influence vertex factory cannot represent (extra bone influences, a missing weight stream).
    pub fn from_skeletal(mesh: &SkeletalMesh) -> Result<GunMeshData, String> {
        let lod = mesh.lods.first().ok_or("gun mesh lacks LOD0")?;
        if lod.extra_bone_influences { return Err(format!("{}: 8 bone influences per vertex (TGPUSkinVertexFactory<true>) is not wired", mesh.name)); }
        let n = lod.num_vertices;
        if lod.positions.len() != n || lod.tangent_x.len() != n || lod.tangent_z.len() != n || lod.influences.len() != n { return Err(format!("{}: vertex stream lengths disagree", mesh.name)); }
        let uv0 = lod.uvs.first().filter(|u| u.len() == n);
        let mut bytes = vec![0u8; n * STRIDE as usize];
        for i in 0..n {
            let o = i * STRIDE as usize;
            for k in 0..3 { bytes[o + 4 * k..o + 4 * k + 4].copy_from_slice(&lod.positions[i][k].to_le_bytes()); }
            bytes[o + 12..o + 16].copy_from_slice(&lod.tangent_x[i]);
            bytes[o + 16..o + 20].copy_from_slice(&lod.tangent_z[i]);
            bytes[o + 20..o + 24].copy_from_slice(&lod.influences[i].bones[..4]);
            bytes[o + 24..o + 28].copy_from_slice(&lod.influences[i].weights[..4]);
            let uv = uv0.map(|u| u[i]).unwrap_or([0.0; 2]);
            bytes[o + 28..o + 32].copy_from_slice(&uv[0].to_le_bytes());
            bytes[o + 32..o + 36].copy_from_slice(&uv[1].to_le_bytes());
            bytes[o + 36..o + 40].copy_from_slice(&lod.colors.get(i).copied().unwrap_or([255; 4]));
        }
        let sections = lod.sections.iter().map(|s| SectionData { first_index: s.base_index, index_count: s.num_triangles * 3, material_index: s.material_index as usize, bone_map: s.bone_map.clone(), max_bone_influences: s.max_bone_influences }).collect();
        Ok(GunMeshData {
            name: mesh.name.clone(), vertices: bytes, indices: lod.indices.clone(), sections,
            bone_names: mesh.ref_skeleton.iter().map(|b| b.name.to_lowercase()).collect(),
            mesh_origin: lod.mesh_origin, mesh_extension: lod.mesh_extension,
            bounds: fn_render::primitive_ub::BoxSphereBounds { origin: mesh.bounds.origin, extent: mesh.bounds.extent, radius: mesh.bounds.radius }, extra_bone_influences: lod.extra_bone_influences,
        })
    }
}
