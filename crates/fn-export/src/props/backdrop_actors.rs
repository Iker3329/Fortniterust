//! `props backdrop-actors`: the placed `FortStaticMeshActor`s of the persistent `Athena_Terrain` level that form the backdrop (ocean planes, far skybox rocks and
//! terraces), their meshes (`meshes/*.bin`, see [`write_mesh`]), resolved materials and the textures they and the landscape material use.
use super::backdrop::{leaf_name, rel_xf};
use super::PropsArgs;
use crate::{write_file, Source};
use fn_mesh::StaticMesh;
use fn_texture::{DecodeOptions, Texture2D};
use fn_uasset::PackageData;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn export_png(src: &Source, path: &str, dir: &std::path::Path, max_dim: usize) -> Result<(String, bool), String> {
    let pkg = super::bp::package_of(path);
    let raw = src.load_raw(pkg)?;
    let tex = Texture2D::from_files(&raw.uasset, &raw.uexp, &raw.ubulk).map_err(|e| format!("{path}: {e}"))?;
    let mip = tex.mip_for_max_dimension(max_dim).ok_or_else(|| format!("{path}: no mip with data"))?;
    let img = tex.decode(mip, DecodeOptions::default()).map_err(|e| format!("{path}: {e}"))?;
    let name = format!("{}.png", leaf_name(path));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    img.write_png(dir.join(&name)).map_err(|e| format!("{path}: {e}"))?;
    Ok((name, tex.srgb))
}

/// `FNBM`: u32 version 1, u32 vertex count, u32 index count, then per vertex f32 pos xyz (UE cm), f32 normal xyz, f32 uv0 xy, u8 rgba colour (255 when absent),
/// then u32 indices (UE winding). LOD 0 only.
fn write_mesh(m: &StaticMesh) -> Result<Vec<u8>, String> {
    let l = m.lods.first().ok_or("mesh without LOD")?;
    let mut b = vec![];
    b.extend_from_slice(b"FNBM");
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&(l.positions.len() as u32).to_le_bytes());
    b.extend_from_slice(&(l.indices.len() as u32).to_le_bytes());
    for i in 0..l.positions.len() {
        for v in l.positions[i] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for v in l.normal(i) {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let uv = l.uvs.first().and_then(|u| u.get(i)).copied().unwrap_or([0.0; 2]);
        for v in uv {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let c = l.colors.get(i).copied().unwrap_or([255; 4]); // FColor memory order B,G,R,A
        b.extend_from_slice(&[c[2], c[1], c[0], c[3]]);
    }
    for i in &l.indices {
        b.extend_from_slice(&i.to_le_bytes());
    }
    Ok(b)
}

pub fn run(src: &Source, a: &PropsArgs) -> Result<(), String> {
    let out = a.out.join("backdrop");
    let pd = PackageData::load(&src.set, super::layout::MAP).map_err(|e| e.to_string())?;
    let pkg = pd.package().map_err(|e| e.to_string())?;
    let mut instances = vec![];
    let mut mesh_done: BTreeMap<String, usize> = BTreeMap::new();
    let mut tex_jobs: Vec<String> = vec![];
    for ai in 0..pkg.exports.len() {
        if pkg.export_class_name(ai) != "FortStaticMeshActor" {
            continue;
        }
        let actor = pkg.exports[ai].name.clone();
        let Some(ci) = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i) == "StaticMeshComponent" && pkg.exports[i].outer_index.export_index() == Some(ai)) else { continue };
        let j = pkg.read_export(ci).map_err(|e| e.to_string())?.props.to_json(&pkg);
        let Some(mesh_path) = j.get("StaticMesh").and_then(|v| v.as_str()).map(String::from) else { continue };
        let xf = rel_xf(&j);
        let mesh_pkg = super::bp::package_of(&mesh_path).to_string();
        let mesh = StaticMesh::parse(&src.package(&mesh_pkg)?).map_err(|e| format!("{mesh_pkg}: {e}"))?;
        let mesh_name = leaf_name(&mesh_path).to_string();
        if !mesh_done.contains_key(&mesh_name) {
            let bytes = write_mesh(&mesh)?;
            write_file(&out.join(format!("meshes/{mesh_name}.bin")), &bytes)?;
            mesh_done.insert(mesh_name.clone(), mesh.lods[0].positions.len());
        }
        let ov = j.get("OverrideMaterials").and_then(|v| v.as_array()).and_then(|a| a.first()).and_then(|v| v.as_str()).map(String::from);
        let mat_path = ov.or_else(|| mesh.materials.first().and_then(|m| m.material.clone()));
        let mut mat = Value::Null;
        if let Some(mp) = &mat_path {
            let r = super::material::resolve(&src.set, mp)?;
            for t in r.textures.values() {
                if !tex_jobs.contains(t) {
                    tex_jobs.push(t.clone());
                }
            }
            mat = json!({"path": mp, "name": r.name, "chain": r.chain, "root": r.root, "blend_mode": r.blend_mode, "two_sided": r.two_sided, "textures": r.textures, "scalars": r.scalars, "vectors": r.vectors, "notes": r.notes});
        }
        instances.push(json!({"actor": actor, "mesh": mesh_name, "mesh_path": mesh_path, "translation_cm": xf.t, "rotation_quat": xf.q, "scale": xf.s,
            "slot_name": mesh.materials.first().map(|m| m.slot_name.clone()), "material": mat, "bounds_cm": {"origin": mesh.bounds.origin, "extent": mesh.bounds.extent, "radius": mesh.bounds.radius}}));
    }
    // extra textures: the landscape material's layer diffuse textures + the ocean's
    for t in [
        "/Game/Packages/DS_Fortnite_Terrain_NoLOD/Textures/T_Grasslands_D.T_Grasslands_D",
        "/Game/Packages/DS_Fortnight/SM/Textures/T_FORT_ForestFloor01.T_FORT_ForestFloor01",
        "/Game/Packages/DS_Fortnight/Mud.Mud",
        "/Game/Packages/DS_Fortnight/SM/Textures/T_Street_Road_D.T_Street_Road_D",
        "/Game/Packages/DS_Fortnite_Terrain_NoLOD/Textures/T_Cave_Exterior_Wall_D.T_Cave_Exterior_Wall_D",
        "/Game/Packages/DS_Fortnight/WhiteRockCustom_D.WhiteRockCustom_D",
        "/Game/Packages/Fortnite_UniqueMaterials_1/Textures/T_Water2_N.T_Water2_N",
        "/Game/Packages/Fortnite_UniqueMaterials_1/Textures/Foam.Foam",
    ] {
        if !tex_jobs.contains(&t.to_string()) {
            tex_jobs.push(t.to_string());
        }
    }
    let mut tex_json: BTreeMap<String, Value> = BTreeMap::new();
    for t in &tex_jobs {
        match export_png(src, t, &out.join("textures"), 1024) {
            Ok((png, srgb)) => {
                tex_json.insert(t.clone(), json!({"png": png, "srgb": srgb}));
            }
            Err(e) => eprintln!("texture {t}: {e}"),
        }
    }
    let mut ws = Value::Null;
    for i in 0..pkg.exports.len() {
        if pkg.export_class_name(i) == "FortWorldSettings" {
            let j = pkg.read_export(i).map_err(|e| e.to_string())?.props.to_json(&pkg);
            ws = json!({"KillZ": j.get("KillZ"), "MapWorldScale": j.get("MapWorldScale"), "MapZOffset": j.get("MapZOffset")});
        }
    }
    let doc = json!({"schema": 1, "source_map": super::layout::MAP, "units": "UE cm, left handed, Z up", "world_settings": ws, "instances": instances, "textures": tex_json,
        "terrain": {"file": "terrain.bin", "landscape_material": "/Game/Athena/Prototype/Terrain/M_Athena_Terrain_Landscape_Inst"}});
    write_file(&out.join("backdrop.json"), serde_json::to_string_pretty(&doc).unwrap().as_bytes())?;
    println!("backdrop.json: {} placed meshes, {} meshes, {} textures", instances.len(), mesh_done.len(), tex_json.len());
    Ok(())
}
