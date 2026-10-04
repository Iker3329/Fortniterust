//! Dump every serialized property of packages / stat rows compactly (research tool).
use fn_pose::{loader, ClipSourceDir};
use fn_ueio::{props::{read_props, Ctx, PropList, PropValue}, Package, Reader};
fn val(pkg: &Package, v: &PropValue, ind: usize) -> String {
    match v {
        PropValue::Struct { name, fields } => format!("struct {name} {{\n{}{}}}", list(pkg, fields, ind + 1), "  ".repeat(ind)),
        PropValue::Array(a) => { let mut s = format!("[{}]\n", a.len()); for (i, x) in a.iter().enumerate() { s += &format!("{}[{i}] {}\n", "  ".repeat(ind + 1), val(pkg, x, ind + 1)); } s }
        PropValue::Object(i) => if *i < 0 { format!("import {}", pkg.import_path(*i)) } else { format!("export {i}") },
        PropValue::Raw(b) => { let mut tags=String::new(); if b.len()>=4&&(b.len()-4)%8==0 { let n=i32::from_le_bytes(b[0..4].try_into().unwrap()); if n as usize*8==b.len()-4 { for k in 0..n as usize { let i=i32::from_le_bytes(b[4+k*8..8+k*8].try_into().unwrap()); let num=i32::from_le_bytes(b[8+k*8..12+k*8].try_into().unwrap()); if let Some(nm)=pkg.names.get(i.max(0) as usize){ tags+=&format!(" TAG<{nm}#{num}>"); } } } } let t: String = b.iter().map(|c| if (32..127).contains(c) { *c as char } else { '.' }).collect(); format!("raw {t}{tags}") }
        o => format!("{:?}", o),
    }
}
fn list(pkg: &Package, p: &PropList, ind: usize) -> String {
    let mut s = String::new();
    for pr in &p.0 { s += &format!("{}{}[{}]: {}\n", "  ".repeat(ind), pr.name, pr.array_index, val(pkg, &pr.value, ind)); }
    s
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let paks = fn_pak::PakSet::open_install(&a[1]).unwrap();
    let src = ClipSourceDir::Pak(&paks);
    match a[2].as_str() {
        "pkg" => { let pkg = loader::package(&src, &a[3]).unwrap();
            for i in 0..pkg.exports.len() { let (s, e) = pkg.export_range(i); println!("== export {i} {} class {}", pkg.exports[i].object_name, pkg.export_class(i));
                let ctx = Ctx { names: &pkg.names, file_version: pkg.summary.file_version };
                match read_props(&mut Reader::at(&pkg.data[..e], s), &ctx) { Ok(p) => print!("{}", list(&pkg, &p, 1)), Err(er) => println!("ERR {er}") } }
            for i in 0..pkg.imports.len() { println!("import {i}: {}", pkg.import_path(-(i as i32 + 1))); } }
        "row" => { let r = fn_pose::loadout_stats::data_table_row(&src, &a[3], &a[4]).unwrap();
            for pr in &r.0 { println!("{}: {:?}", pr.name, pr.value); } }
        "cat" => { let b = paks.read(&a[3]).unwrap(); std::io::Write::write_all(&mut std::io::stdout(), &b).unwrap(); }
        "ct" => { let pd = fn_uasset::PackageData::load(&paks, &a[3]).unwrap(); let pkg = pd.package().unwrap(); let idx = 0usize;
            let tb = fn_uasset::classes::read_table(&pkg, idx).unwrap();
            for (n, p) in &tb.rows { if a.len() < 5 || n.contains(a[4].as_str()) { println!("{n}: {}", p.to_json(&pkg)); } } }
        "grep" => { let needle = a[4].as_bytes(); let mut n = 0; for (_, e) in paks.glob(&a[3]) { if !e.rel_path().ends_with(".uasset") { continue; } n += 1; if let Ok(b) = paks.read(e.rel_path()) { if b.windows(needle.len()).any(|w| w == needle) { println!("{}", e.rel_path()); } } } eprintln!("scanned {n}"); }
        "glob" => { for (_, e) in paks.glob(&a[3]) { println!("{}", e.rel_path()); } }
        _ => {}
    }
}





