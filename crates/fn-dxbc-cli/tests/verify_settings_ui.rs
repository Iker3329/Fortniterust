//! (3a) for the settings-UI materials: slice the shaders fresh out of the 1.8 paks and compare our disassembly with every listing
//! (`reports/settings-ui-shaders-1.8/<Material>/ps_*_shader_<id>.asm`) the old d3dcompiler tooling produced.
mod common;
use common::*;
use fn_dxbc::disassemble;

#[test]
fn settings_ui_listings_match_fresh_extraction() {
    let Some(set) = open() else { eprintln!("no 1.8 install, skipping"); return };
    let root = fn_testdata::reports().join("settings-ui-shaders-1.8");
    let listings = files(&root, "asm");
    if listings.is_empty() { eprintln!("no old settings-ui listings, skipping"); return; }
    let mut cache = PkgCache::new();
    let index = PathIndex::new(&set);
    // packages of every material folder, plus their parents are found by id lookup across all of them
    let mut pkgs: Vec<String> = vec![];
    for d in std::fs::read_dir(&root).unwrap().flatten() {
        if d.path().is_dir() {
            let name = d.file_name().to_string_lossy().to_string();
            if let Some(g) = index.find(&name) { pkgs.push(g); } else { eprintln!("package for {name} not found in paks"); }
        }
    }
    let (mut ok, mut missing, mut bad, mut strict) = (0usize, vec![], vec![], 0usize);
    for l in &listings {
        let stem = l.file_stem().unwrap().to_string_lossy().to_string();
        let id: usize = stem.rsplit('_').next().unwrap().parse().unwrap();
        let want = normalize(&std::fs::read_to_string(l).unwrap());
        let folder = l.parent().unwrap().file_name().unwrap().to_string_lossy().to_string();
        // own package first, then any other
        let mut order: Vec<String> = index.find(&folder).into_iter().collect();
        order.extend(pkgs.iter().cloned());
        let mut found = None;
        for g in &order {
            if let Some(b) = cache.blobs(&set, g).iter().find(|b| b.id == id) {
                let text = disassemble(&b.bytes).unwrap();
                let got = normalize(&text);
                if got == want {
                    // also byte-identical (modulo empty lines)?
                    let raw = |s: &str| s.replace(['\0', '\u{feff}'], "").lines().map(|l| l.trim_end_matches('\r').to_string()).filter(|l| !l.trim().is_empty()).collect::<Vec<_>>();
                    if raw(&std::fs::read_to_string(l).unwrap()) == raw(&text) { strict += 1; }
                    found = Some(true);
                    break;
                }
                found = Some(false);
            }
        }
        match found { Some(true) => ok += 1, Some(false) => bad.push(format!("{folder}/{stem}")), None => missing.push(format!("{folder}/{stem}")) }
    }
    eprintln!("settings-ui listings: {} total, {ok} identical to the fresh extraction ({strict} byte-identical modulo empty lines), {} differ, {} not found", listings.len(), bad.len(), missing.len());
    for b in bad.iter().take(10) { eprintln!("  differs: {b}"); }
    for b in missing.iter().take(10) { eprintln!("  missing: {b}"); }
    assert!(bad.is_empty(), "{} listings differ", bad.len());
    assert!(ok * 100 >= listings.len() * 95, "only {ok} of {} listings reproduced", listings.len());
}

