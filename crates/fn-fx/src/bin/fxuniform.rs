//! Research tool: `fxuniform <package>` lists the uniform expression set candidates of a material's uexp (and the FNames of the uniform expression types found).
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let long = std::env::args().nth(1).ok_or("package")?;
    let (a, x, b) = fn_fx::data::package_files(&set, &long)?;
    let p = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| e.to_string())?;
    let c = fn_dxbc_cli::uniform::set_candidates(&x, &p.names);
    println!("{} candidates", c.len());
    for k in &c { println!("  @{}..{} vectors {} scalars {} textures {}", k.start, k.end, k.set.vectors.len(), k.set.scalars.len(), k.set.textures2d.len()); }
    // FNames whose base name starts with FMaterialUniformExpression
    let kinds: Vec<(usize, &String)> = p.names.iter().enumerate().filter(|(_, n)| n.starts_with("FMaterialUniformExpression")).collect();
    println!("uniform expression names in the table: {:?}", kinds.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>());
    let mut hits = vec![];
    for i in 4..x.len().saturating_sub(8) {
        let idx = i32::from_le_bytes(x[i..i + 4].try_into().unwrap()); let num = i32::from_le_bytes(x[i + 4..i + 8].try_into().unwrap());
        if num == 0 && kinds.iter().any(|(k, _)| *k as i32 == idx) { hits.push(i); }
    }
    println!("{} FName hits; first {:?}", hits.len(), &hits[..hits.len().min(12)]);
    for h in hits.iter().take(3) { let cnt = i32::from_le_bytes(x[h - 4..*h].try_into().unwrap()); println!("  at {h}: preceding count {cnt}"); }
    match fn_dxbc_cli::uniform::parse_set(&x, &p.names) { Ok(s) => println!("parse_set ok: {} vectors {} scalars {} tex", s.vectors.len(), s.scalars.len(), s.textures2d.len()), Err(e) => println!("parse_set: {e}") }
    Ok(())
}
