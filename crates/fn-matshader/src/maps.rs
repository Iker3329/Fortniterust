//! The `FMaterialShaderMap`s of one cooked material package (UE 4.16) and the programs each of them owns.
//!
//! RECOVERED from the original bytes (WeaponMat_Base, BackPackShrinkingEffect):
//! * A cooked material carries one shader map per (quality level, feature level): for `WeaponMat_Base` six maps, three `PCD3D_SM5` followed by three `PCD3D_SM4`. The
//!   map starts at its `FMaterialShaderMapId` (`u32 usage, FGuid base material id, i32 quality level, i32 feature level, ...`, quality 0 = Low, 1 = High, 3 = `Num`
//!   (the unspecialised entry); feature level 2 = SM4, 3 = SM5), continues with the dependency hashes, the friendly-name FString, the
//!   `FMaterialUniformExpressionSet` and the `Compiling <name>: Platform=...` description, and is followed by its shader records.
//! * Every shader record is `FName type + u32 absolute end offset`, the serialised `FShader` (its vertex factory type FName is inside, `TGPUSkinVertexFactoryfalse`,
//!   `FLocalVertexFactory` ...) and the zlib stream of the bytecode. The records of one map follow the map's uniform expression set until the next map's id.
//! No permutation is chosen from a shader name alone: [`MaterialMaps::find`] needs (map, vertex factory, shader type).
use fn_dxbc::extract::{scan_blobs, serialized_stream, Blob};
use fn_dxbc::unreal_packet::{named_bindings, optional_fields, uniform_buffer_names, NamedBinding, ResourceTable};
use fn_dxbc::Shader;
use fn_dxbc_cli::uniform::{compilation_description, preceding_label, set_candidates, UniformSet};

/// UE 4.16 `ERHIFeatureLevel`: ES2 0, ES3_1 1, SM4 2, SM5 3.
pub const FEATURE_SM5: u32 = 3;
pub const FEATURE_SM4: u32 = 2;
/// UE 4.16 `EMaterialQualityLevel`: Low 0, High 1, Medium 2, Num 3.
pub const QUALITY_HIGH: u32 = 1;

pub struct ShaderMap {
    pub index: usize,
    /// Offset of the map's `FMaterialShaderMapId` in the `.uexp` (`None` when the id pattern was not found).
    pub id_start: Option<usize>,
    pub uniform_start: usize,
    pub uniform_end: usize,
    /// `PCD3D_SM5` / `PCD3D_SM4` from the serialised compile description.
    pub platform: String,
    pub quality: Option<u32>,
    pub feature_level: Option<u32>,
    pub uniform: UniformSet,
}

pub struct Program {
    pub map: usize,
    /// Serialised shader type name, e.g. `TBasePassVSFNoLightMapPolicy`.
    pub shader_type: String,
    /// Vertex factory type FName stored inside the record (`TGPUSkinVertexFactoryfalse`, `FLocalVertexFactory`, empty for map-only shaders).
    pub vertex_factory: String,
    /// zlib stream offset in the `.uexp` (the id of the old `shader_<id>` reports).
    pub stream: usize,
    pub blob: Blob,
    pub shader: Shader,
    pub ubo_names: Vec<String>,
    pub resource_table: ResourceTable,
    pub bindings: Vec<NamedBinding>,
}

pub struct MaterialMaps {
    pub label: String,
    pub maps: Vec<ShaderMap>,
    pub programs: Vec<Program>,
}

fn rd(b: &[u8], o: usize) -> u32 { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) }

/// Finds the `FMaterialShaderMapId` head `[u32 usage = 0][guid][i32 quality][i32 feature level]` in `[lo, hi)` (first match).
fn find_id(data: &[u8], lo: usize, hi: usize) -> Option<(usize, u32, u32)> {
    let hi = hi.min(data.len().saturating_sub(28));
    (lo..hi).find_map(|o| {
        let (q, f) = (rd(data, o + 20), rd(data, o + 24));
        (rd(data, o) == 0 && q <= 3 && f <= 3 && data[o + 4..o + 20].iter().filter(|&&b| b != 0).count() > 12).then_some((o, q, f))
    })
}

impl MaterialMaps {
    /// `uexp` = the `.uexp` bytes, `names` the package name table, `header` the `.uasset` total header size, `label` the material's object name.
    pub fn parse(uexp: &[u8], names: &[String], header: usize, label: &str) -> Result<MaterialMaps, String> {
        // the maps: byte-pattern uniform sets with the exact material-label anchor and the compile description
        let mut maps: Vec<ShaderMap> = vec![];
        let mut prev_end = 0usize;
        for c in set_candidates(uexp, names) {
            if c.start < prev_end || !preceding_label(uexp, c.start, label) { continue; }
            let Ok(desc) = compilation_description(uexp, &c, label) else { continue };
            let platform = desc.text.split("Platform=").nth(1).and_then(|r| r.split(',').next()).unwrap_or("").to_owned();
            let id = find_id(uexp, prev_end, c.start);
            maps.push(ShaderMap { index: maps.len(), id_start: id.map(|i| i.0), uniform_start: c.start, uniform_end: c.end, platform, quality: id.map(|i| i.1), feature_level: id.map(|i| i.2), uniform: c.set });
            prev_end = desc.end;
        }
        if maps.is_empty() { return Err(format!("{label}: no shader map (label-anchored uniform expression set) found")); }
        let mut programs = vec![];
        let mut search_from = 0usize;
        for blob in scan_blobs(uexp) {
            let shader = Shader::parse(&blob.bytes).map_err(|e| format!("{label}: shader stream {}: {e}", blob.id))?;
            let st = serialized_stream(uexp, &blob, &shader).map_err(|e| format!("{label}: shader stream {} inline record: {e}", blob.id))?;
            let entry = fn_dxbc::unreal_entry::named_entry(uexp, names, search_from, header, &st).map_err(|e| format!("{label}: shader stream {} named entry: {e}", blob.id))?;
            search_from = st.tail.end;
            // the vertex factory FName sits between the entry header and the inline stream header
            let mut vertex_factory = String::new();
            let mut i = entry.offset + 12;
            while i + 8 <= st.start {
                let idx = i32::from_le_bytes(uexp[i..i + 4].try_into().unwrap());
                let num = i32::from_le_bytes(uexp[i + 4..i + 8].try_into().unwrap());
                if idx >= 0 && num == 0 { if let Some(n) = names.get(idx as usize) { if n.contains("VertexFactory") { vertex_factory = n.clone(); break; } } }
                i += 1;
            }
            let map = maps.iter().rposition(|m| entry.offset >= m.uniform_start).unwrap_or(0);
            let resource_table = ResourceTable::parse(&blob.header).map_err(|e| format!("{label}: shader {} resource table: {e}", blob.id))?;
            let optional = optional_fields(&blob.trailer).map_err(|e| format!("{label}: shader {} optional fields: {e}", blob.id))?;
            let ubo_names = uniform_buffer_names(&optional)?.unwrap_or_default();
            let bindings = named_bindings(&resource_table, Some(&ubo_names), &shader.decls).map_err(|e| format!("{label}: shader {} bindings: {e}", blob.id))?;
            programs.push(Program { map, shader_type: entry.name.clone(), vertex_factory, stream: blob.id, blob, shader, ubo_names, resource_table, bindings });
        }
        Ok(MaterialMaps { label: label.to_owned(), maps, programs })
    }

    /// The shader map the game runs on D3D11 SM5 hardware: feature level 3 and the High material quality (the scalability default on PC), falling back to the
    /// first SM5 map. [`Self::programs_agree_across_maps`] documents that the choice among the SM5 maps does not change the programs.
    pub fn sm5_map(&self) -> Result<&ShaderMap, String> {
        let sm5: Vec<&ShaderMap> = self.maps.iter().filter(|m| m.platform == "PCD3D_SM5").collect();
        sm5.iter().find(|m| m.quality == Some(QUALITY_HIGH) && m.feature_level == Some(FEATURE_SM5)).or(sm5.first()).copied().ok_or_else(|| format!("{}: no SM5 shader map", self.label))
    }
    pub fn find(&self, map: usize, vertex_factory: &str, shader_type: &str) -> Option<&Program> {
        self.programs.iter().find(|p| p.map == map && p.vertex_factory == vertex_factory && p.shader_type == shader_type)
    }
    /// All SM5 maps hold byte-identical programs (same DXBC program hash) for this (vertex factory, shader type), so quality level is immaterial for it.
    pub fn programs_agree_across_maps(&self, vertex_factory: &str, shader_type: &str) -> bool {
        let hashes: Vec<u64> = self.maps.iter().filter(|m| m.platform == "PCD3D_SM5").filter_map(|m| self.find(m.index, vertex_factory, shader_type)).map(|p| fn_dxbc::extract::program_hash(&p.blob.bytes)).collect();
        let n = self.maps.iter().filter(|m| m.platform == "PCD3D_SM5").count();
        hashes.len() == n && hashes.windows(2).all(|w| w[0] == w[1])
    }
    pub fn list(&self, map: usize) -> Vec<String> {
        self.programs.iter().filter(|p| p.map == map).map(|p| format!("{:>7} {} {:<64} {}", p.stream, p.shader.version_string(), p.shader_type, p.vertex_factory)).collect()
    }
}
