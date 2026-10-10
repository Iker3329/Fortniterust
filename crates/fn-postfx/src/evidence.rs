//! Evidence gathering for `reports/postfx-evidence.json`: which post effects are ACTIVE in a 1.8 gameplay frame, and with which
//! parameters, derived only from the install (paks) and the exe's string tables. Pure Rust (fn-pak + fn-uasset).
//!
//! Method (see the report's `method` field):
//! * `TODM_A_Child_Athena` stores only a DELTA against its parent Blueprint `TODM_A` (UE serializes the properties that differ from the
//!   archetype). The earlier extraction (`reports/day-postprocess.json`, 11 values) read only the child's delta; the parent holds ~130
//!   `bOverride_*` flags and the rest of the Day/Evening/Morning/Night `FPostProcessSettings`. This module merges parent + child.
//! * Within `FPostProcessSettings` only values that differ from the C++ constructor default are tagged. A field whose `bOverride_X` flag
//!   is true but which has no tagged value therefore equals the engine default (value ENGINE_DEFAULT_ASSUMED from the UE 4.16 source,
//!   which is not in the install); a tagged value whose `bOverride_X` is not true is NOT applied by the volume blend.
//! * The whole install is scanned for other sources of post-process settings and for baked console-variable strings.

use fn_pak::PakSet;
use fn_uasset::{Package, PackageData, Value};
use rayon::prelude::*;
use serde_json::{json, Map, Value as Json};
use std::collections::BTreeMap;

/// UE 4.16 `FPostProcessSettings::FPostProcessSettings()` defaults of the fields this crate cares about (recalled from the engine source;
/// the install carries no copy). Used ONLY for fields that have neither a tagged value nor a reason to differ.
pub const ENGINE_DEFAULTS_ASSUMED: &[(&str, f64)] = &[
    ("DepthOfFieldFocalRegion", 0.0),
    ("DepthOfFieldNearTransitionRegion", 300.0),
    ("DepthOfFieldFarTransitionRegion", 500.0),
    ("DepthOfFieldNearBlurSize", 15.0),
    ("DepthOfFieldFarBlurSize", 15.0),
    ("DepthOfFieldMaxBokehSize", 15.0),
    ("DepthOfFieldOcclusion", 0.4),
    ("DepthOfFieldSkyFocusDistance", 0.0),
    ("DepthOfFieldVignetteSize", 200.0),
    ("DepthOfFieldDepthBlurRadius", 0.0),
    ("DepthOfFieldDepthBlurAmount", 1.0),
    ("DepthOfFieldFstop", 4.0),
    ("DepthOfFieldScale", 0.0),
    ("MotionBlurAmount", 0.5),
    ("MotionBlurMax", 5.0),
    ("MotionBlurPerObjectSize", 0.5),
    ("BloomIntensity", 0.675),
    ("BloomThreshold", -1.0), // exe ctor value (reports/exe-defaults.json), previously recalled as 1.0
    ("BloomSizeScale", 4.0),
    ("Bloom1Size", 0.3),
    ("Bloom2Size", 1.0),
    ("Bloom3Size", 2.0),
    ("Bloom4Size", 10.0),
    ("Bloom5Size", 30.0),
    ("Bloom6Size", 64.0),
    ("Bloom1Tint", 0.3465),
    ("Bloom2Tint", 0.138),
    ("Bloom3Tint", 0.1176),
    ("Bloom4Tint", 0.066),
    ("Bloom5Tint", 0.066),
    ("Bloom6Tint", 0.061),
    ("BloomDirtMaskIntensity", 0.0), // exe ctor (was recalled as 1.0); no dirt texture is set, so no effect either way
    ("LensFlareIntensity", 1.0),
    ("LensFlareThreshold", 8.0),
    ("SceneFringeIntensity", 0.0),
    ("VignetteIntensity", 0.4),
    ("GrainIntensity", 0.0),
    ("AmbientOcclusionIntensity", 0.5),
    ("AmbientOcclusionRadius", 200.0), // exe ctor (was recalled as 40); the Day/Evening volumes override it with 160
    ("AmbientOcclusionPower", 2.0),
    ("AmbientOcclusionBias", 3.0),
    ("AmbientOcclusionFadeDistance", 8000.0),
    ("AmbientOcclusionFadeRadius", 5000.0),
    ("AmbientOcclusionMipBlend", 0.6),
    ("AmbientOcclusionMipScale", 1.7),
    ("AmbientOcclusionMipThreshold", 0.01),
    ("AmbientOcclusionStaticFraction", 1.0),
    ("AmbientOcclusionQuality", 50.0),
    ("AutoExposureMinBrightness", 0.03), // exe ctor-proven; every phase volume overrides it (Day 0.75, Evening 0.38, Morning 0.35, Night 0.30)
    ("AutoExposureMaxBrightness", 2.0),
    ("AutoExposureBias", 0.0),
    ("AutoExposureSpeedUp", 3.0),
    ("AutoExposureSpeedDown", 1.0),
    ("AutoExposureLowPercent", 80.0),
    ("AutoExposureHighPercent", 98.3),
    ("HistogramLogMin", -8.0),
    ("HistogramLogMax", 4.0),
];

/// Property names whose presence in a package name table marks it as a possible post-process settings carrier.
pub const SCAN_KEYWORDS: &[&str] = &[
    "DepthOfFieldMethod",
    "DepthOfFieldFocalDistance",
    "DepthOfFieldFarBlurSize",
    "MotionBlurAmount",
    "MotionBlurPerObjectSize",
    "bOverride_MotionBlurAmount",
    "bEnableVolumetricFog",
    "VolumetricFogDistance",
    "VolumetricFogExtinctionScale",
    "LensFlareIntensity",
    "SceneFringeIntensity",
    "BloomIntensity",
    "bEnableLightShaftBloom",
    "PostProcessVolume",
];

/// Console variables whose baked strings we look for in blueprints and in the exe.
pub const CVAR_NEEDLES: &[&str] = &[
    "r.MotionBlur", "r.DepthOfField", "r.VolumetricFog", "r.BloomQuality", "r.Bloom", "r.LensFlare", "r.SceneColorFringe",
    "r.AmbientOcclusion", "r.Tonemapper", "r.EyeAdaptation", "r.PostProcess", "r.DefaultFeature", "r.LightShaftQuality",
    "r.Fog", "r.Atmosphere", "sg.PostProcessQuality", "sg.EffectsQuality", "ShowFlag.MotionBlur", "ShowFlag.DepthOfField",
    "ShowFlag.VolumetricFog", "ShowFlag.Bloom", "ShowFlag.LensFlares",
];

fn contains_bytes(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() { return false; }
    let first = needle[0];
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        match hay[i..hay.len() - needle.len() + 1].iter().position(|&b| b == first) {
            None => return false,
            Some(p) => {
                i += p;
                if &hay[i..i + needle.len()] == needle { return true; }
                i += 1;
            }
        }
    }
    false
}

fn utf16(s: &str) -> Vec<u8> { s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect() }

/// Case-sensitive search for `needle` as ASCII and as UTF-16LE.
pub fn contains_text(hay: &[u8], needle: &str) -> bool { contains_bytes(hay, needle.as_bytes()) || contains_bytes(hay, &utf16(needle)) }

/// All settings carried in a `Settings` struct property list as `name -> json`.
fn struct_map(v: &Value, pkg: &Package) -> Map<String, Json> {
    match v.to_json(pkg) { Json::Object(m) => m, _ => Map::new() }
}

fn settings_of(set: &PakSet, long: &str, export: &str) -> Option<(Map<String, Json>, usize)> {
    let data = PackageData::load(set, long).ok()?;
    let pkg = data.package().ok()?;
    let ex = pkg.read_export_named(export)?.ok()?;
    let m = struct_map(ex.props.get("Settings")?, &pkg);
    let n = m.len();
    Some((m, n))
}

/// Merge parent + child tagged settings, resolve `bOverride_*` and classify every tagged / flagged field.
pub fn merge_phase(parent: &Map<String, Json>, child: &Map<String, Json>) -> Json {
    let mut merged: BTreeMap<String, (Json, &'static str)> = BTreeMap::new();
    for (k, v) in parent { merged.insert(k.clone(), (v.clone(), "TODM_A (parent)")); }
    for (k, v) in child { merged.insert(k.clone(), (v.clone(), "TODM_A_Child_Athena (child delta)")); }
    let flag = |k: &str| -> bool { merged.get(&format!("bOverride_{k}")).and_then(|(v, _)| v.as_bool()).unwrap_or(false) };
    let mut applied = Map::new();
    let mut ignored = Map::new();
    let mut flagged_default = Vec::new();
    for (k, (v, src)) in &merged {
        if k.starts_with("bOverride_") || k.starts_with('_') { continue; }
        let entry = json!({ "value": v, "source": src, "provenance": "RECOVERED" });
        if flag(k) { applied.insert(k.clone(), entry); } else { ignored.insert(k.clone(), json!({ "value": v, "source": src, "why": "no bOverride_ flag true (ignored by the volume blend)" })); }
    }
    for (k, (v, _)) in &merged {
        if let Some(name) = k.strip_prefix("bOverride_") {
            if v.as_bool() == Some(true) && !merged.contains_key(name) { flagged_default.push(name.to_string()); }
        }
    }
    let assumed: Map<String, Json> = flagged_default.iter().filter_map(|n| {
        ENGINE_DEFAULTS_ASSUMED.iter().find(|(k, _)| k == n).map(|(k, d)| (k.to_string(), json!({ "value": d, "provenance": "ENGINE_DEFAULT_ASSUMED (flag true, value not tagged => equals the C++ default)" })))
    }).collect();
    json!({
        "applied_tagged": applied,
        "overridden_untagged_equals_engine_default": flagged_default,
        "overridden_untagged_values_assumed": assumed,
        "tagged_but_not_overridden": ignored,
        "overridden_flag_count": merged.iter().filter(|(k, (v, _))| k.starts_with("bOverride_") && v.as_bool() == Some(true)).count(),
    })
}

/// Section 1: the Athena time-of-day Blueprint (parent + child) post-process components.
pub fn todm_section(set: &PakSet) -> Json {
    let mut phases = Map::new();
    for phase in ["Day", "Evening", "Morning", "Night"] {
        let comp = format!("{phase}PostProcessComponent");
        let parent = settings_of(set, "/Game/TimeOfDay/TODM/TODM_A", &comp);
        let child = settings_of(set, "/Game/TimeOfDay/TODM/TODM_A_Child_Athena", &comp);
        let (pm, cm) = (parent.map(|p| p.0).unwrap_or_default(), child.map(|c| c.0).unwrap_or_default());
        let mut j = merge_phase(&pm, &cm);
        j["parent_tagged_fields"] = json!(pm.len());
        j["child_tagged_fields"] = json!(cm.len());
        phases.insert(phase.into(), j);
    }
    // fog / light components that influence volumetric fog and light shafts
    let comp = |long: &str, export: &str| -> Json {
        PackageData::load(set, long).ok().and_then(|d| {
            let pkg = d.package().ok()?;
            let ex = pkg.read_export_named(export)?.ok()?;
            Some(ex.props.to_json(&pkg))
        }).unwrap_or(Json::Null)
    };
    // every Volumetric* value of the TOD class default objects (phase structs), with its path
    fn collect(v: &Json, needle: &str, path: String, out: &mut Vec<Json>) {
        match v {
            Json::Object(m) => for (k, x) in m {
                let p = format!("{path}/{k}");
                if k.contains(needle) && !x.is_object() && !x.is_array() { out.push(json!({ "path": p, "value": x })); }
                collect(x, needle, p, out);
            },
            Json::Array(a) => for (i, x) in a.iter().enumerate() { collect(x, needle, format!("{path}[{i}]"), out); },
            _ => {}
        }
    }
    let mut volumetric = Vec::new();
    for long in ["/Game/TimeOfDay/TODM/TODM_A", "/Game/TimeOfDay/TODM/TODM_A_Child_Athena"] {
        if let Some(j) = PackageData::load(set, long).ok().and_then(|d| { let pkg = d.package().ok()?; let cdo = pkg.read_default_object()?.ok()?; Some(cdo.props.to_json(&pkg)) }) {
            let mut found = Vec::new();
            collect(&j, "Volumetric", String::new(), &mut found);
            for f in found { volumetric.push(json!({ "asset": long, "path": f["path"], "value": f["value"] })); }
        }
    }
    json!({
        "assets": ["/Game/TimeOfDay/TODM/TODM_A", "/Game/TimeOfDay/TODM/TODM_A_Child_Athena"],
        "volumetric_values_in_class_default_objects": volumetric,
        "note": "phase components; the day phase is TimePhaseBegins 7 / PhaseLengthInHours 12 (reports/lighting-overrides.json); the engine blends phases by weight natively (FortTimeOfDayManager, not recovered)",
        "phases": phases,
        "fog_component": { "parent": comp("/Game/TimeOfDay/TODM/TODM_A", "ActiveHeightFogComponent"), "child": comp("/Game/TimeOfDay/TODM/TODM_A_Child_Athena", "ActiveHeightFogComponent") },
        "directional_light": { "parent": comp("/Game/TimeOfDay/TODM/TODM_A", "DayNightDirectionalLightComponent"), "child": comp("/Game/TimeOfDay/TODM/TODM_A_Child_Athena", "DayNightDirectionalLightComponent") },
        "sky_light": { "parent": comp("/Game/TimeOfDay/TODM/TODM_A", "SkyLightComponent"), "child": comp("/Game/TimeOfDay/TODM/TODM_A_Child_Athena", "SkyLightComponent") },
    })
}

/// Recursively collect `PostProcessSettings` structs of an export value (keys only + the interesting values).
fn collect_pp(v: &Value, pkg: &Package, out: &mut Vec<Map<String, Json>>) {
    match v {
        Value::Struct { name, props } => {
            if name == "PostProcessSettings" {
                let mut m = Map::new();
                for p in props.iter() { m.insert(p.name.clone(), p.value.to_json(pkg)); }
                out.push(m);
            } else {
                for p in props.iter() { collect_pp(&p.value, pkg, out); }
            }
        }
        Value::Array(a) | Value::Set(a) => a.iter().for_each(|x| collect_pp(x, pkg, out)),
        _ => {}
    }
}

const EFFECT_KEYS: &[&str] = &[
    "DepthOfFieldMethod", "DepthOfFieldFocalDistance", "DepthOfFieldFocalRegion", "DepthOfFieldNearTransitionRegion", "DepthOfFieldFarTransitionRegion",
    "DepthOfFieldNearBlurSize", "DepthOfFieldFarBlurSize", "DepthOfFieldScale", "MotionBlurAmount", "MotionBlurMax", "MotionBlurPerObjectSize",
    "LensFlareIntensity", "SceneFringeIntensity", "BloomIntensity", "BloomThreshold", "BloomSizeScale", "VignetteIntensity",
];

/// Section 2: install-wide scan for other post-process carriers.
pub fn scan_section(set: &PakSet) -> Json {
    let files: Vec<_> = set.effective().filter(|(_, e)| { let p = e.path.to_ascii_lowercase(); p.ends_with(".uasset") || p.ends_with(".umap") }).collect();
    let hits: Vec<(String, Vec<&'static str>, bool)> = files.par_iter().filter_map(|(pf, e)| {
        let bytes = pf.read(e).ok()?;
        let pkg = Package::parse(&bytes, &[]).ok()?;
        let mut kw: Vec<&'static str> = Vec::new();
        for k in SCAN_KEYWORDS { if pkg.names.iter().any(|n| n == k) { kw.push(k); } }
        if kw.is_empty() { None } else { Some((e.path.clone(), kw, false)) }
    }).collect();
    let mut packages = Vec::new();
    let mut by_kw: BTreeMap<&str, usize> = BTreeMap::new();
    for (path, kw, _) in &hits { for k in kw { *by_kw.entry(k).or_default() += 1; } let _ = path; }
    for (path, kw, _) in &hits {
        let long = fn_pak::pak_path_to_game(path).unwrap_or_else(|| path.clone());
        let Ok(data) = PackageData::load(set, &long) else { continue };
        let Ok(pkg) = data.package() else { continue };
        let mut exports = Vec::new();
        for i in 0..pkg.exports.len() {
            let cls = pkg.export_class_name(i);
            let Ok(obj) = pkg.read_export(i) else { continue };
            let mut pps = Vec::new();
            for p in obj.props.iter() { collect_pp(&p.value, &pkg, &mut pps); }
            let fog_volumetric = obj.props.get("bEnableVolumetricFog").and_then(|v| v.as_bool());
            if pps.is_empty() && fog_volumetric.is_none() && cls != "PostProcessVolume" { continue; }
            let settings: Vec<Json> = pps.iter().map(|m| {
                let mut s = Map::new();
                for k in EFFECT_KEYS { if let Some(v) = m.get(*k) { s.insert(k.to_string(), v.clone()); } }
                let flags: Vec<&String> = m.iter().filter(|(k, v)| k.starts_with("bOverride_") && v.as_bool() == Some(true)).map(|(k, _)| k).collect();
                json!({ "effect_keys": s, "bOverride_true_count": flags.len(), "tagged_fields": m.len() })
            }).collect();
            exports.push(json!({ "export": pkg.exports[i].name, "class": cls, "post_process_settings": settings, "bEnableVolumetricFog": fog_volumetric }));
        }
        packages.push(json!({ "path": path, "keywords": kw, "exports": exports }));
    }
    let post_process_volume_maps: Vec<String> = hits.iter().filter(|(p, kw, _)| p.to_ascii_lowercase().ends_with(".umap") && kw.contains(&"PostProcessVolume")).map(|(p, _, _)| p.clone()).collect();
    let camera_hits: Vec<&String> = hits.iter().filter(|(p, _, _)| { let l = p.to_ascii_lowercase(); l.contains("/blueprints/camera/") || l.contains("cameramode") }).map(|(p, _, _)| p).collect();
    json!({
        "camera_mode_packages_with_post_process_keywords": camera_hits,
        "camera_mode_note": "empty = no Athena_PlayerCameraMode* / Blueprints/Camera package names any DOF / motion blur / bloom / post-process-settings property: the pickaxe (melee) camera mode carries FOV and offsets only",
        "packages_scanned": files.len(),
        "keyword_hit_counts": by_kw,
        "maps_naming_PostProcessVolume": post_process_volume_maps,
        "packages_with_hits": packages,
    })
}

fn day_param(day: &Json, name: &str) -> Json {
    if let Some(a) = day["applied_tagged"].get(name) {
        return json!({ "name": name, "value": a["value"], "provenance": "RECOVERED", "source": a["source"] });
    }
    if let Some(a) = day["overridden_untagged_values_assumed"].get(name) {
        return json!({ "name": name, "value": a["value"], "provenance": "ENGINE_DEFAULT_ASSUMED", "source": "bOverride flag true, value not tagged => UE 4.16 C++ default (recalled)" });
    }
    if let Some(a) = day["tagged_but_not_overridden"].get(name) {
        return json!({ "name": name, "value": Json::Null, "provenance": "IGNORED", "tagged_value": a["value"], "source": a["source"], "why": "serialized but bOverride_ not true: the volume blend does not apply it" });
    }
    let d = ENGINE_DEFAULTS_ASSUMED.iter().find(|(k, _)| *k == name).map(|(_, v)| json!(v)).unwrap_or(Json::Null);
    json!({ "name": name, "value": d, "provenance": "ENGINE_DEFAULT_ASSUMED", "source": "not overridden by the volume: view default (UE 4.16 C++ constructor, recalled)" })
}

/// Section 4: the verdict table. Built from the merged Day settings and the other sections (shader names, exe strings).
pub fn effects_section(report: &Json) -> Json {
    let day = &report["todm"]["phases"]["Day"];
    let p = |n: &str| day_param(day, n);
    let v = |n: &str| day_param(day, n)["value"].as_f64().unwrap_or(f64::NAN);
    let shaders: Vec<String> = report["shader_cache"]["postfx_related"].as_array().map(|a| a.iter().filter_map(|s| s["name"].as_str().map(String::from)).collect()).unwrap_or_default();
    let named = |subs: &[&str]| -> Vec<String> { shaders.iter().filter(|s| subs.iter().any(|x| s.contains(x))).cloned().collect() };
    let exe = &report["cvars"]["exe_string_presence"];
    // numbers
    let (far_size, focal, region, far_tr) = (v("DepthOfFieldFarBlurSize"), v("DepthOfFieldFocalDistance"), v("DepthOfFieldFocalRegion"), v("DepthOfFieldFarTransitionRegion"));
    let sky = v("DepthOfFieldSkyFocusDistance");
    let (amount, maxp) = (v("MotionBlurAmount"), v("MotionBlurMax"));
    let blur_px = |w: f64| far_size * 0.01 * w;
    let mb_halflen = |w: f64, yaw_deg_per_frame: f64, hfov_deg: f64| {
        let ndc = yaw_deg_per_frame.to_radians().tan() / (hfov_deg.to_radians() * 0.5).tan();
        let disp_px = ndc * w * 0.5;
        (disp_px, (disp_px * amount * 0.5).min(maxp * 0.01 * w * 0.5))
    };
    let (d1920, h1920) = mb_halflen(1920.0, 1.5, 80.0);
    let (d1280, h1280) = mb_halflen(1280.0, 1.5, 80.0);
    let tints: Vec<f64> = (1..=6).map(|i| { let t = day_param(day, &format!("Bloom{i}Tint")); t["value"].as_array().and_then(|a| a[0].as_f64()).or(t["value"].as_f64()).unwrap_or(f64::NAN) }).collect();
    let tint_sum: f64 = tints.iter().sum();
    let tint_sum_old: f64 = [0.3465, 0.138, 0.1176, 0.066, 0.066, 0.061].iter().sum();
    let bloom_i = v("BloomIntensity");
    let corner = |i: f64| { let c = 2.0f64.sqrt() * i; 1.0 / (1.0 + c * c).powi(2) };
    let corner_day = corner(v("VignetteIntensity"));
    let corner_old = corner(0.4);
    let tonemap_names: Vec<String> = named(&["TonemapPS"]).into_iter().take(1).collect();
    let bloom_names: Vec<String> = named(&["BloomSetupPS", "DownsamplePS1", "TFilterPS", "TonemapPS"]).into_iter().filter(|s| !s.contains("ES2")).take(8).collect();
    let ao_names: Vec<String> = named(&["AmbientOcclusion"]).into_iter().take(6).collect();
    json!([
        {
            "effect": "depth_of_field",
            "active": "yes (nominal) - Gaussian method, FAR layer only; the visible effect is below one pixel",
            "decision": "DepthOfFieldMethod = DOFM_Gaussian (override flag true, RECOVERED parent). NearBlurSize = 0 (RECOVERED) switches the near layer off (UE runs a layer only when its size >= 0.01, recalled). FarBlurSize = 0.02 % of the view width is >= 0.01 so the far layer runs, but its blur radius is far_size/100 * width = 0.26 px at 1280 and 0.38 px at 1920 (the blur kernel collapses to one tap); what remains is the half-resolution setup/upsample softening of everything beyond (focal 150 m + 0.25 * 500 m) and of the sky (> 2000 m).",
            "parameters": [p("DepthOfFieldMethod"), p("DepthOfFieldFocalDistance"), p("DepthOfFieldFocalRegion"), p("DepthOfFieldNearTransitionRegion"), p("DepthOfFieldFarTransitionRegion"), p("DepthOfFieldNearBlurSize"), p("DepthOfFieldFarBlurSize"), p("DepthOfFieldSkyFocusDistance"), p("DepthOfFieldVignetteSize"), p("DepthOfFieldOcclusion"), p("DepthOfFieldDepthBlurRadius")],
            "derived": { "far_blur_radius_px_at_1280": blur_px(1280.0), "far_blur_radius_px_at_1920": blur_px(1920.0), "far_coc_starts_cm": focal + region, "far_coc_sharp_weight_zero_cm": focal + region + 0.25 * far_tr, "far_coc_full_cm": focal + region + far_tr, "sky_rule_distance_cm": sky },
            "gates": ["r.DepthOfFieldQuality > 0 (2 at the Epic post-process level: decrypted Engine/Config/BaseScalability.ini [PostProcessQuality@3], not overridden by DefaultScalability.ini)", "show flag DepthOfField", "DepthOfFieldVignetteSize / Occlusion / DepthBlur are tagged in TODM_A but NOT overridden (no bOverride flag), so the vignette variant of the setup shader and the depth blur are not used"],
            "shaders": named(&["DOFSetupPS10", "DOFRecombinePS100", "TFilterPS", "TFilterVS"]),
            "shader_variant_used": "FPostProcessDOFSetupPS10 (far only) -> TFilterVS/PS Gaussian H+V -> FPostProcessDOFRecombinePS100",
            "implemented": "fn-postfx: dof.wgsl (DXBC-verified shader maths), blur radius law ASSUMED"
        },
        {
            "effect": "motion_blur_camera",
            "active": "yes - camera-only velocity from depth + previous view-projection",
            "decision": format!("MotionBlurAmount {amount} and MotionBlurMax {maxp} % are RECOVERED (child override, parent 0.25/2.5 replaced). With the engine's tiled pipeline (FPostProcessVelocityFlattenCS -> VelocityGatherCS -> FPostProcessMotionBlurPS<4>) a 1.5 deg/frame turn at 80 deg FOV displaces {d1920:.1} px (1920) and blurs {h1920:.1} px per side ({d1280:.1} px / {h1280:.1} px at 1280). The effect is deliberately light."),
            "parameters": [p("MotionBlurAmount"), p("MotionBlurMax"), p("MotionBlurPerObjectSize")],
            "derived": { "turn_1p5deg_per_frame_displacement_px_1920": d1920, "halflength_px_1920": h1920, "turn_1p5deg_per_frame_displacement_px_1280": d1280, "halflength_px_1280": h1280, "max_halflength_px_1920": maxp * 0.01 * 1920.0 * 0.5 },
            "gates": ["r.MotionBlurQuality > 0 (UE default 4 => the PS<4> variant, 16 taps)", "the 'Motion Blur' option of the video settings (TabVideoOptions \"MotionBlur\"; default unrecoverable: DefaultGameUserSettings.ini is encrypted)", "show flag MotionBlur; not active in scene captures", "Athena_TerrainTOD2 (child of TODM_A, MotionBlurAmount = 0.0) is NOT referenced by Athena_Terrain: the map's WorldTimeOfDayManager is TODM_A_Child_Athena_C"],
            "shaders": named(&["VelocityFlatten", "VelocityGather", "MotionBlurPS"]),
            "implemented": "fn-postfx: mb.wgsl (flatten/gather/PS4; DXBC-verified for the pixel shader, CPU-port-verified for the compute shaders); host constants (velocity scale, max, edge) ASSUMED"
        },
        {
            "effect": "motion_blur_object",
            "active": "yes if the renderer writes a velocity buffer; PerObjectSize is not overridden (engine default 0.5, ASSUMED use)",
            "decision": "The tiled motion blur reads a velocity texture (written 1 where a dynamic object drew) and falls back to camera motion elsewhere; the character / pickaxe are dynamic. fn-render must render a velocity target (see crate README); without it only camera blur is produced.",
            "parameters": [p("MotionBlurPerObjectSize")],
            "shaders": named(&["VelocityFlatten"]),
            "implemented": "fn-postfx accepts an optional velocity view"
        },
        {
            "effect": "volumetric_fog",
            "active": "ON at Shadows High / Epic: Fortnite Scalability ShadowQuality@2 / @3 set r.VolumetricFog=1 (CookedIniVersion.txt L5139 / L5154; 0 at @0 / @1, L5113 / L5126), and fn-render fog.rs / volfog.rs implement it (the TODM ctor enables bEnableVolumetricFog). The OFF evidence below predates the ini record and is kept for history only",
            "decision": "Evidence for OFF: bEnableVolumetricFog is not serialized on the TODM_A / TODM_A_Child_Athena ActiveHeightFogComponent (the component takes its class default); no Blueprint script of either calls SetVolumetricFog (TODM_A only calls the native SetFogDirectionalInscatteringMultiplier and writes the MPC_Skybox_World parameters; DisableLightAndFog only does SetVisibility(false)). Evidence for ON / unknown: the Fortnite fork's component has extra properties (VolumetricFogScatteringScale / AbsorptionScale / bOverrideLightColorsWithFogInscatteringColors, stock 4.16 names are Extinction/Albedo), TODM_Disabled serializes bEnableVolumetricFog = FALSE together with zeroed VolumetricFog* values (delta serialization against TODM_A's template, which hints that the class default may be TRUE), while the older Lobby/Hexmap/PartyBackdrop fog components serialize TRUE (hints the default is FALSE); the TOD phase structs carry per-phase VolumetricFogScatteringScale/Distribution/AbsorptionScale (tuned for Athena: 0.1 / 0.05 / 0.5) and light VolumetricScatteringIntensity, and the exe contains those names (native FortTimeOfDay reads them). The class default of the fork cannot be read (UClass of a native class is not in the paks; locating the C++ constructor in the exe was attempted and not conclusive). BOUND: with the engine remap FogDensity/1000 (recalled) the Day extinction is 0.00022/1000 per cm; over the stock VolumetricFogDistance of 6000 cm the optical depth is 0.0013, so even an enabled volume adds ~0.1 % in-scatter at the Day values (times ScatteringScale 0.1). Not implemented: only worth building if a live capture or native-code disassembly shows SetVolumetricFog(true).",
            "parameters": [
                { "name": "bEnableVolumetricFog (TODM_A / child ActiveHeightFogComponent)", "value": "untagged (class default, unknown in the fork)", "provenance": "RECOVERED absence; default UNKNOWN" },
                { "name": "bEnableVolumetricFog (TODM_Disabled ActiveHeightFogComponent)", "value": false, "provenance": "RECOVERED (explicit)" },
                { "name": "bEnableVolumetricFog (Frontend_Lobby_Default, Hexmap_Temperate01, PartyBackdrop)", "value": true, "provenance": "RECOVERED (explicit)" },
                { "name": "Volumetric* values of the TOD phase structs", "value": report["todm"]["volumetric_values_in_class_default_objects"], "provenance": "RECOVERED (class default objects of TODM_A / TODM_A_Child_Athena)" },
                { "name": "optical depth over 6000 cm at Day density", "value": 0.00022 / 1000.0 * 6000.0, "provenance": "DERIVED (FogDensity 0.00022 from the Day phase, /1000 engine remap recalled, 6000 cm = stock VolumetricFogDistance, assumed)" }
            ],
            "exe_strings": exe,
            "shaders": named(&["Volumetric", "HeightFogAndVolumetricFog"]),
            "implemented": "no"
        },
        {            "effect": "lens_flare",
            "active": "no",
            "decision": "LensFlareIntensity is tagged 0.0 with its override flag true (RECOVERED parent, all four phases): UE skips the effect at intensity 0 (FPostProcessLensFlaresPS is never run).",
            "parameters": [p("LensFlareIntensity")],
            "shaders": named(&["LensFlare"]),
            "implemented": "no (inactive)"
        },
        {
            "effect": "scene_fringe",
            "active": "no",
            "decision": "SceneFringeIntensity has its override flag true but no tagged value, i.e. it equals the C++ default 0.0 (RECOVERED by the diff semantic: a non-default value would have been serialized). The tonemapper's fringe offsets are therefore zero.",
            "parameters": [p("SceneFringeIntensity")],
            "shaders": tonemap_names,
            "implemented": "no (inactive)"
        },
        {
            "effect": "bloom",
            "active": "yes",
            "decision": format!("Six-Gaussian bloom with the Fortnite tints: Bloom3Tint 0.5 (engine 0.1176), Bloom6Size 0 / Bloom6Tint 0 (sixth kernel off), BloomSizeScale 1.0 (engine 4.0) - all RECOVERED from TODM_A and independently present in the storm post-process of PlayerPawn_Athena. Intensity 0.675 and threshold -1.0 are the untouched exe ctor defaults (flag true, no tag); threshold <= -1 skips FPostProcessBloomSetupPS (PostProcessing.cpp RenderHalfResBloomThreshold, exe FUN_1418b5780). Tint sum {tint_sum:.4} vs engine {tint_sum_old:.4}. AddBloom multiplies every tint by TintScale = 1/6 (UE 4.16 source; exe DAT_143feb1d4 = 0x3e2aaaab), the tonemapper adds bloom * BloomIntensity BEFORE the exposure: total gain = tint sum / 6 * 0.675."),
            "parameters": [p("BloomIntensity"), p("BloomThreshold"), p("BloomSizeScale"), p("Bloom1Size"), p("Bloom2Size"), p("Bloom3Size"), p("Bloom4Size"), p("Bloom5Size"), p("Bloom6Size"), p("Bloom1Tint"), p("Bloom2Tint"), p("Bloom3Tint"), p("Bloom4Tint"), p("Bloom5Tint"), p("Bloom6Tint"), p("BloomDirtMaskIntensity")],
            "derived": { "tint_sum_day": tint_sum, "tint_sum_engine_default": tint_sum_old, "effective_total_gain_day": tint_sum / 6.0 * bloom_i, "fn_render_total_gain": bloom_i },
            "shaders": bloom_names,
            "differences_vs_fn_render": [
                "none left: fn-render's former 3x3 mip-tap approximation (per-tap threshold, un-scaled tints = gain 0.754, bloom added after exposure) was replaced by fn_postfx::bloom (2026-10-05)"
            ],
            "implemented": "fn-postfx: bloom.rs + bloom.wgsl + bloom_filter.wgsl, wired into fn-render (setup/downsample shader maths DXBC-verified; kernel exp(-16.7 (x/R)^2), radius Size * width / 200, fast blur, TintScale 1/6 from the UE 4.16 source + 1.8 exe decomp)"
        },
        {
            "effect": "ambient_occlusion",
            "active": "yes (screen-space AO; evidence only here, not ported)",
            "decision": "AO runs with the Day volume: intensity 0.56422 (child), radius 160 cm (parent; fn-render assumes 40), bias 2.4 (engine 3.0), fade distance 550000 / fade radius 200000 (engine 8000 / 5000: AO effectively never fades), mip threshold 0.009; power/mip blend/mip scale/quality at engine defaults. The 4.16 AO is the FPostProcessAmbientOcclusion{Setup,PS,CS} family (compute or pixel variants chosen by quality; setup halves the depth).",
            "parameters": [p("AmbientOcclusionIntensity"), p("AmbientOcclusionRadius"), p("AmbientOcclusionBias"), p("AmbientOcclusionPower"), p("AmbientOcclusionFadeDistance"), p("AmbientOcclusionFadeRadius"), p("AmbientOcclusionMipBlend"), p("AmbientOcclusionMipScale"), p("AmbientOcclusionMipThreshold"), p("AmbientOcclusionQuality"), p("AmbientOcclusionStaticFraction")],
            "shaders": ao_names,
            "implemented": "no: parameters only (differences for fn-render: radius 160 cm, bias 2.4, no distance fade). fn-render's own depth-only AO now runs right after the opaque scene (ssao_pass.rs), before fog / translucency / bloom / the histogram, as UE applies SSAO inside the lighting"
        },
        {
            "effect": "vignette",
            "active": "yes but negligible: VignetteIntensity 0.01 (RECOVERED parent) - the corner mask is 1/(1+(sqrt2*0.01)^2)^2 = 0.9996. fn-render's tonemap uses an ASSUMED 0.4 (corner 0.57): a visible error.",
            "parameters": [p("VignetteIntensity")],
            "derived": { "corner_mask_day": corner_day, "corner_mask_fn_render_0p4": corner_old },
            "implemented": "tonemapper (fn-render): change tonemap::VIGNETTE_INTENSITY to the recovered value"
        },
        {
            "effect": "auto_exposure_and_other_recovered_overrides_missing_in_fn_render",
            "active": "yes",
            "decision": "The merged Day volume also fixes the eye adaptation range: Min 0.75 (child) / Max 1.0 (parent; fn-render assumes 2.0) / Bias 0.47 (child); HistogramLogMin/Max are tagged but NOT overridden (ignored). ColorSaturationShadows/Midtones = 1.25 are overridden (RECOVERED) and absent from the fn-render grading; Film* come from the child.",
            "parameters": [p("AutoExposureMinBrightness"), p("AutoExposureMaxBrightness"), p("AutoExposureBias"), p("AutoExposureSpeedUp"), p("AutoExposureSpeedDown"), p("AutoExposureLowPercent"), p("HistogramLogMin"), p("HistogramLogMax"), p("ColorSaturationShadows"), p("ColorSaturationMidtones"), p("GrainIntensity"), p("AmbientCubemapIntensity")],
            "implemented": "n/a (tonemapper constants for the fn-render owner)"
        },
        {
            "effect": "light_shaft_bloom",
            "active": "yes (sun glow) at Epic only (r.LightShaftQuality 1 at PostProcessQuality@3): DayNightDirectionalLight bEnableLightShaftBloom = true, BloomScale 0.1 (RECOVERED parent); BloomThreshold 0 / BloomTint white / OcclusionDepthRange 100000 are exe ctor defaults",
            "decision": "The TOD sun light enables light-shaft bloom (TDownsampleLightShaftsPixelShader / FBlurLightShaftsPixelShader in the cache). Occlusion shafts are off (bEnableLightShaftOcclusion not serialized, exe ctor false). The serialized LightShaftOverrideDirection is replaced every update by AFortTimeOfDayManager::PositionTheSunAndMoonModels (exe 0x140641e30: SetLightShaftOverrideDirection(-SunlightDirectionalVector)), so the blur origin is the sun MODEL direction. r.LightShaftRenderToSeparateTranslucency = 1 (cooked RendererSettings).",
            "parameters": [ { "name": "bEnableLightShaftBloom", "value": true, "provenance": "RECOVERED" }, { "name": "BloomScale", "value": 0.1, "provenance": "RECOVERED" },
                { "name": "BloomThreshold", "value": 0.0, "provenance": "EXE_CTOR" }, { "name": "BloomTint", "value": [255, 255, 255, 255], "provenance": "EXE_CTOR" },
                { "name": "OcclusionDepthRange", "value": 100000.0, "provenance": "EXE_CTOR" }, { "name": "bEnableLightShaftOcclusion", "value": false, "provenance": "EXE_CTOR" } ],
            "shaders": named(&["LightShaft"]),
            "implemented": "fn-render: light_shafts.rs + shaders/light_shafts.wgsl (UE 4.16 LightShaftRendering.cpp / LightShaftShader.usf: downsample, 3 radial blur passes of 12 samples, additive apply before SceneColorHalfRes). Light-shaft TAA not reproduced (the port has no TAA)."
        }
    ])
}

/// Which packages reference which time-of-day Blueprint (names, imports, soft package references). Decides which TOD is live in Athena.
pub fn tod_reference_section(set: &PakSet) -> Json {
    let needles = ["TODM_A_Child_Athena", "Athena_TerrainTOD2", "Athena_TerrainTOD", "TODM_A"];
    let files: Vec<_> = set.effective().filter(|(_, e)| { let p = e.path.to_ascii_lowercase(); (p.ends_with(".uasset") || p.ends_with(".umap")) && (p.contains("athena") || p.contains("timeofday") || p.contains("/maps/")) }).collect();
    let hits: Vec<Json> = files.par_iter().filter_map(|(pf, e)| {
        let bytes = pf.read(e).ok()?;
        let pkg = Package::parse(&bytes, &[]).ok()?;
        let mut found = Vec::new();
        for n in needles {
            let in_names = pkg.names.iter().any(|x| x == n);
            let in_soft = pkg.soft_package_references.iter().any(|x| x.contains(n));
            let in_imports = pkg.imports.iter().any(|i| i.name == n || i.name.starts_with(&format!("{n}_C")));
            if in_names || in_soft || in_imports { found.push(json!({ "needle": n, "names": in_names, "soft": in_soft, "imports": in_imports })); }
        }
        if found.is_empty() { None } else { Some(json!({ "path": e.path, "found": found })) }
    }).collect();
    // Athena_Terrain's WorldSettings.WorldTimeOfDayManager is a soft class path string inside the map's uexp
    let mut world_tod: Vec<String> = Vec::new();
    for ext in ["umap", "uexp"] {
        if let Ok(bytes) = set.read(&format!("FortniteGame/Content/Athena/Maps/Athena_Terrain.{ext}")) {
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'/' && bytes[i..].starts_with(b"/Game/TimeOfDay/") || bytes[i..].starts_with(b"/Game/Athena/Prototype/Terrain/Athena_TerrainTOD") {
                    let j = bytes[i..].iter().position(|&b| !(b.is_ascii_graphic())).unwrap_or(0) + i;
                    world_tod.push(String::from_utf8_lossy(&bytes[i..j]).into_owned());
                    i = j;
                } else { i += 1; }
            }
        }
    }
    world_tod.sort();
    world_tod.dedup();
    json!({
        "athena_packages_checked": files.len(),
        "athena_terrain_world_time_of_day_strings": world_tod,
        "note": "TODM_A_Child_Athena_C is the TimeOfDay manager class of Athena_Terrain; Athena_TerrainTOD2 (child of TODM_A, MotionBlurAmount = 0) is not referenced by the map.",
        "references": hits,
    })
}

/// Section 3: baked console-variable strings (blueprint constants) and exe presence.
pub fn cvar_section(set: &PakSet, exe: Option<&[u8]>) -> Json {
    let files: Vec<_> = set.effective().filter(|(_, e)| { let p = e.path.to_ascii_lowercase(); (p.ends_with(".uexp") && e.size < 8_000_000) || p.ends_with(".umap") || p.ends_with(".uasset") && e.size < 2_000_000 }).collect();
    let hits: Vec<(String, Vec<&'static str>)> = files.par_iter().filter_map(|(pf, e)| {
        let bytes = pf.read(e).ok()?;
        let found: Vec<&'static str> = CVAR_NEEDLES.iter().copied().filter(|n| contains_text(&bytes, n)).collect();
        if found.is_empty() { None } else { Some((e.path.clone(), found)) }
    }).collect();
    // control: the scanner must find unrelated cvars, otherwise "none of the post-process cvars are baked" would be meaningless
    let control_needles = ["r.ScreenPercentage", "r.Shadow", "r.SetRes", "t.MaxFPS", "sg.", "r.MaxAnisotropy", "r.Streaming"];
    let control_hits: usize = files.par_iter().filter(|(pf, e)| pf.read(e).map(|b| control_needles.iter().any(|n| contains_text(&b, n))).unwrap_or(false)).count();
    let inis: Vec<_> = set.glob("**/*.ini");
    let enc = inis.iter().filter(|(_, e)| e.encrypted).count();
    let exe_hits: Map<String, Json> = match exe {
        Some(b) => ["r.VolumetricFog", "r.MotionBlurQuality", "r.DepthOfFieldQuality", "r.BloomQuality", "r.LensFlareQuality", "r.SceneColorFringeQuality", "r.MotionBlur.Max", "r.DepthOfField.MaxSize",
            "bEnableVolumetricFog", "VolumetricFogScatteringScale", "VolumetricFogAbsorptionScale", "VolumetricFogScatteringDistribution", "SetVolumetricFog", "VolumetricFogDistance"]
            .iter().map(|n| (n.to_string(), json!(contains_text(b, n)))).collect(),
        None => Map::new(),
    };
    json!({
        "packages_with_cvar_strings": hits.iter().map(|(p, f)| json!({ "path": p, "cvars": f })).collect::<Vec<_>>(),
        "control_unrelated_cvar_string_hits": control_hits,
        "packages_scanned_for_cvars": files.len(),
        "config_ini_files": inis.len(),
        "config_ini_encrypted": enc,
        "note": "Engine/Game *.ini (DefaultScalability, DefaultEngine) are encrypted in the paks when config_ini_encrypted == config_ini_files, so r.* defaults cannot be read from data; UE 4.16 code defaults are assumed.",
        "exe_string_presence": exe_hits,
    })
}
