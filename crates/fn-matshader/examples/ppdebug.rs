//! Debug: why a material's shader map is not found (candidate sets, label anchors, descriptions).
use fn_fx::data::{long_package, package_files};
fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("material object path")?;
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let long = long_package(&path).to_owned();
    let (a, x, b) = package_files(&set, &long)?;
    let pkg = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| e.to_string())?;
    let label = path.rsplit('.').next().unwrap();
    let c = fn_dxbc_cli::uniform::set_candidates(&x, &pkg.names);
    println!("{} candidates", c.len());
    for c in &c {
        let lab = fn_dxbc_cli::uniform::preceding_label(&x, c.start, label);
        let d = fn_dxbc_cli::uniform::compilation_description(&x, c, label);
        let pre = &x[c.start.saturating_sub(40)..c.start];
        println!("start {} end {} v{} s{} t{} trailing {} label {} desc {:?}\n  pre {:?}", c.start, c.end, c.set.vectors.len(), c.set.scalars.len(), c.set.textures2d.len(), c.trailing_count, lab, d.as_ref().map(|d| d.text.chars().take(80).collect::<String>()), String::from_utf8_lossy(pre));
        if let Err(_) = d { let after = &x[c.end..(c.end + 400).min(x.len())]; let pos = after.windows(9).position(|w| w == b"Compiling"); println!("  Compiling at +{:?} first i32 {}", pos, i32::from_le_bytes(after[0..4].try_into().unwrap())); }
    }
    Ok(())
}
