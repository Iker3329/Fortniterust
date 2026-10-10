//! Research probe (LANDSCAPE MATERIAL, 2026-10-07): a landscape material instance as `Material::load` resolves it: the root's referenced textures, the uniform
//! set's 2D texture expressions and the texture each slot resolves to. `landscape_mat_probe <object path>...`
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    for p in std::env::args().skip(1) {
        let m = fn_fx::material::Material::load(&set, &p)?;
        println!("==== {p}\nbase {} map {} programs {}", m.base_path, m.shader_map_package, m.cooked.programs.len());
        for (i, t) in m.referenced_textures.iter().enumerate() { println!("  ref[{i}] {t}"); }
        for (k, e) in m.uniform.textures2d.iter().enumerate() { println!("  Texture2D_{k}: {e:?} -> {:?}", m.texture_slot(k)); }
        println!("  texture overrides {:?}", m.texture_overrides);
        println!("  functions {:?}", m.functions);
    }
    Ok(())
}
