//! Generator for the Battle Royale character materials (task "BR character skin/hair/eyeshadow", see crates/fn-render/shaders/char_material_gen.wgsl).
//!
//! For each character material permutation (the package that owns the cooked shader map: `CharacterShader` = default permutation, `CharacterShader_WhiteAO`,
//! `CharacterShader_Gloss`) it picks the base-pass pixel shader `TBasePassPSFNoLightMapPolicy` of `TGPUSkinVertexFactory` (the skinned character's vertex
//! factory), translates it to WGSL with `fn-dxbc`, and emits the material constant buffer recipe (the cooked uniform expression set) as Rust.
//! Also translates the `Eyeshadow` modulate pixel shader.
//!
//! Usage: cargo run -p fn-dxbc-cli --release --example br_character_materials -- crates/fn-render/shaders/char_material_gen.wgsl crates/fn-render/shaders/eyeshadow_gen.wgsl crates/fn-pose/src/br_material_gen.rs [--list]
use fn_dxbc::codegen::translate_wgsl;
use fn_dxbc::{OperandType, ProgramType, Shader};
use fn_dxbc_cli::extract::scan_blobs;
use fn_dxbc_cli::record::{end_offset_candidates, fname_candidates};
use fn_dxbc_cli::spec::{default_install, package_bytes};
use fn_dxbc_cli::uigen::{cb_function, HELPERS};
use fn_dxbc_cli::uniform::{package_names, parse_set, Expr, UniformSet};
use std::collections::BTreeMap;
use std::fmt::Write;

struct Perm { id: &'static str, owner: &'static str, min_outputs: usize }
const PERMS: &[Perm] = &[
    Perm { id: "head", owner: "/Game/Packages/Fortress_SharedMaterials/Base_Material/CharacterShader", min_outputs: 6 },
    Perm { id: "hair", owner: "/Game/Packages/Fortress_SharedMaterials/MaterialInstances/CharacterShader_WhiteAO", min_outputs: 6 },
    Perm { id: "body", owner: "/Game/Packages/Fortress_SharedMaterials/MaterialInstances/CharacterShader_Gloss", min_outputs: 6 },
    Perm { id: "eyeshadow", owner: "/Game/Characters/Player/Common/Eyes/Materials/Eyeshadow", min_outputs: 1 },
];

fn used_cb(sh: &Shader) -> BTreeMap<u32, Vec<u32>> {
    let mut m: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for ins in sh.body() {
        for o in &ins.operands {
            if o.ty.is(OperandType::CONSTANT_BUFFER) && o.index.len() >= 2 {
                let (b, r) = (o.index[0].imm as u32, o.index[1].imm as u32);
                let e = m.entry(b).or_default();
                if !e.contains(&r) { e.push(r); }
            }
        }
    }
    for v in m.values_mut() { v.sort(); }
    m
}

fn replace_word(s: &str, from: &str, to: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while i < b.len() {
        if s[i..].starts_with(from) && (i == 0 || !ident(b[i - 1])) && (i + from.len() >= b.len() || !ident(b[i + from.len()])) {
            out.push_str(to);
            i += from.len();
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

struct Picked { header: Vec<u8>, id: usize, sh: Shader, end_name: String, vf: Vec<String>, same_as: Vec<usize>, ps_ver: String }

/// Base-pass PS `policy` of the skinned vertex factory with extra bone influences, SM5, writing the six GBuffer + velocity targets.
fn pick_ps(set: &fn_pak::PakSet, package: &str, policy: &str, min_outputs: usize) -> Result<(Picked, Vec<u8>, Vec<String>, usize), String> {
    let bytes = package_bytes(set, package)?;
    let ua = set.read_game(package, "uasset").map_err(|e| e.to_string())?;
    let names = package_names(&ua, &bytes)?;
    let pk = fn_uasset::package::Package::parse(&ua, &bytes).map_err(|e| e.to_string())?;
    let header = usize::try_from(pk.summary.total_header_size).map_err(|_| "header")?;
    let blobs = scan_blobs(&bytes);
    let mut found: Vec<Picked> = vec![];
    for b in &blobs {
        let Ok(sh) = Shader::parse(&b.bytes) else { continue };
        if sh.program_type != ProgramType::Pixel || sh.major != 5 || sh.decls.outputs.len() < min_outputs { continue }
        let Ok(stream) = fn_dxbc::extract::serialized_stream(&bytes, b, &sh) else { continue };
        let abs_end = header + stream.tail.end;
        let ends = end_offset_candidates(&bytes, &names, b.id.saturating_sub(2048), b.id, abs_end as u64)?;
        let Some(end) = ends.first() else { continue };
        if end.name != policy { continue }
        let vf: Vec<String> = fname_candidates(&bytes, &names, b.id.saturating_sub(2048), b.id)?.into_iter().map(|n| n.name).filter(|n| n.contains("VertexFactory")).collect();
        if !vf.iter().any(|n| n == "TGPUSkinVertexFactorytrue") { continue }
        found.push(Picked { header: b.header.clone(), id: b.id, ps_ver: sh.version_string(), end_name: end.name.clone(), vf, sh, same_as: vec![] });
    }
    if found.is_empty() { return Err(format!("{package}: no {policy} PS for TGPUSkinVertexFactorytrue")); }
    let first_prog = found[0].sh.instructions.iter().map(|i| (i.op, i.operands.clone(), i.data.clone())).collect::<Vec<_>>();
    let mut p = found.remove(0);
    for o in &found {
        let prog = o.sh.instructions.iter().map(|i| (i.op, i.operands.clone(), i.data.clone())).collect::<Vec<_>>();
        if prog == first_prog { p.same_as.push(o.id); } else { return Err(format!("{package}: PS candidates {} and {} differ", p.id, o.id)); }
    }
    Ok((p, bytes, names, header))
}


/// Pass shaders that decide a masked material's coverage (depth prepass, shadow depth, hit proxy, velocity, lightmap density). A plain `BLEND_Masked`
/// material whose coverage shaders ALL discard unconditionally (`discard_nz l(-1)`: the cooked opacity mask is below the clip value for every pixel)
/// draws nothing in any pass. Returns (per shader-type-name: (shaders, unconditionally discarding)), base-pass shaders (count, discarding).
fn discard_report(set: &fn_pak::PakSet, package: &str) -> Result<(BTreeMap<String, (usize, usize)>, (usize, usize)), String> {
    use fn_dxbc::Opcode;
    let bytes = package_bytes(set, package)?;
    let ua = set.read_game(package, "uasset").map_err(|e| e.to_string())?;
    let names = package_names(&ua, &bytes)?;
    let pk = fn_uasset::package::Package::parse(&ua, &bytes).map_err(|e| e.to_string())?;
    let header = usize::try_from(pk.summary.total_header_size).map_err(|_| "header")?;
    let mut cover: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut base = (0usize, 0usize);
    for b in scan_blobs(&bytes) {
        let Ok(sh) = Shader::parse(&b.bytes) else { continue };
        if sh.program_type != ProgramType::Pixel { continue }
        let Ok(stream) = fn_dxbc::extract::serialized_stream(&bytes, &b, &sh) else { continue };
        let ends = end_offset_candidates(&bytes, &names, b.id.saturating_sub(2048), b.id, (header + stream.tail.end) as u64)?;
        let Some(end) = ends.first() else { continue };
        let uncond = sh.body().any(|i| i.op == Opcode::Discard && i.test_nz() && i.operands.first().is_some_and(|o| o.is_imm() && o.imm.first() == Some(&0xFFFF_FFFF)));
        if end.name.starts_with("TBasePassPS") { base.0 += 1; base.1 += uncond as usize; continue }
        let e = cover.entry(end.name.clone()).or_default();
        e.0 += 1;
        e.1 += uncond as usize;
    }
    Ok((cover, base))
}
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out_wgsl = args.first().ok_or("out-wgsl (character perms)")?;
    let out_eye = args.get(1).ok_or("out-wgsl (eyeshadow)")?;
    let out_rs = args.get(2).ok_or("out-rs")?;
    let list = args.iter().any(|a| a == "--list");
    let set = fn_pak::PakSet::open_install(default_install()).map_err(|e| e.to_string())?;
    let mut wgsl = String::new();
    let mut eye = String::new();
    let mut rs = String::new();
    wgsl.push_str("// GENERATED by `cargo run -p fn-dxbc-cli --example br_character_materials` from the 1.8 cooked materials. Do not edit by hand.\n");
    wgsl.push_str("// Each `ue_<perm>_ps` is the compiled base-pass pixel shader (TBasePassPSFNoLightMapPolicy of TGPUSkinVertexFactory, ps_5_0) of the material\n// permutation, translated instruction for instruction by fn-dxbc. Outputs: o0 emissive (scene colour), o1 normal*0.5+0.5 (+ per-object data), o2 metallic/specular/\n// roughness/shading model id, o3 base colour + AO, o4/o5 velocity and selective outputs.\n");
    wgsl.push_str("diagnostic(off, derivative_uniformity);\n\n");
    eye.push_str("// GENERATED by `cargo run -p fn-dxbc-cli --example br_character_materials` from the 1.8 cooked materials. Do not edit by hand.\n// `eyeshadow_ps`: the compiled base-pass pixel shader of the plain BLEND_Modulate / MSM_Unlit material `Eyeshadow` (TBasePassPSFNoLightMapPolicy of TGPUSkinVertexFactory, ps_5_0),\n// translated instruction for instruction by fn-dxbc. Inputs v[2].x vertex colour R, v[3].xy UV0, v[4].w fog fade, v[5].xyz translated world position. Output o0: the colour that is MULTIPLIED into the scene.\n");
    eye.push_str("diagnostic(off, derivative_uniformity);\n\n");
    rs.push_str("//! GENERATED by `cargo run -p fn-dxbc-cli --example br_character_materials` from the 1.8 cooked materials. Do not edit by hand.\n//! Per material permutation: the material constant buffer recipe (cooked uniform expression set; `p(name, cooked default)` looks every parameter up by name)\n//! and the register -> texture binding of the translated base-pass pixel shader.\n#![allow(dead_code, clippy::all, unused_parens)]\n\n");
    rs.push_str(HELPERS);
    rs.push_str("\n#[derive(Clone, Copy, Debug)]\npub struct TexSlot { pub register: u8, pub param: Option<&'static str>, pub index: i32 }\n#[derive(Clone, Copy, Debug)]\npub struct Perm { pub id: &'static str, pub owner: &'static str, pub shader_id: usize, pub cb_regs: usize, pub textures: &'static [TexSlot] }\n\n");
    let mut perm_entries = vec![];
    for perm in PERMS {
        let (p, bytes, names, _) = pick_ps(&set, perm.owner, "TBasePassPSFNoLightMapPolicy", perm.min_outputs)?;
        let us: UniformSet = parse_set(&bytes, &names)?;
        let used = used_cb(&p.sh);
        eprintln!("== {} ({}) shader_{} {} end={} vf={:?} identical streams {:?}", perm.id, perm.owner, p.id, p.ps_ver, p.end_name, p.vf, p.same_as);
        eprintln!("   used cb regs: {:?}", used);
        eprintln!("   textures2d list: {:?}", us.textures2d.iter().enumerate().map(|(i, e)| match e { Expr::TextureParameter { name, index, .. } => format!("t{i}={name}#{index}"), Expr::Texture { index, .. } => format!("t{i}=#{index}"), _ => format!("t{i}=?") }).collect::<Vec<_>>());
        eprintln!("   resources used: {:?} samplers {:?}", p.sh.decls.resources.iter().map(|r| r.slot).collect::<Vec<_>>(), p.sh.decls.samplers.iter().map(|r| r.slot).collect::<Vec<_>>());
        match fn_dxbc::unreal_packet::ResourceTable::parse(&p.header) {
            Ok(t) => eprintln!("   resource table: bits {:#x} texture_map bindings (buffer, member, register) {:?}; srv {:?} sampler {:?}", t.bits, t.bindings(&t.texture_map).map(|b| b.iter().map(|x| (x.buffer, x.member, x.register)).collect::<Vec<_>>()), t.bindings(&t.srv_map).map(|b| b.len()), t.bindings(&t.sampler_map).map(|b| b.iter().map(|x| (x.buffer, x.member, x.register)).collect::<Vec<_>>())),
            Err(e) => eprintln!("   resource table: {e}"),
        }
        if list { continue }
        let cb3 = us.vectors.len() + us.scalars.len().div_ceil(4);
        let name = format!("{}_ps", perm.id);
        let text = translate_wgsl(&p.sh, &name).map_err(|e| e.0)?;
        // keep only the translated function; bindings become module-level privates / the renderer's own textures
        let start = text.find(&format!("fn {name}(")).ok_or("no fn")?;
        let end = text[start..].find("\nstruct ").map(|e| start + e).ok_or("no struct")?;
        let mut func = text[start..end].to_string();
        let nin = func.split("array<vec4<f32>,").nth(1).and_then(|s| s.trim_start().split('>').next()).and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0);
        for b in 0..4 { func = replace_word(&func, &format!("cb{b}"), &format!("{}_cb{b}", perm.id)); }
        // register tN / sN -> the renderer's own bindings: material parameters Diffuse / Normals / SpecularMasks / M are the primitive's textures (bind group 1), every
        // other texture is a fixed slot `fx_tex<index>` / `fx_samp<index>` (bind group 3). Register tN is `textures2d[N]` (the resource table maps Material-buffer member 2N to tN
        // and member 2N+1 to sN, checked above).
        for t in (0..8).rev() {
            let (tn, sn) = match us.textures2d.get(t) {
                Some(Expr::TextureParameter { name, .. }) => (match name.as_str() { "Diffuse" => "t_diffuse", "Normals" => "t_normal", "SpecularMasks" => "t_spec", "M" => "t_mask", o => return Err(format!("unmapped texture parameter {o}")) }.to_string(), "tex_samp".to_string()),
                Some(Expr::Texture { index, .. }) => (format!("fx_tex{index}"), format!("fx_samp{index}")),
                _ => (format!("unbound_t{t}"), format!("unbound_s{t}")),
            };
            func = replace_word(&func, &format!("t{t}"), &tn);
            func = replace_word(&func, &format!("s{t}"), &sn);
        }
        let target = if perm.id == "eyeshadow" { &mut eye } else { &mut wgsl };
        let _ = writeln!(target, "// ==== {} : {} shader_{} ({}, {}; identical programs also at {:?}) ====", perm.id, perm.owner, p.id, p.ps_ver, p.end_name, p.same_as);
        for b in p.sh.decls.cbuffers.iter() { let _ = writeln!(target, "var<private> {}_cb{}: array<vec4<f32>, {}>;", perm.id, b.slot, b.size); }
        let _ = writeln!(target, "{}", func.trim_end());
        target.push('\n');
        let _ = writeln!(target, "// {}: inputs v[0..{}]", perm.id, nin);
        // rust side
        let _ = writeln!(rs, "// ==== {} ({} shader_{}) ====", perm.id, perm.owner, p.id);
        rs.push_str(&cb_function(&format!("{}", perm.id), &us)?);
        let slots: Vec<String> = us.textures2d.iter().enumerate().map(|(i, e)| match e {
            Expr::TextureParameter { name, index, .. } => format!("TexSlot {{ register: {i}, param: Some({name:?}), index: {index} }}"),
            Expr::Texture { index, .. } => format!("TexSlot {{ register: {i}, param: None, index: {index} }}"),
            _ => String::new(),
        }).collect();
        let _ = writeln!(rs, "pub const {}_TEXTURES: &[TexSlot] = &[{}];", perm.id.to_uppercase(), slots.join(", "));
        perm_entries.push(format!("    Perm {{ id: {:?}, owner: {:?}, shader_id: {}, cb_regs: {}, textures: {}_TEXTURES }},", perm.id, perm.owner, p.id, cb3, perm.id.to_uppercase()));
    }
    // plain masked materials whose coverage shaders all discard unconditionally
    let mut always = vec![];
    for package in ["/Game/Athena/Heroes/Meshes/Bodies/M_Athena_InvisibleGrenades"] {
        let (cover, base) = discard_report(&set, package)?;
        eprintln!("== {package}: coverage shaders {cover:?}; base-pass shaders (total, discarding) {base:?}");
        if cover.values().all(|(n, d)| n == d) && !cover.is_empty() { always.push(package); }
    }
    if list { return Ok(()); }
    let _ = writeln!(rs, "\n/// Plain `BLEND_Masked` materials whose depth-only / shadow-depth / hit-proxy / velocity / lightmap-density pixel shaders ALL discard unconditionally (`discard_nz l(-1)`).\n/// Their base-pass pixel shaders carry no clip (coverage is decided by the depth pre-pass): nothing is drawn in any pass.\npub const ALWAYS_DISCARD: &[&str] = &[{}];", always.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>().join(", "));
    let _ = writeln!(rs, "\npub const PERMS: &[Perm] = &[\n{}\n];", perm_entries.join("\n"));
    std::fs::write(out_wgsl, wgsl).map_err(|e| e.to_string())?;
    std::fs::write(out_eye, eye).map_err(|e| e.to_string())?;
    std::fs::write(out_rs, rs).map_err(|e| e.to_string())?;
    Ok(())
}

