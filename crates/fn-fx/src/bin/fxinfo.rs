//! Research tool: `fxinfo <material object path>...` prints the flags, parameter overrides, referenced textures and packed material buffer.
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    for p in std::env::args().skip(1) {
        let m = fn_fx::material::Material::load(&set, &p)?;
        println!("=== {} (root {})", m.path, m.base_path);
        println!("flags {:?}", m.flags);
        println!("vector overrides {:?}\nscalar overrides {:?}\ntexture overrides {:?}", m.overrides.vectors, m.overrides.scalars, m.texture_overrides);
        println!("functions {:?}", m.functions);
        for (i, t) in m.referenced_textures.iter().enumerate() { println!("  RT[{i}] {t}"); }
        for (k, e) in m.uniform.textures2d.iter().enumerate() { println!("  Texture2D_{k}: {e:?} -> {:?}", m.texture_slot(k)); }
        let pk = m.packed(0.);
        println!("packed cb: {} vectors, {} scalars, {} registers", pk.vectors, pk.scalars, pk.registers);
        for (i, c) in pk.cb.iter().enumerate() { println!("  cb[{i}] = {:?}", c); }
        println!("programs: {}", m.cooked.programs.len());
    }
    Ok(())
}
