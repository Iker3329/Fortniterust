use fn_ueio::props::{read_props, Ctx};
use fn_ueio::{Package, Reader};

fn main() {
    let base = std::env::args().nth(1).expect("path without extension");
    let rd = |e: &str| std::fs::read(format!("{base}.{e}")).unwrap_or_default();
    let pkg = Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")).unwrap();
    println!("{:?}", pkg.summary);
    for (i, im) in pkg.imports.iter().enumerate() {
        println!("import {} {} {} outer {}", -(i as i32) - 1, im.class_name, im.object_name, im.outer_index);
    }
    for (i, e) in pkg.exports.iter().enumerate() {
        println!("export {} {} class={} off={} size={}", i + 1, e.object_name, pkg.export_class(i), e.serial_offset, e.serial_size);
        let (s, en) = pkg.export_range(i);
        let mut r = Reader::at(&pkg.data[..en], s);
        let c = Ctx { names: &pkg.names, file_version: pkg.summary.file_version };
        match read_props(&mut r, &c) {
            Ok(pl) => {
                for p in &pl.0 {
                    let v = format!("{:?}", p.value);
                    println!("   {} : {} = {}", p.name, p.type_name, &v[..v.len().min(300)]);
                }
                println!("   props end at +{} of {}", r.tell() - s, en - s);
            }
            Err(e) => println!("   props err {e}"),
        }
    }
}
