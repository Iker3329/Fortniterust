//! Minimal glTF 2.0 reader for the prop meshes written by `fn-export props` (`fn_mesh::gltf_lods`): one node/mesh per
//! LOD, shared vertex streams per LOD, one index accessor per section. Not a general glTF loader.
use crate::util::*;
use serde_json::Value;
use std::path::Path;

/// One draw range of a LOD.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshSection {
    /// Material slot (index into [`crate::PropKind::materials`]).
    pub material_slot: usize,
    /// Triangle list indices into the LOD's vertex arrays.
    pub indices: Vec<u32>,
}

/// One decoded LOD. Positions / normals / tangents are in the glTF convention of the export: **metres, right handed,
/// Y up** (`(x, y, z)_UE cm -> (x, z, y) * 0.01`). `tangents[i][3]` is the bitangent sign for `cross(normal, tangent.xyz)`.
#[derive(Debug, Clone, Default)]
pub struct LodMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    /// `COLOR_0` as RGBA8 (foliage "wind" / AO paint); empty when the mesh has no vertex colours.
    pub colors: Vec<[u8; 4]>,
    /// `uvs[channel][vertex]`, as stored (no V flip).
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub sections: Vec<MeshSection>,
    pub screen_size: f32,
}

impl LodMesh {
    pub fn triangle_count(&self) -> usize {
        self.sections.iter().map(|s| s.indices.len() / 3).sum()
    }
    /// Axis aligned box `(min, max)` of the positions (metres, Y up).
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut mn = [f32::MAX; 3];
        let mut mx = [f32::MIN; 3];
        for p in &self.positions {
            for c in 0..3 {
                mn[c] = mn[c].min(p[c]);
                mx[c] = mx[c].max(p[c]);
            }
        }
        (mn, mx)
    }
}

struct Gltf<'a> {
    j: &'a Value,
    bin: &'a [u8],
}

impl Gltf<'_> {
    fn view_slice(&self, view: usize, extra_off: usize, len: usize) -> Result<&[u8]> {
        let v = self.j["bufferViews"].get(view).ok_or_else(|| Error::Schema(format!("bufferView {view}")))?;
        let off = v["byteOffset"].as_u64().unwrap_or(0) as usize + extra_off;
        self.bin.get(off..off + len).ok_or_else(|| Error::Schema(format!("bufferView {view}: {off}+{len} outside the .bin ({})", self.bin.len())))
    }

    fn accessor(&self, i: usize) -> Result<(&Value, usize, usize)> {
        let a = self.j["accessors"].get(i).ok_or_else(|| Error::Schema(format!("accessor {i}")))?;
        let comps = match a["type"].as_str().unwrap_or("") {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            t => return schema(format!("accessor {i}: type {t}")),
        };
        Ok((a, comps, a["count"].as_u64().unwrap_or(0) as usize))
    }

    fn f32s(&self, i: usize, want: usize) -> Result<Vec<f32>> {
        let (a, comps, count) = self.accessor(i)?;
        if comps != want || a["componentType"].as_u64() != Some(5126) {
            return schema(format!("accessor {i}: expected float x{want}"));
        }
        let bytes = self.view_slice(a["bufferView"].as_u64().unwrap_or(0) as usize, a["byteOffset"].as_u64().unwrap_or(0) as usize, count * comps * 4)?;
        Ok(bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }

    fn indices(&self, i: usize) -> Result<Vec<u32>> {
        let (a, comps, count) = self.accessor(i)?;
        let ct = a["componentType"].as_u64().unwrap_or(0);
        let size = match ct {
            5123 => 2,
            5125 => 4,
            5121 => 1,
            _ => return schema(format!("accessor {i}: index component type {ct}")),
        };
        if comps != 1 {
            return schema(format!("accessor {i}: indices must be SCALAR"));
        }
        let bytes = self.view_slice(a["bufferView"].as_u64().unwrap_or(0) as usize, a["byteOffset"].as_u64().unwrap_or(0) as usize, count * size)?;
        Ok(match size {
            1 => bytes.iter().map(|&b| b as u32).collect(),
            2 => bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as u32).collect(),
            _ => bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        })
    }

    fn colors(&self, i: usize) -> Result<Vec<[u8; 4]>> {
        let (a, comps, count) = self.accessor(i)?;
        if comps != 4 || a["componentType"].as_u64() != Some(5121) {
            return schema(format!("accessor {i}: COLOR_0 must be normalised u8 VEC4"));
        }
        let bytes = self.view_slice(a["bufferView"].as_u64().unwrap_or(0) as usize, a["byteOffset"].as_u64().unwrap_or(0) as usize, count * 4)?;
        Ok(bytes.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect())
    }
}

/// Load every LOD of `<dir>/<gltf>` (the `.bin` is read from the `buffers[0].uri` next to it).
pub fn load_lods(gltf_path: &Path) -> Result<Vec<LodMesh>> {
    let j = read_json(gltf_path)?;
    let uri = j["buffers"][0]["uri"].as_str().ok_or_else(|| Error::Schema("buffers[0].uri".into()))?;
    let bin_path = gltf_path.parent().unwrap_or(Path::new(".")).join(uri);
    let bin = std::fs::read(&bin_path).map_err(|e| Error::Io(format!("{}: {e}", bin_path.display())))?;
    let g = Gltf { j: &j, bin: &bin };
    let nodes = j["nodes"].as_array().ok_or_else(|| Error::Schema("nodes".into()))?;
    let mut out = vec![];
    for (ni, node) in nodes.iter().enumerate() {
        let mi = node["mesh"].as_u64().ok_or_else(|| Error::Schema(format!("node {ni} has no mesh")))? as usize;
        let prims = j["meshes"][mi]["primitives"].as_array().ok_or_else(|| Error::Schema(format!("mesh {mi} primitives")))?;
        let mut lod = LodMesh { screen_size: f32_of(&node["extras"], "screen_size").unwrap_or(0.0), ..Default::default() };
        let first = prims.first().ok_or_else(|| Error::Schema(format!("mesh {mi} has no primitives")))?;
        let at = &first["attributes"];
        let get = |k: &str| at[k].as_u64().map(|x| x as usize);
        let p = g.f32s(get("POSITION").ok_or_else(|| Error::Schema("no POSITION".into()))?, 3)?;
        lod.positions = p.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
        if let Some(n) = get("NORMAL") {
            lod.normals = g.f32s(n, 3)?.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
        }
        if let Some(t) = get("TANGENT") {
            lod.tangents = g.f32s(t, 4)?.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        }
        if let Some(c) = get("COLOR_0") {
            lod.colors = g.colors(c)?;
        }
        for u in 0.. {
            match get(&format!("TEXCOORD_{u}")) {
                Some(a) => lod.uvs.push(g.f32s(a, 2)?.chunks_exact(2).map(|c| [c[0], c[1]]).collect()),
                None => break,
            }
        }
        for (pi, prim) in prims.iter().enumerate() {
            if prim["attributes"]["POSITION"] != first["attributes"]["POSITION"] {
                return schema(format!("mesh {mi} primitive {pi} does not share the LOD vertex set"));
            }
            let slot = prim["material"].as_u64().unwrap_or(0) as usize;
            let idx = g.indices(prim["indices"].as_u64().ok_or_else(|| Error::Schema("primitive without indices".into()))? as usize)?;
            if let Some(&m) = idx.iter().max() {
                if m as usize >= lod.positions.len() {
                    return schema(format!("mesh {mi} primitive {pi}: index {m} >= {} vertices", lod.positions.len()));
                }
            }
            lod.sections.push(MeshSection { material_slot: slot, indices: idx });
        }
        out.push(lod);
    }
    Ok(out)
}
