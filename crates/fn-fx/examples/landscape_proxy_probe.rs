//! Research probe (LANDSCAPE MATERIAL, 2026-10-08): the landscape actor / component properties that drive the LOD (LODDistanceFactor, LODFalloff, MaxLODLevel,
//! StaticLightingLOD, per-component ForcedLOD / LODBias) of every landscape level. `landscape_proxy_probe`
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let persistent = fn_fx::data::PackageJson::load(&set, "/Game/Athena/Maps/Athena_Terrain")?;
    let mut levels = vec!["/Game/Athena/Maps/Athena_Terrain".to_owned()];
    for e in persistent.exports.iter().filter(|e| e.class.starts_with("LevelStreaming")) { if let Some(wa) = e.props.get("WorldAsset").and_then(|v| v.as_str()) { levels.push(fn_fx::data::long_package(wa).to_owned()); } }
    for lv in levels {
        let p = fn_fx::data::PackageJson::load(&set, &lv)?;
        for e in p.exports.iter().filter(|e| e.class == "Landscape" || e.class == "LandscapeStreamingProxy") {
            let mut j = e.props.clone();
            if let Some(o) = j.as_object_mut() { o.retain(|k, _| !k.contains("Material") && k != "LandscapeComponents" && k != "CollisionComponents" && k != "BodyInstance"); }
            println!("== {lv} {} {}: {}", e.class, e.name, serde_json::to_string(&j).unwrap().chars().take(1500).collect::<String>());
        }
        let mut keys = std::collections::BTreeMap::<String, usize>::new();
        for c in p.of_class("LandscapeComponent") { if let Some(o) = c.props.as_object() { for k in o.keys() { *keys.entry(k.clone()).or_default() += 1; } } for k in ["ForcedLOD", "LODBias", "LODDistanceFactor"] { if let Some(v) = c.props.get(k) { println!("   {} {k} = {v}", c.name); } } }
        println!("   component keys {keys:?}");
    }
    Ok(())
}
