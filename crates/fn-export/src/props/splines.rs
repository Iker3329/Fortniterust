//! ROADS / SPLINES (2026-10-08) `props splines`: every cooked `SplineMeshComponent` of `Athena_Terrain` and its streamed levels (the persistent level + the
//! `LevelStreaming*` list of `layout::levels`), with what `FSplineMeshSceneProxy` reads from it (UE4.16 SplineMeshSceneProxy.cpp:6-40): the mesh, the
//! `OverrideMaterials`, the world transform (`AttachParent` chain inside the level, then the LevelTransform), `SplineParams`, `SplineUpDir`, `ForwardAxis`,
//! `bSmoothInterpRollScale`, `SplineBoundaryMin/Max`, `CastShadow`, the draw distance.
//!
//! WHERE THEY ARE (measured on the 1.8 paks with `fn-inspect exports` / `props`): 139 components, all in the four landscape streaming levels
//! (`Athena_Terrain_LS_01` 60, `_LS_02` 15, `_LS_03` 33, `_LS_05` 31), attached to their `LandscapeStreamingProxy_0:LandscapeSplinesComponent_0`; they are the
//! meshes of the landscape splines (`ULandscapeSplineSegment::UpdateSplineEditorMesh` / `UpdateSplineMeshes`, LandscapeSplines.cpp:2046-2420; 14 segments in the
//! persistent `Landscape1` with `CookedForeignMeshComponents` in the LS levels + 10 in `_LS_03`'s proxy), every one `S_Athena_Stream_2W_Straight_1L_a`: the
//! STREAMS (rivers). No road uses a spline in 1.8: the roads are the `Road_2` landscape layer and the POI / LS static meshes (`S_Elevation_Ground_Z`, `S_Road_*`).
//!
//! Unserialized values take the `USplineMeshComponent` constructor's defaults (SplineMeshComponent.cpp:266-290): SplineParams StartTangent / EndTangent
//! (100, 0, 0), EndPos (100, 0, 0), StartScale / EndScale (1, 1), the rest 0; SplineUpDir (0, 0, 1); bSmoothInterpRollScale false; ForwardAxis X (the enum's
//! zero; LandscapeSplines.cpp:2381 writes `SplineUpDir = (0,0,1)` and the mesh entry's ForwardAxis, X here: the cooked `StartScale` (4, 1) =
//! Width 2048 / mesh BoxExtent.Y 512 and `StartOffset.X` 2048 = -Origin.Y 512 x 4 are the X-forward branch of :2289 / :2301 / :2321);
//! SplineBoundaryMin = Max = 0; CastShadow true (UMeshComponent, the segment's `bCastShadow` default; :2422).
//!
//! Output `<out>/splines/landscape_splines.json` (a separate file: the `poi/*.json` format is not touched; `props poi` skips these components already, their
//! class does not end in `StaticMeshComponent`).
use super::backdrop::rel_xf;
use super::layout::{compose, levels, Xf};
use super::PropsArgs;
use crate::{write_file, Source};
use fn_uasset::PackageData;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

fn v3(v: Option<&Value>, d: [f64; 3]) -> [f64; 3] { v.and_then(|v| v.as_array()).filter(|a| a.len() >= 3).map(|a| [a[0].as_f64().unwrap_or(d[0]), a[1].as_f64().unwrap_or(d[1]), a[2].as_f64().unwrap_or(d[2])]).unwrap_or(d) }
fn v2(v: Option<&Value>, d: [f64; 2]) -> [f64; 2] { v.and_then(|v| v.as_array()).filter(|a| a.len() >= 2).map(|a| [a[0].as_f64().unwrap_or(d[0]), a[1].as_f64().unwrap_or(d[1])]).unwrap_or(d) }

/// `FSplineMeshParams` with the constructor defaults for the members the cook left out (delta serialization against the archetype).
pub fn spline_params(p: Option<&Value>) -> Value {
    let p = p.unwrap_or(&Value::Null);
    json!({
        "start_pos": v3(p.get("StartPos"), [0.; 3]), "start_tangent": v3(p.get("StartTangent"), [100., 0., 0.]), "start_scale": v2(p.get("StartScale"), [1., 1.]),
        "start_roll": p.get("StartRoll").and_then(|v| v.as_f64()).unwrap_or(0.), "start_offset": v2(p.get("StartOffset"), [0., 0.]),
        "end_pos": v3(p.get("EndPos"), [100., 0., 0.]), "end_tangent": v3(p.get("EndTangent"), [100., 0., 0.]), "end_scale": v2(p.get("EndScale"), [1., 1.]),
        "end_roll": p.get("EndRoll").and_then(|v| v.as_f64()).unwrap_or(0.), "end_offset": v2(p.get("EndOffset"), [0., 0.]),
    })
}

/// `ESplineMeshAxis` of a tagged enum value (`"ESplineMeshAxis::Y"`); absent = X (0).
fn axis(v: Option<&Value>) -> u8 { match v.and_then(|v| v.as_str()).map(|s| s.rsplit("::").next().unwrap_or(s)) { Some("Y") => 1, Some("Z") => 2, _ => 0 } }

fn scan(src: &Source, package: &str, level_xf: Xf, out: &mut Vec<Value>, skipped: &mut BTreeMap<String, usize>) -> Result<(), String> {
    let pd = PackageData::load(&src.set, package).map_err(|e| format!("{package}: {e}"))?;
    let pkg = pd.package().map_err(|e| format!("{package}: {e}"))?;
    let leaf = package.rsplit('/').next().unwrap_or(package).to_string();
    let n = pkg.exports.len();
    if !(0..n).any(|i| pkg.export_class_name(i) == "SplineMeshComponent") { return Ok(()); }
    // every component's JSON + AttachParent export index (lazily)
    let mut cache: HashMap<usize, (Value, Option<usize>)> = HashMap::new();
    let mut read = |i: usize| -> Option<(Value, Option<usize>)> {
        if let Some(v) = cache.get(&i) { return Some(v.clone()); }
        let o = pkg.read_export(i).ok()?;
        let parent = o.props.get("AttachParent").and_then(|v| v.as_object()).and_then(|r| r.export_index());
        let r = (o.props.to_json(&pkg), parent);
        cache.insert(i, r.clone());
        Some(r)
    };
    for i in 0..n {
        if pkg.export_class_name(i) != "SplineMeshComponent" || pkg.exports[i].name.starts_with("Default__") { continue; }
        let Some((m, parent)) = read(i) else { *skipped.entry("unreadable".into()).or_insert(0) += 1; continue };
        // world transform: the AttachParent chain (max 8) inside the level, then the level transform
        let mut chain = vec![m.clone()];
        let mut cur = parent;
        while let Some(k) = cur { if chain.len() > 8 { break; } let Some((j, p)) = read(k) else { break }; chain.push(j); cur = p; }
        let mut xf = level_xf;
        for j in chain.iter().rev() { xf = compose(rel_xf(j), xf); }
        let Some(mesh) = m.get("StaticMesh").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else { *skipped.entry("no StaticMesh".into()).or_insert(0) += 1; continue };
        if m.get("bVisible").and_then(|v| v.as_bool()) == Some(false) || m.get("bHiddenInGame").and_then(|v| v.as_bool()) == Some(true) { *skipped.entry("hidden".into()).or_insert(0) += 1; continue; }
        let cull = m.get("CachedMaxDrawDistance").and_then(|v| v.as_f64()).filter(|v| *v > 0.).or_else(|| m.get("LDMaxDrawDistance").and_then(|v| v.as_f64()).filter(|v| *v > 0.)).unwrap_or(0.);
        out.push(json!({
            "level": leaf, "name": pkg.exports[i].name, "mesh": mesh,
            "overrides": m.get("OverrideMaterials").and_then(|v| v.as_array()).cloned().unwrap_or_default(),
            "t": xf.t, "q": xf.q, "s": xf.s,
            "params": spline_params(m.get("SplineParams")),
            "up_dir": v3(m.get("SplineUpDir"), [0., 0., 1.]),
            "forward_axis": axis(m.get("ForwardAxis")),
            "smooth_interp_roll_scale": m.get("bSmoothInterpRollScale").and_then(|v| v.as_bool()).unwrap_or(false),
            "boundary_min": m.get("SplineBoundaryMin").and_then(|v| v.as_f64()).unwrap_or(0.),
            "boundary_max": m.get("SplineBoundaryMax").and_then(|v| v.as_f64()).unwrap_or(0.),
            "cast_shadow": m.get("CastShadow").and_then(|v| v.as_bool()).unwrap_or(true),
            "cull_cm": cull,
            "collision_profile": m.pointer("/BodyInstance/CollisionProfileName").and_then(|v| v.as_str()),
            "collision_enabled": m.pointer("/BodyInstance/CollisionEnabled").and_then(|v| v.as_str()),
        }));
    }
    Ok(())
}

pub fn run(src: &Source, a: &PropsArgs) -> Result<(), String> {
    let lvls = levels(src)?;
    let mut comps = vec![];
    let mut skipped = BTreeMap::new();
    for lv in &lvls { if let Err(e) = scan(src, &lv.package, lv.xf, &mut comps, &mut skipped) { eprintln!("[splines] {e}"); } }
    let mut per_level: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_mesh: BTreeMap<String, usize> = BTreeMap::new();
    for c in &comps { *per_level.entry(c["level"].as_str().unwrap_or("").into()).or_insert(0) += 1; *per_mesh.entry(c["mesh"].as_str().unwrap_or("").into()).or_insert(0) += 1; }
    let doc = json!({
        "version": 1, "map": super::layout::MAP,
        "units": "UE world cm (left handed, Z up); t / q / s = the component's world transform; params = FSplineMeshParams in component space",
        "levels_scanned": lvls.len(), "per_level": per_level, "per_mesh": per_mesh, "skipped": skipped, "components": comps,
    });
    let path = a.out.join("splines").join("landscape_splines.json");
    write_file(&path, serde_json::to_string(&doc).unwrap().as_bytes())?;
    println!("{} spline mesh components over {} levels: {:?}; meshes {:?}; skipped {:?}\nwrote {}", comps.len(), lvls.len(), doc["per_level"], doc["per_mesh"], doc["skipped"], path.display());
    Ok(())
}
