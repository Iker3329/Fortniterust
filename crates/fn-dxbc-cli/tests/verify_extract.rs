//! (3a) the extractor reproduces the old tools' `.dxbc` files byte for byte (same zlib stream ids, same re-signed checksums): sky dome /
//! clouds / sun, the global-shader-cache tonemapper shaders and every HUD material.
mod common;
use common::*;
use fn_dxbc_cli::extract::scan_blobs;
use fn_dxbc_cli::spec::package_bytes;

fn check(set: &fn_pak::PakSet, package: &str, old_dir: &std::path::Path) -> (usize, usize) {
    let blobs = scan_blobs(&package_bytes(set, package).unwrap());
    let (mut same, mut total) = (0, 0);
    for f in files(old_dir, "dxbc") {
        if f.parent().unwrap() != old_dir { continue; }
        total += 1;
        let id: usize = f.file_stem().unwrap().to_string_lossy().rsplit('_').next().unwrap().parse().unwrap();
        let want = std::fs::read(&f).unwrap();
        match blobs.iter().find(|b| b.id == id) {
            Some(b) if b.bytes == want => same += 1,
            Some(_) => eprintln!("{package}: shader_{id} differs"),
            None => eprintln!("{package}: shader_{id} not found"),
        }
    }
    (same, total)
}

#[test]
fn extraction_is_byte_identical_to_old_dxbc_files() {
    let Some(set) = open() else { eprintln!("no 1.8 install, skipping"); return };
    let reports = fn_testdata::reports();
    if !reports.join("sky-shaders").is_dir() { eprintln!("no old reports, skipping"); return; }
    let index = PathIndex::new(&set);
    let (mut same, mut total) = (0, 0);
    let mut add = |r: (usize, usize)| { same += r.0; total += r.1; };
    let arid = "/Game/Packages/Fortress_Sky/MaterialInstance/Arid";
    add(check(&set, &format!("{arid}/M_AridSky"), &reports.join("sky-shaders")));
    add(check(&set, &format!("{arid}/M_AridClouds"), &reports.join("sky-shaders-M_AridClouds")));
    add(check(&set, &format!("{arid}/M_Sun_Arid"), &reports.join("sky-shaders-M_Sun_Arid")));
    add(check(&set, "GlobalShaderCache", &reports.join("tonemap-shaders")));
    for d in std::fs::read_dir(reports.join("hud-shaders")).unwrap().flatten() {
        if !d.path().is_dir() { continue; }
        let name = d.file_name().to_string_lossy().to_string();
        let game = index.find(&name).unwrap_or_else(|| panic!("{name} not in paks"));
        add(check(&set, &game, &d.path()));
    }
    eprintln!("extraction: {same} of {total} old .dxbc files reproduced byte for byte");
    assert_eq!(same, total);
    assert!(total > 400);
}

/// The `<Name>-uniforms.json` writer reproduces the old documents (structure + values).
#[test]
fn uniforms_json_matches_old() {
    let Some(set) = open() else { eprintln!("no 1.8 install, skipping"); return };
    let dir = fn_testdata::reports().join("hud-shaders");
    let index = PathIndex::new(&set);
    let tmp = std::env::temp_dir().join("fn-dxbc-material-export");
    let _ = std::fs::remove_dir_all(&tmp);
    let mut n = 0;
    fn eq(a: &serde_json::Value, b: &serde_json::Value) -> bool {
        use serde_json::Value::*;
        match (a, b) {
            (Number(x), Number(y)) => { let (x, y) = (x.as_f64().unwrap() as f32, y.as_f64().unwrap() as f32); x == y || (x - y).abs() <= 1e-5 * 1.0f32.max(x.abs()).max(y.abs()) }
            (Array(x), Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| eq(a, b)),
            (Object(x), Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| eq(v, w))),
            _ => a == b,
        }
    }
    for j in files(&dir, "json") {
        let stem = j.file_stem().unwrap().to_string_lossy().to_string();
        let Some(name) = stem.strip_suffix("-uniforms") else { continue };
        let game = index.find(name).unwrap();
        fn_dxbc_cli::material::export_package_with(&set, &game, &tmp, true, false).unwrap();
        let mut old: serde_json::Value = serde_json::from_str(std::fs::read_to_string(&j).unwrap().trim_start_matches('\u{feff}')).unwrap();
        let mut new: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(tmp.join(format!("{name}-uniforms.json"))).unwrap()).unwrap();
        old.as_object_mut().unwrap().remove("source");
        new.as_object_mut().unwrap().remove("source");
        // the old textures2d entries of plain `Texture` expressions have no name (undefined -> missing key)
        for v in [&mut old, &mut new] { for t in v["textures2d"].as_array_mut().unwrap() { if t["name"].is_null() { t.as_object_mut().unwrap().remove("name"); } } }
        if !eq(&old, &new) {
            fn first(a: &serde_json::Value, b: &serde_json::Value, p: String) -> Option<String> {
                use serde_json::Value::*;
                match (a, b) {
                    (Number(_), Number(_)) => if eq(a, b) { None } else { Some(format!("{p}: old {a} ours {b}")) },
                    (Array(x), Array(y)) => { if x.len() != y.len() { return Some(format!("{p}: len {} vs {}", x.len(), y.len())); } x.iter().zip(y).enumerate().find_map(|(i, (a, b))| first(a, b, format!("{p}[{i}]"))) }
                    (Object(x), Object(y)) => { for (k, v) in x { match y.get(k) { Some(w) => if let Some(r) = first(v, w, format!("{p}.{k}")) { return Some(r) }, None => return Some(format!("{p}.{k} missing in ours")) } } for k in y.keys() { if !x.contains_key(k) { return Some(format!("{p}.{k} extra in ours")); } } None }
                    _ => if a == b { None } else { Some(format!("{p}: old {a} ours {b}")) },
                }
            }
            panic!("{name}: uniforms json differs at {}", first(&old, &new, String::new()).unwrap_or_default());
        }
        n += 1;
    }
    eprintln!("{n} uniforms documents identical");
    assert!(n >= 10);
}



