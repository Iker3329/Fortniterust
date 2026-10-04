//! Scratch: list every property (any depth) that references sound-ish assets, for one or more packages.
use fn_pak::PakSet;
use fn_uasset::{PackageData, Value};

fn walk(pkg: &fn_uasset::Package, path: &str, v: &Value, pat: &str, out: &mut Vec<String>) {
    match v {
        Value::Object(i) if !i.is_null() => {
            let p = pkg.object_path(*i);
            if p.to_lowercase().contains(pat) {
                out.push(format!("{path} = {p}"));
            }
        }
        Value::SoftObject(s) | Value::Str(s) | Value::Name(s) => {
            if s.to_lowercase().contains(pat) {
                out.push(format!("{path} = {s}"));
            }
        }
        Value::Array(a) | Value::Set(a) => {
            for (i, x) in a.iter().enumerate() {
                walk(pkg, &format!("{path}[{i}]"), x, pat, out)
            }
        }
        Value::Map(m) => {
            for (i, (k, x)) in m.iter().enumerate() {
                walk(pkg, &format!("{path}<{i}>k"), k, pat, out);
                walk(pkg, &format!("{path}<{i}>v"), x, pat, out)
            }
        }
        Value::Struct { props, .. } => {
            for p in props.iter() {
                walk(pkg, &format!("{path}.{}", p.name), &p.value, pat, out)
            }
        }
        _ => {}
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let set = PakSet::open_install(&a[1]).unwrap();
    let pat = a[2].to_lowercase();
    for pk in &a[3..] {
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
            let o = p.read_export(i).unwrap();
            let mut out = vec![];
            for pr in o.props.iter() {
                walk(&p, &pr.name, &pr.value, &pat, &mut out);
            }
            if !out.is_empty() {
                println!("  [{i}] {} ({})", o.name, o.class);
                for l in out {
                    println!("      {l}");
                }
            }
        }
        // names that look like sounds (bytecode references etc.)
        for n in &p.names {
            if n.to_lowercase().contains(&pat) {
                println!("  name: {n}");
            }
        }
    }
}
