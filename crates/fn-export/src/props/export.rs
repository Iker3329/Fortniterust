//! `props export`: decode the harvestable props placed in `Athena_Terrain` and write the runtime asset tree.
//!
//! ```text
//! <out>/catalog.json                       index (schema 1)
//! <out>/textures/<Texture>.png             shared RGBA8 PNGs (<= 2048 px), + textures/index.json (sRGB / compression / sizes)
//! <out>/<Id>/<Mesh>.gltf + .bin            all LODs, shared vertex set per LOD, material per slot (fn_mesh::gltf_lods)
//! <out>/<Id>/materials/<Material>.mat.json texture params (+ roles), blend mode, clip value, two-sided, tint / wind params
//! <out>/<Id>/collision.json                BodySetup simple collision (cm, mesh local UE space)
//! <out>/<Id>/props.json                    health, resource, destruction, render, placement, verification
//! <out>/layout/{all,sample,stats}.json     Athena_Terrain placements
//! <out>/sheet/contact-sheet*.png           CPU rendered check of every exported mesh
//! ```
use super::bp::{leaf, package_of, Bp};
use super::collision::collision_json;
use super::layout::{gather, run_with, Gathered};
use super::material::{self, mat_json, Resolved, TexMeta};
use super::PropsArgs;
use crate::{parallel, write_file, Source};
use fn_mesh::{PropMaterial, StaticMesh};
use fn_texture::{DecodeOptions, Texture2D};
use fn_uasset::PackageData;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub struct Opts {
    pub min_count: usize,
    pub only: Option<String>,
    pub classes: Option<Vec<String>>,
    pub max_tex: usize,
    pub sheet: bool,
    pub report: Option<PathBuf>,
}

fn parse_opts(rest: &[String]) -> Result<Opts, String> {
    let mut o = Opts { min_count: 25, only: None, classes: None, max_tex: 2048, sheet: true, report: None };
    let mut it = rest.iter();
    while let Some(x) = it.next() {
        let mut val = |n: &str| it.next().cloned().ok_or_else(|| format!("{n} needs a value"));
        match x.as_str() {
            "--min-count" => o.min_count = val("--min-count")?.parse().map_err(|_| "bad --min-count")?,
            "--only" => o.only = Some(val("--only")?.to_lowercase()),
            "--classes" => o.classes = Some(val("--classes")?.split(',').map(|s| s.trim().to_string()).collect()),
            "--max-tex" => o.max_tex = val("--max-tex")?.parse().map_err(|_| "bad --max-tex")?,
            "--report" => o.report = Some(PathBuf::from(val("--report")?)),
            "--no-sheet" => o.sheet = false,
            other => return Err(format!("unknown export option {other}")),
        }
    }
    Ok(o)
}

struct ClassInfo {
    class: String,
    bp: String,
    count: usize,
    cat: String,
    sub: String,
    rt: String,
    mesh: String,
    scale_min: f64,
    scale_max: f64,
    scale_sum: f64,
}

pub fn category_of(id: &str, cat: &str, rt: &str, mesh: &str) -> &'static str {
    let l = id.to_lowercase();
    let m = mesh.to_lowercase();
    let resource = cat.starts_with("Resource");
    if l.contains("hedge") {
        "hedge"
    } else if l.contains("log") && l.contains("tree") {
        "log"
    } else if rt == "Wood" && resource && (l.contains("tree") || m.contains("/trees/") || m.contains("tree")) {
        "tree"
    } else if rt == "Stone" && (resource || l.contains("rock") || m.contains("rock")) {
        "rock"
    } else if l.contains("shrub") || l.contains("bush") || l.contains("plant") {
        "bush"
    } else if l.starts_with("car_") || m.contains("/vehicles/") {
        "vehicle"
    } else if l.contains("container") {
        "container"
    } else if l.contains("fence") || cat == "PropFence" {
        "fence"
    } else if l.contains("palette") || l.contains("pallet") {
        "pallet"
    } else if l.contains("crate") {
        "crate"
    } else if ["bench", "chair", "table", "shelves", "bunk", "picnic"].iter().any(|k| l.contains(k)) {
        "furniture"
    } else {
        "other"
    }
}

fn display_of(id: &str) -> String {
    let words: Vec<&str> = id.split('_').filter(|w| !w.is_empty() && !matches!(*w, "Athena" | "Prop" | "Props" | "C")).collect();
    if words.is_empty() {
        id.to_string()
    } else {
        words.join(" ")
    }
}

fn curve_keys(row: &Value) -> Vec<(f64, f64)> {
    row.get("Keys").and_then(|k| k.as_array()).map(|a| a.iter().filter_map(|k| Some((k.get("Time")?.as_f64()?, k.get("Value")?.as_f64()?))).collect()).unwrap_or_default()
}

/// Rows of a `CurveTable` as `name -> keys`.
fn load_curve_table(src: &Source, pkg_path: &str) -> Result<BTreeMap<String, Vec<(f64, f64)>>, String> {
    let pd = PackageData::load(&src.set, pkg_path).map_err(|e| e.to_string())?;
    let pkg = pd.package().map_err(|e| e.to_string())?;
    let i = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i) == "CurveTable").ok_or("no CurveTable export")?;
    let t = fn_uasset::classes::read_table(&pkg, i).map_err(|e| e.to_string())?;
    Ok(t.rows.iter().map(|(n, p)| (n.clone(), curve_keys(&p.to_json(&pkg)))).collect())
}

struct MeshData {
    mesh: StaticMesh,
    pkg: String,
    ext_bounds: Option<Value>,
    static_materials_prop: usize,
}

fn load_mesh(src: &Source, mesh_obj_path: &str) -> Result<MeshData, String> {
    let pkg_path = package_of(mesh_obj_path).to_string();
    let pkg = src.package(&pkg_path)?;
    let mesh = StaticMesh::parse(&pkg).map_err(|e| e.to_string())?;
    let (mut ext, mut nmat) = (None, 0);
    if let Ok(pd) = PackageData::load(&src.set, &pkg_path) {
        if let Ok(p) = pd.package() {
            if let Some(i) = (0..p.exports.len()).find(|&i| p.export_class_name(i) == "StaticMesh") {
                if let Ok(o) = p.read_export(i) {
                    let j = o.props.to_json(&p);
                    ext = j.get("ExtendedBounds").cloned();
                    nmat = j.get("StaticMaterials").and_then(|v| v.as_array()).map_or(0, |a| a.len());
                }
            }
        }
    }
    Ok(MeshData { mesh, pkg: pkg_path, ext_bounds: ext, static_materials_prop: nmat })
}

struct TexOut {
    meta: TexMeta,
}

fn export_texture(src: &Source, path: &str, png_name: &str, dir: &Path, max_dim: usize) -> Result<TexOut, String> {
    let pkg = package_of(path);
    let raw = src.load_raw(pkg)?;
    let tex = Texture2D::from_files(&raw.uasset, &raw.uexp, &raw.ubulk).map_err(|e| format!("{path}: {e}"))?;
    let mip = tex.mip_for_max_dimension(max_dim).ok_or_else(|| format!("{path}: no mip with data"))?;
    let img = tex.decode(mip, DecodeOptions::default()).map_err(|e| format!("{path}: {e}"))?;
    let has_alpha = !img.is_opaque();
    let grey = tex.compression_settings == "TC_Default" && {
        let px = img.to_rgba8();
        let n = (px.len() / 4).max(1);
        let chroma: f64 = px.chunks_exact(4).step_by(7).map(|p| (p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2])) as f64 / 255.0).sum::<f64>() / (n / 7 + 1) as f64;
        chroma < 0.06
    };
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    img.write_png(dir.join(png_name)).map_err(|e| format!("{path}: {e}"))?;
    Ok(TexOut {
        meta: TexMeta {
            name: tex.name.clone(),
            png: png_name.to_string(),
            srgb: tex.srgb,
            compression: tex.compression_settings.clone(),
            format: tex.format.name().to_string(),
            width: img.width as u32,
            height: img.height as u32,
            source_width: tex.size_x as u32,
            source_height: tex.size_y as u32,
            address_x: tex.address_x.clone(),
            address_y: tex.address_y.clone(),
            has_alpha,
            grey,
        },
    })
}

fn rnd(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

struct KindPlan {
    id: String,
    info: ClassInfoLite,
    bp: Bp,
    mesh_obj: String,
}

#[derive(Clone)]
struct ClassInfoLite {
    bp: String,
    count: usize,
    cat: String,
    sub: String,
    rt: String,
    mesh: String,
    scale: (f64, f64, f64),
}

struct KindResult {
    id: String,
    ok: bool,
    err: Option<String>,
    props: Value,
}

pub fn run(src: &Source, a: &PropsArgs) -> Result<(), String> {
    let o = parse_opts(&a.rest)?;
    let t0 = std::time::Instant::now();
    let g = gather(src, a.jobs)?;
    // ---- which classes ----
    let mut cls: BTreeMap<String, ClassInfo> = BTreeMap::new();
    for p in &g.all {
        if let Some((cat, sub, rt, mesh)) = g.kind_of(&p.bp) {
            let s = (p.xf.s[0] + p.xf.s[1] + p.xf.s[2]) / 3.0;
            let e = cls.entry(p.class.clone()).or_insert(ClassInfo { class: p.class.clone(), bp: p.bp.clone(), count: 0, cat, sub, rt, mesh, scale_min: f64::MAX, scale_max: f64::MIN, scale_sum: 0.0 });
            e.count += 1;
            e.scale_min = e.scale_min.min(s);
            e.scale_max = e.scale_max.max(s);
            e.scale_sum += s;
        }
    }
    let mut plans: Vec<KindPlan> = vec![];
    for (id, c) in &cls {
        let want = match (&o.classes, &o.only) {
            (Some(l), _) => l.iter().any(|x| x == id),
            (None, Some(f)) => id.to_lowercase().contains(f),
            _ => c.count >= o.min_count || matches!(category_of(id, &c.cat, &c.rt, &c.mesh), "tree" | "rock"),
        };
        if !want {
            continue;
        }
        let Some(Some(bp)) = g.bps.get(&c.bp) else { continue };
        let Some(mesh_obj) = bp.static_mesh() else { continue };
        plans.push(KindPlan { id: id.clone(), info: ClassInfoLite { bp: c.bp.clone(), count: c.count, cat: c.cat.clone(), sub: c.sub.clone(), rt: c.rt.clone(), mesh: c.mesh.clone(), scale: (c.scale_min, c.scale_max, c.scale_sum / c.count as f64) }, bp: bp.clone(), mesh_obj });
    }
    println!("{} harvestable classes in the map, exporting {}", cls.len(), plans.len());

    // ---- data tables ----
    let health_tbl = load_curve_table(src, "/Game/Balance/DataTables/AttributesBuildingProps")?;
    let rates = load_curve_table(src, "/Game/Balance/DataTables/ResourceRates")?;
    let athena_rates = load_curve_table(src, "/Game/Athena/Balance/DataTables/AthenaResourceRates")?;

    // ---- meshes ----
    let meshes = parallel(&plans, a.jobs, |p| load_mesh(src, &p.mesh_obj));

    // ---- materials ----
    let mut mat_paths: BTreeSet<String> = BTreeSet::new();
    let mut slot_paths: Vec<Vec<(Option<String>, &'static str)>> = vec![];
    for (p, m) in plans.iter().zip(&meshes) {
        let mut slots = vec![];
        if let Ok(md) = m {
            let ov: Vec<Option<String>> = p.bp.smc.get("OverrideMaterials").and_then(|v| v.as_array()).map(|a| a.iter().map(|x| x.as_str().filter(|s| !s.is_empty()).map(String::from)).collect()).unwrap_or_default();
            for (i, sm) in md.mesh.materials.iter().enumerate() {
                match ov.get(i).cloned().flatten() {
                    Some(o) => slots.push((Some(o), "component_override")),
                    None => slots.push((sm.material.clone(), "mesh")),
                }
            }
        }
        for (path, _) in &slots {
            if let Some(p) = path {
                mat_paths.insert(p.clone());
            }
        }
        slot_paths.push(slots);
    }
    let mat_list: Vec<String> = mat_paths.into_iter().collect();
    println!("{} distinct materials", mat_list.len());
    let resolved: HashMap<String, Result<Resolved, String>> = mat_list.iter().cloned().zip(parallel(&mat_list, a.jobs, |p| material::resolve(&src.set, p))).collect();

    // ---- textures ----
    let mut tex_paths: BTreeSet<String> = BTreeSet::new();
    for r in resolved.values().flatten() {
        for t in r.textures.values() {
            tex_paths.insert(t.clone());
        }
    }
    let tex_list: Vec<String> = tex_paths.into_iter().collect();
    let mut png_names: HashMap<String, String> = HashMap::new();
    let mut taken: HashMap<String, String> = HashMap::new();
    for t in &tex_list {
        let base = leaf(t).to_string();
        let mut name = base.clone();
        let mut n = 2;
        while let Some(prev) = taken.get(&name) {
            if prev == t {
                break;
            }
            name = format!("{base}_{n}");
            n += 1;
        }
        taken.insert(name.clone(), t.clone());
        png_names.insert(t.clone(), format!("{name}.png"));
    }
    println!("{} distinct textures", tex_list.len());
    let tex_dir = a.out.join("textures");
    let tex_res = parallel(&tex_list, a.jobs, |t| export_texture(src, t, &png_names[t], &tex_dir, o.max_tex));
    let mut tex_meta: BTreeMap<String, TexMeta> = BTreeMap::new();
    let mut tex_errors: Vec<Value> = vec![];
    for (t, r) in tex_list.iter().zip(tex_res) {
        match r {
            Ok(x) => {
                tex_meta.insert(t.clone(), x.meta);
            }
            Err(e) => tex_errors.push(json!({"texture": t, "error": e})),
        }
    }
    let tex_index: Map<String, Value> = tex_meta
        .iter()
        .map(|(p, m)| (m.png.clone(), json!({"ue_path": p, "srgb": m.srgb, "compression": m.compression, "format": m.format, "width": m.width, "height": m.height, "source_width": m.source_width, "source_height": m.source_height, "has_alpha": m.has_alpha, "grey": m.grey})))
        .collect();
    write_file(&tex_dir.join("index.json"), serde_json::to_string_pretty(&Value::Object(tex_index)).unwrap().as_bytes())?;

    // ---- per kind ----
    let mut results: Vec<KindResult> = vec![];
    for ((p, m), slots) in plans.iter().zip(meshes).zip(&slot_paths) {
        let r = write_kind(src, a, &o, p, m, slots, &resolved, &tex_meta, &health_tbl, &rates, &athena_rates);
        results.push(match r {
            Ok(props) => KindResult { id: p.id.clone(), ok: true, err: None, props },
            Err(e) => {
                eprintln!("FAIL {}: {e}", p.id);
                KindResult { id: p.id.clone(), ok: false, err: Some(e), props: Value::Null }
            }
        });
    }

    // ---- catalog ----
    let ok: Vec<&KindResult> = results.iter().filter(|r| r.ok).collect();
    let mut cat_kinds: Vec<Value> = ok
        .iter()
        .map(|r| {
            let p = &r.props;
            json!({"id": r.id, "dir": r.id, "display": p["display"], "category": p["category"], "resource": p["resource"]["type"], "health": p["health"]["max"], "placed_in_map": p["placement"]["athena_terrain_count"], "lod0_vertices": p["mesh"]["lods"][0]["vertices"]})
        })
        .collect();
    let order = |c: &str| match c {
        "tree" => 0,
        "rock" => 1,
        "bush" | "hedge" | "log" => 2,
        _ => 3,
    };
    cat_kinds.sort_by(|x, y| (order(x["category"].as_str().unwrap_or("")), std::cmp::Reverse(x["placed_in_map"].as_u64().unwrap_or(0))).cmp(&(order(y["category"].as_str().unwrap_or("")), std::cmp::Reverse(y["placed_in_map"].as_u64().unwrap_or(0)))));
    let catalog = json!({
        "schema": 1,
        "game": "Fortnite 1.8 (UE 4.16, CL 3724489)",
        "source_map": super::layout::MAP,
        "units": {"mesh": "metres, Y up (glTF)", "collision": "cm, mesh local UE space", "layout": "cm, UE world space"},
        "textures_dir": "textures",
        "kinds": cat_kinds,
    });
    write_file(&a.out.join("catalog.json"), serde_json::to_string_pretty(&catalog).unwrap().as_bytes())?;

    // ---- layout ----
    run_with(&g, a)?;
    let layout_stats: Value = std::fs::read_to_string(a.out.join("layout/stats.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);

    // ---- sheet ----
    let mut sheet_files: Vec<String> = vec![];
    if o.sheet {
        match super::sheet::run(&a.out) {
            Ok(f) => sheet_files = f,
            Err(e) => eprintln!("contact sheet: {e}"),
        }
    }

    // ---- report ----
    let report_path = o.report.clone().unwrap_or_else(|| PathBuf::from("reports/props-1.8.json"));
    let report = build_report(&g, &results, &tex_errors, &tex_meta, &mat_list, &resolved, &layout_stats, &sheet_files, &a.out, t0.elapsed().as_secs_f64());
    write_file(&report_path, serde_json::to_string_pretty(&report).unwrap().as_bytes())?;
    println!("exported {} / {} kinds, {} textures, {} materials; report {} ({:.1}s)", ok.len(), results.len(), tex_meta.len(), mat_list.len(), report_path.display(), t0.elapsed().as_secs_f64());
    if results.iter().any(|r| !r.ok) {
        return Err(format!("{} kinds failed", results.iter().filter(|r| !r.ok).count()));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_kind(
    src: &Source,
    a: &PropsArgs,
    _o: &Opts,
    p: &KindPlan,
    m: Result<MeshData, String>,
    slots: &[(Option<String>, &'static str)],
    resolved: &HashMap<String, Result<Resolved, String>>,
    tex_meta: &BTreeMap<String, TexMeta>,
    health_tbl: &BTreeMap<String, Vec<(f64, f64)>>,
    rates: &BTreeMap<String, Vec<(f64, f64)>>,
    athena_rates: &BTreeMap<String, Vec<(f64, f64)>>,
) -> Result<Value, String> {
    let md = m.map_err(|e| format!("mesh {}: {e}", p.mesh_obj))?;
    let mesh = &md.mesh;
    let kind_dir = a.out.join(&p.id);
    let mesh_name = mesh.name.clone();

    // materials -> .mat.json + glTF material descriptors
    let mut prop_mats: Vec<PropMaterial> = vec![];
    let mut mat_entries: Vec<Value> = vec![];
    let mut written: BTreeSet<String> = BTreeSet::new();
    for (i, (path, source)) in slots.iter().enumerate() {
        let sm = &mesh.materials[i];
        let Some(path) = path else {
            prop_mats.push(PropMaterial { name: format!("{}_missing", sm.slot_name), slot_name: sm.slot_name.clone(), base_color_factor: [0.6, 0.6, 0.6, 1.0], ..Default::default() });
            mat_entries.push(json!({"slot": i, "slot_name": sm.slot_name, "name": null, "mat_json": null, "source": "missing"}));
            continue;
        };
        let r = match resolved.get(path) {
            Some(Ok(r)) => r,
            Some(Err(e)) => return Err(format!("material {path}: {e}")),
            None => return Err(format!("material {path}: not resolved")),
        };
        let file = format!("{}.mat.json", r.name);
        let rel = format!("materials/{file}");
        if written.insert(rel.clone()) {
            let j = mat_json(r, tex_meta, "../../textures");
            write_file(&kind_dir.join(&rel), serde_json::to_string_pretty(&j).unwrap().as_bytes())?;
        }
        let slots_c = material::choose_slots(r, tex_meta);
        let uri = |param: &Option<String>| param.as_ref().and_then(|pn| r.textures.get(pn)).and_then(|t| tex_meta.get(t)).map(|m| format!("../textures/{}", m.png));
        let alpha_mode = if r.blend_mode.contains("Masked") { "MASK" } else if r.blend_mode.contains("Translucent") || r.blend_mode.contains("Additive") { "BLEND" } else { "OPAQUE" };
        prop_mats.push(PropMaterial {
            name: r.name.clone(),
            slot_name: sm.slot_name.clone(),
            base_color_uri: uri(&slots_c.base_color),
            normal_uri: uri(&slots_c.normal),
            mask_uri: uri(&slots_c.mask),
            alpha_mode: alpha_mode.into(),
            alpha_cutoff: r.clip as f32,
            double_sided: r.two_sided,
            base_color_factor: [1.0; 4],
            mat_json_uri: Some(rel.clone()),
        });
        mat_entries.push(json!({"slot": i, "slot_name": sm.slot_name, "name": r.name, "ue_path": path, "mat_json": rel, "source": source}));
    }

    fn_mesh::export_static_mesh_lods(mesh, &kind_dir, &mesh_name, &prop_mats).map_err(|e| e.to_string())?;

    // collision
    let col = collision_json(&src.set, &md.pkg, mesh.body_setup.as_deref())?;
    write_file(&kind_dir.join("collision.json"), serde_json::to_string_pretty(&col).unwrap().as_bytes())?;

    // verification against the uasset's own metadata
    let checks = mesh.validate();
    let (pmin, pmax) = mesh.position_bounds(0).ok_or("LOD0 has no positions")?;
    let ext = md.ext_bounds.as_ref();
    let get3 = |k: &str| -> Option<[f64; 3]> {
        let a = ext?.get(k)?.as_array()?;
        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
    };
    let mut bounds_err = f64::NAN;
    let mut bounds_vs_prop = f64::NAN;
    if let (Some(o), Some(e)) = (get3("Origin"), get3("BoxExtent")) {
        bounds_vs_prop = (0..3).map(|c| (o[c] - mesh.bounds.origin[c] as f64).abs().max((e[c] - mesh.bounds.extent[c] as f64).abs())).fold(0.0, f64::max);
        // decoded vertex box against the stored box (the stored box may be larger than the LOD0 hull; report the overshoot)
        bounds_err = (0..3).map(|c| (pmin[c] as f64 - (o[c] - e[c])).abs().max((pmax[c] as f64 - (o[c] + e[c])).abs())).fold(0.0, f64::max);
    }
    let roundtrip = fn_propdata::load_lods(&kind_dir.join(format!("{mesh_name}.gltf")));
    let (rt_ok, rt_note) = match &roundtrip {
        Ok(l) => {
            let same = l.len() == mesh.lods.len() && l.iter().zip(&mesh.lods).all(|(a, b)| a.positions.len() == b.positions.len() && a.triangle_count() == b.indices.len() / 3 && a.sections.len() == b.sections.len());
            (same, if same { "ok".to_string() } else { "LOD / vertex / triangle counts differ".to_string() })
        }
        Err(e) => (false, e.to_string()),
    };
    let lods_json: Vec<Value> = mesh
        .lods
        .iter()
        .enumerate()
        .map(|(i, l)| {
            json!({
                "vertices": l.positions.len(), "triangles": l.indices.len() / 3, "screen_size": mesh.screen_size.get(i).copied().unwrap_or(0.0),
                "uv_channels": l.uvs.len(), "vertex_colors": !l.colors.is_empty(), "index_buffer_32bit": l.indices_32bit, "full_precision_uvs": l.uv_full_precision, "high_precision_tangents": l.high_precision_tangents,
                "sections": l.sections.iter().map(|s| json!({"material_slot": s.material_index, "triangles": s.num_triangles, "first_index": s.first_index, "min_vertex": s.min_vertex_index, "max_vertex": s.max_vertex_index})).collect::<Vec<_>>(),
            })
        })
        .collect();
    let problems: Vec<String> = checks.iter().flat_map(|c| c.problems.clone()).collect();
    let verification = json!({
        "lods": checks.iter().map(|c| json!({"vertices": c.vertices, "section_triangles": c.section_triangles, "index_triangles": c.index_triangles, "section_vertex_ranges_exact": c.section_ranges_exact, "ok": c.problems.is_empty()})).collect::<Vec<_>>(),
        "problems": problems,
        "native_bounds_vs_ExtendedBounds_max_err_cm": if bounds_vs_prop.is_nan() { Value::Null } else { json!(rnd(bounds_vs_prop)) },
        "lod0_vertex_box_vs_ExtendedBounds_max_err_cm": if bounds_err.is_nan() { Value::Null } else { json!(rnd(bounds_err)) },
        "static_materials_property_count": md.static_materials_prop,
        "static_materials_native_count": mesh.materials.len(),
        "gltf_roundtrip_through_fn_propdata": rt_ok, "gltf_roundtrip_note": rt_note,
    });

    // health / resource
    let health_row = format!("{}.{}.FortHealthSet.MaxHealth", p.info.cat, p.info.sub);
    let health = health_tbl.get(&health_row).and_then(|k| k.first()).map(|k| k.1);
    let amount = p.bp.cdo.get("BuildingResourceAmountOverride");
    let amount_row = amount.and_then(|a| a.get("RowName")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let amount_table = amount.and_then(|a| a.get("CurveTable")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let pair = |t: &BTreeMap<String, Vec<(f64, f64)>>| -> (Value, Value) {
        match t.get(&amount_row) {
            Some(k) if k.len() >= 2 => (json!(k[0].1), json!(k[1].1)),
            Some(k) if k.len() == 1 => (json!(k[0].1), Value::Null),
            _ => (Value::Null, Value::Null),
        }
    };
    let (r0, r1) = pair(rates);
    let (ar0, ar1) = pair(athena_rates);
    let str_of = |k: &str| p.bp.cdo.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from);
    // other destruction / replacement hints present in the class
    let hints: Map<String, Value> = p.bp.cdo.iter().filter(|(k, _)| ["Replace", "Ruin", "Stump", "Destroy", "Broken", "Fractur", "Debris", "Death", "Shake", "Wobble", "Hit"].iter().any(|h| k.contains(h))).map(|(k, v)| (k.clone(), v.clone())).collect();
    let cull = p.bp.smc.get("CachedMaxDrawDistance").or_else(|| p.bp.smc.get("LDMaxDrawDistance")).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rscale = p.bp.cdo.get("RandomScaleRange").and_then(|v| v.as_array()).filter(|a| a.len() == 2).map(|a| json!([a[0], a[1]]));
    let mut wind = Map::new();
    for (k, v) in &p.bp.cdo {
        if k.contains("Wind") || k.contains("Leaf") || k.contains("Emit") {
            wind.insert(k.clone(), v.clone());
        }
    }
    let mut ue_props: Map<String, Value> = p.bp.cdo.clone();
    for k in ["UberGraphFrame", "RootComponent", "StaticMeshComponent"] {
        ue_props.remove(k);
    }
    let category = category_of(&p.id, &p.info.cat, &p.info.rt, &p.info.mesh);
    let (bmin, bmax) = mesh.bounds.box_min_max();
    let props = json!({
        "schema": 1,
        "id": p.id,
        "display": display_of(&p.id),
        "category": category,
        "blueprint": p.info.bp,
        "class_chain": p.bp.chain.iter().map(|l| l.package.clone()).collect::<Vec<_>>(),
        "mesh": {
            "name": mesh_name, "ue_path": p.mesh_obj, "gltf": format!("{mesh_name}.gltf"),
            "bounds_cm": {"origin": mesh.bounds.origin, "extent": mesh.bounds.extent, "radius": mesh.bounds.radius, "min": bmin, "max": bmax},
            "lod_for_collision": mesh.lod_for_collision, "light_map_coordinate_index": mesh.light_map_coordinate_index,
            "sockets": mesh.sockets, "lods": lods_json,
        },
        "materials": mat_entries,
        "collision": "collision.json",
        "health": {"max": health, "attribute_row": health_row, "found": health.is_some(), "attribute_key": {"category": p.info.cat, "sub_category": p.info.sub}},
        "resource": {
            "type": p.info.rt, "amount_row": amount_row, "amount_table": amount_table,
            "rate_t0": r0, "rate_t1": r1, "athena_rate_t0": ar0, "athena_rate_t1": ar1,
            "semantic": "values of the cooked ResourceRates / AthenaResourceRates curve row at key time 0 and 1; how the engine maps hits to resources is native FortBuildingActor code, not recoverable from the data",
        },
        "destruction": {
            "death_particles": str_of("DeathParticles"), "death_sound": str_of("DeathSound"), "hit_sound": str_of("PlayHitSound"), "loot_tier_group": str_of("DestructionLootTierGroup"),
            "replacement_class": Value::Null,
            "related_properties": hints,
            "note": "no stump / ruin / fracture replacement is referenced by the class: the actor is removed and DeathParticles plays",
        },
        "render": {
            "cull_distance_cm": cull, "cull_distance_m": cull * 0.01,
            "random_scale": p.bp.cdo.get("bRandomScaleOnPlacement").and_then(|v| v.as_bool()).unwrap_or(false),
            "random_scale_range": rscale, "random_yaw": p.bp.cdo.get("bRandomYawOnPlacement").and_then(|v| v.as_bool()).unwrap_or(false),
            "cast_shadow": p.bp.smc.get("CastShadow").and_then(|v| v.as_bool()).unwrap_or(true),
            "wind": wind,
        },
        "placement": {"athena_terrain_count": p.info.count, "scale_min": rnd(p.info.scale.0), "scale_max": rnd(p.info.scale.1), "scale_mean": rnd(p.info.scale.2)},
        "verification": verification,
        "ue_properties": {"class_default_object": ue_props, "static_mesh_component": p.bp.smc},
    });
    write_file(&kind_dir.join("props.json"), serde_json::to_string_pretty(&props).unwrap().as_bytes())?;
    Ok(props)
}

#[allow(clippy::too_many_arguments)]
fn build_report(
    g: &Gathered,
    results: &[KindResult],
    tex_errors: &[Value],
    tex_meta: &BTreeMap<String, TexMeta>,
    mats: &[String],
    resolved: &HashMap<String, Result<Resolved, String>>,
    layout: &Value,
    sheets: &[String],
    out: &Path,
    secs: f64,
) -> Value {
    let kinds: Vec<Value> = results
        .iter()
        .map(|r| {
            if !r.ok {
                return json!({"id": r.id, "ok": false, "error": r.err});
            }
            let p = &r.props;
            let lods = p["mesh"]["lods"].as_array().cloned().unwrap_or_default();
            let tex: BTreeSet<String> = p["materials"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m["ue_path"].as_str())
                .filter_map(|mp| resolved.get(mp).and_then(|r| r.as_ref().ok()))
                .flat_map(|r| r.textures.values().filter_map(|t| tex_meta.get(t).map(|m| m.name.clone())))
                .collect();
            json!({
                "id": r.id, "ok": true, "category": p["category"], "mesh": p["mesh"]["ue_path"],
                "lod_vertices": lods.iter().map(|l| l["vertices"].clone()).collect::<Vec<_>>(),
                "lod_triangles": lods.iter().map(|l| l["triangles"].clone()).collect::<Vec<_>>(),
                "materials": p["materials"].as_array().into_iter().flatten().map(|m| m["name"].clone()).collect::<Vec<_>>(),
                "textures": tex,
                "health": p["health"]["max"], "resource": p["resource"]["type"], "amount_row": p["resource"]["amount_row"],
                "rate_t0": p["resource"]["rate_t0"], "athena_rate_t0": p["resource"]["athena_rate_t0"],
                "collision_shapes": std::fs::read_to_string(out.join(&r.id).join("collision.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).map(|c| c["shapes"].as_array().map_or(0, |a| a.len())),
                "placed_in_map": p["placement"]["athena_terrain_count"],
                "verification_ok": p["verification"]["problems"].as_array().map_or(false, |a| a.is_empty()) && p["verification"]["gltf_roundtrip_through_fn_propdata"] == json!(true),
                "bounds_max_err_cm": p["verification"]["native_bounds_vs_ExtendedBounds_max_err_cm"],
            })
        })
        .collect();
    let mat_errors: Vec<Value> = mats.iter().filter_map(|m| resolved.get(m).and_then(|r| r.as_ref().err()).map(|e| json!({"material": m, "error": e}))).collect();
    json!({
        "tool": "fn-export props", "game": "Fortnite 1.8 (UE 4.16)", "seconds": secs,
        "totals": {"kinds_exported": results.iter().filter(|r| r.ok).count(), "kinds_failed": results.iter().filter(|r| !r.ok).count(), "textures": tex_meta.len(), "materials": mats.len(), "blueprint_actor_placements_in_map": g.all.len()},
        "kinds": kinds,
        "texture_errors": tex_errors, "material_errors": mat_errors,
        "contact_sheets": sheets,
        "layout": layout,
        "unrecovered": [
            "destruction visuals: DeathParticles / DeathSound / PlayHitSound are exported as object paths only (particle systems and sound cues are not decoded here); no stump / ruin / fracture mesh exists in the class data",
            "per-hit resource amounts: ResourceRates / AthenaResourceRates curve rows are exported verbatim; their evaluation is native FortBuildingActor code",
            "weak points / hit shake / health regeneration: no cooked data found on the prop classes (native code); the old runtime values stay provisional",
            "material graph arithmetic and the static switch values of material instances live in cooked native data; texture roles are exact only when an input is wired directly to a texture parameter, otherwise a name / compression-setting heuristic",
            "convex collision hulls are exported as vertex points only (PhysX cooked blob not decoded)",
            "FoliageInstancedStaticMeshComponent instances (S_WavyGrass_01 ground cover, ~115k) are counted but not decoded; they are non-colliding grass, not harvestable",
            "storm / wind animation: wind material parameter values are exported, the wind vertex-shader math (WorldPositionOffset graph) is not",
        ],
    })
}
