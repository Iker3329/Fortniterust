//! Verification of the static mesh decoder (sky dome / sun-moon quad) against the old UEViewer `.pskx` exports and the
//! `SKYM` binaries the old runtime loads. Gated on the data existing; run with `-- --nocapture` for the table.

mod common;
use common::*;
use fn_mesh::gltf::{self, Options};
use fn_mesh::skym::static_mesh_to_skym;
use fn_mesh::StaticMesh;
use std::collections::HashMap;
use std::path::Path;

const SKY_DIR: &str = "Packages/Fortress_Sky";
/// (package path below Content, old pskx path below assets/sky, old SKYM bin if any)
const MESHES: &[(&str, &str, Option<&str>)] = &[
    ("Packages/Fortress_Sky/Mesh_NoLOD/FortniteSkyDome_Optimized", "Game/Packages/Fortress_Sky/Mesh_NoLOD/FortniteSkyDome_Optimized.pskx", Some("dome_optimized.bin")),
    ("Packages/Fortress_Sky/Mesh_NoLOD/FortniteSkyDome_Highpoly", "Game/Packages/Fortress_Sky/Mesh_NoLOD/FortniteSkyDome_Highpoly.pskx", None),
    ("Packages/Fortress_Sky/MaterialInstance/Arid/SM_FlatSunMoon_Arid", "Game/Packages/Fortress_Sky/MaterialInstance/Arid/SM_FlatSunMoon_Arid.pskx", Some("sunmoon.bin")),
];

/// Minimal PSK/PSKX chunk reader.
struct Psk {
    chunks: HashMap<String, (usize, usize, usize)>, // id -> (offset, item size, count)
    data: Vec<u8>,
}

impl Psk {
    fn load(path: &Path) -> Psk {
        let data = std::fs::read(path).unwrap();
        let mut chunks = HashMap::new();
        let mut o = 0;
        while o + 32 <= data.len() {
            let id = String::from_utf8_lossy(&data[o..o + 20]).trim_end_matches('\0').to_string();
            let size = i32::from_le_bytes(data[o + 24..o + 28].try_into().unwrap()) as usize;
            let count = i32::from_le_bytes(data[o + 28..o + 32].try_into().unwrap()) as usize;
            o += 32;
            chunks.insert(id, (o, size, count));
            o += size * count;
        }
        Psk { chunks, data }
    }
    fn f32(&self, o: usize) -> f32 {
        f32::from_le_bytes(self.data[o..o + 4].try_into().unwrap())
    }
    fn i32(&self, o: usize) -> i32 {
        i32::from_le_bytes(self.data[o..o + 4].try_into().unwrap())
    }
    fn u16(&self, o: usize) -> u16 {
        u16::from_le_bytes(self.data[o..o + 2].try_into().unwrap())
    }
}

fn prop_vec(props: &str, key: &str) -> Option<[f32; 3]> {
    let i = props.find(key)?;
    let s = &props[i..];
    let num = |tag: &str| -> f32 {
        let j = s.find(tag).unwrap() + tag.len();
        s[j..].split(|c: char| c == ' ' || c == ',' || c == '}').next().unwrap().parse().unwrap()
    };
    Some([num("X="), num("Y="), num("Z=")])
}

#[test]
fn static_meshes_match_old_pskx_and_skym() {
    let (Some(td), Some(sky)) = (testdata(), old_sky()) else {
        eprintln!("NOTE: test data or old sky assets missing; skipping static mesh verification");
        return;
    };
    let _ = SKY_DIR;
    println!("\nstatic mesh vs old PSKX / SKYM");
    println!("{:<28} {:>5} {:>7} {:>7} {:>9} {:>9} {:>9} {:>6} {:>6} {:>10}", "mesh", "lods", "verts", "tris", "pos err", "uv err", "idx", "mats", "bbox", "SKYM bytes");
    for (pkg_path, psk_path, skym_path) in MESHES {
        let pkg = load_package(&td.join(pkg_path));
        let mesh = StaticMesh::parse(&pkg).unwrap();
        let l = &mesh.lods[0];
        let psk = Psk::load(&sky.join(psk_path));

        // vertices (wedges)
        let (wo, wsize, wcount) = psk.chunks.get("VTXW3200").or_else(|| psk.chunks.get("VTXW0000")).copied().unwrap();
        let wide = psk.chunks.contains_key("VTXW3200");
        assert_eq!(wcount, l.positions.len(), "{pkg_path}: wedge count");
        let (po, _, pcount) = psk.chunks["PNTS0000"];
        let mut pos_err = 0f32;
        let mut uv_err = 0f32;
        let mut bbox_min = [f32::MAX; 3];
        let mut bbox_max = [f32::MIN; 3];
        for i in 0..wcount {
            let w = wo + i * wsize;
            let pi = if wide { psk.i32(w) as usize } else { psk.u16(w) as usize };
            assert!(pi < pcount);
            let p = [psk.f32(po + pi * 12), psk.f32(po + pi * 12 + 4), psk.f32(po + pi * 12 + 8)];
            // PSK mirrors Y
            let mine = [l.positions[i][0], -l.positions[i][1], l.positions[i][2]];
            for c in 0..3 {
                pos_err = pos_err.max((p[c] - mine[c]).abs());
                bbox_min[c] = bbox_min[c].min(l.positions[i][c]);
                bbox_max[c] = bbox_max[c].max(l.positions[i][c]);
            }
            // UV0: old export used UEViewer's half2float (zero -> 3.05e-5); compare with that bug emulated
            let u = [psk.f32(w + 4), psk.f32(w + 8)];
            let mine_uv = [l.uvs[0][i][0], l.uvs[0][i][1]];
            let mine_uv_legacy = [gltf_legacy(mine_uv[0]), gltf_legacy(mine_uv[1])];
            uv_err = uv_err.max((u[0] - mine_uv_legacy[0]).abs()).max((u[1] - mine_uv_legacy[1]).abs());
            assert!((u[0] - mine_uv[0]).abs() < 3.2e-5 && (u[1] - mine_uv[1]).abs() < 3.2e-5);
        }
        // extra UV chunks
        for ch in 1..l.num_tex_coords {
            let (eo, _, ecount) = psk.chunks[&format!("EXTRAUVS{}", ch - 1)];
            assert_eq!(ecount, wcount);
            for i in 0..wcount {
                let u = [psk.f32(eo + i * 8), psk.f32(eo + i * 8 + 4)];
                for c in 0..2 {
                    assert!((u[c] - l.uvs[ch][i][c]).abs() < 3.2e-5, "{pkg_path}: uv{ch} vertex {i}");
                    uv_err = uv_err.max((u[c] - gltf_legacy(l.uvs[ch][i][c])).abs());
                }
            }
        }
        // vertex colours (R,G,B,A in the PSK)
        if let Some(&(co, _, ccount)) = psk.chunks.get("VERTEXCOLOR") {
            assert_eq!(ccount, wcount);
            assert_eq!(l.colors.len(), wcount);
            for i in 0..wcount {
                let c = [psk.data[co + i * 4], psk.data[co + i * 4 + 1], psk.data[co + i * 4 + 2], psk.data[co + i * 4 + 3]];
                assert_eq!(c, [l.colors[i][2], l.colors[i][1], l.colors[i][0], l.colors[i][3]], "{pkg_path}: colour {i}");
            }
        } else {
            assert!(l.colors.is_empty());
        }
        // faces: per section in order, first two wedge indices swapped
        let (fo, fsize, fcount) = psk.chunks.get("FACE3200").or_else(|| psk.chunks.get("FACE0000")).copied().unwrap();
        let wide_f = psk.chunks.contains_key("FACE3200");
        let tris: usize = l.sections.iter().map(|s| s.num_triangles as usize).sum();
        assert_eq!(fcount, tris, "{pkg_path}: face count");
        let mut f = 0;
        let mut idx_bad = 0;
        for (si, s) in l.sections.iter().enumerate() {
            for t in 0..s.num_triangles as usize {
                let o = fo + f * fsize;
                let w: [u32; 3] = if wide_f { [psk.i32(o) as u32, psk.i32(o + 4) as u32, psk.i32(o + 8) as u32] } else { [psk.u16(o) as u32, psk.u16(o + 2) as u32, psk.u16(o + 4) as u32] };
                let a = s.first_index as usize + t * 3;
                let mine = [l.indices[a + 1], l.indices[a], l.indices[a + 2]];
                if w != mine {
                    idx_bad += 1;
                }
                let mat = psk.data[o + if wide_f { 12 } else { 6 }];
                assert_eq!(mat as usize, si);
                f += 1;
            }
        }
        // materials
        let (mo, msize, mcount) = psk.chunks["MATT0000"];
        assert_eq!(mcount, l.sections.len());
        let mut mats_ok = true;
        for i in 0..mcount {
            let raw = &psk.data[mo + i * msize..mo + i * msize + 64];
            let name = String::from_utf8_lossy(raw).trim_end_matches('\0').to_string();
            let mine = mesh.materials.get(l.sections[i].material_index as usize).and_then(|m| m.material_name.clone()).unwrap_or(format!("material_{i}"));
            // the old UEViewer decode lost the material reference of the sky meshes (placeholder `material_N`)
            if name != mine && name != format!("material_{i}") {
                mats_ok = false;
                eprintln!("{pkg_path}: material {i}: old {name:?} mine {mine:?}");
            }
        }
        // bounds from the package properties (ExtendedBounds) vs the decoded native bounds
        let props = std::fs::read_to_string(sky.join(psk_path.replace(".pskx", ".props.txt"))).ok();
        let mut bbox_ok = true;
        if let Some(p) = props {
            if let (Some(o), Some(e)) = (prop_vec(&p, "Origin"), prop_vec(&p, "BoxExtent")) {
                for c in 0..3 {
                    // props.txt prints 6 significant digits
                    bbox_ok &= (o[c] - mesh.bounds.origin[c]).abs() <= 1e-5 * mesh.bounds.origin[c].abs().max(10.0) && (e[c] - mesh.bounds.extent[c]).abs() <= 1e-5 * mesh.bounds.extent[c].abs().max(10.0);
                }
            }
        }
        // the geometry's own bounding box sits inside the stored bounds
        for c in 0..3 {
            assert!(bbox_min[c] >= mesh.bounds.origin[c] - mesh.bounds.extent[c] - 1e-2);
            assert!(bbox_max[c] <= mesh.bounds.origin[c] + mesh.bounds.extent[c] + 1e-2);
        }

        // SKYM bytes
        let skym = static_mesh_to_skym(&mesh, 0, &Options { legacy_uev_half_uv: true }).unwrap();
        let skym_state = match skym_path {
            Some(p) => {
                let old = std::fs::read(sky.join(p)).unwrap();
                if old == skym {
                    "identical".to_string()
                } else {
                    let d = old.iter().zip(&skym).filter(|(a, b)| a != b).count();
                    format!("{} diff / {}", d, old.len().max(skym.len()))
                }
            }
            None => format!("{} B (no old)", skym.len()),
        };
        println!(
            "{:<28} {:>5} {:>7} {:>7} {:>9.1e} {:>9.1e} {:>9} {:>6} {:>6} {:>10}",
            mesh.name,
            mesh.lods.len(),
            wcount,
            tris,
            pos_err,
            uv_err,
            if idx_bad == 0 { "identical".to_string() } else { format!("{idx_bad} bad") },
            mats_ok,
            bbox_ok,
            skym_state
        );
        assert_eq!(pos_err, 0.0, "{pkg_path}: positions");
        assert_eq!(uv_err, 0.0, "{pkg_path}: uvs (with legacy half decode)");
        assert_eq!(idx_bad, 0);
        assert!(mats_ok, "{pkg_path}: materials");
        assert!(bbox_ok, "{pkg_path}: bounds");
        if skym_path.is_some() {
            assert_eq!(skym_state, "identical", "{pkg_path}: SKYM differs from the old file");
        }

        // glTF export sanity (static layout)
        let (json, bin) = gltf::static_mesh_to_gltf(&mesh, &mesh.name);
        let g = Gltf::from_parts(&json, bin);
        let prims = g.json["meshes"][0]["primitives"].as_array().unwrap();
        assert_eq!(prims.len(), l.sections.len());
        assert!(g.json["skins"].is_null());
        for (pi, p) in prims.iter().enumerate() {
            let (n, _) = g.values(p["attributes"]["POSITION"].as_u64().unwrap() as usize);
            assert_eq!(n, 3);
            let uvk = p["attributes"].as_object().unwrap().keys().filter(|k| k.starts_with("TEXCOORD_")).count();
            assert_eq!(uvk, l.num_tex_coords);
            let ic = g.accessor(p["indices"].as_u64().unwrap() as usize).1;
            assert_eq!(ic, l.sections[pi].num_triangles as usize * 3);
        }
    }
}

fn gltf_legacy(v: f32) -> f32 {
    // UEViewer half2float emulation (same as fn_mesh::gltf::Options::legacy_uev_half_uv)
    let bits = v.to_bits();
    let e = (bits >> 23) & 0xff;
    if e >= 113 || !v.is_finite() {
        return v;
    }
    let mant = (v.abs() * 16777216.0f32) as u32;
    f32::from_bits((bits & 0x8000_0000) | (112u32 << 23) | (mant << 13))
}
