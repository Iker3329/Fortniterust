//! Research tool: `fxscan <long package path> [vertex factory] [shader type...]` lists the cooked programs (stream, SM, shader type, vertex factory) and, for the
//! named (vertex factory, shader type) pairs, the uniform buffers, resource bindings and the signatures.
use fn_fx::cooked::Cooked;
use fn_fx::data::package_files;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pkg = args.first().ok_or("package path")?;
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let (asset, uexp, ubulk) = package_files(&set, pkg)?;
    let p = fn_ueio::Package::parse(&asset, &uexp, &ubulk).map_err(|e| e.to_string())?;
    if std::env::var_os("FXSCAN_IMPORTS").is_some() {
        for (i, imp) in p.imports.iter().enumerate() { println!("import {:>3} {:<22} {}", i, imp.class_name, p.import_path(-(i as i32) - 1)); }
    }
    let cooked = Cooked::parse(&uexp, &p.names, p.summary.total_header_size as usize)?;
    if args.len() < 3 {
        for l in cooked.list() { println!("{l}"); }
        return Ok(());
    }
    let vf = &args[1];
    for t in &args[2..] {
        let Some(prog) = cooked.find(vf, t) else { println!("no program {vf} / {t}"); continue };
        println!("=== {} {} stream {} ({})", prog.shader_type, prog.vertex_factory, prog.stream, prog.shader.version_string());
        println!("uniform buffers: {:?}", prog.uniform_buffer_names);
        for b in &prog.bindings { println!("  binding {b:?}"); }
        println!("  table {:?}", prog.resource_table);
        println!("  cbuffer decls {:?}", prog.shader.decls.cbuffers);
        println!("  inputs {:?}", fn_fx::cooked::container_input_registers(&prog.blob.bytes));
        println!("  outputs {:?}", fn_fx::cooked::container_output_registers(&prog.blob.bytes));
    }
    Ok(())
}
