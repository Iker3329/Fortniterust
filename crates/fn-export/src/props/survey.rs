//! Survey every Blueprint under the prop / Athena environment directories.
use super::bp::{load_bp, package_of};
use super::PropsArgs;
use crate::{parallel, Source};
use fn_mesh::StaticMesh;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const DIRS: &[&str] = &[
    "FortniteGame/Content/Building/ActorBlueprints/Prop/*.uasset",
    "FortniteGame/Content/Athena/Environments/Blueprints/*.uasset",
    "FortniteGame/Content/Athena/Prototype/Terrain/*.uasset",
    "FortniteGame/Content/Building/ActorBlueprints/Containers/*.uasset",
];

pub fn run(src: &Source, a: &PropsArgs) -> Result<(), String> {
    let mut pkgs: Vec<String> = vec![];
    for d in DIRS {
        for (_, e) in src.set.glob(d) {
            if let Some(g) = fn_pak::pak_path_to_game(&e.path) {
                pkgs.push(g);
            }
        }
    }
    pkgs.sort();
    pkgs.dedup();
    println!("{} candidate packages", pkgs.len());
    let rows = parallel(&pkgs, a.jobs, |p| match load_bp(&src.set, p) {
        Ok(bp) => {
            let (cat, sub) = bp.attr_key();
            json!({
                "package": p,
                "chain": bp.chain.iter().map(|l| package_of(&l.package).to_string()).collect::<Vec<_>>(),
                "attr_category": cat, "attr_sub": sub,
                "resource_type": bp.resource_type(),
                "static_mesh": bp.static_mesh(),
                "override_materials": bp.smc.get("OverrideMaterials"),
                "amount_row": bp.cdo.get("BuildingResourceAmountOverride"),
                "loot_tier": bp.cdo.get("DestructionLootTierGroup"),
                "has_smc": !bp.smc.is_empty(),
            })
        }
        Err(e) => json!({"package": p, "error": e}),
    });
    let out = a.file.clone().unwrap_or_else(|| a.out.join("survey.json"));
    crate::write_file(&out, serde_json::to_string_pretty(&Value::Array(rows)).unwrap().as_bytes())?;
    println!("wrote {}", out.display());
    Ok(())
}

/// `meshcheck --file survey.json [prefix]`: decode every distinct static mesh of the survey rows whose attribute category
/// starts with `prefix` (default `Resource`) and report LOD / vertex / triangle counts or the decode error.
pub fn meshcheck(src: &Source, a: &PropsArgs) -> Result<(), String> {
    let f = a.file.clone().ok_or("--file survey.json required")?;
    let rows: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&f).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let prefix = a.rest.first().cloned().unwrap_or_else(|| "Resource".into());
    let mut meshes = BTreeSet::new();
    for r in &rows {
        if r["attr_category"].as_str().map_or(false, |c| c.starts_with(&prefix)) {
            if let Some(m) = r["static_mesh"].as_str() {
                meshes.insert(package_of(m).to_string());
            }
        }
    }
    let meshes: Vec<String> = meshes.into_iter().collect();
    println!("{} distinct meshes", meshes.len());
    let res = parallel(&meshes, a.jobs, |m| {
        let r = src.package(m).and_then(|p| StaticMesh::parse(&p).map_err(|e| e.to_string()));
        match r {
            Ok(sm) => {
                let lods: Vec<Value> = sm
                    .lods
                    .iter()
                    .map(|l| json!({"verts": l.positions.len(), "tris": l.indices.len() / 3, "uv": l.num_tex_coords, "colors": l.colors.len(), "sections": l.sections.len(), "i32": l.indices_32bit}))
                    .collect();
                json!({"mesh": m, "ok": true, "lods": lods, "materials": sm.materials.iter().map(|x| x.material.clone()).collect::<Vec<_>>()})
            }
            Err(e) => json!({"mesh": m, "ok": false, "error": e}),
        }
    });
    let bad = res.iter().filter(|r| r["ok"] == json!(false)).count();
    println!("{} ok, {bad} failed", res.len() - bad);
    for r in res.iter().filter(|r| r["ok"] == json!(false)) {
        println!("FAIL {} {}", r["mesh"], r["error"]);
    }
    let out = a.out.join("meshcheck.json");
    crate::write_file(&out, serde_json::to_string_pretty(&Value::Array(res)).unwrap().as_bytes())?;
    Ok(())
}
