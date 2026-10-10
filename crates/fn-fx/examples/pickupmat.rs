//! Research tool: what the cooked package of a pickup-effect material holds (flags, instance parameters, uniform expression set, program permutations).
//! `pickupmat <object path>...`
use fn_fx::material::Material;
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("--reads") { for p in &args[2..] { if let Err(e) = reads(&set, &args[1], p) { println!("  FAILED {e}"); } } return Ok(()); }
    for p in std::env::args().skip(1) {
        println!("==== {p}");
        let m = match Material::load(&set, &p) { Ok(m) => m, Err(e) => { println!("  LOAD FAILED: {e}"); continue } };
        println!("  base {}  flags {:?}", m.base_path, m.flags);
        println!("  overrides: scalars {:?} vectors {:?} textures {:?}", m.overrides.scalars, m.overrides.vectors, m.texture_overrides);
        println!("  referenced textures: {:?}", m.referenced_textures);
        println!("  functions: {:?}", m.functions);
        for (i, e) in m.uniform.vectors.iter().enumerate() { println!("  vec[{i}] {e:?}"); }
        for (i, e) in m.uniform.scalars.iter().enumerate() { println!("  scalar[{i}] {e:?}"); }
        for (i, e) in m.uniform.textures2d.iter().enumerate() { println!("  tex[{i}] {e:?}"); }
        for l in m.cooked.list() { println!("  prog {l}"); }
    }
    Ok(())
}

/// `pickupmat --reads <vf> <material>...`: constant buffer registers each base-pass program of the permutation reads, per named buffer.
pub fn reads(set: &fn_pak::PakSet, vf: &str, path: &str) -> Result<(), String> {
    use fn_dxbc::OperandType;
    let m = Material::load(set, path)?;
    println!("==== reads {path} / {vf}");
    for ty in ["TBasePassVSFNoLightMapPolicy", "TBasePassPSFNoLightMapPolicy"] {
        let Some(p) = m.cooked.find(vf, ty) else { println!("  {ty}: absent"); continue };
        let mut by: std::collections::BTreeMap<String, std::collections::BTreeSet<u32>> = Default::default();
        let mut attrs = vec![];
        for i in p.shader.body() {
            for o in i.operands.iter() {
                if o.ty.0 == OperandType::CONSTANT_BUFFER {
                    let slot = o.index.first().map(|x| x.imm as usize).unwrap_or(0);
                    let name = p.uniform_buffer_names.as_ref().and_then(|n| n.get(slot)).cloned().unwrap_or(format!("cb{slot}"));
                    by.entry(name).or_default().insert(o.index.get(1).map(|x| x.imm as u32).unwrap_or(0));
                }
            }
        }
        for d in p.shader.decls.inputs.iter().filter(|d| d.file == OperandType::INPUT) { attrs.push(d.reg); }
        println!("  {ty} outputs: {:?}", p.shader.decls.outputs.iter().map(|d| (d.file, d.reg, d.sysvalue)).collect::<Vec<_>>());
        println!("  {ty}: inputs regs {attrs:?}");
        for (k, v) in by { println!("    {k}: {v:?}"); }
    }
    Ok(())
}
