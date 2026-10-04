//! glTF 2.0 writer for **textured static props with every LOD**.
//!
//! The legacy exporter in [`crate::gltf`] writes LOD0 only, one private vertex set per section and debug-coloured
//! materials (byte compatible with the old UEViewer output; it must not change). Props need more, so this writer is
//! separate and additive:
//!
//! * one glTF `mesh` + `node` per LOD (`LOD0`, `LOD1`, ...); the scene lists LOD0 only, the other nodes are addressable
//!   by name (`extras.lods` on the document lists node / mesh / screen size / vertex and triangle counts of each);
//! * **one shared vertex set per LOD** (`POSITION`, `NORMAL`, `TANGENT`, optional `COLOR_0`, `TEXCOORD_0..n`), one
//!   index accessor per section (a slice of the LOD's index buffer, `u16` or `u32` depending on the vertex count) so
//!   the vertex / triangle counts of the file equal the uasset's own LOD data exactly;
//! * glTF material `i` = static material slot `i`, with the textures / alpha mode the caller resolved from the `.mat.json`.
//!
//! Axis convention is the legacy one: UE cm, left handed Z-up -> metres, right handed Y-up with `(x, y, z) -> (x, z, y) * 0.01`
//! (a reflection: triangle indices are unchanged and keep their facing). **Tangent sign**: the swap reflects the frame, so
//! the glTF bitangent `cross(N, T) * w` equals the swapped UE bitangent only for `w = -sign_UE`; this writer stores
//! `w = -sign_UE` (the legacy skeletal/static exporter stores `+sign_UE`; see its docs).
use crate::static_mesh::{StaticLod, StaticMesh};
use crate::{unpack_normal_unit, tangent_sign};
use fn_ueio::{Error, Result};
use std::fmt::Write as _;

/// What the caller knows about one material slot.
#[derive(Debug, Clone, PartialEq)]
pub struct PropMaterial {
    /// glTF material name (the material asset name).
    pub name: String,
    /// Static material slot name of the mesh.
    pub slot_name: String,
    /// Relative URIs.
    pub base_color_uri: Option<String>,
    pub normal_uri: Option<String>,
    pub mask_uri: Option<String>,
    /// `OPAQUE`, `MASK` or `BLEND`.
    pub alpha_mode: String,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    pub base_color_factor: [f32; 4],
    /// Relative URI of the full `.mat.json` (goes into `extras`).
    pub mat_json_uri: Option<String>,
}

impl Default for PropMaterial {
    fn default() -> Self {
        PropMaterial {
            name: String::new(),
            slot_name: String::new(),
            base_color_uri: None,
            normal_uri: None,
            mask_uri: None,
            alpha_mode: "OPAQUE".into(),
            alpha_cutoff: 0.5,
            double_sided: false,
            base_color_factor: [1.0; 4],
            mat_json_uri: None,
        }
    }
}

fn js(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn f(v: f32) -> String {
    if v.is_finite() {
        format!("{v}")
    } else {
        "0".into()
    }
}

struct View {
    byte_offset: usize,
    byte_length: usize,
}

struct Acc {
    view: usize,
    byte_offset: usize,
    component: u32,
    count: usize,
    ty: &'static str,
    normalized: bool,
    min_max: Option<([f32; 3], [f32; 3])>,
}

#[derive(Default)]
struct Doc {
    bin: Vec<u8>,
    views: Vec<View>,
    accs: Vec<Acc>,
}

impl Doc {
    fn view(&mut self, bytes: &[u8]) -> usize {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let off = self.bin.len();
        self.bin.extend_from_slice(bytes);
        self.views.push(View { byte_offset: off, byte_length: bytes.len() });
        self.views.len() - 1
    }
    fn acc(&mut self, view: usize, byte_offset: usize, component: u32, count: usize, ty: &'static str, normalized: bool, min_max: Option<([f32; 3], [f32; 3])>) -> usize {
        self.accs.push(Acc { view, byte_offset, component, count, ty, normalized, min_max });
        self.accs.len() - 1
    }
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(v.len() * 4);
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

fn norm3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        v
    }
}

/// One LOD: returns `(position, normal, tangent, color, uvs, per-section index accessors)`.
struct LodAcc {
    position: usize,
    normal: usize,
    tangent: usize,
    color: Option<usize>,
    uvs: Vec<usize>,
    sections: Vec<usize>,
}

fn add_lod(doc: &mut Doc, lod: &StaticLod) -> Result<LodAcc> {
    let n = lod.positions.len();
    if n == 0 || lod.indices.is_empty() {
        return Err(Error::Invalid("LOD has no geometry".into()));
    }
    if lod.tangent_x.len() != n || lod.tangent_z.len() != n || lod.uvs.iter().any(|u| u.len() != n) || (!lod.colors.is_empty() && lod.colors.len() != n) {
        return Err(Error::Invalid("vertex streams have different lengths".into()));
    }
    let mut pos = Vec::with_capacity(n * 3);
    let mut nrm = Vec::with_capacity(n * 3);
    let mut tan = Vec::with_capacity(n * 4);
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for i in 0..n {
        let p = lod.positions[i];
        let g = [p[0] * 0.01, p[2] * 0.01, p[1] * 0.01];
        for c in 0..3 {
            mn[c] = mn[c].min(g[c]);
            mx[c] = mx[c].max(g[c]);
        }
        pos.extend_from_slice(&g);
        let nu = norm3(unpack_normal_unit(lod.tangent_z[i]));
        nrm.extend_from_slice(&[nu[0], nu[2], nu[1]]);
        let tu = norm3(unpack_normal_unit(lod.tangent_x[i]));
        tan.extend_from_slice(&[tu[0], tu[2], tu[1], -tangent_sign(lod.tangent_z[i])]);
    }
    let pv = doc.view(&f32_bytes(&pos));
    let nv = doc.view(&f32_bytes(&nrm));
    let tv = doc.view(&f32_bytes(&tan));
    let position = doc.acc(pv, 0, 5126, n, "VEC3", false, Some((mn, mx)));
    let normal = doc.acc(nv, 0, 5126, n, "VEC3", false, None);
    let tangent = doc.acc(tv, 0, 5126, n, "VEC4", false, None);
    let color = if lod.colors.is_empty() {
        None
    } else {
        // memory order of FColor is B, G, R, A
        let mut b = Vec::with_capacity(n * 4);
        for c in &lod.colors {
            b.extend_from_slice(&[c[2], c[1], c[0], c[3]]);
        }
        let v = doc.view(&b);
        Some(doc.acc(v, 0, 5121, n, "VEC4", true, None))
    };
    let mut uvs = vec![];
    for uv in &lod.uvs {
        let mut b = Vec::with_capacity(n * 8);
        for t in uv {
            b.extend_from_slice(&t[0].to_le_bytes());
            b.extend_from_slice(&t[1].to_le_bytes());
        }
        let v = doc.view(&b);
        uvs.push(doc.acc(v, 0, 5126, n, "VEC2", false, None));
    }
    // indices: one view for the whole LOD index buffer, one accessor (slice) per section
    let wide = n > 65535;
    let mut ib = Vec::with_capacity(lod.indices.len() * if wide { 4 } else { 2 });
    for &i in &lod.indices {
        if i as usize >= n {
            return Err(Error::Invalid(format!("index {i} outside {n} vertices")));
        }
        if wide {
            ib.extend_from_slice(&i.to_le_bytes());
        } else {
            ib.extend_from_slice(&(i as u16).to_le_bytes());
        }
    }
    let iv = doc.view(&ib);
    let (comp, size) = if wide { (5125, 4) } else { (5123, 2) };
    let mut sections = vec![];
    for (si, s) in lod.sections.iter().enumerate() {
        let a = s.first_index as usize;
        let cnt = 3 * s.num_triangles as usize;
        if a + cnt > lod.indices.len() {
            return Err(Error::Invalid(format!("section {si} exceeds the index buffer")));
        }
        sections.push(doc.acc(iv, a * size, comp, cnt, "SCALAR", false, None));
    }
    Ok(LodAcc { position, normal, tangent, color, uvs, sections })
}

/// Build `(<name>.gltf text, <name>.bin bytes)` for every LOD of `mesh`. `materials[i]` describes material slot `i`
/// (missing entries get a neutral grey material named after the slot).
pub fn static_mesh_to_gltf_lods(mesh: &StaticMesh, name: &str, materials: &[PropMaterial]) -> Result<(String, Vec<u8>)> {
    if mesh.lods.is_empty() {
        return Err(Error::Invalid("mesh has no LODs".into()));
    }
    let mut doc = Doc::default();
    let mut lod_accs = vec![];
    for (i, l) in mesh.lods.iter().enumerate() {
        lod_accs.push(add_lod(&mut doc, l).map_err(|e| Error::Invalid(format!("LOD {i}: {e}")))?);
    }

    // images / textures (deduplicated by uri)
    let mut images: Vec<String> = vec![];
    let mut img = |u: &Option<String>| -> Option<usize> {
        let u = u.as_ref()?;
        Some(match images.iter().position(|x| x == u) {
            Some(i) => i,
            None => {
                images.push(u.clone());
                images.len() - 1
            }
        })
    };
    let mut mats_json = String::new();
    let n_slots = mesh.materials.len().max(materials.len());
    for slot in 0..n_slots {
        let sm = mesh.materials.get(slot);
        let m = materials.get(slot).cloned().unwrap_or_else(|| PropMaterial {
            name: sm.and_then(|s| s.material_name.clone()).unwrap_or_else(|| format!("material_{slot}")),
            slot_name: sm.map(|s| s.slot_name.clone()).unwrap_or_default(),
            base_color_factor: [0.5, 0.5, 0.5, 1.0],
            ..Default::default()
        });
        let bc = img(&m.base_color_uri);
        let nm = img(&m.normal_uri);
        let mk = img(&m.mask_uri);
        let mut s = String::new();
        let _ = write!(s, "    {{\n      \"name\": {},\n      \"pbrMetallicRoughness\": {{\n        \"baseColorFactor\": [{}, {}, {}, {}],\n", js(&m.name), f(m.base_color_factor[0]), f(m.base_color_factor[1]), f(m.base_color_factor[2]), f(m.base_color_factor[3]));
        if let Some(i) = bc {
            let _ = write!(s, "        \"baseColorTexture\": {{\"index\": {i}}},\n");
        }
        let _ = write!(s, "        \"metallicFactor\": 0.0,\n        \"roughnessFactor\": 0.9\n      }},\n");
        if let Some(i) = nm {
            let _ = write!(s, "      \"normalTexture\": {{\"index\": {i}}},\n");
        }
        let _ = write!(s, "      \"alphaMode\": {},\n", js(&m.alpha_mode));
        if m.alpha_mode == "MASK" {
            let _ = write!(s, "      \"alphaCutoff\": {},\n", f(m.alpha_cutoff));
        }
        let _ = write!(s, "      \"doubleSided\": {},\n      \"extras\": {{\"slot\": {slot}, \"slot_name\": {}", m.double_sided, js(&m.slot_name));
        if let Some(p) = sm.and_then(|x| x.material.as_ref()) {
            let _ = write!(s, ", \"ue_material\": {}", js(p));
        }
        if let Some(u) = &m.mat_json_uri {
            let _ = write!(s, ", \"mat_json\": {}", js(u));
        }
        if let Some(i) = mk {
            let _ = write!(s, ", \"mask_texture_image\": {i}");
        }
        s.push_str("}\n    }");
        if !mats_json.is_empty() {
            mats_json.push_str(",\n");
        }
        mats_json.push_str(&s);
    }

    let mut out = String::new();
    out.push_str("{\n  \"asset\": {\"version\": \"2.0\", \"generator\": \"fn-mesh gltf_lods (UE4.16 static mesh, all LODs)\"},\n");
    out.push_str("  \"scene\": 0,\n  \"scenes\": [{\"nodes\": [0]}],\n");
    let _ = write!(out, "  \"nodes\": [\n");
    for (i, l) in mesh.lods.iter().enumerate() {
        let ss = mesh.screen_size.get(i).copied().unwrap_or(0.0);
        let _ = write!(
            out,
            "    {{\"name\": \"LOD{i}\", \"mesh\": {i}, \"extras\": {{\"lod\": {i}, \"screen_size\": {}, \"vertices\": {}, \"triangles\": {}}}}}{}\n",
            f(ss),
            l.positions.len(),
            l.indices.len() / 3,
            if i + 1 < mesh.lods.len() { "," } else { "" }
        );
    }
    out.push_str("  ],\n");
    let _ = write!(out, "  \"meshes\": [\n");
    for (i, (l, a)) in mesh.lods.iter().zip(&lod_accs).enumerate() {
        let _ = write!(out, "    {{\"name\": {}, \"primitives\": [\n", js(&format!("{name}_LOD{i}")));
        for (si, s) in l.sections.iter().enumerate() {
            let mut attrs = format!("\"POSITION\": {}, \"NORMAL\": {}, \"TANGENT\": {}", a.position, a.normal, a.tangent);
            if let Some(c) = a.color {
                let _ = write!(attrs, ", \"COLOR_0\": {c}");
            }
            for (ui, u) in a.uvs.iter().enumerate() {
                let _ = write!(attrs, ", \"TEXCOORD_{ui}\": {u}");
            }
            let _ = write!(
                out,
                "      {{\"attributes\": {{{attrs}}}, \"indices\": {}, \"material\": {}, \"mode\": 4, \"extras\": {{\"section\": {si}, \"triangles\": {}, \"material_index\": {}}}}}{}\n",
                a.sections[si],
                s.material_index.max(0),
                s.num_triangles,
                s.material_index,
                if si + 1 < l.sections.len() { "," } else { "" }
            );
        }
        let _ = write!(out, "    ]}}{}\n", if i + 1 < mesh.lods.len() { "," } else { "" });
    }
    out.push_str("  ],\n");
    let _ = write!(out, "  \"materials\": [\n{mats_json}\n  ],\n");
    if !images.is_empty() {
        out.push_str("  \"images\": [");
        out.push_str(&images.iter().map(|u| format!("{{\"uri\": {}}}", js(u))).collect::<Vec<_>>().join(", "));
        out.push_str("],\n  \"textures\": [");
        out.push_str(&(0..images.len()).map(|i| format!("{{\"source\": {i}, \"sampler\": 0}}")).collect::<Vec<_>>().join(", "));
        out.push_str("],\n  \"samplers\": [{\"magFilter\": 9729, \"minFilter\": 9987, \"wrapS\": 10497, \"wrapT\": 10497}],\n");
    }
    let (bmin, bmax) = mesh.bounds.box_min_max();
    let lods_json: Vec<String> = mesh
        .lods
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{{\"node\": {i}, \"mesh\": {i}, \"screen_size\": {}, \"vertices\": {}, \"triangles\": {}, \"sections\": {}, \"uv_channels\": {}, \"vertex_colors\": {}}}", f(mesh.screen_size.get(i).copied().unwrap_or(0.0)), l.positions.len(), l.indices.len() / 3, l.sections.len(), l.uvs.len(), !l.colors.is_empty()))
        .collect();
    let _ = write!(
        out,
        "  \"extras\": {{\"name\": {}, \"units\": \"metres, Y up (UE cm Z-up via (x,z,y)*0.01)\", \"tangent_w\": \"-sign_UE (glTF correct after the axis swap)\", \"ue_bounds_cm\": {{\"min\": [{}, {}, {}], \"max\": [{}, {}, {}]}}, \"lods\": [{}]}},\n",
        js(name),
        f(bmin[0]),
        f(bmin[1]),
        f(bmin[2]),
        f(bmax[0]),
        f(bmax[1]),
        f(bmax[2]),
        lods_json.join(", ")
    );

    let _ = write!(out, "  \"buffers\": [{{\"uri\": {}, \"byteLength\": {}}}],\n", js(&format!("{name}.bin")), doc.bin.len());
    out.push_str("  \"bufferViews\": [\n");
    for (i, v) in doc.views.iter().enumerate() {
        let _ = write!(out, "    {{\"buffer\": 0, \"byteOffset\": {}, \"byteLength\": {}}}{}\n", v.byte_offset, v.byte_length, if i + 1 < doc.views.len() { "," } else { "" });
    }
    out.push_str("  ],\n  \"accessors\": [\n");
    for (i, a) in doc.accs.iter().enumerate() {
        let mut s = format!("    {{\"bufferView\": {}, \"byteOffset\": {}, \"componentType\": {}, \"count\": {}, \"type\": \"{}\"", a.view, a.byte_offset, a.component, a.count, a.ty);
        if a.normalized {
            s.push_str(", \"normalized\": true");
        }
        if let Some((mn, mx)) = a.min_max {
            let _ = write!(s, ", \"min\": [{}, {}, {}], \"max\": [{}, {}, {}]", f(mn[0]), f(mn[1]), f(mn[2]), f(mx[0]), f(mx[1]), f(mx[2]));
        }
        s.push('}');
        let _ = write!(out, "{s}{}\n", if i + 1 < doc.accs.len() { "," } else { "" });
    }
    out.push_str("  ]\n}\n");
    Ok((out, doc.bin))
}

/// Write `<dir>/<name>.gltf` and `<dir>/<name>.bin`.
pub fn export_static_mesh_lods(mesh: &StaticMesh, dir: &std::path::Path, name: &str, materials: &[PropMaterial]) -> Result<()> {
    let (json, bin) = static_mesh_to_gltf_lods(mesh, name, materials)?;
    std::fs::create_dir_all(dir).map_err(|e| Error::Invalid(format!("{}: {e}", dir.display())))?;
    std::fs::write(dir.join(format!("{name}.gltf")), json).map_err(|e| Error::Invalid(e.to_string()))?;
    std::fs::write(dir.join(format!("{name}.bin")), bin).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::static_mesh::{StaticLod, StaticMaterial, StaticSection};
    use crate::{Bounds, UvChannelInfo};

    /// A grid mesh with `n` x `n` quads in two sections, two UV sets and vertex colours.
    fn grid(n: usize, with_colors: bool) -> StaticLod {
        let w = n + 1;
        let mut l = StaticLod::default();
        for y in 0..w {
            for x in 0..w {
                l.positions.push([x as f32 * 10.0, y as f32 * 10.0, (x + y) as f32]);
                l.tangent_x.push([255, 127, 127, 255]);
                l.tangent_z.push([127, 127, 255, if x % 2 == 0 { 255 } else { 0 }]);
            }
        }
        l.uvs = vec![l.positions.iter().map(|p| [p[0] / 100.0, p[1] / 100.0]).collect(), l.positions.iter().map(|p| [p[1], p[0]]).collect()];
        l.num_tex_coords = 2;
        if with_colors {
            l.colors = (0..w * w).map(|i| [(i % 256) as u8, 1, 2, 255]).collect();
        }
        for y in 0..n {
            for x in 0..n {
                let (a, b, c, d) = ((y * w + x) as u32, (y * w + x + 1) as u32, ((y + 1) * w + x) as u32, ((y + 1) * w + x + 1) as u32);
                l.indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
        let half = (n / 2) * n * 6;
        let tris = l.indices.len() / 3;
        let mm = |idx: &[u32]| (*idx.iter().min().unwrap(), *idx.iter().max().unwrap());
        let (a0, a1) = mm(&l.indices[..half]);
        let (b0, b1) = mm(&l.indices[half..]);
        l.sections = vec![
            StaticSection { material_index: 0, first_index: 0, num_triangles: (half / 3) as u32, min_vertex_index: a0, max_vertex_index: a1, enable_collision: true, cast_shadow: true },
            StaticSection { material_index: 1, first_index: half as u32, num_triangles: (tris - half / 3) as u32, min_vertex_index: b0, max_vertex_index: b1, enable_collision: true, cast_shadow: true },
        ];
        l
    }

    fn mesh(lods: Vec<StaticLod>) -> StaticMesh {
        let mat = |n: &str| StaticMaterial { slot_name: n.into(), material: Some(format!("/Game/M/{n}.{n}")), material_name: Some(n.into()), uv_channel: UvChannelInfo::default() };
        StaticMesh {
            name: "T".into(),
            bounds: Bounds { origin: [0.0; 3], extent: [50.0; 3], radius: 90.0 },
            materials: vec![mat("Bark"), mat("Leaves")],
            lods,
            screen_size: [1.0, 0.5, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0],
            lighting_guid: [0; 16],
            body_setup: None,
            nav_collision: None,
            sockets: vec![],
            lod_for_collision: 0,
            light_map_coordinate_index: 1,
        }
    }

    fn parse(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("valid glTF JSON")
    }

    #[test]
    fn multi_lod_document_is_consistent() {
        let m = mesh(vec![grid(6, true), grid(4, true), grid(2, false)]);
        assert!(m.validate().iter().all(|c| c.problems.is_empty()), "{:?}", m.validate());
        let mats = vec![
            PropMaterial { name: "Bark".into(), slot_name: "Bark".into(), base_color_uri: Some("../textures/B.png".into()), normal_uri: Some("../textures/BN.png".into()), ..Default::default() },
            PropMaterial { name: "Leaves".into(), slot_name: "Leaves".into(), base_color_uri: Some("../textures/B.png".into()), alpha_mode: "MASK".into(), alpha_cutoff: 0.33, double_sided: true, ..Default::default() },
        ];
        let (json, bin) = static_mesh_to_gltf_lods(&m, "T", &mats).unwrap();
        let j = parse(&json);
        assert_eq!(j["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(j["scenes"][0]["nodes"], serde_json::json!([0]));
        assert_eq!(j["images"].as_array().unwrap().len(), 2, "shared uri deduplicated");
        assert_eq!(j["materials"][1]["alphaMode"], "MASK");
        assert_eq!(j["materials"][1]["doubleSided"], true);
        // every accessor lies inside its view and the view inside the bin
        let views = j["bufferViews"].as_array().unwrap();
        for v in views {
            assert!(v["byteOffset"].as_u64().unwrap() + v["byteLength"].as_u64().unwrap() <= bin.len() as u64);
        }
        assert_eq!(j["buffers"][0]["byteLength"].as_u64().unwrap() as usize, bin.len());
        // counts of the file = counts of the LOD data
        for (li, l) in m.lods.iter().enumerate() {
            let prims = j["meshes"][li]["primitives"].as_array().unwrap();
            assert_eq!(prims.len(), l.sections.len());
            let pos = &j["accessors"][prims[0]["attributes"]["POSITION"].as_u64().unwrap() as usize];
            assert_eq!(pos["count"].as_u64().unwrap() as usize, l.positions.len());
            let mut tris = 0;
            for (si, p) in prims.iter().enumerate() {
                let a = &j["accessors"][p["indices"].as_u64().unwrap() as usize];
                tris += a["count"].as_u64().unwrap() as usize / 3;
                assert_eq!(a["count"].as_u64().unwrap() as u32, l.sections[si].num_triangles * 3);
                assert_eq!(p["attributes"].get("COLOR_0").is_some(), !l.colors.is_empty());
                assert!(p["attributes"].get("TEXCOORD_1").is_some());
            }
            assert_eq!(tris, l.indices.len() / 3);
        }
        // position accessor min/max are the swapped, scaled UE box of the LOD
        let pos = &j["accessors"][j["meshes"][0]["primitives"][0]["attributes"]["POSITION"].as_u64().unwrap() as usize];
        let (mn, mx) = m.position_bounds(0).unwrap();
        assert!((pos["max"][1].as_f64().unwrap() - mx[2] as f64 * 0.01).abs() < 1e-6);
        assert!((pos["max"][2].as_f64().unwrap() - mx[1] as f64 * 0.01).abs() < 1e-6);
        assert!((pos["min"][0].as_f64().unwrap() - mn[0] as f64 * 0.01).abs() < 1e-6);
    }

    #[test]
    fn index_buffer_roundtrips_u16() {
        let l = grid(5, false);
        let m = mesh(vec![l.clone()]);
        let (json, bin) = static_mesh_to_gltf_lods(&m, "T", &[]).unwrap();
        let j = parse(&json);
        let p = &j["meshes"][0]["primitives"][1];
        let a = &j["accessors"][p["indices"].as_u64().unwrap() as usize];
        assert_eq!(a["componentType"], 5123);
        let v = &j["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let base = v["byteOffset"].as_u64().unwrap() as usize + a["byteOffset"].as_u64().unwrap() as usize;
        let n = a["count"].as_u64().unwrap() as usize;
        let got: Vec<u32> = (0..n).map(|i| u16::from_le_bytes([bin[base + 2 * i], bin[base + 2 * i + 1]]) as u32).collect();
        assert_eq!(got, l.section_indices(1));
    }

    #[test]
    fn indices_become_32_bit_above_65535_vertices() {
        let l = grid(300, false); // 301^2 = 90601 vertices
        assert!(l.positions.len() > 65535);
        let m = mesh(vec![l.clone()]);
        let (json, bin) = static_mesh_to_gltf_lods(&m, "T", &[]).unwrap();
        let j = parse(&json);
        let a = &j["accessors"][j["meshes"][0]["primitives"][0]["indices"].as_u64().unwrap() as usize];
        assert_eq!(a["componentType"], 5125);
        let v = &j["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let base = v["byteOffset"].as_u64().unwrap() as usize + a["byteOffset"].as_u64().unwrap() as usize;
        let last = a["count"].as_u64().unwrap() as usize - 1;
        let at = |i: usize| u32::from_le_bytes([bin[base + 4 * i], bin[base + 4 * i + 1], bin[base + 4 * i + 2], bin[base + 4 * i + 3]]);
        assert_eq!(at(0), l.indices[0]);
        assert_eq!(at(last), l.indices[last]);
        assert!(l.indices.iter().any(|&i| i > 65535));
    }

    #[test]
    fn validate_reports_broken_sections() {
        let mut l = grid(4, false);
        l.sections[0].max_vertex_index += 1;
        l.sections[1].material_index = 9;
        let c = mesh(vec![l]).validate();
        assert!(!c[0].section_ranges_exact);
        assert!(c[0].problems.iter().any(|p| p.contains("material index 9")));
    }

    #[test]
    fn tangent_w_is_negated_ue_sign() {
        let m = mesh(vec![grid(2, false)]);
        let (json, bin) = static_mesh_to_gltf_lods(&m, "T", &[]).unwrap();
        let j = parse(&json);
        let a = &j["accessors"][j["meshes"][0]["primitives"][0]["attributes"]["TANGENT"].as_u64().unwrap() as usize];
        let v = &j["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let base = v["byteOffset"].as_u64().unwrap() as usize;
        let w = |i: usize| f32::from_le_bytes([bin[base + 16 * i + 12], bin[base + 16 * i + 13], bin[base + 16 * i + 14], bin[base + 16 * i + 15]]);
        // vertex 0: x = 0 -> UE sign +1 -> glTF -1; vertex 1: x = 1 -> UE sign -1 -> glTF +1
        assert_eq!(w(0), -1.0);
        assert_eq!(w(1), 1.0);
    }
}
