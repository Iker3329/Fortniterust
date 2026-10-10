//! Prints every row of a DataTable (tagged properties, Debug form): `dump_table <table package> [row filter]`.
use fn_pose::{loader, ClipSourceDir};
use fn_ueio::{props::{read_props, Ctx}, Reader};
fn main() -> Result<(), String> {
    let table = std::env::args().nth(1).ok_or("table package")?;
    let filter = std::env::args().nth(2);
    let paks = fn_pak::PakSet::open_install(std::env::var("FN_TEST_ROOT").map_err(|_| "set FN_TEST_ROOT")?).map_err(|e| e.to_string())?;
    let src = ClipSourceDir::Pak(&paks);
    let pkg = loader::package(&src, &table)?;
    let idx = pkg.find_export("DataTable").ok_or("no DataTable export")?;
    let (start, end) = pkg.export_range(idx);
    let ctx = Ctx { names: &pkg.names, file_version: pkg.summary.file_version };
    let mut r = Reader::at(&pkg.data[..end], start);
    read_props(&mut r, &ctx).map_err(|e| e.to_string())?;
    if r.i32().map_err(|e| e.to_string())? != 0 { r.bytes(16).map_err(|e| e.to_string())?; }
    let n = r.i32().map_err(|e| e.to_string())?;
    for _ in 0..n {
        let name = r.i32().map_err(|e| e.to_string())? as usize;
        let number = r.i32().map_err(|e| e.to_string())?;
        let props = read_props(&mut r, &ctx).map_err(|e| e.to_string())?;
        let row = if number > 0 { format!("{}_{}", pkg.names[name], number - 1) } else { pkg.names[name].clone() };
        if filter.as_ref().map_or(true, |f| row.contains(f.as_str())) { println!("{row}: {props:?}"); }
    }
    Ok(())
}
