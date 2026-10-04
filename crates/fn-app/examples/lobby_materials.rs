//! Lists the materials of the recovered lobby scene with their flags and the base-pass programs their cooked shader maps hold.
#![allow(dead_code)]
#[path = "../src/lobby_world.rs"]
mod lobby_world;

use std::collections::BTreeMap;

fn main() {
    let root = std::env::var_os("FN_ROOT").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let set = fn_pak::PakSet::open_install(&root).expect("paks");
    let d = lobby_world::read_lobby_scene_from(&set).expect("scene");
    // unique meshes -> default materials
    let mut meshes: BTreeMap<String, Vec<Option<String>>> = BTreeMap::new();
    for s in d.statics.iter().filter(|s| s.visible) {
        if meshes.contains_key(&s.mesh) { continue; }
        let long = s.mesh.split('.').next().unwrap().to_owned();
        let a = set.read_game(&long, "uasset").map(|b| b.into_owned()).unwrap_or_default();
        let x = set.read_game(&long, "uexp").map(|b| b.into_owned()).unwrap_or_default();
        let b = set.read_game(&long, "ubulk").map(|b| b.into_owned()).unwrap_or_default();
        let pkg = match fn_ueio::Package::parse(&a, &x, &b) { Ok(p) => p, Err(e) => { println!("MESH ERR {} {e}", s.mesh); continue } };
        match fn_mesh::StaticMesh::parse(&pkg) {
            Ok(m) => { meshes.insert(s.mesh.clone(), m.materials.iter().map(|x| x.material.clone()).collect()); }
            Err(e) => println!("MESH PARSE ERR {} {e}", s.mesh),
        }
    }
    let mut mats: BTreeMap<String, usize> = BTreeMap::new();
    for s in d.statics.iter().filter(|s| s.visible) {
        let defaults = meshes.get(&s.mesh).cloned().unwrap_or_default();
        let n = defaults.len().max(s.material_overrides.len());
        for i in 0..n {
            let m = s.material_overrides.get(i).cloned().flatten().or_else(|| defaults.get(i).cloned().flatten());
            if let Some(m) = m { *mats.entry(m).or_default() += 1; }
        }
    }
    println!("{} unique meshes, {} unique materials", meshes.len(), mats.len());
    for (path, uses) in &mats {
        match fn_fx::material::Material::load(&set, path) {
            Err(e) => println!("MAT {path} x{uses}: LOAD ERROR {e}"),
            Ok(m) => {
                let base: Vec<String> = m.cooked.programs.iter().filter(|p| p.shader.major >= 5 && p.vertex_factory == "FLocalVertexFactory" && p.shader_type.starts_with("TBasePass")).map(|p| p.shader_type.clone()).collect();
                println!("MAT {path} x{uses}: base={} blend={} shading={} two_sided={} tex={} localVF-basepass={:?}", m.base_path.rsplit('/').next().unwrap_or(""), m.flags.blend, m.flags.shading, m.flags.two_sided, m.referenced_textures.len(), base);
            }
        }
    }
}
