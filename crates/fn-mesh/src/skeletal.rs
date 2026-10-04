//! `USkeletalMesh` decoding for UE 4.16 / Fortnite 1.8 (UEViewer game tag `fn18`).
//!
//! # Version emulation (fn18)
//!
//! Packages are unversioned, so the engine's latest custom versions for the build apply. UEViewer emulates them as
//! `GAME_FN18 = GAME_UE4(16)+3`. The ones that matter for skeletal meshes:
//!
//! | custom version | value | consequence |
//! |---|---|---|
//! | `FSkeletalMeshCustomVersion` | **7** (`UseSeparateSkinWeightBuffer`; real UE 4.16 is 9) | sections are combined with chunks (>=1), `TriangleSorting` byte **and** `ClothAssetSubmeshIndex` (i16) are still serialised, skin weights are a separate buffer, `FStaticLODModel` is the *pre-4.19* single structure (no render-data split) |
//! | `FEditorObjectVersion` | 17 | `bCastShadow` per section, `FSkeletalMaterial` is `Material, SlotName, UVChannelData` |
//! | `FRenderingObjectVersion` | 15 | `FMeshUVChannelInfo` follows each material slot |
//! | `FRecomputeTangentCustomVersion` | 1 | `bRecomputeTangent` per section / (not per material) |
//! | `FCoreObjectVersion` | 2 | no `bSerializeImportedMaterialSlotName` |
//!
//! `bool` members of native structures are 32 bit integers.
//!
//! # Native layout after the tagged properties (+ `bHasGuid` i32)
//!
//! ```text
//! FStripDataFlags (2)
//! FBoxSphereBounds ImportedBounds       origin(12) extent(12) radius(4)
//! TArray<FSkeletalMaterial> Materials   i32 n; { i32 objectRef, FName slotName, FMeshUVChannelInfo{bool,bool,f32[4]} = 24 }
//! FReferenceSkeleton RefSkeleton        see skeleton.rs (bone infos, 40 byte FTransforms, name map)
//! TArray<FStaticLODModel> LODModels     i32 n; each LOD below
//! (nothing else we need; Skeleton/LODInfo/MorphTargets/Sockets/bHasVertexColors are tagged properties)
//! ```
//!
//! ## FStaticLODModel (fn18 / FSkeletalMeshCustomVersion 7)
//!
//! ```text
//! FStripDataFlags (2)                               cooked: global bit0 (editor data stripped) is set
//! TArray<FSkelMeshSection> Sections                 i32 n; each:
//!     FStripDataFlags (2)
//!     i16 MaterialIndex
//!     i32 BaseIndex, i32 NumTriangles
//!     u8  TriangleSorting                           (fn18 quirk, removed in custom version 13)
//!     i32 bDisabled, i16 CorrespondClothSectionIndex  (ArVer >= 254, versions < 15 / < 14)
//!     u8  bEnableClothLOD_DEPRECATED                (ArVer >= 280)
//!     i32 bRecomputeTangent                         (RecomputeTangent version >= 1)
//!     i32 bCastShadow                               (EditorObjectVersion >= 8)
//!     u32 BaseVertexIndex
//!     [editor data stripped -> no soft vertices]
//!     TArray<u16> BoneMap                           section local bone index -> mesh bone index
//!     i32 NumVertices                               (custom version >= 4)
//!     i32 MaxBoneInfluences
//!     TArray<FApexClothPhysToRenderVertData> ClothMappingData     64 bytes each
//!     TArray<FVector> PhysicalMeshVertices, PhysicalMeshNormals   (versions < 14)
//!     i16 CorrespondClothAssetIndex
//!     i16 ClothAssetSubmeshIndex                    (fn18 quirk: version < 8 = NewClothingSystemAdded)
//! FMultisizeIndexContainer Indices                  u8 dataSize(2|4); bulk array (i32 elemSize, i32 count, data)
//! TArray<i16> ActiveBoneIndices
//! i32 Size (legacy); i32 NumVertices
//! TArray<i16> RequiredBones
//! TArray<i32> MeshToImportVertexMap; i32 MaxImportVertex     (ArVer >= 152)
//! i32 NumTexCoords
//! FSkeletalMeshVertexBuffer                         FStripDataFlags(2); i32 NumTexCoords; bool bUseFullPrecisionUVs;
//!                                                   FVector MeshExtension, MeshOrigin; bulk array of vertices:
//!                                                   TangentX (4 x u8) TangentZ (4 x u8) Position (3 x f32) UV[NumTexCoords] (2 x f16 or 2 x f32)
//! FSkinWeightVertexBuffer                           FStripDataFlags(2); bool bExtraBoneInfluences; i32 NumVertices;
//!                                                   bulk array: per vertex u8 BoneIndex[4|8], u8 BoneWeight[4|8]
//! [if bHasVertexColors] FColorVertexBuffer          FStripDataFlags(2); i32 Stride; i32 NumVertices; bulk array of FColor
//! [if !classStripped(1)] FMultisizeIndexContainer AdjacencyIndexBuffer
//! [if any section HasClothData] FSkeletalMeshVertexClothBuffer   FStripDataFlags(2) + skipped bulk array
//! ```
//!
//! `FPackedNormal` in 4.16 is stored *without* the 0x80 offset of 4.20+: each byte is `x*127.5+127.5`. TangentZ's
//! w byte holds the binormal sign (>= 128: +1).
//!
//! Skin weights are stored per vertex as bone index/weight pairs sorted by weight; bone indices are *section local*
//! (index into the section's `BoneMap`). Vertices of a section are contiguous: `BaseVertexIndex ..
//! BaseVertexIndex+NumVertices`.

use crate::common::*;
use crate::skeleton::{read_ref_skeleton, read_sockets, Bone, Socket};
use fn_ueio::props::PropValue;
use fn_ueio::{f16_to_f32, invalid, BulkData, Package, Reader, Result, BULK_AT_END, BULK_SEPARATE};

/// `FMeshUVChannelInfo` (texture streaming UV density data).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UvChannelInfo {
    pub initialized: bool,
    pub override_densities: bool,
    pub local_uv_densities: [f32; 4],
}

impl UvChannelInfo {
    pub(crate) fn read(r: &mut Reader) -> Result<UvChannelInfo> {
        let initialized = bool32(r)?;
        let override_densities = bool32(r)?;
        let mut d = [0.0; 4];
        for x in d.iter_mut() {
            *x = r.f32()?;
        }
        Ok(UvChannelInfo { initialized, override_densities, local_uv_densities: d })
    }
}

/// One material slot of a skeletal mesh (`FSkeletalMaterial`).
#[derive(Debug, Clone, PartialEq)]
pub struct SkeletalMaterial {
    /// `MaterialSlotName`.
    pub slot_name: String,
    /// Object path of the referenced material (`/Game/.../M_X.M_X`), `None` when the reference is null.
    pub material: Option<String>,
    /// Short object name of the material (`M_X`), as used by the old UEViewer exports.
    pub material_name: Option<String>,
    pub uv_channel: UvChannelInfo,
}

/// Mesh section (`FSkelMeshSection`, UE 4.13+ with merged chunk data).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Section {
    pub material_index: i16,
    /// First index in the LOD index buffer.
    pub base_index: u32,
    pub num_triangles: u32,
    /// First vertex of this section in the LOD vertex buffers.
    pub base_vertex: u32,
    pub num_vertices: u32,
    /// Section local bone index -> mesh (RefSkeleton) bone index.
    pub bone_map: Vec<u16>,
    pub max_bone_influences: i32,
    pub cast_shadow: bool,
    pub recompute_tangent: bool,
    pub disabled: bool,
    /// The section has APEX cloth mapping data.
    pub has_cloth_data: bool,
    pub cloth_asset_index: i16,
}

/// Up to 8 bone influences of one vertex (fn18 stores `u8` section-local indices).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Influences {
    /// Section local bone indices (index into `Section::bone_map`).
    pub bones: [u8; 8],
    /// Weights 0..=255 (normalised to sum 255 by the engine).
    pub weights: [u8; 8],
}

/// One LOD (`FStaticLODModel`): sections, index buffer and struct-of-arrays vertex data.
#[derive(Debug, Clone, Default)]
pub struct SkeletalLod {
    pub sections: Vec<Section>,
    /// Triangle list indices (always widened to u32).
    pub indices: Vec<u32>,
    /// The file stored 32 bit indices.
    pub indices_32bit: bool,
    pub active_bone_indices: Vec<u16>,
    pub required_bones: Vec<u16>,
    pub num_vertices: usize,
    pub num_tex_coords: usize,
    /// Positions in UE space (cm).
    pub positions: Vec<[f32; 3]>,
    /// `FPackedNormal` TangentX bytes as stored (`x*127.5+127.5`), see [`unpack_normal`].
    pub tangent_x: Vec<[u8; 4]>,
    /// `FPackedNormal` TangentZ (normal) bytes as stored; byte 3 is the binormal sign.
    pub tangent_z: Vec<[u8; 4]>,
    /// UV sets: `uvs[channel][vertex]` (halves are widened).
    pub uvs: Vec<Vec<[f32; 2]>>,
    /// The file stored full precision (f32) UVs.
    pub uv_full_precision: bool,
    /// The skin weight buffer holds 8 influences per vertex (else 4).
    pub extra_bone_influences: bool,
    pub influences: Vec<Influences>,
    /// Vertex colours in memory order of `FColor` (B, G, R, A) when `bHasVertexColors`.
    pub colors: Vec<[u8; 4]>,
    pub mesh_extension: [f32; 3],
    pub mesh_origin: [f32; 3],
}

impl SkeletalLod {
    /// Per-vertex index of the owning section (sequential chunk walk exactly like UEViewer's `ConvertMesh`; vertices
    /// not covered by a section map to the last seen one).
    pub fn vertex_sections(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.num_vertices);
        let mut sec: isize = -1;
        let mut last_vertex: i64 = -1;
        for v in 0..self.num_vertices as i64 {
            while v >= last_vertex {
                sec += 1;
                match self.sections.get(sec as usize) {
                    Some(s) => last_vertex = s.base_vertex as i64 + s.num_vertices as i64,
                    None => {
                        sec = self.sections.len() as isize - 1;
                        last_vertex = i64::MAX;
                    }
                }
            }
            out.push(sec.max(0) as usize);
        }
        out
    }

    /// Unit vertex normal (TangentZ) in UE space.
    pub fn normal(&self, vertex: usize) -> [f32; 3] {
        unpack_normal_unit(self.tangent_z[vertex])
    }

    /// Unit tangent (TangentX) in UE space.
    pub fn tangent(&self, vertex: usize) -> [f32; 3] {
        unpack_normal_unit(self.tangent_x[vertex])
    }

    /// Binormal sign (+1 / -1) stored in TangentZ.w.
    pub fn binormal_sign(&self, vertex: usize) -> f32 {
        tangent_sign(self.tangent_z[vertex])
    }

    /// Resolve the skin influences of a vertex to *mesh* bone indices through its section bone map.
    /// Returns `(bone, weight)` for all non-zero weights, in stored order (up to 8).
    pub fn resolved_influences(&self, vertex: usize, section: usize) -> Vec<(u16, u8)> {
        let inf = &self.influences[vertex];
        let n = if self.extra_bone_influences { 8 } else { 4 };
        let map = &self.sections[section].bone_map;
        (0..n).filter(|&i| inf.weights[i] != 0).map(|i| (map.get(inf.bones[i] as usize).copied().unwrap_or(0), inf.weights[i])).collect()
    }
}

/// Per-LOD info (`FSkeletalMeshLODInfo` tagged struct).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LodInfo {
    pub screen_size: f32,
    pub hysteresis: f32,
    /// Per-section material remap for this LOD (`INDEX_NONE` = no remap).
    pub lod_material_map: Vec<i32>,
}

/// Decoded `USkeletalMesh`.
#[derive(Debug, Clone)]
pub struct SkeletalMesh {
    /// Export name (e.g. `SK_M_Med_Soldier_04`).
    pub name: String,
    pub bounds: Bounds,
    pub materials: Vec<SkeletalMaterial>,
    /// Bones in mesh (RefSkeleton) order with reference pose; `retarget` is always `Animation` here.
    pub ref_skeleton: Vec<Bone>,
    /// Object path of the `Skeleton` asset property.
    pub skeleton_path: Option<String>,
    pub has_vertex_colors: bool,
    pub lods: Vec<SkeletalLod>,
    pub lod_info: Vec<LodInfo>,
    /// Sockets defined on the mesh itself (`Sockets` property).
    pub sockets: Vec<Socket>,
}

impl SkeletalMesh {
    /// Decode the first `SkeletalMesh` export of the package.
    pub fn parse(pkg: &Package) -> Result<SkeletalMesh> {
        let idx = find_class(pkg, "SkeletalMesh")?;
        let ExportBody { props, mut reader, end } = export_body(pkg, idx)?;
        let names = &pkg.names;
        let has_vertex_colors = props.bool("bHasVertexColors").unwrap_or(false);
        let r = &mut reader;

        let _strip = StripFlags::read(r)?;
        let bounds = Bounds::read(r)?;
        let n = array_count(r, 4)?;
        let mut materials = Vec::with_capacity(n);
        for _ in 0..n {
            let mref = r.i32()?;
            let slot_name = fname(r, names)?;
            // FEditorObjectVersion >= RefactorMeshEditorMaterials: no bEnableShadowCasting / bRecomputeTangent here.
            let uv_channel = UvChannelInfo::read(r)?;
            materials.push(SkeletalMaterial { slot_name, material: object_path(pkg, mref), material_name: object_name(pkg, mref), uv_channel });
        }
        let ref_skeleton = read_ref_skeleton(r, names)?;

        let n_lods = array_count(r, 4)?;
        let mut lods = Vec::with_capacity(n_lods);
        for li in 0..n_lods {
            let lod = read_lod(r, has_vertex_colors).map_err(|e| fn_ueio::Error::Invalid(format!("LOD {li}: {e}")))?;
            lods.push(lod);
        }
        if r.tell() > end {
            return invalid("SkeletalMesh read past export end");
        }

        let skeleton_path = props.object("Skeleton").and_then(|i| object_path(pkg, i));
        let lod_info = props
            .array("LODInfo")
            .map(|a| {
                a.iter()
                    .map(|v| {
                        let f = v.as_fields();
                        LodInfo {
                            screen_size: f.and_then(|f| f.float("ScreenSize").or_else(|| f.float("DisplayFactor"))).unwrap_or(1.0) as f32,
                            hysteresis: f.and_then(|f| f.float("LODHysteresis")).unwrap_or(0.0) as f32,
                            lod_material_map: match f.and_then(|f| f.get("LODMaterialMap")) {
                                Some(PropValue::Array(m)) => m.iter().map(|x| x.as_f64().unwrap_or(-1.0) as i32).collect(),
                                _ => Vec::new(),
                            },
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let sockets = read_sockets(pkg, props.array("Sockets"));
        Ok(SkeletalMesh { name: pkg.exports[idx].object_name.clone(), bounds, materials, ref_skeleton, skeleton_path, has_vertex_colors, lods, lod_info, sockets })
    }

    /// Material slot used by `section` of `lod`, honouring `LODMaterialMap` the way UEViewer does.
    pub fn section_material_index(&self, lod: usize, section: usize) -> usize {
        let mi = self.lods[lod].sections[section].material_index as i32;
        if let Some(info) = self.lod_info.get(lod) {
            if let Some(&m) = info.lod_material_map.get(section) {
                if m >= 0 && (m as usize) < self.materials.len() {
                    return m as usize;
                }
            }
        }
        mi.max(0) as usize
    }

    /// Bone index by name in the mesh's reference skeleton.
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.ref_skeleton.iter().position(|b| b.name == name)
    }
}

fn read_section(r: &mut Reader) -> Result<Section> {
    let mut s = Section::default();
    let strip = StripFlags::read(r)?;
    s.material_index = r.i16()?;
    if !strip.server_stripped() {
        s.base_index = r.u32()?;
        s.num_triangles = r.u32()?;
    }
    let _triangle_sorting = r.u8()?; // fn18: custom version < RemoveTriangleSorting(13)
    s.disabled = bool32(r)?; // < DeprecateSectionDisabledFlag(15)
    let _correspond_cloth_section = r.i16()?; // < RemoveDuplicatedClothingSections(14)
    let _enable_cloth_lod = r.u8()?; // ArVer >= APEX_CLOTH_LOD(280)
    s.recompute_tangent = bool32(r)?;
    s.cast_shadow = bool32(r)?;
    if !strip.server_stripped() {
        s.base_vertex = r.u32()?;
    }
    if !strip.editor_stripped() {
        // Editor-only soft vertices: never present in cooked fn18 data.
        return invalid("section contains editor-only soft vertices (not cooked)");
    }
    s.bone_map = read_u16_array(r)?;
    s.num_vertices = r.u32()?;
    s.max_bone_influences = r.i32()?;
    let cloth = array_count(r, 64)?;
    r.skip(cloth * 64)?; // FApexClothPhysToRenderVertData
    s.has_cloth_data = cloth > 0;
    // versions < RemoveDuplicatedClothingSections: PhysicalMeshVertices / PhysicalMeshNormals (TArray<FVector>)
    let pv = array_count(r, 12)?;
    r.skip(pv * 12)?;
    let pn = array_count(r, 12)?;
    r.skip(pn * 12)?;
    s.cloth_asset_index = r.i16()?;
    let _cloth_asset_submesh = r.i16()?; // fn18 quirk: version < NewClothingSystemAdded(8)
    Ok(s)
}

/// `FMultisizeIndexContainer`: returns `(indices, is_32bit)`.
fn read_index_container(r: &mut Reader) -> Result<(Vec<u32>, bool)> {
    // ArVer >= VER_UE4_KEEP_SKEL_MESH_INDEX_DATA: no bOldNeedCPUAccess
    let data_size = r.u8()?;
    let (esize, count) = bulk_header(r)?;
    if count == 0 {
        return Ok((Vec::new(), data_size == 4));
    }
    match (data_size, esize) {
        (2, 2) => Ok(((0..count).map(|_| r.u16().map(u32::from)).collect::<Result<_>>()?, false)),
        (4, 4) | (0, 4) => Ok(((0..count).map(|_| r.u32()).collect::<Result<_>>()?, true)),
        _ => invalid(format!("unsupported index buffer: dataSize {data_size} elemSize {esize}")),
    }
}

/// Skip an `FIntBulkData` / `FByteBulkData` block (header + inline payload).
fn skip_bulk_data(r: &mut Reader) -> Result<()> {
    let b = BulkData::read(r)?;
    if !b.is_unused() && b.flags & (BULK_AT_END | BULK_SEPARATE) == 0 {
        r.skip(b.size_on_disk.max(0) as usize)?;
    }
    Ok(())
}

fn read_lod(r: &mut Reader, has_vertex_colors: bool) -> Result<SkeletalLod> {
    let mut lod = SkeletalLod::default();
    let strip = StripFlags::read(r)?;
    let n = array_count(r, 20)?;
    for _ in 0..n {
        lod.sections.push(read_section(r)?);
    }
    let (indices, is32) = read_index_container(r)?;
    lod.indices = indices;
    lod.indices_32bit = is32;
    lod.active_bone_indices = read_i16_array(r)?.into_iter().map(|b| b as u16).collect();
    let _size = r.i32()?;
    if !strip.server_stripped() {
        lod.num_vertices = r.i32()?.max(0) as usize;
    }
    lod.required_bones = read_i16_array(r)?.into_iter().map(|b| b as u16).collect();
    if !strip.editor_stripped() {
        skip_bulk_data(r)?; // RawPointIndices
    }
    let m = array_count(r, 4)?;
    r.skip(m * 4)?; // MeshToImportVertexMap
    let _max_import_vertex = r.i32()?;

    if strip.server_stripped() {
        return Ok(lod);
    }
    lod.num_tex_coords = r.i32()?.max(0) as usize;

    // FSkeletalMeshVertexBuffer
    let vstrip = StripFlags::read(r)?;
    let num_tex = r.i32()?;
    let full_precision = bool32(r)?;
    if num_tex <= 0 || num_tex > 8 {
        return invalid(format!("implausible NumTexCoords {num_tex}"));
    }
    let num_tex = num_tex as usize;
    lod.mesh_extension = r.vec3()?;
    lod.mesh_origin = r.vec3()?;
    lod.uv_full_precision = full_precision;
    lod.num_tex_coords = num_tex;
    if !vstrip.server_stripped() {
        let (esize, count) = bulk_header(r)?;
        let uv_size = if full_precision { 8 } else { 4 } * num_tex;
        let expect = 8 + 12 + uv_size;
        if esize != expect {
            return invalid(format!("vertex stride {esize} != expected {expect} ({num_tex} UV sets, full precision {full_precision})"));
        }
        lod.positions.reserve(count);
        lod.tangent_x.reserve(count);
        lod.tangent_z.reserve(count);
        lod.uvs = vec![Vec::with_capacity(count); num_tex];
        for _ in 0..count {
            let tx = r.bytes(4)?;
            let tz = r.bytes(4)?;
            lod.tangent_x.push([tx[0], tx[1], tx[2], tx[3]]);
            lod.tangent_z.push([tz[0], tz[1], tz[2], tz[3]]);
            lod.positions.push(r.vec3()?);
            for u in lod.uvs.iter_mut() {
                if full_precision {
                    u.push([r.f32()?, r.f32()?]);
                } else {
                    u.push([f16_to_f32(r.u16()?), f16_to_f32(r.u16()?)]);
                }
            }
        }
        if lod.num_vertices == 0 {
            lod.num_vertices = count;
        } else if lod.num_vertices != count {
            return invalid(format!("LOD NumVertices {} != vertex buffer count {count}", lod.num_vertices));
        }
    }

    // FSkinWeightVertexBuffer (custom version >= UseSeparateSkinWeightBuffer(7))
    let wstrip = StripFlags::read(r)?;
    let extra = bool32(r)?;
    let wnum = r.i32()?;
    lod.extra_bone_influences = extra;
    if !wstrip.server_stripped() {
        let (esize, count) = bulk_header(r)?;
        let n = if extra { 8 } else { 4 };
        if esize != n * 2 {
            return invalid(format!("skin weight stride {esize} != {}", n * 2));
        }
        if count != lod.positions.len() || wnum as usize != count {
            return invalid(format!("skin weight count {count} (header {wnum}) != vertex count {}", lod.positions.len()));
        }
        lod.influences.reserve(count);
        for _ in 0..count {
            let raw = r.bytes(n * 2)?;
            let mut inf = Influences::default();
            inf.bones[..n].copy_from_slice(&raw[..n]);
            inf.weights[..n].copy_from_slice(&raw[n..2 * n]);
            lod.influences.push(inf);
        }
    }

    if has_vertex_colors {
        // FColorVertexBuffer (UseSharedColorBufferFormat(6) <= 7)
        let cstrip = StripFlags::read(r)?;
        let _stride = r.i32()?;
        let cnum = r.i32()?;
        if !cstrip.server_stripped() && cnum > 0 {
            let (esize, count) = bulk_header(r)?;
            if esize != 4 {
                return invalid(format!("colour stride {esize} != 4"));
            }
            for _ in 0..count {
                let c = r.bytes(4)?;
                lod.colors.push([c[0], c[1], c[2], c[3]]);
            }
        }
    }

    if !strip.class_stripped(1) {
        let _adjacency = read_index_container(r)?;
    }
    if lod.sections.iter().any(|s| s.has_cloth_data) {
        // FSkeletalMeshVertexClothBuffer
        let cs = StripFlags::read(r)?;
        if !cs.server_stripped() {
            skip_bulk_array(r)?;
            // custom version >= CompactClothVertexBuffer(10) would add a TArray<u64>: not in fn18
        }
    }
    Ok(lod)
}
