//! Scratch (ambience task): class histogram of the Athena_Terrain persistent level + sublevels, filtered by substrings; dumps matching exports as JSON.
//! usage: ambscan <install> <comma substrings> [dump-class-substring]
use fn_pak::PakSet;
use fn_uasset::PackageData;
use std::collections::BTreeMap;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let set = PakSet::open_install(&a[1]).unwrap();
    let want: Vec<String> = a[2].split(',').map(|s| s.to_lowercase()).collect();
    let dump = a.get(3).map(|s| s.to_lowercase());
    let map = "/Game/Athena/Maps/Athena_Terrain";
    let mut levels = vec![(map.to_string(), String::new())];
    {
        let pd = PackageData::load(&set, map).unwrap();
        let pkg = pd.package().unwrap();
        for i in 0..pkg.exports.len() {
            if pkg.export_class_name(i).starts_with("LevelStreaming") {
                let o = pkg.read_export(i).unwrap();
                let j = o.props.to_json(&pkg);
                println!("STREAM {} {}", pkg.export_class_name(i), j);
                if let Some(wa) = j.get("WorldAsset").and_then(|v| v.as_str()) {
                    levels.push((wa.split('.').next().unwrap().to_string(), String::new()));
                }
            }
        }
    }
    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    let lvl_filter = a.get(4).cloned().unwrap_or_default();
    for (lp, _) in &levels {
        if !lp.contains(&lvl_filter) { continue; }
        let Ok(pd) = PackageData::load(&set, lp) else { println!("cannot load {lp}"); continue };
        let Ok(pkg) = pd.package() else { continue };
        for i in 0..pkg.exports.len() {
            let cn = pkg.export_class_name(i);
            let l = cn.to_lowercase();
            if want.iter().any(|w| l.contains(w)) {
                *hist.entry(cn.clone()).or_insert(0) += 1;
                if dump.as_ref().map_or(false, |d| l.contains(d.as_str())) {
                    match pkg.read_export(i) {
                        Ok(o) => println!("DUMP {lp} #{i} {} {} {}", cn, pkg.exports[i].name, o.props.to_json(&pkg)),
                        Err(e) => println!("DUMPERR {lp} #{i} {cn} {e}"),
                    }
                }
            }
        }
    }
    println!("levels {}", levels.len());
    for (c, n) in hist {
        println!("{n:6} {c}");
    }
}
