//! Dev tool (core agent): print the rows of a DataTable / CurveTable whose name contains a filter.
//! `cargo run -p fn-building --example dump_table -- <paks dir> <package> [row filter]`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let set = fn_pak::PakSet::open_dir(&a[1]).expect("paks");
    let pd = fn_uasset::PackageData::load(&set, &a[2]).expect("package");
    let pkg = pd.package().expect("parse");
    let tb = fn_uasset::classes::read_table(&pkg, 0).expect("table");
    let f = a.get(3).map(|s| s.to_lowercase()).unwrap_or_default();
    println!("{} rows", tb.rows.len());
    for (n, p) in &tb.rows {
        if !n.to_lowercase().contains(&f) { continue; }
        if let Some(keys) = p.get("Keys").and_then(fn_uasset::Value::as_array) {
            let k: Vec<String> = keys.iter().map(|k| format!("({}, {})", k.field("Time").and_then(|v| v.as_f32()).unwrap_or(f32::NAN), k.field("Value").and_then(|v| v.as_f32()).unwrap_or(f32::NAN))).collect();
            println!("{n}: {}", k.join(" "));
        } else {
            println!("{n}: {:?}", p);
        }
    }
}
