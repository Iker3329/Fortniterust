//! Research probe: the serialized loose shader parameters (`fn_fx::shader_params`) of a material's base-pass programs, next to its uniform expression set.
//! `shaderparams_probe <material object path>...`
use fn_fx::material::Material;
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    for path in std::env::args().skip(1) {
        let m = Material::load(&set, &path)?;
        println!("==== {path}: uniform vectors {} scalars {} tex2d {} per-frame scalars {} vectors {}", m.uniform.vectors.len(), m.uniform.scalars.len(), m.uniform.textures2d.len(), m.uniform.per_frame_scalars.len(), m.uniform.per_frame_vectors.len());
        for p in &m.cooked.programs {
            if p.shader.major < 5 || !p.shader_type.starts_with("TBasePass") { continue; }
            let globals: Vec<(usize, u32)> = p.uniform_buffer_names.as_ref().map(|n| n.iter().enumerate().filter(|(_, s)| s.is_empty()).map(|(i, _)| (i, 0)).collect()).unwrap_or_default();
            match &p.params {
                None => println!("  {} {}: NO PARAMS", p.shader_type, p.vertex_factory),
                Some(x) => println!("  {} {}: debug {:?} matcb {} pfS {:?} pfV {:?} prevS {} prevV {} vf {:?} dither {:?} derived {} bytes, bpvs {:?}, globals cbs {:?}", p.shader_type, p.vertex_factory, x.debug_counts, x.material_cb_size,
                    x.per_frame_scalars.iter().map(|s| (s.buffer_index, s.base_index, s.num_bytes)).collect::<Vec<_>>(), x.per_frame_vectors.iter().map(|s| (s.buffer_index, s.base_index, s.num_bytes)).collect::<Vec<_>>(),
                    x.per_frame_prev_scalars.len(), x.per_frame_prev_vectors.len(), x.vertex_factory, x.non_instanced_dither_lod_factor, x.derived.len(), x.base_pass_vs(&p.shader_type), globals),
            }
        }
    }
    Ok(())
}
