//! Research probe (MATERIAL PARAMETERS, 2026-10-06): which cooked SM5 programs of the given materials read `View` float4 125..128 (DiffuseOverrideParameter,
//! SpecularOverrideParameter, NormalOverrideParameter, RoughnessOverrideParameter at 128.xy), i.e. which ones the FSceneView override defaults change.
//! `view_override_probe <material object path>...`
use fn_fx::material::Material;
use std::collections::BTreeSet;
fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    for path in std::env::args().skip(1) {
        let m = match Material::load(&set, &path) { Ok(m) => m, Err(e) => { println!("{path}: LOAD FAILED {e}"); continue } };
        let mut hits: BTreeSet<String> = BTreeSet::new();
        for p in m.cooked.programs.iter().filter(|p| p.shader.major >= 5) {
            let Some(vi) = p.uniform_buffer_names.as_ref().and_then(|n| n.iter().position(|n| n == "View")) else { continue };
            let asm = fn_dxbc::disasm::disassemble(&p.blob.bytes).unwrap_or_default();
            for slot in 125..=128 {
                let pat = format!("cb{vi}[{slot}].");
                let mut rest = asm.as_str();
                while let Some(at) = rest.find(&pat) {
                    let sw: String = rest[at + pat.len()..].chars().take_while(|c| "xyzw".contains(*c)).collect();
                    // 128.zw are the previous-frame times, not an override
                    if slot != 128 || sw.contains('x') || sw.contains('y') { hits.insert(format!("{} / {}: View[{slot}].{sw}", p.vertex_factory, p.shader_type)); }
                    rest = &rest[at + pat.len()..];
                }
            }
        }
        println!("== {path} ({} / {}): {} program reads", m.flags.blend, m.flags.shading, hits.len());
        for h in hits.iter().filter(|h| h.contains("BasePass") || h.contains("Translucen") || h.contains("Forward")).take(8) { println!("  {h}"); }
    }
    Ok(())
}
