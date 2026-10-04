//! Assembles one gun pipeline's WGSL from the (vertex, pixel) pair of ORIGINAL base-pass programs: both translated from DXBC by `fn_dxbc::codegen`, their resources
//! renamed / re-bound into bind group 1 (VS bindings below 96 + stage offsets, see [`Binding`]), shader-resource-view buffers (the GPU-skin bone matrices) turned into
//! uniform arrays, and an entry point pair written around the translated functions:
//! * vertex attributes -> the `v[]` input registers (by the `ATTRIBUTE n` of the program's own input signature; integer attributes are bit-cast),
//! * VS outputs -> inter-stage locations by the output register, matched to the pixel program's inputs by (semantic, index) with the program's interpolation modes,
//! * the pixel program's five GBuffer outputs -> `gun_resolve` of the renderer (`gun_resolve.wgsl`).
use fn_dxbc::{OperandType, ProgramType};
use fn_render::gun_material::{GunBindTy, GunLayoutEntry};
use std::fmt::Write;

use crate::maps::Program;

/// Vec4s of a bone-matrix uniform array (3 per bone, 256 bones: the GPU-skin chunk limit).
pub const BONE_VEC4S: u32 = 768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Stage { Vertex, Pixel }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ResKind { Uniform, BoneBuffer, Tex2D, Sampler }

/// One declared resource of the assembled module (binding in group 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResDecl {
    pub stage: Stage,
    /// Slot of the DXBC declaration (`cb<slot>`, `t<slot>`, `s<slot>`).
    pub slot: u32,
    pub kind: ResKind,
    pub binding: u32,
    /// Array length (float4s) of uniform buffers.
    pub size: u32,
}

pub struct Binding;
impl Binding {
    pub fn of(stage: Stage, kind: ResKind, slot: u32) -> u32 {
        let base = match (stage, kind) {
            (Stage::Vertex, ResKind::Uniform) => 0, (Stage::Pixel, ResKind::Uniform) => 16,
            (Stage::Vertex, ResKind::BoneBuffer) => 32, (Stage::Pixel, ResKind::BoneBuffer) => 40,
            (Stage::Vertex, ResKind::Tex2D) => 48, (Stage::Pixel, ResKind::Tex2D) => 64,
            (Stage::Vertex, ResKind::Sampler) => 80, (Stage::Pixel, ResKind::Sampler) => 96,
        };
        base + slot
    }
}

pub struct Assembled {
    pub wgsl: String,
    pub resources: Vec<ResDecl>,
    /// `ATTRIBUTE n` indices the vertex program reads.
    pub vs_attributes: Vec<u32>,
}

/// Replaces the identifiers `cb<N>` / `t<N>` / `s<N>` / `icb` of a translated module by `<prefix>cb<N>` ... (whole-identifier matches only).
fn rename_resources(text: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_alphabetic() || c == '_' {
            let s = i;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
            let id = &text[s..i];
            let digits = |p: &str| id.strip_prefix(p).is_some_and(|r| !r.is_empty() && r.bytes().all(|d| d.is_ascii_digit()));
            if digits("cb") || digits("t") || digits("s") || digits("u") || id == "icb" { out.push_str(prefix); }
            out.push_str(id);
        } else { out.push(c); i += 1; }
    }
    out
}

/// Re-binds the `@group(G) @binding(B) var ...` lines of one stage into group 1 and collects them.
fn rebind(text: &str, stage: Stage, res: &mut Vec<ResDecl>) -> Result<String, String> {
    let mut out = String::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("@group(") else { out.push_str(line); out.push('\n'); continue };
        let (g, rest) = rest.split_once(") @binding(").ok_or("bad group line")?;
        let (b, rest) = rest.split_once(')').ok_or("bad binding line")?;
        let (group, slot): (u32, u32) = (g.parse().map_err(|_| "group")?, b.parse().map_err(|_| "binding")?);
        let decl = rest.trim();
        let name = decl.split(':').next().unwrap_or("").trim().rsplit(' ').next().unwrap_or("").to_owned();
        let (kind, size, text) = if decl.starts_with("var<uniform>") {
            if group != 0 { return Err(format!("uniform in group {group}: {line}")); }
            let sz = decl.split("vec4<f32>, ").nth(1).and_then(|r| r.split('>').next()).and_then(|n| n.parse::<u32>().ok()).ok_or("uniform size")?;
            (ResKind::Uniform, sz, None)
        } else if decl.starts_with("var<storage, read>") && decl.contains("array<vec4<f32>>") {
            // SRV buffer of float4s (the GPU-skin bone matrices): a uniform array of the chunk limit
            (ResKind::BoneBuffer, BONE_VEC4S, Some(format!("var<uniform> {name}: array<vec4<f32>, {BONE_VEC4S}>;")))
        } else if decl.contains("texture_2d<f32>") { (ResKind::Tex2D, 0, None) }
        else if decl.contains(": sampler;") { (ResKind::Sampler, 0, None) }
        else { return Err(format!("unsupported resource declaration: {line}")); };
        let binding = Binding::of(stage, kind, slot);
        res.push(ResDecl { stage, slot, kind, binding, size });
        match text {
            Some(t) => { let _ = writeln!(out, "@group(1) @binding({binding}) {t}"); }
            None => { let _ = writeln!(out, "@group(1) @binding({binding}){rest}"); }
        }
    }
    Ok(out)
}

/// The translated function of `shader` without the stock `NAME_main` entry point / output struct.
fn function_wgsl(shader: &fn_dxbc::Shader, name: &str) -> Result<String, String> {
    let w = fn_dxbc::codegen::translate_wgsl(shader, name).map_err(|e| e.0)?;
    Ok(match w.find("\nstruct ") { Some(p) => w[..p + 1].to_string(), None => w })
}
fn array_len(text: &str, name: &str) -> (Option<usize>, usize) {
    let head = text.find(&format!("fn {name}(")).map(|p| &text[p..]).unwrap_or("");
    let line = head.lines().next().unwrap_or("");
    let nums: Vec<usize> = line.split("vec4<f32>, ").skip(1).filter_map(|r| r.split('>').next().and_then(|n| n.parse().ok())).collect();
    if line.contains("(v:") { (nums.first().copied(), nums.get(1).copied().unwrap_or(1)) } else { (None, nums.first().copied().unwrap_or(1)) }
}
fn interp_attr(d: &fn_dxbc::IoDecl) -> &'static str {
    match (d.sysvalue.is_some(), d.interp) { (true, _) | (_, 1) => "@interpolate(flat) ", (_, 4) => "@interpolate(linear) ", (_, 3) => "@interpolate(perspective, centroid) ", (_, 5) => "@interpolate(linear, centroid) ", _ => "" }
}

/// Builds the module for a (vertex, pixel) pair. `view_slot` = the pixel program's uniform-buffer slot named `View` (used to rebuild the world position).
pub fn assemble(vs: &Program, ps: &Program) -> Result<Assembled, String> {
    if vs.shader.program_type != ProgramType::Vertex || ps.shader.program_type != ProgramType::Pixel { return Err("assemble needs a vertex and a pixel program".into()); }
    let vsw = function_wgsl(&vs.shader, "vs_fx")?;
    let psw = function_wgsl(&ps.shader, "ps_fx")?;
    let (vsw, psw) = (rename_resources(&vsw, "v"), rename_resources(&psw, "p"));
    let mut resources = vec![];
    let vsw = rebind(&vsw, Stage::Vertex, &mut resources)?;
    let psw = rebind(&psw, Stage::Pixel, &mut resources)?;
    // the module directive is added once by the renderer (it must precede every declaration)
    let (vsw, psw) = (vsw.replace("diagnostic(off, derivative_uniformity);\n", ""), psw.replace("diagnostic(off, derivative_uniformity);\n", ""));
    let (vin, _vout) = array_len(&vsw, "vs_fx");
    let (pin, _pout) = array_len(&psw, "ps_fx");

    let vs_cont = fn_dxbc::Container::parse(&vs.blob.bytes).map_err(|e| e.to_string())?;
    let ps_cont = fn_dxbc::Container::parse(&ps.blob.bytes).map_err(|e| e.to_string())?;
    let vs_in = vs_cont.input_signature().ok_or("vertex program has no input signature")?;
    let vs_out = vs_cont.output_signature().ok_or("vertex program has no output signature")?;
    let ps_in = ps_cont.input_signature().ok_or("pixel program has no input signature")?;

    let mut s = String::new();
    s.push_str(&vsw); s.push('\n'); s.push_str(&psw); s.push('\n');
    // ---- vertex entry
    let mut attrs: Vec<u32> = vec![];
    let mut vs_fields = String::new();
    let mut vs_init = String::new();
    for d in vs.shader.decls.inputs.iter().filter(|d| d.file == OperandType::INPUT && d.sysvalue.is_none()) {
        let Some(e) = vs_in.elements.iter().find(|e| e.name == "ATTRIBUTE" && e.register == d.reg) else { return Err(format!("vertex input register {} has no ATTRIBUTE element", d.reg)) };
        if attrs.contains(&e.semantic_index) { continue; }
        attrs.push(e.semantic_index);
        match e.comp_type {
            1 => { let _ = writeln!(vs_fields, "    @location({}) a{}: vec4<u32>,", e.semantic_index, e.semantic_index); let _ = writeln!(vs_init, "    v[{}] = bitcast<vec4<f32>>(in.a{});", d.reg, e.semantic_index); }
            2 => { let _ = writeln!(vs_fields, "    @location({}) a{}: vec4<i32>,", e.semantic_index, e.semantic_index); let _ = writeln!(vs_init, "    v[{}] = bitcast<vec4<f32>>(in.a{});", d.reg, e.semantic_index); }
            _ => { let _ = writeln!(vs_fields, "    @location({}) a{}: vec4<f32>,", e.semantic_index, e.semantic_index); let _ = writeln!(vs_init, "    v[{}] = in.a{};", d.reg, e.semantic_index); }
        }
    }
    let position_reg = vs.shader.decls.outputs.iter().find(|d| d.file == OperandType::OUTPUT && d.sysvalue == Some(1)).map(|d| d.reg).ok_or("vertex program has no SV_Position output")?;
    // pixel inputs -> vertex outputs by semantic
    struct Varying { vs_reg: u32, ps_reg: u32, attr: &'static str }
    let mut varyings: Vec<Varying> = vec![];
    let mut ps_builtins: Vec<(u32, u32)> = vec![];
    let mut seen = vec![];
    for d in ps.shader.decls.inputs.iter().filter(|d| d.file == OperandType::INPUT) {
        if seen.contains(&d.reg) { continue; }
        seen.push(d.reg);
        match d.sysvalue {
            Some(sv @ (1 | 9)) => ps_builtins.push((d.reg, sv)),
            Some(x) => return Err(format!("unsupported pixel program system input {x}")),
            None => {
                let pe = ps_in.elements.iter().find(|e| e.register == d.reg).ok_or_else(|| format!("pixel input register {} has no signature element", d.reg))?;
                let ve = vs_out.elements.iter().find(|e| e.name == pe.name && e.semantic_index == pe.semantic_index).ok_or_else(|| format!("pixel input {}{} is not written by the vertex program", pe.name, pe.semantic_index))?;
                varyings.push(Varying { vs_reg: ve.register, ps_reg: d.reg, attr: interp_attr(d) });
            }
        }
    }
    s.push_str("struct GunVsIn {\n"); s.push_str(&vs_fields); s.push_str("}\nstruct GunVsOut {\n    @builtin(position) pos: vec4<f32>,\n");
    for v in &varyings { let _ = writeln!(s, "    @location({}) {}o{}: vec4<f32>,", v.vs_reg, v.attr, v.vs_reg); }
    s.push_str("}\n@vertex fn gun_vs_main(in: GunVsIn) -> GunVsOut {\n");
    let _ = writeln!(s, "    var v: array<vec4<f32>, {}>;", vin.unwrap_or(0).max(1));
    s.push_str(&vs_init);
    s.push_str("    let o = vs_fx(v);\n    var r: GunVsOut;\n");
    let _ = writeln!(s, "    r.pos = o[{position_reg}];");
    for v in &varyings { let _ = writeln!(s, "    r.o{0} = o[{0}];", v.vs_reg); }
    s.push_str("    return r;\n}\n");
    // ---- fragment entry
    let view_slot = ps.ubo_names.iter().position(|n| n == "View").ok_or("pixel program has no View uniform buffer")?;
    let view_decl = resources.iter().find(|r| r.stage == Stage::Pixel && r.kind == ResKind::Uniform && r.slot as usize == view_slot).ok_or("pixel program does not declare its View buffer")?;
    if view_decl.size < 61 { return Err(format!("pixel View buffer declares only {} float4s (the world position rebuild reads slots 40..43 and 60)", view_decl.size)); }
    s.push_str("struct GunFsIn {\n    @builtin(position) frag: vec4<f32>,\n    @builtin(front_facing) front: bool,\n");
    for v in &varyings { let _ = writeln!(s, "    @location({}) {}i{}: vec4<f32>,", v.vs_reg, v.attr, v.ps_reg); }
    s.push_str("}\n@fragment fn gun_fs_main(in: GunFsIn) -> @location(0) vec4<f32> {\n");
    let _ = writeln!(s, "    var v: array<vec4<f32>, {}>;", pin.unwrap_or(0).max(1));
    for v in &varyings { let _ = writeln!(s, "    v[{}] = in.i{};", v.ps_reg, v.ps_reg); }
    for (reg, sv) in &ps_builtins {
        match sv {
            // UE reads the pixel's view depth (W of the clip position) from SV_Position.w; WebGPU hands over 1 / W
            1 => { let _ = writeln!(s, "    v[{reg}] = vec4<f32>(in.frag.x, in.frag.y, in.frag.z, 1.0 / in.frag.w);"); }
            _ => { let _ = writeln!(s, "    v[{reg}] = vec4<f32>(bitcast<f32>(select(0u, 0xffffffffu, in.front)), 0.0, 0.0, 0.0);"); }
        }
    }
    s.push_str("    let o = ps_fx(v);\n");
    let c = format!("pcb{view_slot}");
    let _ = writeln!(s, "    let h = in.frag.x * {c}[40] + in.frag.y * {c}[41] + in.frag.z * {c}[42] + {c}[43];");
    let _ = writeln!(s, "    let world_ue = h.xyz / h.w + {c}[60].xyz;");
    s.push_str("    let world_m = vec3<f32>(world_ue.x, world_ue.z, world_ue.y) * 0.01;\n");
    // GBuffer: A = normal * 0.5 + 0.5, B = (metallic, specular, roughness, shading model + mask), C = base colour + AO, scene colour = emissive + sky diffuse
    s.push_str("    let n_ue = normalize(o[1].xyz * 2.0 - 1.0);\n    let n_w = vec3<f32>(n_ue.x, n_ue.z, n_ue.y);\n");
    s.push_str("    return gun_resolve(world_m, n_w, o[0].xyz, o[3].xyz, o[2].x, o[2].y, o[2].z);\n}\n");
    Ok(Assembled { wgsl: s, resources, vs_attributes: attrs })
}

pub fn layout_of(resources: &[ResDecl]) -> Vec<GunLayoutEntry> {
    resources.iter().map(|r| GunLayoutEntry {
        binding: r.binding,
        ty: match r.kind { ResKind::Uniform | ResKind::BoneBuffer => GunBindTy::Uniform, ResKind::Tex2D => GunBindTy::Tex2D, ResKind::Sampler => GunBindTy::Sampler },
        vertex: r.stage == Stage::Vertex, fragment: r.stage == Stage::Pixel,
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn renames_whole_identifiers_only() {
        let t = "@group(0) @binding(1) var<uniform> cb1: array<vec4<f32>, 4>;\nlet x = cb1[2].x + t0.x + s10 + qt0 + icb[1].x + t0x;";
        let r = rename_resources(t, "p");
        assert!(r.contains("pcb1: array") && r.contains("pcb1[2].x + pt0.x + ps10 + qt0 + picb[1].x + t0x"), "{r}");
    }
    #[test] fn rebinding_turns_srv_buffers_into_uniform_arrays() {
        let t = "@group(0) @binding(4) var<uniform> vcb4: array<vec4<f32>, 21>;\n@group(1) @binding(0) var<storage, read> vt0: array<vec4<f32>>;\n@group(1) @binding(3) var vt3: texture_2d<f32>;\n@group(2) @binding(1) var vs1: sampler;\nfn x() {}\n";
        let mut res = vec![];
        let o = rebind(t, Stage::Vertex, &mut res).unwrap();
        assert!(o.contains("@group(1) @binding(4) var<uniform> vcb4") && o.contains("@group(1) @binding(32) var<uniform> vt0: array<vec4<f32>, 768>;") && o.contains("@group(1) @binding(51) var vt3") && o.contains("@group(1) @binding(81) var vs1"), "{o}");
        assert_eq!(res.iter().map(|r| (r.kind, r.slot, r.binding)).collect::<Vec<_>>(), vec![(ResKind::Uniform, 4, 4), (ResKind::BoneBuffer, 0, 32), (ResKind::Tex2D, 3, 51), (ResKind::Sampler, 1, 81)]);
    }
}
