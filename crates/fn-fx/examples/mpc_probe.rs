//! Research probe (SKY LIGHT, 2026-10-06): which cooked programs of the given materials read a material parameter collection, which collection it is
//! (`MaterialParameterCollectionInfos[N]` = `MaterialCollectionN`) and which float4 slots of it the DXBC reads (`cbS[i]` with S = that uniform buffer's slot).
//! `mpc_probe <material object path>...` (FortniteMaterialParameters: 59 scalars = 15 float4s, then one float4 per vector: SkyLightColorAndIntensity = 41).
use fn_fx::material::Material;
use std::collections::BTreeSet;
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    for path in std::env::args().skip(1) {
        let m = match Material::load(&set, &path) { Ok(m) => m, Err(e) => { println!("{path}: LOAD FAILED {e}"); continue } };
        // the collections in MaterialCollectionN order (the root material's MaterialParameterCollectionInfos)
        let root = fn_fx::data::PackageJson::load(&set, fn_fx::data::long_package(&m.base_path)).ok();
        let infos: Vec<String> = root.as_ref().and_then(|r| r.exports.iter().find(|e| e.class == "Material")).and_then(|e| e.props.get("MaterialParameterCollectionInfos")).and_then(|v| v.as_array()).map(|a| a.iter().map(|i| i.get("ParameterCollection").and_then(|p| p.as_str()).unwrap_or("?").rsplit('.').next().unwrap_or("?").to_owned()).collect()).unwrap_or_default();
        let mut lines: BTreeSet<String> = BTreeSet::new();
        for p in m.cooked.programs.iter().filter(|p| p.shader.major >= 5) {
            let Some(names) = &p.uniform_buffer_names else { continue };
            for (slot, n) in names.iter().enumerate() {
                let Some(k) = n.strip_prefix("MaterialCollection").and_then(|k| k.parse::<usize>().ok()) else { continue };
                let asm = fn_dxbc::disasm::disassemble(&p.blob.bytes).unwrap_or_default();
                let pat = format!("cb{slot}[");
                let mut idx: BTreeSet<u32> = BTreeSet::new();
                let mut rest = asm.as_str();
                while let Some(at) = rest.find(&pat) { let tail = &rest[at + pat.len()..]; if let Some(end) = tail.find(']') { if let Ok(i) = tail[..end].parse::<u32>() { idx.insert(i); } } rest = &rest[at + pat.len()..]; }
                let mut sw: BTreeSet<String> = BTreeSet::new(); let mut r2 = asm.as_str(); while let Some(at) = r2.find(&pat) { let tail = &r2[at + pat.len()..]; if let Some(end) = tail.find(']') { let comp: String = tail[end + 1..].chars().skip(1).take_while(|c| "xyzw".contains(*c)).collect(); sw.insert(format!("{}.{}", &tail[..end], comp)); } r2 = &r2[at + pat.len()..]; }
                lines.insert(format!("  {} / {}: {n} = {} reads float4 {:?} swizzles {:?}", p.vertex_factory, p.shader_type, infos.get(k).cloned().unwrap_or_else(|| "?".into()), idx, sw));
            }
        }
        println!("== {path} (collections {infos:?})\n  shader map ParameterCollections {:?}\n  resolved (Material::collections) {:?}", m.uniform.parameter_collections, m.collections);
        for l in &lines { println!("{l}"); }
    }
    Ok(())
}
