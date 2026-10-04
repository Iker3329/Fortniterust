//! `postfx-evidence [--root <FortniteGame>] [--out reports/postfx-evidence.json] [--dump-shaders <dir>] [--skip-scan]`
use fn_pak::PakSet;
use fn_postfx::{evidence, shadercache};
use serde_json::{json, Value};
use std::path::PathBuf;

const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

#[allow(unused_variables, unused_assignments)]
fn main() {
    let mut root = PathBuf::from(std::env::var("FN_ROOT").unwrap_or_else(|_| DEFAULT_ROOT.into()));
    let mut out = PathBuf::from("reports/postfx-evidence.json");
    let mut dump: Option<PathBuf> = None;
    let mut skip_scan = false;
    let mut disasm_dir: Option<PathBuf> = None;
    let mut disasm_filter: Vec<String> = Vec::new();
    #[allow(unused_variables, unused_mut)]
    let mut disasm_hex = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = args.next().expect("--root value").into(),
            "--out" => out = args.next().expect("--out value").into(),
            "--dump-shaders" => dump = Some(args.next().expect("--dump-shaders dir").into()),
            "--skip-scan" => skip_scan = true,
            "--disasm" => { if cfg!(not(feature = "dxbc")) { panic!("--disasm needs `--features dxbc`"); } disasm_dir = Some(args.next().expect("--disasm dir").into()) }
            "--hex" => disasm_hex = true,
            "--filter" => disasm_filter.push(args.next().expect("--filter substring")),
            other => panic!("unknown argument {other}"),
        }
    }
    let t0 = std::time::Instant::now();
    let set = PakSet::open_install(&root).expect("open install");
    eprintln!("opened {} paks, {} entries", set.len(), set.total_entries());

    // shader cache
    let gsc = set.read("Engine/GlobalShaderCache-PCD3D_SM5.bin").expect("global shader cache").into_owned();
    let shaders = shadercache::extract(&gsc);
    eprintln!("{} DXBC blobs in {:?}", shaders.len(), t0.elapsed());
    if let Some(dir) = &dump {
        std::fs::create_dir_all(dir).unwrap();
        for s in &shaders {
            let safe: String = s.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect();
            std::fs::write(dir.join(format!("{safe}__{}.dxbc", s.id)), &s.dxbc).unwrap();
        }
    }
    #[cfg(feature = "dxbc")]
    if let Some(dir) = &disasm_dir {
        // d3dcompiler-format listings (fn-dxbc) of every shader whose name matches the filter (default: all)
        std::fs::create_dir_all(dir).unwrap();
        for s in shaders.iter().filter(|s| disasm_filter.is_empty() || disasm_filter.iter().any(|f| s.name.contains(f.as_str()))) {
            let safe: String = s.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect();
            let listing = if disasm_hex { fn_dxbc::disassemble_hex(&s.dxbc) } else { fn_dxbc::disassemble(&s.dxbc) };
            match listing {
                Ok(t) => std::fs::write(dir.join(format!("{safe}__{}.asm", s.id)), t).unwrap(),
                Err(e) => eprintln!("disasm {} failed: {e:?}", s.name),
            }
        }
    }
    let relevant = ["DOF", "DepthOfField", "Dof", "MotionBlur", "Velocity", "Bloom", "LensFlare", "Fringe", "Volumetric", "ExponentialHeightFog", "Tonemap", "EyeAdaptation", "AmbientOcclusion", "WeightedSampleSum", "LightShaft", "Downsample", "Gaussian", "Combine"];
    let shader_list: Vec<Value> = shaders.iter().filter(|s| relevant.iter().any(|r| s.name.contains(r))).map(|s| json!({ "id": s.id, "name": s.name, "profile": s.profile, "dxbc_bytes": s.dxbc.len() })).collect();

    let exe = std::fs::read(root.join("Binaries/Win64/FortniteClient-Win64-Shipping.exe")).ok();
    let mut report = json!({
        "generated_by": "crates/fn-postfx postfx-evidence (pure Rust: fn-pak + fn-uasset)",
        "install": root.display().to_string(),
        "todm": evidence::todm_section(&set),
        "shader_cache": { "file": "Engine/GlobalShaderCache-PCD3D_SM5.bin", "dxbc_blobs": shaders.len(), "postfx_related": shader_list },
    });
    if !skip_scan {
        report["tod_references"] = evidence::tod_reference_section(&set);
        report["scan"] = evidence::scan_section(&set);
        report["cvars"] = evidence::cvar_section(&set, exe.as_deref());
    }
    report["effects"] = evidence::effects_section(&report);
    if let Some(perf) = out.parent().map(|p| p.join("postfx-demo/perf.json")).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str::<Value>(&s).ok()) { report["perf_rtx2060"] = perf; }
    if let Some(p) = out.parent() { std::fs::create_dir_all(p).ok(); }
    std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    eprintln!("wrote {} in {:?}", out.display(), t0.elapsed());
}
