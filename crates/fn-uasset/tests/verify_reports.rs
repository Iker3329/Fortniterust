//! Verification against the Node prototype's evidence reports (`FortniteRuntime/reports/*.json`).
//!
//! Every test re-reads the *same packages from the 1.8 paks* with the Rust parser and asserts that the values the
//! Node tools extracted (with byte-scanning heuristics) are reproduced exactly (JSON subset comparison: floats are
//! compared as exact f64 values, object references by object name, extra keys on our side are allowed).
//!
//! Tests are skipped (with a message) when the install or the reports are not present on this machine.
//! Override locations with `FN_ROOT` / `FN_REPORTS`.

use fn_pak::PakSet;
use fn_uasset::{Package, PackageData, Value};
use serde_json::Value as J;
use std::path::PathBuf;
use std::sync::OnceLock;

fn fn_root() -> PathBuf {
    PathBuf::from(std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into()))
}
fn reports() -> PathBuf {
    PathBuf::from(
        std::env::var("FN_REPORTS")
            .unwrap_or_else(|_| r"C:\Users\Brady\Documents\Codex\2026-10-02\im-seeing-titkoks-where-people-are-2\FortniteRuntime\reports".into()),
    )
}

fn set() -> Option<&'static PakSet> {
    static SET: OnceLock<Option<PakSet>> = OnceLock::new();
    SET.get_or_init(|| {
        if !fn_root().exists() {
            return None;
        }
        PakSet::open_install(fn_root()).ok()
    })
    .as_ref()
}

fn report(name: &str) -> Option<J> {
    let p = reports().join(name);
    let b = std::fs::read(&p).ok()?;
    let s = String::from_utf8_lossy(&b);
    serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()
}

/// (install, report) or skip.
macro_rules! need {
    ($rep:expr) => {{
        let Some(s) = set() else {
            eprintln!("SKIP: install not present");
            return;
        };
        let Some(r) = report($rep) else {
            eprintln!("SKIP: report {} not present", $rep);
            return;
        };
        (s, r)
    }};
}

fn load<'a>(s: &'a PakSet, path: &str) -> PackageData<'a> {
    PackageData::load(s, path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn export_json(pkg: &Package, name: &str) -> J {
    let o = pkg.read_export_named(name).unwrap_or_else(|| panic!("no export {name} in {}", pkg.name)).unwrap();
    assert!(o.error.is_none(), "{name}: {:?}", o.error);
    o.props.to_json(pkg)
}

fn last_component(s: &str) -> &str {
    s.rsplit(['.', ':', '/']).next().unwrap_or(s)
}

/// Subset comparison of a Node-tool JSON value against ours.
fn eq_node(node: &J, mine: &J, path: &str) -> Result<(), String> {
    match (node, mine) {
        (J::Null, J::Null) => Ok(()),
        (J::Bool(a), J::Bool(b)) if a == b => Ok(()),
        (J::Number(a), J::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            if a == b || (a.is_nan() && b.is_nan()) {
                Ok(())
            } else {
                Err(format!("{path}: node {a} != rust {b}"))
            }
        }
        (J::String(a), J::String(b)) => {
            if let Some(n) = a.strip_prefix("import:").or_else(|| a.strip_prefix("export:")) {
                if last_component(b) == n {
                    return Ok(());
                }
                return Err(format!("{path}: node object {a} != rust {b}"));
            }
            // some Node reports strip object references to bare names
            if a == b || (b.starts_with('/') && last_component(b) == a) {
                Ok(())
            } else {
                Err(format!("{path}: node {a:?} != rust {b:?}"))
            }
        }
        (J::Array(a), J::Array(b)) => {
            if a.len() != b.len() {
                return Err(format!("{path}: node array len {} != rust {}", a.len(), b.len()));
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                eq_node(x, y, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (J::Object(a), _) => {
            // Node array-of-struct wrapper {struct, items:[..]}
            if a.contains_key("struct") && a.contains_key("items") && a.len() == 2 {
                return eq_node(&a["items"], mine, path);
            }
            // Node soft object wrapper {type,size,rawHex,path}
            if a.contains_key("path") && a.contains_key("type") {
                return eq_node(&a["path"], mine, path);
            }
            let J::Object(b) = mine else {
                return Err(format!("{path}: node object vs rust {mine}"));
            };
            for (k, v) in a {
                if k == "struct" {
                    continue;
                }
                let Some(m) = b.get(k) else {
                    return Err(format!("{path}.{k}: missing in rust"));
                };
                eq_node(v, m, &format!("{path}.{k}"))?;
            }
            Ok(())
        }
        _ => Err(format!("{path}: node {node} vs rust {mine}")),
    }
}

fn check(node: &J, mine: &J, what: &str, errs: &mut Vec<String>) {
    if let Err(e) = eq_node(node, mine, what) {
        errs.push(e);
    }
}

fn assert_clean(errs: Vec<String>, checked: usize) {
    if !errs.is_empty() {
        for e in errs.iter().take(40) {
            eprintln!("MISMATCH {e}");
        }
        panic!("{} of {checked} comparisons mismatched", errs.len());
    }
    eprintln!("ok: {checked} comparisons identical");
}

// ------------------------------------------------------------------------------------------------

#[test]
fn day_postprocess() {
    let (s, rep) = need!("day-postprocess.json");
    let d = load(s, "/Game/TimeOfDay/TODM/TODM_A_Child_Athena");
    let pkg = d.package().unwrap();
    let comp = export_json(&pkg, rep["owner"].as_str().unwrap());
    let settings = &comp["Settings"];
    let mut errs = Vec::new();
    let params = rep["parameters"].as_object().unwrap();
    for (k, v) in params {
        check(v, &settings[k], &format!("DayPostProcessComponent.Settings.{k}"), &mut errs);
    }
    assert_clean(errs, params.len());
}

fn find_prop_offset(pkg: &Package, export: &str, prop: &str) -> Option<u32> {
    let o = pkg.read_export_named(export)?.ok()?;
    o.props.iter().find(|p| p.name == prop).map(|p| p.offset - pkg.asset_bytes().len() as u32)
}

#[test]
fn sun_shadow_lighting() {
    let (s, rep) = need!("sun-shadow-lighting.json");
    let da = load(s, "/Game/TimeOfDay/TODM/TODM_A");
    let a = da.package().unwrap();
    let dc = load(s, "/Game/TimeOfDay/TODM/TODM_A_Child_Athena");
    let c = dc.package().unwrap();
    let light = export_json(&a, "DayNightDirectionalLightComponent");
    let sky = export_json(&a, "SkyLightComponent");
    let light_c = export_json(&c, "DayNightDirectionalLightComponent");
    let sky_c = export_json(&c, "SkyLightComponent");
    let cdo = export_json(&a, "Default__TODM_A_C");
    let dl = &rep["directionalLight"];
    let mut errs = Vec::new();
    let mut n = 0;
    let mut chk = |key: &str, mine: &J| {
        n += 1;
        check(&dl[key], mine, &format!("directionalLight.{key}"), &mut errs);
    };
    chk("RelativeRotation_PitchYawRoll_deg", &light["RelativeRotation"]);
    chk("SunriseDirectionalLightRotation_deg", &cdo["SunriseDirectionalLightRotation"]);
    chk("SunsetDirectionalLightRotation_deg", &cdo["SunsetDirectionalLightRotation"]);
    chk("DirectionalLightRotation_deg", &cdo["DirectionalLightRotation"]);
    chk("StartTimeOfDayInGame", &cdo["StartTimeOfDayInGame"]);
    chk("DefaultTimeOfDaySpeed", &cdo["DefaultTimeOfDaySpeed"]);
    chk("DynamicShadowDistanceMovableLight_cm", &light["DynamicShadowDistanceMovableLight"]);
    chk("CascadeDistributionExponent", &light["CascadeDistributionExponent"]);
    chk("ShadowSharpen", &light["ShadowSharpen"]);
    chk("Intensity_parent", &light["Intensity"]);
    chk("DisabledBrightness_parent", &light["DisabledBrightness"]);
    chk("DisabledBrightness_athena", &light_c["DisabledBrightness"]);
    chk("bUsedAsAtmosphereSunLight", &light["bUsedAsAtmosphereSunLight"]);
    chk("bUseRayTracedDistanceFieldShadows", &light["bUseRayTracedDistanceFieldShadows"]);
    chk("LightShaftOverrideDirection", &light["LightShaftOverrideDirection"]);
    for (k, v) in rep["skyLight"]["parent"].as_object().unwrap() {
        n += 1;
        check(v, &sky[k], &format!("skyLight.parent.{k}"), &mut errs);
    }
    for (k, v) in rep["skyLight"]["athena"].as_object().unwrap() {
        n += 1;
        check(v, &sky_c[k], &format!("skyLight.athena.{k}"), &mut errs);
    }

    // The Node tool byte-scanned the Athena CDO for phase tags; the structural reader must meet the same ones in the
    // same order (floats, SkyLightColor linear colors, LightColor 8-bit colors).
    let cdo_obj = c.read_default_object().unwrap().unwrap();
    let mut found: Vec<(String, J)> = Vec::new();
    fn walk(v: &Value, name: &str, out: &mut Vec<(String, J)>) {
        match v {
            Value::Float(f) if matches!(name, "TimePhaseBegins" | "PhaseLengthInHours" | "Brightness") => out.push((name.into(), serde_json::json!(*f as f64))),
            Value::LinearColor(c) if name == "SkyLightColor" => out.push((name.into(), serde_json::json!(c.iter().map(|x| *x as f64).collect::<Vec<_>>()))),
            Value::Color(c) if name == "LightColor" => out.push((name.into(), serde_json::json!({"r": c[0], "g": c[1], "b": c[2]}))),
            Value::Struct { props, .. } => props.iter().for_each(|p| walk(&p.value, &p.name, out)),
            Value::Array(a) => a.iter().for_each(|x| walk(x, name, out)),
            _ => {}
        }
    }
    for p in cdo_obj.props.iter() {
        walk(&p.value, &p.name, &mut found);
    }
    let scan = rep["athenaPhaseScan"].as_array().unwrap();
    assert_eq!(scan.len(), found.len(), "athenaPhaseScan entry count");
    for (i, (e, (name, val))) in scan.iter().zip(&found).enumerate() {
        n += 1;
        assert_eq!(e["name"].as_str().unwrap(), name, "phase scan #{i} name");
        check(&e["value"], val, &format!("athenaPhaseScan[{i}].{name}"), &mut errs);
    }

    // property-tag byte offsets inside the .uexp
    let off = &rep["uexpOffsets"]["TODM_A"];
    for (k, export_prop) in [("RelativeRotation", "RelativeRotation"), ("DynamicShadowDistanceMovableLight", "DynamicShadowDistanceMovableLight"), ("CascadeDistributionExponent", "CascadeDistributionExponent")] {
        n += 1;
        let mine = find_prop_offset(&a, "DayNightDirectionalLightComponent", export_prop).expect("prop");
        let theirs = off[k].as_u64().unwrap();
        if mine as u64 != theirs {
            errs.push(format!("uexpOffsets.TODM_A.{k}: node {theirs} != rust {mine}"));
        }
    }
    assert_clean(errs, n);
}

// ---- camera ---------------------------------------------------------------------------------

fn camera_flat_samples(v: &J) -> J {
    // Node: SamplePoints.items -> {pitchAngle, viewOffsetCm, largeBodyExtraCm}
    match v.get("SamplePoints") {
        Some(sp) => J::Array(
            sp.as_array()
                .unwrap()
                .iter()
                .map(|s| serde_json::json!({"pitchAngle": s["PitchAngle"], "viewOffsetCm": s["ViewOffset"], "largeBodyExtraCm": s["LargeBodyTypeAddtnlOffset"]}))
                .collect(),
        ),
        None => v.clone(),
    }
}

/// Merge parent CDO properties under child (Blueprint inheritance: child overrides parent).
fn resolve_camera(s: &PakSet, name: &str) -> serde_json::Map<String, J> {
    let path = format!("/Game/Blueprints/Camera/Athena/{name}");
    let d = load(s, &path);
    let pkg = d.package().unwrap();
    let cls = pkg.exports.iter().position(|e| e.name.ends_with("_C") && !e.name.starts_with("Default__")).unwrap();
    let sup = pkg.exports[cls].super_index;
    let mut merged = if sup.is_import() {
        let pn = pkg.index_name(sup).unwrap();
        let pn = pn.trim_end_matches("_C").to_string();
        // parent is another Athena camera blueprint if it exists in the same directory, else a native class
        if s.contains(&format!("FortniteGame/Content/Blueprints/Camera/Athena/{pn}.uasset")) {
            resolve_camera(s, &pn)
        } else {
            serde_json::Map::new()
        }
    } else {
        serde_json::Map::new()
    };
    let cdo = pkg.read_default_object().unwrap().unwrap();
    if let J::Object(o) = cdo.props.to_json(&pkg) {
        for (k, v) in o {
            merged.insert(k, v);
        }
    }
    merged
}

#[test]
fn camera_modes() {
    let (s, rep) = need!("camera-modes.json");
    let mut errs = Vec::new();
    let mut n = 0;

    // pickaxe link
    let imp = load(s, "/Game/Weapons/FORT_Melee/Blueprints/Impact/B_Melee_Impact_Pickaxe_Athena");
    let imp = imp.package().unwrap();
    let imp_cdo = imp.read_default_object().unwrap().unwrap().props.to_json(&imp);
    let link = &rep["pickaxeCameraLink"];
    for k in ["CameraBase3PClass", "CameraBase1PClass"] {
        n += 1;
        check(&link[k], &imp_cdo[k], &format!("pickaxeCameraLink.{k}"), &mut errs);
    }
    let wid = load(s, "/Game/Athena/Items/Weapons/WID_Harvest_Pickaxe_Athena_C_T01");
    let wid = wid.package().unwrap();
    let wid_props = export_json(&wid, "WID_Harvest_Pickaxe_Athena_C_T01");
    n += 1;
    check(&link["widWeaponActorClass"], &wid_props["WeaponActorClass"], "pickaxeCameraLink.widWeaponActorClass", &mut errs);

    // camera manager
    let mgr = load(s, "/Game/Blueprints/Camera/Original/MainPlayerCamera");
    let mgr = mgr.package().unwrap();
    let mp = export_json(&mgr, "Default__MainPlayerCamera_C");
    let cm = &rep["cameraManager"];
    for k in ["Base3PClass", "Targeting3PClass", "CinematicCameraClass", "DBNOCameraClass", "DeathCameraClass", "SkydiveGlideCameraClass", "SkydiveDiveCameraClass", "SkydiveParachuteCameraClass"] {
        n += 1;
        check(&cm[k], &mp[k], &format!("cameraManager.{k}"), &mut errs);
    }
    let protos = cm["prototypes"].as_array().unwrap();
    let mine_protos = mp["CameraPrototypes"].as_array().unwrap();
    assert_eq!(protos.len(), mine_protos.len(), "prototype count");
    for (i, (p, m)) in protos.iter().zip(mine_protos).enumerate() {
        n += 1;
        check(&p["name"], &m["PrototypeName"], &format!("prototypes[{i}].name"), &mut errs);
        let ov = p["overrides"].as_array().unwrap();
        let mo = m["ModeOverrides"].as_array().unwrap();
        assert_eq!(ov.len(), mo.len(), "overrides count of prototype {i}");
        for (j, (o, mm)) in ov.iter().zip(mo).enumerate() {
            n += 1;
            let want = o.as_str().unwrap().replace("import:", "");
            let (a, b) = want.split_once(" -> ").unwrap();
            let got_a = last_component(mm["OriginalClass"].as_str().unwrap());
            let got_b = last_component(mm["ClassOverride"].as_str().unwrap());
            if (a, b) != (got_a, got_b) {
                errs.push(format!("prototypes[{i}].overrides[{j}]: node {want} vs rust {got_a} -> {got_b}"));
            }
        }
    }

    // weapon camera selection
    let wsel = rep["weaponCameraSelection"].as_object().unwrap();
    for (bp, want) in wsel {
        if want.get("error").is_some() {
            continue;
        }
        let hits = s.glob(&format!("FortniteGame/Content/Weapons/**/{bp}.uasset"));
        assert!(!hits.is_empty(), "weapon blueprint {bp} not found");
        let d = load(s, hits[0].1.rel_path());
        let pkg = d.package().unwrap();
        let mut mine = None;
        for i in 0..pkg.exports.len() {
            if pkg.exports[i].name.starts_with("Default__") {
                let o = pkg.read_export(i).unwrap();
                if o.props.get("CameraBase3PClass").is_some() || o.props.get("CameraTargeting3PClass").is_some() || o.props.get("CameraBase1PClass").is_some() {
                    mine = Some(o.props.to_json(&pkg));
                    break;
                }
            }
        }
        let mine = mine.unwrap_or_else(|| panic!("no camera CDO in {bp}"));
        for (k, v) in want.as_object().unwrap() {
            n += 1;
            check(v, &mine[k], &format!("weaponCameraSelection.{bp}.{k}"), &mut errs);
        }
    }

    // pawn
    let pawn = load(s, "/Game/Athena/PlayerPawn_Athena_Generic");
    let pawn = pawn.package().unwrap();
    let cap = export_json(&pawn, "CollisionCylinder");
    let mesh = export_json(&pawn, "CharacterMesh0");
    let mv = export_json(&pawn, "CharMoveComp");
    let pw = &rep["pawn"];
    n += 3;
    check(&pw["capsuleHalfHeightCm"], &cap["CapsuleHalfHeight"], "pawn.capsuleHalfHeightCm", &mut errs);
    check(&pw["meshRelativeLocationCm"], &mesh["RelativeLocation"], "pawn.meshRelativeLocationCm", &mut errs);
    check(&pw["jumpZVelocity"], &mv["JumpZVelocity"], "pawn.jumpZVelocity", &mut errs);

    // every camera mode with Blueprint inheritance resolved
    let key_map: &[(&str, &str)] = &[
        ("FOV_horizontalDeg", "FOV"),
        ("CameraSpaceForwardDistanceCm", "CameraSpaceForwardDistance"),
        ("CameraOriginInterpSpeed_XYZ", "CameraOriginInterpSpeed"),
        ("ViewTargetSpaceViewOffset", "ViewTargetSpaceViewOffset"),
        ("ViewTargetSpaceViewOffset_Crouch", "ViewTargetSpaceViewOffset_Crouch"),
        ("ViewTargetSpaceViewOffsetInterpSpeed_XYZ", "ViewTargetSpaceViewOffseInterpSpeed"),
        ("CameraSpaceViewOffset", "CameraSpaceViewOffset"),
        ("CollisionPushOutDistanceCm", "CollisionPushOutDistance"),
        ("HidePawnPenetrationPercent", "HidePawnPenetrationPercent"),
        ("TransitionTime", "TransitionTime"),
        ("TransitionOutTime", "TransitionOutTime"),
        ("TransitionParams", "TransitionParams"),
        ("bSupportsShoulderSwap", "bSupportsShoulderSwap"),
    ];
    let modes = rep["modes"].as_object().unwrap();
    for (name, m) in modes {
        let merged = resolve_camera(s, name);
        let eff = m["effective"].as_object().unwrap();
        for (rk, pk) in key_map {
            let Some(want) = eff.get(*rk) else { continue };
            if want.is_null() {
                continue; // Node `undefined` -> absent
            }
            n += 1;
            let mine = merged.get(*pk).cloned().unwrap_or(J::Null);
            let mine = if rk.starts_with("ViewTargetSpaceViewOffset") && !rk.contains("InterpSpeed") || *rk == "CameraSpaceViewOffset" { camera_flat_samples(&mine) } else { mine };
            check(want, &mine, &format!("modes.{name}.{rk}"), &mut errs);
        }
        if let Some(f) = eff.get("PenetrationAvoidanceFeelers") {
            n += 1;
            let mine = merged["PenetrationAvoidanceFeelers"].as_array().unwrap();
            let want = f.as_array().unwrap();
            assert_eq!(mine.len(), want.len(), "{name} feeler count");
            for (i, (w, mm)) in want.iter().zip(mine).enumerate() {
                for (wk, mk) in [("adjustmentRot_PitchYawRoll", "AdjustmentRot"), ("worldWeight", "WorldWeight"), ("pawnWeight", "PawnWeight"), ("extentCm", "Extent"), ("traceInterval", "TraceInterval")] {
                    check(&w[wk], &mm[mk], &format!("modes.{name}.feelers[{i}].{wk}"), &mut errs);
                }
            }
        }
    }
    assert_clean(errs, n);
}

// ---- movement -------------------------------------------------------------------------------

fn collect_floats(v: &Value, name: &str, offset: u32, out: &mut Vec<(String, f32, u32)>) {
    match v {
        Value::Float(f) => out.push((name.to_string(), *f, offset)),
        Value::Struct { props, .. } => props.iter().for_each(|p| collect_floats(&p.value, &p.name, p.offset, out)),
        Value::Array(a) | Value::Set(a) => a.iter().for_each(|x| collect_floats(x, name, offset, out)),
        Value::Map(m) => m.iter().for_each(|(k, x)| {
            collect_floats(k, name, offset, out);
            collect_floats(x, name, offset, out)
        }),
        _ => {}
    }
}

#[test]
fn movement_overrides() {
    let (s, rep) = need!("movement-overrides.json");
    let values = rep["values"].as_array().unwrap();
    let mut errs = Vec::new();
    let mut cache: std::collections::HashMap<String, Vec<(String, f32, u32)>> = Default::default();
    for v in values {
        let file = v["file"].as_str().unwrap().replace('\\', "/");
        let long = format!("/Game/{}", file.split("/Game/").nth(1).unwrap().trim_end_matches(".uasset"));
        let floats = cache.entry(long.clone()).or_insert_with(|| {
            let d = load(s, &long);
            let pkg = d.package().unwrap();
            let mut out = Vec::new();
            for i in 0..pkg.exports.len() {
                let o = pkg.read_export(i).unwrap();
                assert!(o.error.is_none(), "{long} #{i}: {:?}", o.error);
                for p in o.props.iter() {
                    collect_floats(&p.value, &p.name, p.offset, &mut out);
                }
            }
            let hs = pkg.asset_bytes().len() as u32;
            out.into_iter().map(|(n, f, o)| (n, f, o.wrapping_sub(hs))).collect()
        });
        let name = v["name"].as_str().unwrap();
        let want = v["value"].as_f64().unwrap();
        let off = v["offset"].as_u64().unwrap() as u32;
        // Node reports the tag start (.uexp-relative); the value follows 25 bytes later
        let hit = floats.iter().any(|(n, f, o)| n == name && *f as f64 == want && *o == off);
        if !hit {
            let near: Vec<_> = floats.iter().filter(|(n, ..)| n == name).collect();
            errs.push(format!("{long}: {name}={want} @{off} not found by the structural reader (same-name floats: {near:?})"));
        }
    }
    assert_clean(errs, values.len());
}

// ---- animation blueprint: nested structs, maps, class defaults, Kismet --------------------------

const ANIM_BP: &str = "/Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_AnimBlueprint";

fn strip_guid_suffix(k: &str) -> String {
    // "Foo_12_0123456789ABCDEF0123456789ABCDEF" -> "Foo"
    let b = k.as_bytes();
    if k.len() > 33 && b[k.len() - 33] == b'_' && k[k.len() - 32..].bytes().all(|c| c.is_ascii_hexdigit()) {
        let head = &k[..k.len() - 33];
        if let Some(i) = head.rfind('_') {
            if head[i + 1..].bytes().all(|c| c.is_ascii_digit()) {
                return head[..i].to_string();
            }
        }
        return head.to_string();
    }
    k.to_string()
}

#[test]
fn animation_blueprint() {
    let (s, rep) = need!("animation-state-machines.json");
    let d = load(s, ANIM_BP);
    let pkg = d.package().unwrap();
    let mut errs = Vec::new();
    let mut n = 0;

    // --- class export: BakedStateMachines (deeply nested tagged structs + arrays)
    let cls = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i) == "AnimBlueprintGeneratedClass").expect("anim class");
    let cobj = pkg.read_export(cls).unwrap();
    assert!(cobj.error.is_none());
    let baked = cobj.props.to_json(&pkg)["BakedStateMachines"].clone();
    let baked = baked.as_array().expect("BakedStateMachines");
    let machines = rep["machines"].as_array().unwrap();
    assert_eq!(baked.len(), machines.len(), "machine count");
    for (mi, m) in machines.iter().enumerate() {
        let bm = &baked[mi];
        n += 1;
        check(&m["name"], &bm["MachineName"], &format!("machines[{mi}].name"), &mut errs);
        let states = m["states"].as_array().unwrap();
        let bstates = bm["States"].as_array().unwrap();
        assert_eq!(states.len(), bstates.len(), "state count of {}", m["name"]);
        let btrans = bm["Transitions"].as_array().unwrap();
        for (si, st) in states.iter().enumerate() {
            n += 2;
            check(&st["name"], &bstates[si]["StateName"], &format!("machines[{mi}].states[{si}].name"), &mut errs);
            check(&st["conduit"], &bstates[si]["bIsAConduit"], &format!("machines[{mi}].states[{si}].conduit"), &mut errs);
            let want_tr = st["transitions"].as_array().unwrap();
            let have_tr = bstates[si]["Transitions"].as_array().cloned().unwrap_or_default();
            assert_eq!(want_tr.len(), have_tr.len(), "transition count {} / {}", m["name"], st["name"]);
            for (ti, t) in want_tr.iter().enumerate() {
                let bt = &btrans[have_tr[ti]["TransitionIndex"].as_u64().unwrap() as usize];
                n += 3;
                check(&t["to"], &bt["NextState"], &format!("machines[{mi}].states[{si}].transitions[{ti}].to"), &mut errs);
                let dur = bt["CrossfadeDuration"].as_f64().unwrap();
                let dur5: f64 = format!("{dur:.5}").parse().unwrap();
                if dur5 != t["duration"].as_f64().unwrap() {
                    errs.push(format!("machines[{mi}].states[{si}].transitions[{ti}].duration {dur5} != {}", t["duration"]));
                }
                let bm_name = bt["BlendMode"].as_str().unwrap().replace("EAlphaBlendOption::", "");
                if bm_name != t["blendMode"].as_str().unwrap() {
                    errs.push(format!("blendMode {bm_name} != {}", t["blendMode"]));
                }
            }
        }
    }

    // --- CDO scalar defaults
    let cdo = pkg.read_default_object().unwrap().unwrap().props.to_json(&pkg);
    for (k, v) in rep["defaults"].as_object().unwrap() {
        n += 1;
        check(v, &cdo[k], &format!("defaults.{k}"), &mut errs);
    }

    // --- CoreAnims_Male: TMap<enum, struct> inside the CDO
    let ca = rep["coreAnims"]["CoreAnims_Male"].as_object().unwrap();
    let map = cdo["CoreAnims_Male"].as_array().expect("CoreAnims_Male map");
    assert_eq!(map.len(), ca.len(), "CoreAnims_Male entries");
    for entry in map {
        let key = entry["key"].as_str().unwrap().replace("EFortWeaponCoreAnimation::", "");
        let want = ca.get(&key).unwrap_or_else(|| panic!("CoreAnims_Male key {key} missing in node report"));
        let val = entry["value"].as_object().unwrap();
        for (wk, wv) in want.as_object().unwrap() {
            n += 1;
            // find our field whose cleaned name equals wk
            let mine = val.iter().find(|(k, _)| strip_guid_suffix(k) == *wk).map(|(_, v)| v);
            match mine {
                Some(m) => check(wv, m, &format!("CoreAnims_Male.{key}.{wk}"), &mut errs),
                None => errs.push(format!("CoreAnims_Male.{key}.{wk}: field missing")),
            }
        }
    }

    // --- Kismet: the ExecuteUbergraph function. Statement boundaries must coincide with the Node disassembler.
    let ex = pkg.exports.iter().position(|e| e.name.starts_with("ExecuteUbergraph_")).expect("ubergraph");
    let f = fn_uasset::classes::read_function(&pkg, ex).unwrap();
    let dis = fn_uasset::kismet::disassemble(&pkg, f.script);
    assert!(dis.error.is_none(), "disassembly error: {:?}", dis.error);
    assert!(dis.reached_end && dis.end == f.script.len(), "script must end exactly at EX_EndOfScript");
    let base = f.script.as_ptr() as usize - pkg.export_bytes(ex).unwrap().as_ptr() as usize;
    // Port of the Node tool's temporary-substitution pass (state-machine-evidence.cjs) over *our* statement text.
    let mine_assigns = node_style_assigns(&dis.lines);
    let by_at: std::collections::HashMap<usize, &(usize, String, String)> = mine_assigns.iter().map(|x| (x.0 + base, x)).collect();
    let assigns = rep["eventGraphAssignments"].as_array().unwrap();
    let var_count = mine_assigns.iter().filter(|a| a.1.starts_with("V:")).count();
    assert_eq!(var_count, assigns.len(), "number of Blueprint-variable assignments");
    for a in assigns {
        n += 2;
        let at = a["at"].as_u64().unwrap() as usize;
        let var = a["variable"].as_str().unwrap();
        match by_at.get(&at) {
            Some((_, target, value)) => {
                if *target != format!("V:{var}") {
                    errs.push(format!("eventGraphAssignments @{at}: target {target} != V:{var}"));
                }
                if value != a["value"].as_str().unwrap() {
                    errs.push(format!("eventGraphAssignments @{at} {var}:\n   node: {}\n   rust: {value}", a["value"].as_str().unwrap()));
                }
            }
            None => errs.push(format!("eventGraphAssignments @{at}: no assignment statement starts here in the Rust disassembly")),
        }
    }    assert_clean(errs, n);
}



// ---- port of the Node tool's expression substitution --------------------------------------------

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut d, mut cur) = (0i32, String::new());
    for ch in s.chars() {
        if ch == '(' {
            d += 1;
        }
        if ch == ')' {
            d -= 1;
        }
        if ch == ',' && d == 0 {
            out.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn atom(s: &str) -> String {
    let s = s.strip_prefix("V:").unwrap_or(s);
    if let Some(d) = s.strip_prefix("byte ") {
        if !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()) {
            return d.to_string();
        }
    }
    s.to_string()
}

/// `^(math|call) (\w+)\((.*)\)$`
fn parse_call<'a>(token: &'a str, kw: &str) -> Option<(&'a str, &'a str)> {
    let rest = token.strip_prefix(kw)?.strip_prefix(' ')?;
    let open = rest.find('(')?;
    let name = &rest[..open];
    if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') || !rest.ends_with(')') {
        return None;
    }
    Some((name, &rest[open + 1..rest.len() - 1]))
}

fn expand(token: &str, defs: &std::collections::HashMap<String, String>, depth: u32) -> String {
    if depth > 12 {
        return token.to_string();
    }
    if token.starts_with("L:") {
        if let Some(d) = defs.get(token) {
            return expand(d, defs, depth + 1);
        }
    }
    if let Some((f, inner)) = parse_call(token, "math") {
        let a: Vec<String> = split_args(inner).iter().map(|x| expand(x, defs, depth + 1)).collect();
        let bin = |op: &str| format!("({} {op} {})", a[0], a.get(1).cloned().unwrap_or_default());
        return match f {
            "Not_PreBool" => format!("!{}", a[0]),
            "BooleanAND" => bin("&&"),
            "BooleanOR" => bin("||"),
            "Greater_FloatFloat" => bin(">"),
            "GreaterEqual_FloatFloat" => bin(">="),
            "Less_FloatFloat" => bin("<"),
            "LessEqual_FloatFloat" => bin("<="),
            "EqualEqual_FloatFloat" | "EqualEqual_ByteByte" => bin("=="),
            "NotEqual_ByteByte" => bin("!="),
            "Subtract_FloatFloat" => bin("-"),
            "Add_FloatFloat" => bin("+"),
            "Multiply_FloatFloat" => bin("*"),
            "Divide_FloatFloat" => bin("/"),
            _ => format!("{f}({})", a.join(", ")),
        };
    }
    if let Some((f, inner)) = parse_call(token, "call") {
        let a: Vec<String> = split_args(inner).iter().map(|x| expand(x, defs, depth + 1)).collect();
        return format!("{f}({})", a.join(", "));
    }
    atom(token)
}

/// `(?:let\[[^\]]*\]|letbool|letobj) (target) = (rhs)` -> (target, rhs); `target_is_local` selects the `L:\S+` form.
fn split_let(text: &str, local_only: bool) -> Option<(String, String)> {
    let rest = if let Some(r) = text.strip_prefix("let[") {
        let close = r.find(']')?;
        r[close + 1..].strip_prefix(' ')?
    } else if let Some(r) = text.strip_prefix("letbool ") {
        r
    } else if let Some(r) = text.strip_prefix("letobj ") {
        r
    } else {
        return None;
    };
    if local_only {
        if !rest.starts_with("L:") {
            return None;
        }
        let sp = rest.find(' ')?;
        let target = &rest[..sp];
        let value = rest[sp..].strip_prefix(" = ")?;
        return Some((target.to_string(), value.to_string()));
    }
    let eq = rest.find(" = ")?;
    Some((rest[..eq].to_string(), rest[eq + 3..].to_string()))
}

/// (offset, target, substituted value) for every non-temporary assignment (Node `assigns`).
fn node_style_assigns(lines: &[(usize, String)]) -> Vec<(usize, String, String)> {
    let mut defs: std::collections::HashMap<String, String> = Default::default();
    let mut out = Vec::new();
    for (at, text) in lines {
        if text == "popflow" {
            defs.clear();
            continue;
        }
        if let Some((t, v)) = split_let(text, true) {
            defs.insert(t, v);
            continue;
        }
        if let Some((t, v)) = split_let(text, false) {
            let value = expand(&v, &defs, 0);
            out.push((*at, t, value));
        }
    }
    out
}
