//! Writes `reports/exe-defaults.json`: every native default recovered from the 1.8 exe (read-only static analysis).
//! Usage: `report [out.json]` (default `<crate>/reports/exe-defaults.json`). `FN_EXE` overrides the exe path.
use fn_exe_defaults::classes::*;
use fn_exe_defaults::cvar::*;
use fn_exe_defaults::ctor::*;
use fn_exe_defaults::known::*;
use fn_exe_defaults::pps;
use fn_exe_slate::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const INT_NAMES: &[&str] = &["DynamicShadowCascades", "FarShadowCascadeCount", "CubemapResolution", "ShadowMapChannel", "LPVSize"];

fn r(x: f32) -> Value { json!((x as f64 * 1e6).round() / 1e6) }
fn hx(x: u32) -> String { format!("0x{x:x}") }

fn field_json(f: &pps::FieldValue, pe: &Pe) -> Value {
    let is_int = INT_NAMES.contains(&f.name.as_str());
    let how = match (f.explicit, f.kind) {
        (true, "byte_enum") => "byte immediate store in ctor",
        (true, _) => "dword immediate / movss from .rdata / 128-bit .rdata constant store in ctor",
        (false, _) => "no store in ctor after the memset(this,0,size): zero-initialised",
    };
    let _ = pe;
    json!({
        "name": f.name, "offset": hx(f.offset), "kind": f.kind,
        "value": if f.components.len() == 1 { if is_int { json!(f.raw[0] as i32) } else { r(f.components[0]) } } else { Value::Array(f.components.iter().map(|c| r(*c)).collect()) },
        "raw_u32": f.raw.iter().map(|x| hx(*x)).collect::<Vec<_>>(),
        "store_rva": f.store_rva.map(hx),
        "derived": how,
        "confidence": if f.explicit { "high" } else { "high (zero-init)" },
    })
}

fn chain(pe: &Pe, xi: &XrefIndex, spec: &[(&str, &str)]) -> Value {
    // effective view: per-class ctors applied base -> derived; props merged by offset (derived wins)
    let mut eff_dw: BTreeMap<i64, (u32, u32)> = BTreeMap::new();
    let mut eff_props: BTreeMap<u32, fn_exe_defaults::reflect::Prop> = BTreeMap::new();
    let mut per_class = vec![];
    let mut all_stores: Vec<Store> = vec![];
    let mut bit_ops: Vec<Value> = vec![];
    for (cn, hint) in spec {
        let Some(mut ci) = find_class(pe, xi, cn) else { per_class.push(json!({"class": cn, "error": "class not found"})); continue };
        if let Some((rg, p)) = props_by_hint_max(pe, xi, hint, ci.sizeof_class) { ci.z_construct = Some(rg); ci.props = p; }
        let (b, e) = pe.func_containing(ci.ctor).unwrap_or((ci.ctor, ci.ctor + 0x200));
        let stores = collect_stores(pe, b, e);
        for (o, v) in pps::final_dwords(&stores) { eff_dw.insert(o, v); }
        for p in &ci.props { eff_props.insert(p.offset, p.clone()); }
        per_class.push(json!({
            "class": cn, "sizeof": hx(ci.sizeof_class), "get_private_static_class_rva": hx(ci.get_private_static_class),
            "class_constructor_thunk_rva": hx(ci.default_ctor_thunk), "ctor_rva": hx(ci.ctor), "ctor_bytes": e - b,
            "z_construct_rva": ci.z_construct.map(|z| hx(z.0)), "reflected_properties": ci.props.len(),
        }));
        for bo in collect_bit_ops(pe, b, e) { bit_ops.push(json!({"class": cn, "rva": hx(bo.rva), "disp": hx(bo.disp as u32), "size": bo.size, "op": if bo.is_or { "or" } else { "and" }, "imm": hx(bo.imm as u32)})); }
        all_stores.extend(stores);
    }
    let props: Vec<_> = eff_props.values().cloned().collect();
    let fields = pps::field_values(&props, &eff_dw, &all_stores);
    let named: Vec<Value> = fields.iter().filter(|f| f.explicit).map(|f| field_json(f, pe)).collect();
    // stores that hit no reflected float/struct property (bool bitfields, padding, FColor, UObject ptrs)
    let covered: Vec<(i64, i64)> = fields.iter().map(|f| (f.offset as i64, f.offset as i64 + 4 * f.raw.len() as i64)).collect();
    let mut unnamed = vec![];
    for (o, (v, rva)) in &eff_dw {
        if !covered.iter().any(|(a, b)| o >= a && o < b) {
            let near = eff_props.range(..=(*o as u32)).next_back().map(|(_, p)| p.name.clone());
            unnamed.push(json!({"offset": hx(*o as u32), "raw_u32": hx(*v), "as_f32": r(f32::from_bits(*v)), "store_rva": hx(*rva), "preceding_property": near}));
        }
    }
    json!({ "classes": per_class, "defaults": named, "unnamed_stores": unnamed, "bit_field_or_and_ops": bit_ops })
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/reports/exe-defaults.json").to_string());
    let exe = exe_path();
    let pe = Pe::open(&exe).expect("open exe");
    let xi = XrefIndex::build(&pe);

    // ---- cvars ----
    let regs = scan(&pe, &xi);
    let mut cv: BTreeMap<String, Vec<&CvarReg>> = BTreeMap::new();
    for c in &regs {
        if !matches!(c.vtable_slot, 0x00 | 0x08 | 0x10 | 0x18 | 0x20 | 0x28) { continue; }
        if c.value == CvarValue::Unknown { continue; }
        cv.entry(c.name.clone()).or_default().push(c);
    }
    let mut cvars = vec![];
    let (mut known_ok, mut known_bad) = (0, vec![]);
    for (name, regs) in &cv {
        let c0 = regs[0];
        let kind = slot_kind(c0.vtable_slot);
        let (ty, val, how, conf): (&str, Value, String, &str) = match &c0.value {
            CvarValue::Int(i) => ("int", json!(i), format!("`mov r8d,imm32` before `call [IConsoleManager+0x10]` (RegisterConsoleVariable<int>) in thunk {}", hx(c0.func_rva)), "high"),
            CvarValue::Float(f) => ("float", r(*f), format!("`movss xmm2,[.rdata]` / zeroing idiom before `call [IConsoleManager+0x8]` (RegisterConsoleVariable<float>) in thunk {}", hx(c0.func_rva)), "high"),
            CvarValue::Str(s) => ("string", json!(s), format!("FString built from literal in thunk {} (heuristic: last UTF-16 literal before the name)", hx(c0.func_rva)), "medium"),
            CvarValue::RefInit { target_rva, init, float, in_raw_data } => {
                let bits = init.unwrap_or(0);
                (if *float { "float(ref)" } else { "int(ref)" },
                 if *float { r(f32::from_bits(bits)) } else { json!(bits as i32) },
                 format!("FAutoConsoleVariableRef: initial value of the backing global at RVA {} ({}) read from the image", hx(*target_rva), if *in_raw_data { ".data initialised bytes" } else { ".bss zero-fill; a dynamic initializer could change it" }),
                 if *in_raw_data { "high" } else { "medium" })
            }
            CvarValue::Unknown => unreachable!(),
        };
        let distinct: Vec<String> = { let mut v: Vec<String> = regs.iter().map(|c| format!("{:?}", c.value)).collect(); v.sort(); v.dedup(); v };
        let mut rec = json!({
            "name": name, "type": ty, "kind": kind, "default": val, "flags": c0.flags.map(hx), "help": c0.help,
            "thunk_rva": hx(c0.func_rva), "call_rva": hx(c0.call_rva), "derived": how, "confidence": conf,
        });
        if distinct.len() > 1 { rec["conflict"] = json!(distinct); }
        // known-value cross-check
        let kv = KNOWN_INT_CVARS.iter().find(|k| k.0 == name).map(|k| (k.1 as f64, cvar_i64(&c0.value) == Some(k.1)))
            .or_else(|| KNOWN_FLOAT_CVARS.iter().find(|k| k.0 == name).map(|k| (k.1 as f64, cvar_f32(&c0.value).map(|f| (f - k.1).abs() < 1e-4).unwrap_or(false))));
        if let Some((want, ok)) = kv { rec["ue416_known"] = json!({"expected": want, "match": ok}); if ok { known_ok += 1 } else { known_bad.push(name.clone()) } }
        cvars.push(rec);
    }

    // ---- FPostProcessSettings ----
    let pp = pps::find(&pe, &xi).expect("FPostProcessSettings ctor");
    let dw = pps::final_dwords(&pp.stores);
    let fv = pps::field_values(&pp.props, &dw, &pp.stores);
    let callers: Vec<String> = xi.callers_of(pp.func.0).iter().map(|c| hx(*c)).collect();
    let pps_fields: Vec<Value> = fv.iter().map(|f| field_json(f, &pe)).collect();

    // ---- classes ----
    let dir = chain(&pe, &xi, &[("ULightComponentBase", "LightColor"), ("ULightComponent", "ShadowSharpen"), ("UDirectionalLightComponent", "DynamicShadowCascades")]);
    let sky = chain(&pe, &xi, &[("ULightComponentBase", "LightColor"), ("USkyLightComponent", "LowerHemisphereColor")]);
    let fog = chain(&pe, &xi, &[("UExponentialHeightFogComponent", "FogDensity")]);
    let ppc = chain(&pe, &xi, &[("UPostProcessComponent", "BlendRadius")]);
    let ppv = chain(&pe, &xi, &[("APostProcessVolume", "BlendRadius")]);

    // ---- comparison with the Rust assumptions ----
    let mut cmp = vec![];
    for a in ASSUMED_PPS {
        let rec = fv.iter().find(|f| f.name == a.name);
        let (rv, status) = match rec { Some(f) => (f.components[0], if (f.components[0] - a.value).abs() <= 1e-4 * a.value.abs().max(1.0) { "match" } else { "DIFFERENT" }), None => (f32::NAN, "not-recovered") };
        cmp.push(json!({"name": a.name, "struct": "FPostProcessSettings", "assumed": r(a.value), "recovered": if rv.is_nan() { Value::Null } else { r(rv) }, "status": status, "used_in": a.used_in, "assumed_note": a.note}));
    }
    // bloom tints
    for (i, want) in [0.3465f32, 0.138, 0.5, 0.066, 0.066].iter().enumerate() {
        let n = format!("Bloom{}Tint", i + 1);
        if let Some(f) = fv.iter().find(|f| f.name == n) {
            let st = if i == 2 { "match-with-caveat" } else if (f.components[0] - want).abs() < 1e-3 { "match" } else { "DIFFERENT" };
            cmp.push(json!({"name": n, "struct": "FPostProcessSettings", "assumed": r(*want), "recovered": f.components.iter().map(|c| r(*c)).collect::<Vec<_>>(), "status": st, "used_in": ["crates/fn-postfx/src/params.rs:132 tints"], "assumed_note": if i == 2 { "Day volume serializes 0.5; engine default is 0.1176 (only matters if the serialized override is dropped)" } else { "ENGINE_DEFAULT" }}));
        }
    }
    let class_val = |section: &Value, name: &str| -> Option<f32> {
        section["defaults"].as_array()?.iter().find(|d| d["name"] == name).and_then(|d| d["value"].as_f64()).map(|v| v as f32)
    };
    for (cls, prop, want, used, note) in ASSUMED_CLASS {
        let sec = match *cls { "UDirectionalLightComponent" => &dir, "USkyLightComponent" => &sky, _ => &dir };
        let rv = class_val(sec, prop);
        let status = match rv { Some(v) if (v - want).abs() <= 1e-4 * want.abs().max(1.0) => "match", Some(_) if *prop == "CascadeDistributionExponent" => "default-differs-serialized-2-wins", Some(_) => "DIFFERENT", None => "not-recovered" };
        cmp.push(json!({"name": format!("{cls}.{prop}"), "assumed": r(*want), "recovered": rv.map(r), "status": status, "used_in": used, "assumed_note": note}));
    }

    let doc = json!({
        "meta": {
            "exe": exe.display().to_string(), "image_base": hx(pe.image_base as u32), "arch": "x64 PE, UE 4.16 shipping (static read only, never executed)",
            "methods": {
                "cvars": "static-init thunks `IConsoleManager::Get()->RegisterConsoleVariable(Name, Default, Help, Flags)` (vtable slot 0x00 string, 0x08 float via xmm2, 0x10 int via r8d, 0x20 FAutoConsoleVariableRef<float>, 0x28 FAutoConsoleVariableRef<int>); name = `lea rdx,[UTF-16 literal]`; default = argument register tracked by a straight-line constant tracker; flags = [rsp+0x20]",
                "FPostProcessSettings": "ctor located by a 12-value default fingerprint; property names/offsets from Z_Construct_UScriptStruct_FPostProcessSettings (UFloatProperty ctor arg r9d = CPP_PROPERTY_BASE offset); the ctor does memset(this,0,0x4C0) then stores non-zero defaults",
                "classes": "IMPLEMENT_CLASS GetPrivateStaticClass -> ClassConstructor thunk -> InternalConstructor -> real ctor; property offsets from the class's Z_Construct_UClass_*; chains applied base -> derived",
            },
            "counts": {"cvars_resolved": cvars.len(), "cvar_registrations_scanned": regs.len(), "known_ue416_cvars_matching": known_ok, "known_ue416_cvars_mismatching": known_bad},
            "notes": [
                "bool bit-field properties (bOverride_*, AmbientOcclusionRadiusInWS, bLowerHemisphereIsBlack ...) are registered through a runtime bit-probe, so their names are not paired with offsets; their bytes appear under unnamed_stores / not at all.",
                "Defaults are code-level. Config .ini files (DefaultEngine.ini, Scalability) and serialized assets override them at runtime; the paks' ini are encrypted so cvar overrides cannot be read statically.",
            ],
        },
        "post_process_settings": {
            "struct_size_memset": "0x4C0", "ctor_rva": hx(pp.func.0), "ctor_bytes": pp.func.1 - pp.func.0, "memset_call_rva": hx(pp.memset_call),
            "z_construct_rva": hx(pp.z_construct.0), "callers_of_ctor": callers, "fields": pps_fields,
        },
        "UDirectionalLightComponent": dir,
        "USkyLightComponent": sky,
        "UExponentialHeightFogComponent": fog,
        "UPostProcessComponent": ppc,
        "APostProcessVolume": ppv,
        "comparison_with_rust_assumptions": cmp,
        "cvars": cvars,
    });
    std::fs::create_dir_all(std::path::Path::new(&out).parent().unwrap()).ok();
    std::fs::write(&out, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    println!("wrote {out}: {} cvars ({} known ok, bad {:?}), {} pps fields, {} comparisons", cvars_len(&doc), known_ok, known_bad, doc["post_process_settings"]["fields"].as_array().unwrap().len(), cmp_len(&doc));
}
fn cvars_len(d: &Value) -> usize { d["cvars"].as_array().map(|a| a.len()).unwrap_or(0) }
fn cmp_len(d: &Value) -> usize { d["comparison_with_rust_assumptions"].as_array().map(|a| a.len()).unwrap_or(0) }

fn cvar_i64(v: &CvarValue) -> Option<i64> { match v { CvarValue::Int(i) => Some(*i), CvarValue::RefInit { init, float: false, .. } => Some(init.unwrap_or(0) as i32 as i64), _ => None } }
fn cvar_f32(v: &CvarValue) -> Option<f32> { match v { CvarValue::Float(f) => Some(*f), CvarValue::RefInit { init, float: true, .. } => Some(f32::from_bits(init.unwrap_or(0))), _ => None } }


