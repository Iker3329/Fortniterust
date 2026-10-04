//! Parse every .uasset below a directory (with sibling .uexp/.ubulk) and read each export's tagged properties.
use fn_ueio::props::{read_props, Ctx};
use fn_ueio::{Package, Reader};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn walk(p: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(d) = std::fs::read_dir(p) {
        for e in d.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out)
            } else if p.extension().map_or(false, |x| x == "uasset") {
                out.push(p)
            }
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).expect("dir");
    let mut files = vec![];
    walk(Path::new(&root), &mut files);
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    let (mut ok, mut bad, mut raw_props) = (0, 0, 0);
    for f in &files {
        let rd = |e: &str| std::fs::read(f.with_extension(e)).unwrap_or_default();
        let pkg = match Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")) {
            Ok(p) => p,
            Err(e) => {
                println!("PKG FAIL {}: {e}", f.display());
                bad += 1;
                continue;
            }
        };
        let c = Ctx { names: &pkg.names, file_version: pkg.summary.file_version };
        for i in 0..pkg.exports.len() {
            *classes.entry(pkg.export_class(i)).or_default() += 1;
            let (s, e) = pkg.export_range(i);
            let mut r = Reader::at(&pkg.data[..e], s);
            match read_props(&mut r, &c) {
                Ok(pl) => {
                    for p in &pl.0 {
                        if matches!(p.value, fn_ueio::PropValue::Raw(_)) {
                            raw_props += 1;
                        }
                    }
                }
                Err(err) => {
                    println!("PROP FAIL {} export {} ({}): {err}", f.display(), i + 1, pkg.export_class(i));
                    bad += 1;
                }
            }
        }
        ok += 1;
    }
    println!("packages ok {ok} failures {bad} raw props {raw_props}");
    for (k, v) in classes {
        println!("{k}: {v}");
    }
}
