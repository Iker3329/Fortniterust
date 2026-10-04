//! End-to-end: pak -> fn-ueio -> fn-texture / fn-mesh / fn-anim -> files, compared with the old UEViewer exports.
//! Skipped (with a note) when the 1.8 install or the old prototype tree is not on this machine.
use std::path::{Path, PathBuf};
use std::process::Command;

fn old_root() -> PathBuf {
    std::env::var_os("FN_OLD_ROOT").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap().join("FortniteRuntime"))
}
fn paks() -> PathBuf {
    std::env::var_os("FN_PAKS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:/Users/Brady/Downloads/1.8/Fortnite/Fortnite/FortniteGame/Content/Paks"))
}

fn run(what: &str, only: &str, out: &Path) -> Option<serde_json::Value> {
    let old = old_root();
    if !paks().is_dir() || !old.join("tools/asset-manifest.json").is_file() {
        eprintln!("skipping e2e: 1.8 paks or old tree missing");
        return None;
    }
    let st = Command::new(env!("CARGO_BIN_EXE_fn-export"))
        .args(["--paks", paks().to_str().unwrap(), "--manifest", old.join("tools/asset-manifest.json").to_str().unwrap(), "--out", out.join("assets").to_str().unwrap()])
        .args(["--verify", old.join("assets").to_str().unwrap(), "--only", only, what])
        .status()
        .expect("run fn-export");
    assert!(st.success(), "fn-export {what} failed");
    let rep = std::fs::read_to_string(out.join("reports/export-assets.json")).unwrap();
    Some(serde_json::from_str(&rep).unwrap())
}

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("fn-export-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

#[test]
fn textures_match_old_pngs_exactly() {
    let out = tmp("tex");
    let Some(r) = run("textures", "", &out) else { return };
    let arr = r["textures"].as_array().unwrap();
    assert!(arr.len() > 200);
    for e in arr {
        assert_eq!(e["ok"], true, "{e}");
        assert_eq!(e["detail"]["verify"]["max_abs"], 0, "{e}");
        assert_eq!(e["detail"]["verify"]["diff_pixels"], 0, "{e}");
    }
}

#[test]
fn animations_match_old_md5_text_exactly() {
    let out = tmp("anim");
    let Some(r) = run("animations", "", &out) else { return };
    let arr = r["animations"].as_array().unwrap();
    assert_eq!(arr.len(), 388);
    for e in arr {
        assert_eq!(e["ok"], true, "{e}");
        assert_eq!(e["detail"]["md5_identical_to_old"], true, "{e}");
        assert!(e["detail"]["fnanim_roundtrip_max_pos_err_cm"].as_f64().unwrap() < 1e-3, "{e}");
    }
}

#[test]
fn meshes_export_with_old_bin_layout() {
    let out = tmp("mesh");
    let Some(r) = run("meshes", "Soldier_04", &out) else { return };
    let e = &r["meshes"].as_array().unwrap()[0];
    assert_eq!(e["ok"], true, "{e}");
    assert_eq!(e["detail"]["vertices"], 10454); // LOD0 total: sections of 7493 + 924 + 2037
    assert_eq!(e["detail"]["verify"]["bin"]["differing_bytes"], 0);
    assert!(out.join("assets/exported/Game/Characters/Player/Male/Medium/Bodies/M_Med_Soldier_04/Materials/M_Med_Soldier_04.mat.json").is_file());
}
