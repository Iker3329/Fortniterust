//! tools/menu-shaders.cjs ported: permutation choice, cbuffer evaluation and the asm files vs reports/menu-shaders/.
mod common;
use common::*;
use serde_json::Value;

#[test]
fn menu_shaders_match_old_reports() {
    let Some(set) = open() else { eprintln!("no 1.8 install, skipping"); return };
    let old_dir = fn_testdata::reports().join("menu-shaders");
    let widgets_path = fn_testdata::reports().join("menu-widgets.json");
    let (Ok(old_txt), Ok(w_txt)) = (std::fs::read_to_string(old_dir.join("cbuffers.json")), std::fs::read_to_string(&widgets_path)) else { eprintln!("old menu reports missing, skipping"); return };
    let old: Value = serde_json::from_str(old_txt.trim_start_matches('\u{feff}')).unwrap();
    let widgets: Value = serde_json::from_str(w_txt.trim_start_matches('\u{feff}')).unwrap();
    let out = std::env::temp_dir().join("fn-dxbc-menu-shaders");
    let _ = std::fs::remove_dir_all(&out);
    let new = fn_dxbc_cli::menu::run(&set, &widgets, &out).unwrap();
    // roles: same permutations, blob counts, expression counts
    for (role, o) in old["roles"].as_object().unwrap() {
        let n = &new["roles"][role];
        for k in ["owner", "shaderBlobs", "plainPermutation", "slatePermutation", "vectors", "scalars"] { assert_eq!(n[k], o[k], "role {role} field {k}"); }
    }
    // asm files (tolerant diff)
    let mut asm_ok = 0;
    for f in files(&old_dir, "asm") {
        let want = normalize(&std::fs::read_to_string(&f).unwrap());
        let got = normalize(&std::fs::read_to_string(out.join(f.file_name().unwrap())).unwrap());
        assert_eq!(got, want, "{}", f.display());
        asm_ok += 1;
    }
    // constant buffers
    let mut regs = 0;
    for (name, o) in old["materials"].as_object().unwrap() {
        let n = &new["materials"][name];
        assert_eq!(n["package"], o["package"], "{name} package");
        assert_eq!(n["registers"], o["registers"], "{name} registers");
        let (oc, nc) = (o["cb"].as_array().unwrap(), n["cb"].as_array().unwrap());
        assert_eq!(oc.len(), nc.len());
        for (i, (a, b)) in oc.iter().zip(nc.iter()).enumerate() {
            for k in 0..4 {
                let (x, y) = (a[k].as_f64().unwrap() as f32, b[k].as_f64().unwrap() as f32);
                assert!((x - y).abs() <= 1e-5 * 1.0f32.max(x.abs()).max(y.abs()), "{name} cb[{i}][{k}]: old {x} ours {y}");
                regs += 1;
            }
        }
        assert_eq!(n["slots"].as_object().unwrap().len(), o["slots"].as_object().unwrap().len(), "{name} named slots");
        assert_eq!(n["derived"].as_object().unwrap().len(), o["derived"].as_object().unwrap().len(), "{name} derived registers");
    }
    eprintln!("menu shaders: {} roles, {} material instances ({} cb components), {asm_ok} asm files identical", old["roles"].as_object().unwrap().len(), old["materials"].as_object().unwrap().len(), regs);
}
