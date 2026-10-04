//! Mesh stage: manifest `meshes[]` -> glTF (+ bin) next to `exported/<package dir>/`, every mesh material -> `.mat.json`,
//! and the sky meshes (`sky/dome_optimized.bin`, `sky/sunmoon.bin` in the old SKYM layout, `sky/*.gltf`).
use crate::mat::material_json;
use crate::source::Source;
use crate::{entry, parallel, write_file, Args, Stage};
use fn_mesh::gltf;
use fn_mesh::{SkeletalMesh, StaticMesh};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;

const SKY_MESHES: &[(&str, &str)] = &[
    ("Game/Packages/Fortress_Sky/Mesh_NoLOD/FortniteSkyDome_Optimized", "dome_optimized.bin"),
    ("Game/Packages/Fortress_Sky/MaterialInstance/Arid/SM_FlatSunMoon_Arid", "sunmoon.bin"),
];
const SKY_GLTF: &[&str] = &[
    "Game/Packages/Fortress_Sky/Mesh_NoLOD/FortniteSkyDome_Optimized",
    "Game/Packages/Fortress_Sky/Mesh_NoLOD/FortniteSkyDome_Highpoly",
    "Game/Packages/Fortress_Sky/MaterialInstance/Arid/SM_FlatSunMoon_Arid",
];
const SKY_MATERIALS: &[&str] = &[
    "Game/Packages/Fortress_Sky/MaterialInstance/Arid/M_AridClouds",
    "Game/Packages/Fortress_Sky/MaterialInstance/Arid/M_AridClouds_Inst_Day",
    "Game/Packages/Fortress_Sky/MaterialInstance/Arid/M_AridSky",
    "Game/Packages/Fortress_Sky/MaterialInstance/Arid/M_AridSky_Inst_Day",
    "Game/Packages/Fortress_Sky/MaterialInstance/Arid/M_Sun_Arid",
];

fn split_pkg(pkg: &str) -> (&str, &str) {
    match pkg.rfind('/') {
        Some(i) => (&pkg[..i], &pkg[i + 1..]),
        None => ("", pkg),
    }
}

/// Export the material package `long` (e.g. `/Game/.../M_X.M_X` or `/Game/.../M_X`) to `<out>/exported/<pkg>.mat.json`.
fn export_material(src: &Source, long: &str, out: &Path) -> Result<Value, String> {
    let pkg_path = long.split('.').next().unwrap_or(long).trim_start_matches('/').to_string();
    let pkg = src.package(&pkg_path)?;
    let (_, name) = split_pkg(&pkg_path);
    let j = material_json(&pkg, name)?;
    let dest = out.join("exported").join(format!("{pkg_path}.mat.json"));
    write_file(&dest, serde_json::to_string_pretty(&j).unwrap().as_bytes())?;
    Ok(json!({"material": pkg_path, "class": j["class"]}))
}

/// Compare produced glTF/bin with the old UEViewer export: sizes and differing byte count of the .bin.
fn verify_gltf(new_gltf: &Path, old_root: &Path, rel: &Path) -> Value {
    let old_gltf = old_root.join(rel);
    let cmp = |a: &Path, b: &Path| -> Value {
        match (std::fs::read(a), std::fs::read(b)) {
            (Ok(x), Ok(y)) => {
                let diff = if x.len() == y.len() { x.iter().zip(&y).filter(|(p, q)| p != q).count() as i64 } else { -1 };
                json!({"new_len": x.len(), "old_len": y.len(), "differing_bytes": diff})
            }
            (Err(e), _) | (_, Err(e)) => json!({"error": e.to_string()}),
        }
    };
    json!({"bin": cmp(&new_gltf.with_extension("bin"), &old_gltf.with_extension("bin")), "gltf": cmp(new_gltf, &old_gltf)})
}

fn export_mesh(src: &Source, pkg_name: &str, args: &Args) -> Result<Value, String> {
    let pkg = src.package(pkg_name)?;
    let (dir, name) = split_pkg(pkg_name);
    let out_dir: PathBuf = args.out.join("exported").join(dir);
    let mut detail = serde_json::Map::new();
    let mut materials: Vec<String> = vec![];
    if pkg.find_export("SkeletalMesh").is_some() {
        let m = SkeletalMesh::parse(&pkg).map_err(|e| e.to_string())?;
        gltf::export_skeletal_mesh(&m, &out_dir, name).map_err(|e| e.to_string())?;
        detail.insert("kind".into(), json!("skeletal"));
        detail.insert("vertices".into(), json!(m.lods.first().map_or(0, |l| l.num_vertices)));
        detail.insert("bones".into(), json!(m.ref_skeleton.len()));
        detail.insert("lods".into(), json!(m.lods.len()));
        materials = m.materials.iter().filter_map(|x| x.material.clone()).collect();
    } else {
        let m = StaticMesh::parse(&pkg).map_err(|e| e.to_string())?;
        gltf::export_static_mesh(&m, &out_dir, name).map_err(|e| e.to_string())?;
        detail.insert("kind".into(), json!("static"));
        detail.insert("vertices".into(), json!(m.lods.first().map_or(0, |l| l.positions.len())));
        materials = m.materials.iter().filter_map(|x| x.material.clone()).collect();
    }
    if let Some(old) = &args.verify {
        let rel = Path::new("exported").join(dir).join(format!("{name}.gltf"));
        detail.insert("verify".into(), verify_gltf(&out_dir.join(format!("{name}.gltf")), old, &rel));
    }
    let mut mats = vec![];
    materials.sort();
    materials.dedup();
    for m in &materials {
        mats.push(match export_material(src, m, &args.out) {
            Ok(v) => v,
            Err(e) => json!({"material": m, "error": e}),
        });
    }
    detail.insert("materials".into(), Value::Array(mats));
    Ok(Value::Object(detail))
}

fn export_sky(src: &Source, args: &Args) -> Vec<Value> {
    let mut out = vec![];
    let sky_dir = args.out.join("sky");
    for (pkg, file) in SKY_MESHES {
        let t = Instant::now();
        let r = (|| -> Result<Value, String> {
            let p = src.package(pkg)?;
            let m = StaticMesh::parse(&p).map_err(|e| e.to_string())?;
            let bytes = fn_mesh::skym::static_mesh_to_skym(&m, 0, &gltf::Options::default()).map_err(|e| e.to_string())?;
            write_file(&sky_dir.join(file), &bytes)?;
            let mut d = json!({"bytes": bytes.len()});
            if let Some(old) = &args.verify {
                if let Ok(o) = std::fs::read(old.join("sky").join(file)) {
                    d["old_identical"] = json!(o == bytes);
                    let legacy = fn_mesh::skym::static_mesh_to_skym(&m, 0, &gltf::Options { legacy_uev_half_uv: true }).map_err(|e| e.to_string())?;
                    d["old_identical_with_legacy_uev_half_uv"] = json!(o == legacy);
                    d["old_len"] = json!(o.len());
                }
            }
            Ok(d)
        })();
        out.push(entry("sky-skym", pkg, &format!("sky/{file}"), &r, t.elapsed().as_millis()));
    }
    for pkg in SKY_GLTF {
        let t = Instant::now();
        let r = (|| -> Result<Value, String> {
            let p = src.package(pkg)?;
            let m = StaticMesh::parse(&p).map_err(|e| e.to_string())?;
            let (_, name) = split_pkg(pkg);
            gltf::export_static_mesh(&m, &sky_dir, name).map_err(|e| e.to_string())?;
            Ok(json!({"lods": m.lods.len()}))
        })();
        out.push(entry("sky-gltf", pkg, &format!("sky/{}.gltf", split_pkg(pkg).1), &r, t.elapsed().as_millis()));
    }
    for pkg in SKY_MATERIALS {
        let t = Instant::now();
        let r = export_material(src, pkg, &args.out);
        out.push(entry("sky-material", pkg, &format!("exported/{pkg}.mat.json"), &r, t.elapsed().as_millis()));
    }
    out
}

pub fn run(src: &Source, manifest: &Value, args: &Args) -> Stage {
    let mut pkgs: Vec<String> = manifest["meshes"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect();
    if let Some(f) = &args.only {
        pkgs.retain(|p| p.to_lowercase().contains(f.as_str()));
    }
    println!("meshes: {} packages", pkgs.len());
    let results = parallel(&pkgs, args.jobs, |p| {
        let t = Instant::now();
        let r = export_mesh(src, p, args);
        (r, t.elapsed().as_millis())
    });
    let mut entries = vec![];
    let mut failures = 0;
    for (p, (r, ms)) in pkgs.iter().zip(results) {
        if r.is_err() {
            failures += 1;
        }
        entries.push(entry("mesh", p, &format!("exported/{p}.gltf"), &r, ms));
    }
    if args.only.is_none() {
        for e in export_sky(src, args) {
            if e["ok"] == json!(false) {
                failures += 1;
            }
            entries.push(e);
        }
    }
    println!("meshes: {failures} failures");
    Stage { entries, failures }
}
