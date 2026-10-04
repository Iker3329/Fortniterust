//! Scratch (ambience task): dump every export (class, name, tagged props as JSON, trailing size) of the given packages, plus the name table.
//! usage: pkgdump <install> <package>... [--names]
use fn_pak::PakSet;
use fn_uasset::PackageData;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let set = PakSet::open_install(&a[1]).unwrap();
    let names = a.iter().any(|x| x == "--names");
    for pk in a[2..].iter().filter(|x| !x.starts_with("--")) {
        let d = match PackageData::load(&set, pk) {
            Ok(d) => d,
            Err(e) => {
                println!("{pk}: {e}");
                continue;
            }
        };
        let p = d.package().unwrap();
        println!("== {pk}");
        for i in 0..p.exports.len() {
            match p.read_export(i) {
                Ok(o) => println!("[{i}] {} ({}) trailing {} {}", o.name, o.class, o.trailing.len(), o.props.to_json(&p)),
                Err(e) => println!("[{i}] {} ERR {e}", p.exports[i].name),
            }
        }
        if std::env::args().any(|x| x == "--code") {
            for i in 0..p.exports.len() {
                if p.export_class_name(i) != "Function" { continue; }
                if let Ok(f) = fn_uasset::classes::read_function(&p, i) {
                    if f.script.is_empty() { continue; }
                    let d = fn_uasset::kismet::disassemble(&p, f.script);
                    println!("--- FUNCTION {} ({} bytes, err {:?})", p.exports[i].name, f.script.len(), d.error);
                    for (off, l) in d.lines { println!("{off:5}: {l}"); }
                }
            }
        }
        if names {
            for n in &p.names {
                println!("  name: {n}");
            }
        }
    }
}
