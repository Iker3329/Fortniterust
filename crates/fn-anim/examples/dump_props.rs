//! Dump one property of an export in full: dump_props <path without extension> <property name>
use fn_ueio::props::{read_props, Ctx};
use fn_ueio::{Package, Reader};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let base = &args[1];
    let rd = |e: &str| std::fs::read(format!("{base}.{e}")).unwrap_or_default();
    let pkg = Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")).unwrap();
    for i in 0..pkg.exports.len() {
        let (s, en) = pkg.export_range(i);
        let mut r = Reader::at(&pkg.data[..en], s);
        let c = Ctx { names: &pkg.names, file_version: pkg.summary.file_version };
        if let Ok(pl) = read_props(&mut r, &c) {
            for p in &pl.0 {
                if p.name == args[2] {
                    println!("{:#?}", p.value);
                }
            }
        }
    }
}
