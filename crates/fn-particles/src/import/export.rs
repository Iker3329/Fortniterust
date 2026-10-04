//! Asset-tree writer: systems (`systems/<Name>.json`), materials (`materials/<Name>.json`), textures (`textures/<Name>.png`), static meshes (`meshes/<Name>.json`) and `index.json`.
use super::material;
use super::system::import_system;
use crate::model::*;
use fn_pak::PakSet;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn short(path: &str) -> String { path.rsplit(|c| c == '.' || c == '/').next().unwrap_or(path).to_string() }
fn pkg_of(path: &str) -> &str { path.split('.').next().unwrap_or(path) }
fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?; }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Everything a system references, over every enabled LOD.
pub struct Refs { pub materials: BTreeSet<String>, pub meshes: BTreeSet<String>, pub textures: BTreeSet<String> }
pub fn refs_of(sys: &ParticleSystem) -> Refs {
    let mut r = Refs { materials: BTreeSet::new(), meshes: BTreeSet::new(), textures: BTreeSet::new() };
    for e in &sys.emitters { for l in e.lods.iter().filter(|l| l.enabled) {
        match &l.type_data {
            TypeData::Mesh { mesh, .. } => { if !mesh.is_empty() { r.meshes.insert(mesh.clone()); } }
            _ => { if !l.required.material.is_empty() { r.materials.insert(l.required.material.clone()); } }
        }
        for m in &l.modules { if let ModuleKind::MeshMaterial { materials } = &m.kind { for x in materials { r.materials.insert(x.clone()); } } }
        if let Some(t) = &l.required.cutout_texture { r.textures.insert(t.clone()); }
    } }
    r
}

/// `StaticMesh` LOD 0 -> [`MeshDef`] in game space.
pub fn export_mesh(set: &PakSet, path: &str) -> Result<MeshDef, String> {
    let long = pkg_of(path);
    let read = |ext: &str| set.read_game(long, ext).map(|c| c.into_owned()).unwrap_or_default();
    let (ua, ue) = (set.read_game(long, "uasset").map_err(|e| format!("{long}: {e}"))?.into_owned(), read("uexp"));
    let pkg = fn_ueio::Package::parse(&ua, &ue, &[]).map_err(|e| format!("{long}: {e}"))?;
    let m = fn_mesh::StaticMesh::parse(&pkg).map_err(|e| format!("{long}: {e}"))?;
    let lod = m.lods.first().ok_or_else(|| format!("{long}: no LOD"))?;
    let mut d = MeshDef { name: short(path), ..Default::default() };
    for (i, p) in lod.positions.iter().enumerate() {
        d.positions.push([p[0] * 0.01, p[2] * 0.01, p[1] * 0.01]);
        let n = lod.normal(i); d.normals.push([n[0], n[2], n[1]]);
        d.uvs.push(lod.uvs.first().and_then(|u| u.get(i)).copied().unwrap_or([0.0, 0.0]));
    }
    d.indices = lod.indices.clone();
    d.sections = lod.sections.iter().map(|s| (s.first_index, s.num_triangles * 3, s.material_index.max(0) as u32)).collect();
    d.materials = m.materials.iter().map(|x| x.material.clone().unwrap_or_default()).collect();
    Ok(d)
}

fn export_texture(set: &PakSet, ue_path: &str, out: &Path) -> Result<TextureRef, String> {
    let long = pkg_of(ue_path);
    let rd = |ext: &str| set.read_game(long, ext).map(|c| c.into_owned()).unwrap_or_default();
    let ua = set.read_game(long, "uasset").map_err(|e| format!("{long}: {e}"))?.into_owned();
    let tex = fn_texture::Texture2D::from_files(&ua, &rd("uexp"), &rd("ubulk")).map_err(|e| format!("{long}: {e}"))?;
    let mip = tex.mip_for_max_dimension(1024).or_else(|| tex.best_mip()).ok_or_else(|| format!("{long}: no mip data"))?;
    let img = tex.decode(mip, fn_texture::DecodeOptions::default()).map_err(|e| format!("{long}: {e}"))?;
    let rgba = img.to_rgba8();
    let has_alpha = rgba.chunks_exact(4).any(|p| p[3] != 255);
    let name = short(ue_path);
    let buf = image::RgbaImage::from_raw(img.width as u32, img.height as u32, rgba).ok_or("bad image size")?;
    let png = format!("textures/{name}.png");
    if let Some(p) = out.join(&png).parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())?; }
    buf.save(out.join(&png)).map_err(|e| format!("{png}: {e}"))?;
    Ok(TextureRef { name, ue_path: ue_path.to_string(), png, srgb: tex.srgb, compression: tex.compression_settings.clone(), width: img.width as u32, height: img.height as u32, has_alpha })
}

#[derive(Default)]
pub struct Report { pub systems: Vec<String>, pub materials: usize, pub textures: usize, pub meshes: usize, pub failures: Vec<String>, pub unsupported: Vec<(String, String, usize)> }

/// Exports `packages` (long package paths of particle systems) to `out`.
pub fn export_all(set: &PakSet, packages: &[String], out: &Path) -> Result<Report, String> {
    let mut rep = Report::default();
    let mut all = Refs { materials: BTreeSet::new(), meshes: BTreeSet::new(), textures: BTreeSet::new() };
    let mut index: BTreeMap<String, String> = BTreeMap::new();
    for p in packages {
        match import_system(set, p) {
            Ok(sys) => {
                let r = refs_of(&sys);
                all.materials.extend(r.materials); all.meshes.extend(r.meshes); all.textures.extend(r.textures);
                for (c, n) in sys.unsupported() { rep.unsupported.push((sys.name.clone(), c, n)); }
                write(&out.join("systems").join(format!("{}.json", sys.name)), serde_json::to_string(&sys).map_err(|e| e.to_string())?.as_bytes())?;
                index.insert(sys.package.clone(), sys.name.clone()); rep.systems.push(sys.name);
            }
            Err(e) => rep.failures.push(format!("system {p}: {e}")),
        }
    }
    // meshes first: their own materials join the material set
    let mut mesh_names = BTreeMap::new();
    for m in &all.meshes {
        match export_mesh(set, m) {
            Ok(d) => { for x in &d.materials { if !x.is_empty() { all.materials.insert(x.clone()); } } write(&out.join("meshes").join(format!("{}.json", d.name)), serde_json::to_string(&d).map_err(|e| e.to_string())?.as_bytes())?; mesh_names.insert(m.clone(), d.name.clone()); rep.meshes += 1; }
            Err(e) => rep.failures.push(format!("mesh {m}: {e}")),
        }
    }
    let mut tex_cache: BTreeMap<String, TextureRef> = BTreeMap::new();
    for m in &all.materials {
        match material::resolve(set, m) {
            Ok(mut def) => {
                for (_, t) in def.textures.iter_mut() {
                    if !tex_cache.contains_key(&t.ue_path) {
                        match export_texture(set, &t.ue_path, out) { Ok(r) => { tex_cache.insert(t.ue_path.clone(), r); } Err(e) => { rep.failures.push(format!("texture {}: {e}", t.ue_path)); continue; } }
                    }
                    *t = tex_cache[&t.ue_path].clone();
                }
                def.textures.retain(|_, t| !t.png.is_empty());
                material::choose_color_texture(&mut def);
                write(&out.join("materials").join(format!("{}.json", def.name)), serde_json::to_string_pretty(&def).map_err(|e| e.to_string())?.as_bytes())?; rep.materials += 1;
            }
            Err(e) => rep.failures.push(format!("material {m}: {e}")),
        }
    }
    for t in &all.textures { if !tex_cache.contains_key(t) { match export_texture(set, t, out) { Ok(r) => { tex_cache.insert(t.clone(), r); } Err(e) => rep.failures.push(format!("texture {t}: {e}")) } } }
    rep.textures = tex_cache.len();
    let idx = serde_json::json!({ "schema": 1, "systems": index, "meshes": mesh_names });
    write(&out.join("index.json"), serde_json::to_string_pretty(&idx).map_err(|e| e.to_string())?.as_bytes())?;
    let _: PathBuf = out.to_path_buf();
    Ok(rep)
}
