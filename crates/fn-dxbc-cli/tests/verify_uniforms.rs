//! The uniform-expression evaluator (port of tools/hud-uniforms.cjs / menu-material-eval.cjs) against the old tools' JSON outputs:
//! reports/hud-shaders/*-uniforms.json (defaults) for every HUD material.
mod common;
use common::*;
use fn_dxbc_cli::spec::package_bytes;
use fn_dxbc_cli::uniform::*;

fn variant(e: &Expr) -> &'static str {
    match e {
        Expr::VectorParameter { .. } => "VectorParameter", Expr::ScalarParameter { .. } => "ScalarParameter", Expr::Constant { .. } => "Constant", Expr::ComponentSwizzle { .. } => "ComponentSwizzle",
        Expr::FoldedMath { .. } => "FoldedMath", Expr::AppendVector { .. } => "AppendVector", Expr::TrigMath { .. } => "TrigMath", Expr::Max { .. } => "Max", Expr::Texture { .. } => "Texture", Expr::TextureParameter { .. } => "TextureParameter",
        Expr::Time => "Time", Expr::RealTime => "RealTime",
    }
}
fn num(v: &serde_json::Value) -> f32 { v.as_f64().unwrap_or(f64::NAN) as f32 }
fn close(a: f32, b: f32) -> bool { a == b || (a - b).abs() <= 1e-5 * 1.0f32.max(a.abs()).max(b.abs()) }

#[test]
fn hud_uniform_sets_match_old_json() {
    let Some(set) = open() else { eprintln!("no 1.8 install, skipping"); return };
    let dir = fn_testdata::reports().join("hud-shaders");
    let jsons = files(&dir, "json");
    if jsons.is_empty() { eprintln!("no old hud reports, skipping"); return; }
    let index = PathIndex::new(&set);
    let (mut ok, mut slots) = (0, 0);
    for j in &jsons {
        let stem = j.file_stem().unwrap().to_string_lossy().to_string();
        let Some(name) = stem.strip_suffix("-uniforms") else { continue };
        let old: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(j).unwrap().trim_start_matches('\u{feff}')).unwrap();
        let game = index.find(name).unwrap_or_else(|| panic!("package {name} not found"));
        let asset = set.read_game(&game, "uasset").unwrap().into_owned();
        let uexp = set.read_game(&game, "uexp").unwrap().into_owned();
        let names = package_names(&asset, &uexp).unwrap();
        let us = parse_set_with(&uexp, &names, false).unwrap_or_else(|e| panic!("{name}: {e}"));
        let packed = evaluate(&us, &Overrides::default(), 0.0);
        let ov = old["vectors"].as_array().unwrap();
        let os = old["scalars"].as_array().unwrap();
        assert_eq!(us.vectors.len(), ov.len(), "{name}: vector count");
        assert_eq!(us.scalars.len(), os.len(), "{name}: scalar count");
        assert_eq!(us.textures2d.len(), old["textures2d"].as_array().unwrap().len(), "{name}: tex2d count");
        for (i, o) in ov.iter().enumerate() {
            assert_eq!(variant(&us.vectors[i]), o["expr"]["t"].as_str().unwrap(), "{name} v{i} type");
            let slot = o["slot"].as_u64().unwrap() as usize;
            for k in 0..4 { assert!(close(packed.cb[slot][k], num(&o["value"][k])), "{name} cb[{slot}][{k}]: ours {} old {}", packed.cb[slot][k], o["value"][k]); }
            slots += 1;
        }
        for (i, o) in os.iter().enumerate() {
            assert_eq!(variant(&us.scalars[i]), o["expr"]["t"].as_str().unwrap(), "{name} s{i} type");
            let slot = o["slot"].as_u64().unwrap() as usize;
            let comp = "xyzw".find(o["comp"].as_str().unwrap()).unwrap();
            assert!(close(packed.cb[slot][comp], num(&o["value"])), "{name} cb[{slot}].{comp}: ours {} old {}", packed.cb[slot][comp], o["value"]);
            slots += 1;
        }
        ok += 1;
    }
    eprintln!("{ok} HUD materials: uniform expression sets parsed and evaluated identically to the old JSON ({slots} registers/components checked)");
    assert!(ok >= 10);
    let _ = package_bytes;
}

