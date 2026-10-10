//! Research probe (MATERIAL PARAMETERS, 2026-10-06): the scene-lit WGSL fn-fx assembles for a material's base pass (`assemble_scene_lit` with the renderer's
//! default prelude), written to a file, plus which tail it ends in (`fx_scene_lit` / `fx_scene_lit_sm` = the GBufferD shading-model resolve).
//! `scene_lit_probe <material object path> <out.wgsl> [vertex factory] [vs type] [ps type]`
use fn_fx::material::Material;
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (path, out) = (a.first().ok_or("material path")?, a.get(1).ok_or("out file")?);
    let vf = a.get(2).map(String::as_str).unwrap_or("FLocalVertexFactory");
    let (vs_t, ps_t) = (a.get(3).map(String::as_str).unwrap_or("TBasePassVSFNoLightMapPolicy"), a.get(4).map(String::as_str).unwrap_or("TBasePassPSFNoLightMapPolicy"));
    let m = Material::load(&set, path)?;
    let vs = m.cooked.find(vf, vs_t).ok_or("no vs")?;
    let ps = m.cooked.find(vf, ps_t).ok_or("no ps")?;
    let clip = if m.flags.blend == "BLEND_Masked" && !ps.shader.body().any(|i| i.op == fn_dxbc::Opcode::Discard) { m.cooked.find(vf, "FDepthOnlyPS") } else { None };
    let asm = fn_fx::program::assemble_scene_lit(vs, ps, clip, &fn_render::fx::default_scene_lit_prelude())?;
    std::fs::write(out, &asm.wgsl).map_err(|e| e.to_string())?;
    println!("{path}: blend {} shading {} clip {} | ubs ps {:?} | tail fx_scene_lit_sm {} fx_scene_lit {} | outputs {:?}", m.flags.blend, m.flags.shading, clip.is_some(), ps.uniform_buffer_names,
        asm.wgsl.contains("return fx_scene_lit_sm("), asm.wgsl.contains("return fx_scene_lit("), ps.shader.decls.outputs.iter().map(|d| d.reg).collect::<Vec<_>>());
    Ok(())
}
