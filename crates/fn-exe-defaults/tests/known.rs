//! Known-value and PE-offset tests. They read the 1.8 exe (static read only, never executed); when it is not installed the
//! tests print a note and pass (so CI machines without the game still build). Override the path with `FN_EXE`.
use fn_exe_defaults::{classes::*, cvar::*, ctor::*, known::*, pps};
use fn_exe_slate::*;
use std::sync::OnceLock;

struct Fx { pe: Pe, xi: XrefIndex, cvars: Vec<CvarReg> }
fn fx() -> Option<&'static Fx> {
    static F: OnceLock<Option<Fx>> = OnceLock::new();
    F.get_or_init(|| {
        let p = exe_path();
        if !p.exists() { eprintln!("exe not found at {}: skipping", p.display()); return None; }
        let pe = Pe::open(&p).unwrap();
        let xi = XrefIndex::build(&pe);
        let cvars = scan(&pe, &xi);
        Some(Fx { pe, xi, cvars })
    }).as_ref()
}
fn cvar<'a>(f: &'a Fx, name: &str) -> &'a CvarReg {
    f.cvars.iter().find(|c| c.name == name && matches!(c.vtable_slot, 0x00 | 0x08 | 0x10 | 0x18 | 0x20 | 0x28)).unwrap_or_else(|| panic!("cvar {name} not found"))
}
fn as_f32(v: &CvarValue) -> f32 { match v { CvarValue::Float(f) => *f, CvarValue::RefInit { init, float: true, .. } => f32::from_bits(init.unwrap_or(0)), o => panic!("not a float: {o:?}") } }
fn as_i64(v: &CvarValue) -> i64 { match v { CvarValue::Int(i) => *i, CvarValue::RefInit { init, float: false, .. } => init.unwrap_or(0) as i32 as i64, o => panic!("not an int: {o:?}") } }

// ---------------- PE offsets ----------------
#[test]
fn pe_layout() {
    let Some(f) = fx() else { return };
    assert_eq!(f.pe.image_base, 0x1_4000_0000);
    let text = f.pe.section_named(".text").unwrap();
    assert_eq!((text.rva, text.vsize, text.raw_off), (0x1000, 0x03fa_303f, 0x600));
    let rd = f.pe.section_named(".rdata").unwrap();
    assert_eq!((rd.rva, rd.raw_off), (0x03fa_5000, 0x03fa_3800));
    // RVA <-> file offset (string located by a byte search at file offset 0x44bb780)
    assert_eq!(f.pe.rva_to_off(0x44b_cf80), Some(0x44b_b780));
    assert_eq!(f.pe.off_to_rva(0x44b_b780), Some(0x44b_cf80));
    assert_eq!(f.pe.utf16_at_off(0x44b_b780), "r.Shadow.MaxResolution");
    assert_eq!(f.pe.funcs.len(), 240449);
}

#[test]
fn cvar_thunk_addresses() {
    let Some(f) = fx() else { return };
    let c = cvar(f, "r.Shadow.MaxResolution");
    assert_eq!((c.func_rva, c.name_lea_rva, c.call_rva, c.vtable_slot), (0x109e70, 0x109ea4, 0x109eab, 0x10));
    assert_eq!(c.flags, Some(0x60));
    assert_eq!(c.help.starts_with("Max square dimensions"), true);
    let t = cvar(f, "r.TonemapperFilm");
    assert_eq!((t.func_rva, t.flags), (0x14e980, Some(0x20)));
}

// ---------------- cvar known values (UE 4.16 source) ----------------
#[test]
fn cvar_known_ue416_defaults() {
    let Some(f) = fx() else { return };
    for (n, want) in KNOWN_INT_CVARS { assert_eq!(as_i64(&cvar(f, n).value), *want, "{n}"); }
    for (n, want) in KNOWN_FLOAT_CVARS { assert!((as_f32(&cvar(f, n).value) - want).abs() < 1e-4, "{n}"); }
}

#[test]
fn cvar_post_and_shadow_values() {
    let Some(f) = fx() else { return };
    // values recovered here (not in the "known" list): regression pins
    assert_eq!(as_i64(&cvar(f, "r.Shadow.MaxCSMResolution").value), 2048);
    assert!((as_f32(&cvar(f, "r.Shadow.CSMDepthBias").value) - 20.0).abs() < 1e-6);
    assert!((as_f32(&cvar(f, "r.Shadow.CSMSplitPenumbraScale").value) - 0.5).abs() < 1e-6);
    assert!((as_f32(&cvar(f, "r.Shadow.DistanceScale").value) - 1.0).abs() < 1e-6);
    assert_eq!(as_i64(&cvar(f, "r.EyeAdaptationQuality").value), 2);
    // r.Streaming.PoolSize uses `or r8d,-1` (-1 = automatic): checks the `or reg,-1` idiom in the tracker
    assert_eq!(as_i64(&cvar(f, "r.Streaming.PoolSize").value), -1);
    // FAutoConsoleVariableRef<float> backed by an initialised .data global
    assert!((as_f32(&cvar(f, "r.Shadow.RadiusThreshold").value) - 0.03).abs() < 1e-6);
}

// ---------------- FPostProcessSettings ----------------
#[test]
fn post_process_settings_ctor() {
    let Some(f) = fx() else { return };
    let pp = pps::find(&f.pe, &f.xi).expect("ctor");
    assert_eq!(pp.func.0, 0x4d3210);
    let dw = pps::final_dwords(&pp.stores);
    let fv = pps::field_values(&pp.props, &dw, &pp.stores);
    let get = |n: &str| fv.iter().find(|x| x.name == n).unwrap_or_else(|| panic!("{n}"));
    // reflection offsets (CPP_PROPERTY_BASE) match the ctor store offsets
    assert_eq!(get("BloomIntensity").offset, 0x208);
    assert_eq!(get("AutoExposureMinBrightness").offset, 0x2d4);
    let near = |n: &str, v: f32| assert!((get(n).components[0] - v).abs() < 1e-5, "{n} = {} want {v}", get(n).components[0]);
    near("WhiteTemp", 6500.0);
    near("BloomIntensity", 0.675);
    near("BloomThreshold", -1.0);
    near("AutoExposureMinBrightness", 0.03);
    near("AutoExposureMaxBrightness", 2.0);
    near("AutoExposureSpeedUp", 3.0);
    near("AutoExposureSpeedDown", 1.0);
    near("AutoExposureBias", 0.0);
    near("HistogramLogMin", -8.0);
    near("HistogramLogMax", 4.0);
    near("VignetteIntensity", 0.4);
    near("AmbientOcclusionRadius", 200.0);
    near("FilmSlope", 0.88);
    near("FilmToe", 0.55);
    near("FilmShoulder", 0.26);
    near("FilmBlackClip", 0.0);
    near("FilmWhiteClip", 0.04);
    near("ScreenPercentage", 100.0);
    assert_eq!(get("FilmWhitePoint").components, vec![1.0, 1.0, 1.0, 1.0]);
    // UPostProcessComponent::UPostProcessComponent builds its Settings member (+0x2A0) with this ctor
    let call_in_ppc = f.xi.callers_of(0x4d3210).iter().any(|c| (0x20c6d50..0x20c6dbb).contains(c));
    assert!(call_in_ppc, "UPostProcessComponent ctor must call the settings ctor");
}

// ---------------- light / fog / sky constructors ----------------
fn class_val(f: &Fx, cname: &str, hint: &str, prop: &str) -> f32 {
    let mut ci = find_class(&f.pe, &f.xi, cname).unwrap();
    let (rg, p) = props_by_hint_max(&f.pe, &f.xi, hint, ci.sizeof_class).unwrap();
    ci.z_construct = Some(rg);
    ci.props = p;
    let (b, e) = f.pe.func_containing(ci.ctor).unwrap();
    let dw = pps::final_dwords(&collect_stores(&f.pe, b, e));
    let off = ci.props.iter().find(|p| p.name == prop).unwrap_or_else(|| panic!("{prop}")).offset as i64;
    f32::from_bits(dw.get(&off).unwrap_or_else(|| panic!("no store for {prop}")).0)
}

#[test]
fn directional_light_ctor() {
    let Some(f) = fx() else { return };
    let ci = find_class(&f.pe, &f.xi, "UDirectionalLightComponent").unwrap();
    assert_eq!((ci.sizeof_class, ci.get_private_static_class, ci.ctor), (0x430, 0x258c890, 0x20a4750));
    let v = |p: &str| class_val(f, "UDirectionalLightComponent", "DynamicShadowCascades", p);
    assert_eq!(v("DynamicShadowCascades").to_bits(), 3); // int property
    assert_eq!(v("CascadeDistributionExponent"), 3.0);
    assert_eq!(v("LightSourceAngle"), 1.0);
    assert!((v("CascadeTransitionFraction") - 0.1).abs() < 1e-6);
    assert_eq!(v("DynamicShadowDistanceMovableLight"), 20000.0);
}

#[test]
fn fog_and_sky_ctors() {
    let Some(f) = fx() else { return };
    let fog = |p: &str| class_val(f, "UExponentialHeightFogComponent", "FogDensity", p);
    assert!((fog("FogDensity") - 0.02).abs() < 1e-6);
    assert!((fog("FogHeightFalloff") - 0.2).abs() < 1e-6);
    assert_eq!(fog("FogMaxOpacity"), 1.0);
    assert_eq!(fog("FogCutoffDistance"), 0.0);
    assert_eq!(fog("DirectionalInscatteringExponent"), 4.0);
    assert_eq!(fog("DirectionalInscatteringStartDistance"), 10000.0);
    let sky = |p: &str| class_val(f, "USkyLightComponent", "LowerHemisphereColor", p);
    // Intensity (ULightComponentBase, +0x2A4) is overridden by the sky light ctor: its own store of 1.0
    let ci = find_class(&f.pe, &f.xi, "USkyLightComponent").unwrap();
    let (b, e) = f.pe.func_containing(ci.ctor).unwrap();
    let dw = pps::final_dwords(&collect_stores(&f.pe, b, e));
    assert_eq!(f32::from_bits(dw[&0x2a4].0), 1.0);
    let bi = find_class(&f.pe, &f.xi, "ULightComponentBase").unwrap();
    let (bb, be) = f.pe.func_containing(bi.ctor).unwrap();
    let bdw = pps::final_dwords(&collect_stores(&f.pe, bb, be));
    assert_eq!(bdw[&0x2a8].0, 0xffff_ffff, "LightColor = FColor::White (loaded from the initialised .data global)");
    assert_eq!(sky("OcclusionExponent"), 1.0);
    // ULightComponentBase default Intensity is pi (the sky light ctor overrides it with 1)
    let base = class_val(f, "ULightComponentBase", "LightColor", "Intensity");
    assert!((base - std::f32::consts::PI).abs() < 1e-6);
}


