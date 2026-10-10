//! Research probe (LANDSCAPE MATERIAL, 2026-10-07): what an `Athena_Terrain_LS_xx` sublevel holds for the landscape material: the LandscapeComponents (with their
//! material instance, heightmap / weightmap textures and scale-bias), the `LandscapeMaterialInstanceConstant`s and the cooked programs of their inline shader maps
//! (owner export by serial range, vertex factory, shader type, loose parameters).
//! `landscape_probe [long umap path] [--comp N] [--mic N] [--asm <dir>]`
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let long = args.iter().find(|a| a.starts_with('/')).cloned().unwrap_or_else(|| "/Game/Athena/Maps/Landscape/Athena_Terrain_LS_00".into());
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let a = set.read_game(&long, "umap").map_err(|e| e.to_string())?.into_owned();
    let x = set.read_game(&long, "uexp").map_err(|e| e.to_string())?.into_owned();
    let b = set.read_game(&long, "ubulk").map(|c| c.into_owned()).unwrap_or_default();
    let p = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| e.to_string())?;
    let header = p.summary.total_header_size as usize;
    let mut hist: std::collections::BTreeMap<String, usize> = Default::default();
    for i in 0..p.exports.len() { *hist.entry(p.export_class(i).to_string()).or_default() += 1; }
    println!("{long}: {} exports, header {header}, uexp {} bytes\n{hist:?}", p.exports.len(), x.len());
    let data = fn_uasset::PackageData::load(&set, &long).map_err(|e| e.to_string())?;
    let pkg = data.package().map_err(|e| e.to_string())?;
    let comps: Vec<usize> = (0..pkg.exports.len()).filter(|&i| pkg.export_class_name(i) == "LandscapeComponent").collect();
    let mics: Vec<usize> = (0..pkg.exports.len()).filter(|&i| pkg.export_class_name(i) == "LandscapeMaterialInstanceConstant").collect();
    let ci: usize = arg("--comp").and_then(|s| s.parse().ok()).unwrap_or(0);
    if let Some(&c) = comps.get(ci) {
        let j = pkg.read_export(c).map_err(|e| e.to_string())?.props.to_json(&pkg);
        println!("\n== LandscapeComponent {} ({}):\n{}", pkg.exports[c].name, c, serde_json::to_string_pretty(&j).unwrap());
    }
    let mi: usize = arg("--mic").and_then(|s| s.parse().ok()).unwrap_or(0);
    if let Some(&m) = mics.get(mi) {
        let j = pkg.read_export(m).map_err(|e| e.to_string())?.props.to_json(&pkg);
        let s = serde_json::to_string_pretty(&j).unwrap();
        println!("\n== MIC {} ({}), serial {} + {}:\n{}", pkg.exports[m].name, m, p.exports[m].serial_offset, p.exports[m].serial_size, if s.len() > 20000 { &s[..20000] } else { &s });
    }
    // parents of every MIC
    let mut parents: std::collections::BTreeMap<String, usize> = Default::default();
    for &m in &mics { let j = pkg.read_export(m).map_err(|e| e.to_string())?.props.to_json(&pkg); *parents.entry(j.get("Parent").and_then(|v| v.as_str()).unwrap_or("?").to_owned()).or_default() += 1; }
    println!("\nMIC parents: {parents:?}");
    if args.iter().any(|a| a == "--no-programs") { return Ok(()); }
    let t0 = std::time::Instant::now();
    let cooked = fn_fx::cooked::Cooked::parse(&x, &p.names, header)?;
    println!("\n{} programs parsed in {:.1}s", cooked.programs.len(), t0.elapsed().as_secs_f32());
    let owner = |stream: usize| -> Option<usize> { let abs = stream + header; (0..p.exports.len()).find(|&i| { let (o, s) = (p.exports[i].serial_offset as usize, p.exports[i].serial_size as usize); abs >= o && abs < o + s }) };
    let mut per: std::collections::BTreeMap<(String, String), usize> = Default::default();
    let mut per_owner: std::collections::BTreeMap<String, usize> = Default::default();
    for pr in &cooked.programs { if pr.shader.major >= 5 { *per.entry((pr.vertex_factory.clone(), pr.shader_type.clone())).or_default() += 1; } *per_owner.entry(owner(pr.stream).map_or("?".into(), |o| format!("{} {}", p.export_class(o), p.exports[o].object_name))).or_default() += 1; }
    for ((vf, ty), n) in &per { println!("  {n:4} {vf} / {ty}"); }
    println!("owners: {} distinct; first 10: {:?}", per_owner.len(), per_owner.iter().take(10).collect::<Vec<_>>());
    for ty in ["TBasePassVSFNoLightMapPolicy", "TBasePassPSFNoLightMapPolicy", "TDepthOnlyVS<false>", "TShadowDepthVSVertexShadowDepth_OutputDepthfalse"] { if let Some(&m) = mics.get(mi) { println!("same across maps {ty}: {:?}", same_across_maps(&cooked, |s| owner(s) == Some(m), "FLandscapeVertexFactory", ty)); } }
    // the base pass pair of the chosen MIC
    let Some(&m) = mics.get(mi) else { return Ok(()) };
    let asm_dir = arg("--asm").map(std::path::PathBuf::from);
    let (o, s) = (p.exports[m].serial_offset as usize - header, p.exports[m].serial_size as usize);
    match fn_fx::uniform::parse_set(&x[o..o + s], &p.names) {
        Ok(u) => println!("\nuniform set of MIC {mi}: vectors {} scalars {} tex2d {} {:?}", u.vectors.len(), u.scalars.len(), u.textures2d.len(), u.textures2d),
        Err(e) => println!("\nuniform set of MIC {mi}: {e}"),
    }
    for pr in cooked.programs.iter().filter(|pr| owner(pr.stream) == Some(m) && pr.shader.major >= 5 && pr.vertex_factory == "FLandscapeVertexFactory" && (pr.shader_type.starts_with("TBasePass") || pr.shader_type.contains("DepthOnly") || pr.shader_type.contains("ShadowDepth"))) {
        println!("\n---- {} / {} (stream {})", pr.vertex_factory, pr.shader_type, pr.stream);
        if let Ok(c) = fn_dxbc::Container::parse(&pr.blob.bytes) {
            if let Some(s) = c.input_signature() { for e in &s.elements { println!("    in  {}{} reg {} mask {:#x} comp {}", e.name, e.semantic_index, e.register, e.mask, e.comp_type); } }
        }
        println!("  outputs {:?}", fn_fx::cooked::container_output_registers(&pr.blob.bytes));
        println!("  ubs {:?}", pr.uniform_buffer_names);
        println!("  bindings {:?}", pr.bindings.iter().map(|b| (format!("{:?}", b.kind), b.binding.register, b.binding.member)).collect::<Vec<_>>());
        println!("  cbuffers {:?}", pr.shader.decls.cbuffers.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>());
        match &pr.params {
            None => println!("  params: NONE"),
            Some(q) => println!("  params: matcb {} vf {:?} derived {} bytes {:?}\n  vf params {} bytes {:?}\n  fog {:?}", q.material_cb_size, q.vertex_factory, q.derived.len(), q.derived, q.vf_params.len(), q.vf_params, q.height_fog(&pr.shader_type).map(|f| f.values.iter().map(|v| (v.buffer_index, v.base_index, v.num_bytes)).collect::<Vec<_>>())),
        }
        if let Some(d) = &asm_dir {
            let _ = std::fs::create_dir_all(d);
            let name = format!("{}_{}", pr.shader_type, pr.stream);
            if let Ok(t) = fn_dxbc::disasm::disassemble(&pr.blob.bytes) { let _ = std::fs::write(d.join(format!("{name}.asm")), t); }
        }
    }
    Ok(())
}
// (appended) are the SM5 base-pass programs of one MIC byte-identical across its shader maps (quality levels)?
#[allow(dead_code)]
pub fn same_across_maps(c: &fn_fx::cooked::Cooked, owner: impl Fn(usize) -> bool, vf: &str, ty: &str) -> (usize, bool) {
    let v: Vec<&Vec<u8>> = c.programs.iter().filter(|p| owner(p.stream) && p.shader.major >= 5 && p.vertex_factory == vf && p.shader_type == ty).map(|p| &p.blob.bytes).collect();
    (v.len(), v.windows(2).all(|w| w[0] == w[1]))
}
