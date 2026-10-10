//! Dumps what a post-process material's `FFPostProcessMaterialPS_HighEnd` binds: uniform buffers, textures, samplers (probe for new post passes).
//!   ppprobe <material object path>
use fn_matshader::material::ResolvedMaterial;
fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("material path")?;
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let mat = ResolvedMaterial::load(&set, &path)?;
    println!("blend {} map {}", mat.flags.blend, mat.map_index);
    for p in mat.maps.programs.iter().filter(|p| p.map == mat.map_index) { println!("program {} vf '{}'", p.shader_type, p.vertex_factory); }
    let prog = mat.maps.find(mat.map_index, "", "FFPostProcessMaterialPS_HighEnd").ok_or("no post program")?;
    println!("ubos {:?}", prog.ubo_names);
    for b in &prog.bindings { println!("binding {:?} reg {} member {} buf '{}'", b.kind, b.binding.register, b.binding.member, b.buffer_name); }
    let body = fn_dxbc::codegen::translate_wgsl(&prog.shader, "ps_fx").map_err(|e| e.0)?;
    for l in body.lines().filter(|l| l.starts_with("@group")) { println!("{l}"); }
    std::fs::write("ppprobe.wgsl", &body).map_err(|e| e.to_string())?;
    println!("2d textures {:?}", mat.map().uniform.textures2d.len());
    for (i, v) in mat.map().uniform.vectors.iter().enumerate() { println!("vec {i}: {v:?}"); }
    for (i, v) in mat.map().uniform.scalars.iter().enumerate() { println!("scalar {i}: {v:?}"); }
    println!("packed {:?}", mat.packed(0.0).cb);
    Ok(())
}
