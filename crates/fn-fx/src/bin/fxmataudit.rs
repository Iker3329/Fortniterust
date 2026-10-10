//! Research tool (FOLIAGE / CHARACTER MATERIALS audit, 2026-10-05): `fxmataudit <material object path>...` prints, per material, what the COOKED data says:
//! the instance chain's shader-map owner, the effective flags (`Material::load`: bOverride-gated base property overrides), the root `UMaterial`'s connected
//! `WorldPositionOffset` / `SubsurfaceColor` inputs, which vertex factories have a base-pass VS / PS, the GBuffer targets the base-pass PS writes (o5 = GBufferD
//! with r.BasePassOutputsVelocity=1), the uniform buffers the base-pass VS reads (Material / MaterialCollectionN = vertex-stage material expressions), and the
//! shadow-depth / depth-only programs. Output: one markdown table row per material (`--md`) or a detailed dump.
use fn_dxbc::OperandType;

const VFS: [&str; 3] = ["FLocalVertexFactory", "FInstancedStaticMeshVertexFactory", "TGPUSkinVertexFactoryfalse"];

fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let md = args.iter().any(|a| a == "--md");
    // `--wid <WID package>...`: the material slots of each item definition's `WeaponMeshOverride` skeletal mesh (what the held weapon draws with)
    if args.first().map(String::as_str) == Some("--wid") {
        for wid in &args[1..] {
            let pkg = fn_fx::data::PackageJson::load(&set, wid)?;
            let Some(mesh) = pkg.exports.iter().find(|e| e.class.ends_with("ItemDefinition")).and_then(|d| d.props.get("WeaponMeshOverride")).and_then(|v| v.as_str()).map(str::to_owned) else { println!("{wid}: no WeaponMeshOverride"); continue };
            let long = fn_fx::data::long_package(&mesh);
            let (a, x, b) = fn_fx::data::package_files(&set, long)?;
            let p = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| format!("{long}: {e}"))?;
            let sk = fn_mesh::SkeletalMesh::parse(&p).map_err(|e| format!("{long}: {e}"))?;
            for (i, m) in sk.materials.iter().enumerate() { println!("{wid} {mesh} slot {i} {}", m.material.clone().unwrap_or_default()); }
        }
        return Ok(());
    }
    for p in args.iter().filter(|a| !a.starts_with("--")) {
        let m = match fn_fx::material::Material::load(&set, p) { Ok(m) => m, Err(e) => { println!("| `{p}` | LOAD FAILED: {e} |"); continue } };
        // the material inputs (WorldPositionOffset, SubsurfaceColor, ...) are WITH_EDITORONLY_DATA and stripped by the cooker: vertex-stage material code is read off the
        // cooked base-pass VS instead (it binds `Material` / `MaterialCollectionN` only when the material has vertex expressions: WPO, customized UVs)
        let root = fn_fx::data::PackageJson::load(&set, fn_fx::data::long_package(&m.base_path)).ok();
        let rm = root.as_ref().and_then(|r| r.exports.iter().find(|e| e.class == "Material"));
        let usage: Vec<String> = rm.map(|e| e.props.as_object().map(|o| o.iter().filter(|(k, v)| k.starts_with("bUsedWith") && v.as_bool() == Some(true)).map(|(k, _)| k.trim_start_matches("bUsedWith").to_owned()).collect()).unwrap_or_default()).unwrap_or_default();
        // MaterialParameterCollections the shader-map package imports (the uniform set references them by StateId only)
        let mut mpcs: Vec<String> = vec![];
        for pk in [m.shader_map_package.clone(), fn_fx::data::long_package(&m.base_path).to_owned()] { for x in fn_uasset::PackageData::load(&set, &pk).ok().and_then(|pd| pd.package().ok().map(|p| p.imports.iter().enumerate().filter(|(_, i)| i.class_name == "MaterialParameterCollection").map(|(k, _)| p.object_path(fn_uasset::PackageIndex(-(k as i32) - 1))).collect::<Vec<String>>())).unwrap_or_default() { if !mpcs.contains(&x) { mpcs.push(x); } } }
        let vs_mat = |vf: &str| m.cooked.find(vf, "TBasePassVSFNoLightMapPolicy").and_then(|v| v.uniform_buffer_names.clone()).map_or(false, |u| u.iter().any(|n| n == "Material" || n.starts_with("MaterialCollection")));
        let wpo = VFS.iter().any(|vf| vs_mat(vf));
        let mut vf_rows = vec![];
        for vf in VFS {
            let vs = m.cooked.find(vf, "TBasePassVSFNoLightMapPolicy");
            let ps = m.cooked.find(vf, "TBasePassPSFNoLightMapPolicySkylight").or_else(|| m.cooked.find(vf, "TBasePassPSFNoLightMapPolicy"));
            if vs.is_none() && ps.is_none() { continue }
            let outs: Vec<u32> = ps.map(|p| p.shader.decls.outputs.iter().filter(|d| d.file == OperandType::OUTPUT).map(|d| d.reg).collect()).unwrap_or_default();
            let vs_ubs = vs.and_then(|v| v.uniform_buffer_names.clone()).unwrap_or_default();
            let vs_tex = vs.map(|v| v.shader.decls.resources.len()).unwrap_or(0);
            let shadow_vs = m.cooked.programs.iter().filter(|q| q.vertex_factory == vf && q.shader_type.starts_with("TShadowDepthVS")).map(|q| q.shader_type.clone()).collect::<Vec<_>>();
            let shadow_ps = m.cooked.programs.iter().filter(|q| q.vertex_factory == vf && q.shader_type.starts_with("TShadowDepthPS")).count();
            let depth_ps = m.cooked.find(vf, "FDepthOnlyPS").is_some();
            vf_rows.push((vf, vs.is_some(), ps.is_some(), outs, vs_ubs, vs_tex, shadow_vs, shadow_ps, depth_ps));
        }
        if md {
            let vfs = vf_rows.iter().map(|(vf, vs, ps, outs, ubs, _, sh, shp, dp)| {
                let short = vf.trim_start_matches('F').trim_end_matches("VertexFactory").replace("TGPUSkinVertexFactoryfalse", "GPUSkin").replace("TGPUSkin", "GPUSkin");
                format!("{short}: VS {} PS {} o{:?} VS-UB [{}] shadow {}{} depth-PS {}", yn(*vs), yn(*ps), outs, ubs.join(","), sh.len(), if *shp > 0 { "+PS" } else { "" }, yn(*dp))
            }).collect::<Vec<_>>().join("<br>");
            println!("| `{}` | `{}` | `{}` | {} | {} | {} | VS material code {} | {} | {} |", short(&m.path), short(&m.base_path), short(&m.shader_map_package), m.flags.shading.trim_start_matches("MSM_"), m.flags.blend.trim_start_matches("BLEND_"), if m.flags.two_sided { "2-sided" } else { "1-sided" }, yn(wpo), if mpcs.is_empty() { "-".to_owned() } else { mpcs.iter().map(|p| short(p).to_owned()).collect::<Vec<_>>().join(", ") }, if vfs.is_empty() { "NO SHADER MAP".into() } else { vfs });
        } else {
            println!("=== {}\n  root {}\n  shader map {}\n  flags {:?}\n  base-pass VS material code {wpo}, MPC imports {mpcs:?}\n  usage {usage:?}", m.path, m.base_path, m.shader_map_package, m.flags);
            for (vf, vs, ps, outs, ubs, tex, sh, shp, dp) in &vf_rows { println!("  {vf}: base VS {vs} PS {ps} PS outputs {outs:?} VS UBs {ubs:?} VS resources {tex} shadow VS {sh:?} shadow PS {shp} depth PS {dp}"); }
            let names = |es: &Vec<fn_fx::uniform::Expr>| es.iter().map(|e| format!("{e:?}").chars().take(90).collect::<String>()).collect::<Vec<_>>();
            println!("  uniform vectors {:?}\n  uniform scalars {:?}\n  per-frame scalars {} vectors {}", names(&m.uniform.vectors), names(&m.uniform.scalars), m.uniform.per_frame_scalars.len(), m.uniform.per_frame_vectors.len());
            println!("  programs: {:?}", m.cooked.list());
        }
    }
    Ok(())
}

fn yn(b: bool) -> &'static str { if b { "yes" } else { "no" } }
fn short(p: &str) -> &str { p.rsplit('/').next().unwrap_or(p).split('.').next().unwrap_or(p) }
