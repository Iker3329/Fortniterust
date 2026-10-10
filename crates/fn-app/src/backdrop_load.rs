//! BACKDROP HOOK (runtime half): reads `<assets>/props/backdrop/` (written by `fn-export props backdrop`) and hands the real landscape / ocean / skybox
//! geometry to `fn_render::backdrop`. See that module for how the 1.8 map is composed and what is approximated.
use fn_render::backdrop::{self, BackdropData, BackdropInput, BdMaterial, Kind, MeshBin, Placement};
use glam::Vec3;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `<root>/props/backdrop` of the first asset root holding a `backdrop.json` (or `FN_BACKDROP_DIR`).
pub fn find_dir(roots: &[PathBuf]) -> Option<PathBuf> {
    if let Some(d) = fn_live::var_os("FN_BACKDROP_DIR").map(PathBuf::from) { if d.join("backdrop.json").is_file() { return Some(d); } }
    roots.iter().map(|r| r.join("props").join("backdrop")).find(|d| d.join("backdrop.json").is_file())
}

/// The layout centre (UE cm) the props are placed around: `center_abs` of `<props>/layout/sample.json`, or the `--world-center` override.
pub fn read_centre(dir: &Path) -> Result<[f32; 3], String> {
    if let Some(c) = crate::poi::world_center() { return Ok(c); } // POI (dev)
    let p = dir.parent().ok_or("no props dir")?.join("layout").join("sample.json");
    let v: Value = serde_json::from_slice(&std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(|e| e.to_string())?;
    let a = v["center_abs"].as_array().ok_or("sample.json: center_abs")?;
    Ok([f(a.first(), 0.), f(a.get(1), 0.), f(a.get(2), 0.)])
}

fn f(v: Option<&Value>, d: f32) -> f32 { v.and_then(|x| x.as_f64()).map_or(d, |x| x as f32) }
fn vec4(m: &Value, name: &str, d: [f32; 4]) -> [f32; 4] {
    m.pointer(&format!("/vectors/{name}")).and_then(|v| v.as_array()).map(|a| [f(a.first(), d[0]), f(a.get(1), d[1]), f(a.get(2), d[2]), f(a.get(3), d[3])]).unwrap_or(d)
}

/// Terrain layer texture tiling in metres. RECOVERED: road 1024 cm (`RoadSize`), rock 768 cm (`TexSize_1`), grass 2 m (the plate's repeat); ASSUMED: forest / mud 5 m.
pub const TILES_M: [f32; 4] = [5., 5., 10.24, 7.68];
/// ASSUMED rock-layer slope window (1 - normal.y).
pub const ROCK_SLOPE: [f32; 2] = [0.30, 0.55];

pub fn load(dir: &Path, centre_cm: [f32; 3], anchor: Vec3, plate_half: f32) -> Result<BackdropData, String> {
    let t0 = std::time::Instant::now();
    let doc: Value = serde_json::from_slice(&std::fs::read(dir.join("backdrop.json")).map_err(|e| format!("backdrop.json: {e}"))?).map_err(|e| format!("backdrop.json: {e}"))?;
    let tex_dir = dir.join("textures");
    let tex = |ue: &str| -> Option<image::RgbaImage> {
        let png = doc.pointer(&format!("/textures/{}/png", ue.replace('/', "~1")))?.as_str()?;
        backdrop::load_png(&tex_dir.join(png)).map_err(|e| eprintln!("[backdrop] {e}")).ok()
    };
    let by_leaf = |leaf: &str| -> Option<image::RgbaImage> {
        let key = doc.get("textures")?.as_object()?.keys().find(|k| k.rsplit('.').next() == Some(leaf))?.clone();
        tex(&key)
    };
    // SRGB BEFORE FILTER (2026-10-07, MATCH track): the cooked `SRGB` flag of a texture (backdrop.json `srgb`, the exporter's Texture2D read); false if absent
    let srgb = |ue: &str| doc.pointer(&format!("/textures/{}/srgb", ue.replace('/', "~1"))).and_then(|v| v.as_bool()).unwrap_or(false);
    let srgb_leaf = |leaf: &str| doc.get("textures").and_then(|t| t.as_object()).and_then(|t| t.keys().find(|k| k.rsplit('.').next() == Some(leaf))).map_or(false, |k| srgb(k));
    let terrain = match std::fs::read(dir.join("terrain.bin")) {
        Ok(b) => Some(backdrop::parse_terrain(&b)?),
        Err(e) => { eprintln!("[backdrop] terrain.bin: {e}: no landscape"); None }
    };
    let mut materials: Vec<BdMaterial> = vec![];
    // 0: landscape (M_Athena_Terrain_Landscape layers)
    materials.push(BdMaterial {
        kind: Kind::Terrain,
        textures: [by_leaf("T_Grasslands_D"), by_leaf("T_FORT_ForestFloor01"), by_leaf("Mud"), by_leaf("T_Street_Road_D"), by_leaf("T_Cave_Exterior_Wall_D"), by_leaf("WhiteRockCustom_D")],
        srgb: ["T_Grasslands_D", "T_FORT_ForestFloor01", "Mud", "T_Street_Road_D", "T_Cave_Exterior_Wall_D", "WhiteRockCustom_D"].map(|n| srgb_leaf(n)),
        params: [[TILES_M[0], TILES_M[1], TILES_M[2], TILES_M[3]], [2., ROCK_SLOPE[0], ROCK_SLOPE[1], 0.], [1., 1., 1., 0.], [0.; 4]],
    });
    let mut mat_of: HashMap<String, usize> = HashMap::new();
    let mut placements = vec![];
    let mut meshes: HashMap<String, Arc<MeshBin>> = HashMap::new();
    let mut sea_cm = -5312.;
    for inst in doc.get("instances").and_then(|v| v.as_array()).ok_or("backdrop.json: instances")? {
        let name = inst["mesh"].as_str().unwrap_or("").to_string();
        if !name.contains("Ocean") && crate::backdrop_real::enabled() { continue; } // BACKDROP REAL: the far terraces / rocks are drawn on their real cooked material (backdrop_real.rs)
        let m = &inst["material"];
        let key = m["path"].as_str().unwrap_or("").to_string();
        let mi = if let Some(&i) = mat_of.get(&key) { i } else {
            let tx = |p: &str| m.pointer(&format!("/textures/{p}")).and_then(|v| v.as_str()).and_then(&tex);
            let tx_srgb = |p: &str| m.pointer(&format!("/textures/{p}")).and_then(|v| v.as_str()).map_or(false, |k| srgb(k));
            let mat = if name.contains("Ocean") {
                let wc = vec4(m, "Water_Color", [0.0381, 0.046494, 0.06, 0.]);
                let wc1 = vec4(m, "Water_Color_1", [0.007125, 0.012682, 0.025, 0.]);
                // ASSUMED: normal texture tiling (60 m) and pan speed (0.4 m/s, `FarNoiseSpeed`); RECOVERED: flatness near / far
                BdMaterial { kind: Kind::Ocean, textures: [by_leaf("T_Water2_N"), None, None, None, None, None], srgb: [srgb_leaf("T_Water2_N"), false, false, false, false, false], params: [wc, wc1, [60., f(m.pointer("/scalars/NormalNear"), 0.9266), f(m.pointer("/scalars/NormalFar"), 0.9753), 0.4 * 3.], [0.; 4]] }
            } else {
                let tint = vec4(m, "DiffuseColor1", [1., 1., 1., 1.]);
                BdMaterial { kind: Kind::Skybox, textures: [tx("OverlayUV0Texture"), tx("DiffuseTex"), tx("Mask"), None, None, None],
                    srgb: [tx_srgb("OverlayUV0Texture"), tx_srgb("DiffuseTex"), tx_srgb("Mask"), false, false, false],
                    params: [[f(m.pointer("/scalars/OverlayDiffuseSize"), 1.), f(m.pointer("/scalars/DiffuseTile"), 1.), 0.35, 0.5], [tint[0], tint[1], tint[2], 1.], [0.; 4], [0.; 4]] }
            };
            materials.push(mat);
            mat_of.insert(key, materials.len() - 1);
            materials.len() - 1
        };
        if name.contains("Ocean") { sea_cm = f(inst["translation_cm"].get(2), sea_cm); }
        let mesh = if let Some(x) = meshes.get(&name) { x.clone() } else {
            let b = std::fs::read(dir.join("meshes").join(format!("{name}.bin"))).map_err(|e| format!("{name}.bin: {e}"))?;
            let x = Arc::new(backdrop::parse_mesh(&b)?);
            meshes.insert(name.clone(), x.clone());
            x
        };
        let a3 = |k: &str| { let a = inst[k].as_array().cloned().unwrap_or_default(); [f(a.first(), 0.), f(a.get(1), 0.), f(a.get(2), 0.)] };
        let q = inst["rotation_quat"].as_array().cloned().unwrap_or_default();
        placements.push(Placement { name: inst["actor"].as_str().unwrap_or(&name).to_string(), mesh, t: a3("translation_cm"), q: [f(q.first(), 0.), f(q.get(1), 0.), f(q.get(2), 0.), f(q.get(3), 1.)], s: a3("scale"), material: mi });
    }
    let data = BackdropData::build(BackdropInput { terrain, placements, materials, terrain_material: 0, centre_cm, anchor, plate_half, sea_cm });
    // TERRAIN: the same landscape as walkable ground for every ground query of the process (fn_terrain::world), in the backdrop's mapping
    if let Ok(b) = std::fs::read(dir.join("terrain.bin")) {
        match fn_terrain::parse_terrain(&b).and_then(fn_terrain::Heightfield::new) {
            Ok(hf) => fn_terrain::install(Some(Arc::new(fn_terrain::World { hf, map: fn_terrain::Mapping { centre_cm, anchor, ref_z_cm: data.ground_ref_cm } }))),
            Err(e) => eprintln!("[terrain] {e}: flat ground"),
        }
    }
    let tris: usize = data.meshes.iter().map(|m| m.lods[0].1 as usize / 3).sum();
    eprintln!("[backdrop] {} meshes ({} triangles at full detail), sea at y {:.1} m, {:.0} ms; {}", data.meshes.len(), tris, data.sea_y, t0.elapsed().as_secs_f32() * 1000., data.notes.join("; "));
    Ok(data)
}
