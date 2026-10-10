//! Research probe (GRASS, 2026-10-06): what the cooked `M_FieldGrassMatching` shader map holds for the instanced static-mesh vertex factory
//! (`FInstancedStaticMeshVertexFactory`, the foliage components' VF): the program list, each program's input / output signature, uniform buffer names,
//! serialized loose parameters and (with `--asm <dir>`) the disassembly + translated WGSL of the base-pass / depth programs.
//! `grass_probe [material object path] [--asm <dir>]`
use fn_fx::material::Material;
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.iter().find(|a| a.starts_with('/')).cloned().unwrap_or_else(|| "/Game/Environments/Foliage/Materials/M_FieldGrassMatching.M_FieldGrassMatching".into());
    let asm_dir = args.iter().position(|a| a == "--asm").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
    let m = Material::load(&set, &path)?;
    println!("==== {path}\nshader map package {}\nflags {:?}", m.shader_map_package, m.flags);
    println!("uniform: vectors {} scalars {} tex2d {} per-frame scalars {} vectors {}", m.uniform.vectors.len(), m.uniform.scalars.len(), m.uniform.textures2d.len(), m.uniform.per_frame_scalars.len(), m.uniform.per_frame_vectors.len());
    for (k, _) in m.uniform.textures2d.iter().enumerate() { println!("  Texture2D_{k} -> {:?}", m.texture_slot(k)); }
    println!("scalar overrides {:?}\nvector overrides {:?}", m.overrides.scalars, m.overrides.vectors);
    let pk = m.packed(0.);
    for (i, c) in pk.cb.iter().enumerate() { println!("  Material cb[{i}] = {:?}", c); }
    println!("programs ({}):", m.cooked.programs.len());
    for p in &m.cooked.programs {
        let ubs = p.uniform_buffer_names.clone().unwrap_or_default();
        println!("  [{}] {} / {} sm{}.{} ubs {:?}", p.stream, p.vertex_factory, p.shader_type, p.shader.major, p.shader.minor, ubs);
    }
    let vf_filter = args.iter().position(|a| a == "--vf").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "FInstancedStaticMeshVertexFactory".into());
    for p in m.cooked.programs.iter().filter(|p| p.vertex_factory == vf_filter && p.shader.major >= 5) {
        println!("\n---- {} / {} (stream {})", p.vertex_factory, p.shader_type, p.stream);
        println!("  inputs  {:?}", fn_fx::cooked::container_input_registers(&p.blob.bytes));
        println!("  outputs {:?}", fn_fx::cooked::container_output_registers(&p.blob.bytes));
        if let Ok(c) = fn_dxbc::Container::parse(&p.blob.bytes) { if let Some(s) = c.input_signature() { for e in &s.elements { println!("    in  {}{} reg {} mask {:#x} comp {}", e.name, e.semantic_index, e.register, e.mask, e.comp_type); } } }
        println!("  ubs {:?}", p.uniform_buffer_names);
        println!("  bindings {:?}", p.bindings.iter().map(|b| (format!("{:?}", b.kind), b.binding.register, b.binding.member)).collect::<Vec<_>>());
        match &p.params {
            None => println!("  params: NONE"),
            Some(x) => println!("  params: matcb {} pfS {:?} pfV {:?} vf {:?} bpvs {:?} derived {} bytes\n  vf params {} bytes {:?}\n  instanced vf {:?}\n  fog {:?}\n  shadow {:?}", x.material_cb_size, x.per_frame_scalars.iter().map(|s| (s.buffer_index, s.base_index, s.num_bytes)).collect::<Vec<_>>(), x.per_frame_vectors.iter().map(|s| (s.buffer_index, s.base_index, s.num_bytes)).collect::<Vec<_>>(), x.vertex_factory, x.base_pass_vs(&p.shader_type), x.derived.len(), x.vf_params.len(), x.vf_params, x.instanced_vf(), x.height_fog(&p.shader_type).map(|f| f.values.iter().map(|v| (v.buffer_index, v.base_index, v.num_bytes)).collect::<Vec<_>>()), x.shadow_depth_vs(&p.shader_type)),
        }
        if let Some(d) = &asm_dir {
            let _ = std::fs::create_dir_all(d);
            let name = format!("{}_{}", p.shader_type, p.stream);
            if let Ok(t) = fn_dxbc::disasm::disassemble(&p.blob.bytes) { let _ = std::fs::write(d.join(format!("{name}.asm")), t); }
            if let Ok(w) = fn_fx::cooked::translated_function_wgsl(&p.shader, "f") { let _ = std::fs::write(d.join(format!("{name}.wgsl")), w); }
        }
    }
    Ok(())
}
