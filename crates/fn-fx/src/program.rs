//! Assembles one render pipeline's WGSL from a (vertex shader, pixel shader) pair of the original cooked programs: both translated by `fn_dxbc::codegen`, their resource
//! declarations renamed / re-bound into one module (`vcb1` / `pt0` ..., VS bindings 0.., PS bindings 16..), and an entry point pair written around the translated
//! functions (vertex attributes -> the `v[]` input registers, outputs -> inter-stage locations equal to the output register numbers, `SV_Position`).
use crate::cooked::{container_input_registers, container_output_registers, translated_function_wgsl, Program};
use fn_dxbc::{OperandType, ProgramType};
use std::collections::BTreeMap;
use std::fmt::Write;

/// Where the shader stage reads a vertex attribute from: the `ATTRIBUTE n` index of the vertex declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VsInput { pub attribute: u32, pub register: u32, pub mask: u8 }

/// One declared resource of the assembled module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResDecl {
    pub stage: Stage,
    /// Slot of the DXBC declaration (`cb<slot>`, `t<slot>`, `s<slot>`).
    pub slot: u32,
    pub kind: ResKind,
    pub group: u32,
    pub binding: u32,
    /// Array length for uniform buffers (float4s).
    pub size: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Pixel,
    /// The depth-only pixel shader (the masked-material clip, see [`assemble`]).
    ClipPixel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ResKind { Uniform, Tex2D, Tex3D, Cube, Sampler, StorageBuf }

pub const PIXEL_BINDING_BASE: u32 = 16;
pub const CLIP_BINDING_BASE: u32 = 32;

pub struct Assembled {
    pub wgsl: String,
    pub resources: Vec<ResDecl>,
    pub vs_inputs: Vec<VsInput>,
    /// Output registers of the vertex shader that are interpolated to the pixel shader (everything but SV_Position).
    pub varyings: Vec<u32>,
    /// Pixel shader outputs (SV_Target registers).
    pub targets: u32,
    /// The pixel shader reads `SV_Position` (its `.w` is handed over as the view depth, see [`assemble`]).
    pub ps_uses_position: bool,
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
            let digits = |p: &str| id.strip_prefix(p).map_or(false, |r| !r.is_empty() && r.bytes().all(|d| d.is_ascii_digit()));
            if digits("cb") || digits("t") || digits("s") || id == "icb" { out.push_str(prefix); }
            out.push_str(id);
        } else { out.push(c); i += 1; }
    }
    out
}

/// Re-binds the `@group(G) @binding(B) var ...` lines of one stage (`base` added to the binding) and collects them.
fn rebind(text: &str, stage: Stage, base: u32, res: &mut Vec<ResDecl>) -> Result<String, String> {
    let mut out = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("@group(") {
            let (g, rest) = rest.split_once(") @binding(").ok_or("bad group line")?;
            let (b, rest) = rest.split_once(')').ok_or("bad binding line")?;
            let (group, binding): (u32, u32) = (g.parse().map_err(|_| "group")?, b.parse().map_err(|_| "binding")?);
            if group > 2 { return Err(format!("unsupported resource group {group}: {line}")); }
            let name_decl = rest.trim();
            let (kind, size, slot) = if name_decl.starts_with("var<uniform>") {
                let sz = name_decl.split("vec4<f32>, ").nth(1).and_then(|r| r.split('>').next()).and_then(|n| n.parse::<u32>().ok()).ok_or("uniform size")?;
                (ResKind::Uniform, sz, binding)
            } else if name_decl.starts_with("var<storage, read>") { (ResKind::StorageBuf, 0, binding) }
            else if name_decl.contains("texture_2d<f32>") { (ResKind::Tex2D, 0, binding) }
            else if name_decl.contains("texture_3d<f32>") { (ResKind::Tex3D, 0, binding) }
            else if name_decl.contains("texture_cube<f32>") { (ResKind::Cube, 0, binding) }
            else if name_decl.contains(": sampler;") { (ResKind::Sampler, 0, binding) }
            else { return Err(format!("unsupported resource declaration: {line}")); };
            res.push(ResDecl { stage, slot, kind, group, binding: binding + base, size });
            let _ = writeln!(out, "@group({group}) @binding({}){rest}", binding + base);
        } else { out.push_str(line); out.push('\n'); }
    }
    Ok(out)
}

fn function_array_len(text: &str, name: &str) -> (Option<usize>, usize) {
    // `fn NAME(v: array<vec4<f32>, N>) -> array<vec4<f32>, M>`
    let head = text.find(&format!("fn {name}(")).map(|p| &text[p..]).unwrap_or("");
    let line = head.lines().next().unwrap_or("");
    let nums: Vec<usize> = line.split("vec4<f32>, ").skip(1).filter_map(|r| r.split('>').next().and_then(|n| n.parse().ok())).collect();
    if line.contains("(v:") { (nums.first().copied(), nums.get(1).copied().unwrap_or(1)) } else { (None, nums.first().copied().unwrap_or(1)) }
}

/// Builds the module. `float_attr` marks the attributes that are integer vectors in the vertex declaration (unused ones are skipped).
/// `clip`: the `FDepthOnlyPS` of a masked material. With `r.EarlyZPassOnlyMaterialMasking` the base-pass pixel shader of a masked material has NO `discard` (the depth pre-pass
/// clipped); the FX pass has no pre-pass, so the depth-only pixel shader (same material graph, same interpolants: matched by semantic) runs first in the fragment entry.
pub fn assemble(vs: &Program, ps: &Program, lit: bool, clip: Option<&Program>) -> Result<Assembled, String> {
    if vs.shader.program_type != ProgramType::Vertex || ps.shader.program_type != ProgramType::Pixel { return Err("assemble needs a vertex and a pixel program".into()); }
    let vsw = translated_function_wgsl(&vs.shader, "vs_fx")?;
    let psw = translated_function_wgsl(&ps.shader, "ps_fx")?;
    let (vsw, psw) = (rename_resources(&vsw, "v"), rename_resources(&psw, "p"));
    let mut resources = vec![];
    let vsw = rebind(&vsw, Stage::Vertex, 0, &mut resources)?;
    let psw = rebind(&psw, Stage::Pixel, PIXEL_BINDING_BASE, &mut resources)?;
    // the declarations of the two modules use the same helper names (icb); a translated module without `icb` / diagnostics duplicates is fine, but the directive may appear once only
    let (vsw, psw) = (vsw.replace("diagnostic(off, derivative_uniformity);\n", ""), psw.replace("diagnostic(off, derivative_uniformity);\n", ""));
    let mut clipw = String::new();
    let mut clip_map: Vec<(u32, u32)> = vec![]; // (clip pixel input register, vertex output register)
    let mut clip_len = 1usize;
    if let Some(cp) = clip {
        let w = translated_function_wgsl(&cp.shader, "dps_fx")?;
        let w = rebind(&rename_resources(&w, "d"), Stage::ClipPixel, CLIP_BINDING_BASE, &mut resources)?.replace("diagnostic(off, derivative_uniformity);\n", "");
        clip_len = function_array_len(&w, "dps_fx").0.unwrap_or(0);
        let vso = container_output_registers(&vs.blob.bytes);
        for d in cp.shader.decls.inputs.iter().filter(|d| d.file == OperandType::INPUT && d.sysvalue.is_none()) {
            let Some((n, i, _, _)) = container_input_registers(&cp.blob.bytes).into_iter().find(|(_, _, r, _)| *r == d.reg) else { continue };
            let Some((_, _, vreg, _)) = vso.iter().find(|(vn, vi, _, _)| *vn == n && *vi == i) else { return Err(format!("clip pixel input {n}{i} is not written by the vertex shader")) };
            clip_map.push((d.reg, *vreg));
        }
        clipw = w;
    }
    let (vin, vout) = function_array_len(&vsw, "vs_fx");
    let (pin, pout) = function_array_len(&psw, "ps_fx");
    let vin = vin.unwrap_or(0);
    let sig_in = container_input_registers(&vs.blob.bytes);
    let sig_out = container_output_registers(&vs.blob.bytes);
    let _ = sig_out;
    let mut vs_inputs = vec![];
    for d in vs.shader.decls.inputs.iter().filter(|d| d.file == OperandType::INPUT && d.sysvalue.is_none()) {
        let Some((_, attr, reg, mask)) = sig_in.iter().find(|(n, _, r, _)| n == "ATTRIBUTE" && *r == d.reg) else { return Err(format!("vertex input register {} has no ATTRIBUTE signature element", d.reg)) };
        if vs_inputs.iter().any(|v: &VsInput| v.register == *reg) { continue; }
        vs_inputs.push(VsInput { attribute: *attr, register: *reg, mask: *mask });
    }
    // vertex shader outputs: the signature of the VS
    let mut out_decls: BTreeMap<u32, bool> = BTreeMap::new(); // register -> is position
    for d in vs.shader.decls.outputs.iter().filter(|d| d.file == OperandType::OUTPUT) { out_decls.insert(d.reg, d.sysvalue == Some(1)); }
    let position_reg = out_decls.iter().find(|(_, p)| **p).map(|(r, _)| *r).ok_or("vertex shader has no SV_Position output")?;
    let varyings: Vec<u32> = out_decls.keys().copied().filter(|r| *r != position_reg).collect();
    // pixel shader inputs
    let mut ps_in: Vec<(u32, Option<u32>)> = vec![];
    for d in ps.shader.decls.inputs.iter().filter(|d| d.file == OperandType::INPUT) { if !ps_in.iter().any(|(r, _)| *r == d.reg) { ps_in.push((d.reg, d.sysvalue)); } }
    for (r, sv) in &ps_in { if sv.is_none() && !varyings.contains(r) { return Err(format!("pixel input register {r} is not written by the vertex shader")); } }
    let targets = ps.shader.decls.outputs.iter().filter(|d| d.file == OperandType::OUTPUT).count() as u32;

    let mut s = String::new();
    s.push_str("diagnostic(off, derivative_uniformity);\n");
    if lit { s.push_str("struct FxLight { to_sun: vec4<f32>, sun: vec4<f32>, sky: vec4<f32> }\n@group(3) @binding(0) var<uniform> fx_light: FxLight;\n"); }
    s.push_str(&vsw);
    s.push('\n');
    s.push_str(&psw);
    s.push('\n');
    s.push_str(&clipw);
    s.push('\n');
    // vertex entry
    s.push_str("struct VsIn {\n");
    for v in &vs_inputs { let _ = writeln!(s, "    @location({}) a{}: vec4<f32>,", v.attribute, v.attribute); }
    s.push_str("}\nstruct VsOut {\n    @builtin(position) pos: vec4<f32>,\n");
    for r in &varyings { let _ = writeln!(s, "    @location({r}) o{r}: vec4<f32>,"); }
    s.push_str("}\n@vertex fn vs_main(in: VsIn) -> VsOut {\n");
    let _ = writeln!(s, "    var v: array<vec4<f32>, {}>;", vin.max(1));
    for v in &vs_inputs { let _ = writeln!(s, "    v[{}] = in.a{};", v.register, v.attribute); }
    s.push_str("    let o = vs_fx(v);\n    var r: VsOut;\n");
    let _ = writeln!(s, "    r.pos = o[{position_reg}];");
    for r in &varyings { let _ = writeln!(s, "    r.o{r} = o[{r}];"); }
    s.push_str("    return r;\n}\n");
    // fragment entry
    s.push_str("struct FsIn {\n    @builtin(position) frag: vec4<f32>,\n    @builtin(front_facing) front: bool,\n");
    let mut fs_regs: Vec<u32> = ps_in.iter().filter(|(_, sv)| sv.is_none()).map(|(r, _)| *r).collect();
    for (_, v) in &clip_map { if !fs_regs.contains(v) { fs_regs.push(*v); } }
    for r in &fs_regs { let _ = writeln!(s, "    @location({r}) i{r}: vec4<f32>,"); }
    s.push_str("}\n");
    s.push_str("@fragment fn fs_main(in: FsIn) -> @location(0) vec4<f32> {\n");
    let _ = writeln!(s, "    var v: array<vec4<f32>, {}>;", pin.unwrap_or(0).max(1));
    let mut uses_pos = false;
    for (r, sv) in &ps_in {
        match sv {
            None => { let _ = writeln!(s, "    v[{r}] = in.i{r};"); }
            // UE reads the pixel's view depth (W of the clip position) from SV_Position.w; WebGPU hands over 1 / W
            Some(1) => { uses_pos = true; let _ = writeln!(s, "    v[{r}] = vec4<f32>(in.frag.x, in.frag.y, in.frag.z, 1.0 / in.frag.w);"); }
            Some(9) => { let _ = writeln!(s, "    v[{r}] = vec4<f32>(bitcast<f32>(select(0u, 0xffffffffu, in.front)), 0.0, 0.0, 0.0);"); }
            Some(x) => return Err(format!("unsupported pixel shader system input {x}")),
        }
    }
    if clip.is_some() {
        if clip_len > 0 {
            let _ = writeln!(s, "    var vd: array<vec4<f32>, {clip_len}>;");
            for (cr, vr) in &clip_map { let _ = writeln!(s, "    vd[{cr}] = in.i{vr};"); }
            s.push_str("    let clipped = dps_fx(vd);\n");
        } else { s.push_str("    let clipped = dps_fx();\n"); }
    }
    s.push_str("    let o = ps_fx(v);\n");
    if lit {
        // the original program writes the deferred GBuffer: target 0 = emissive / scene colour, 1 = world normal (n * 0.5 + 0.5), 3 = base colour. The renderer's own lighting
        // (sun x N.L + sky hemisphere, the model of every other prop of the port) shades the base colour; the original deferred light accumulation is not run.
        s.push_str("    let n_ue = normalize(o[1].xyz * 2.0 - 1.0);\n    let n = vec3<f32>(n_ue.x, n_ue.z, n_ue.y);\n    let lit = o[3].xyz * (fx_light.sun.xyz * max(dot(n, fx_light.to_sun.xyz), 0.0) + fx_light.sky.xyz * clamp(0.5 + 0.5 * n.y, 0.0, 1.0));\n    return vec4<f32>(o[0].xyz + lit, 1.0);\n}\n");
    } else {
        s.push_str("    return o[0];\n}\n");
    }
    let _ = (vout, pout);
    Ok(Assembled { wgsl: s, resources, vs_inputs, varyings, targets, ps_uses_position: uses_pos })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn renames_whole_identifiers_only() {
        let t = "@group(0) @binding(1) var<uniform> cb1: array<vec4<f32>, 4>;\nlet x = cb1[2].x + t0.x + s10 + qt0 + icb[1].x + t0x;";
        let r = rename_resources(t, "p");
        assert!(r.contains("pcb1: array") && r.contains("pcb1[2].x + pt0.x + ps10 + qt0 + picb[1].x + t0x"), "{r}");
    }
    #[test] fn rebinding_collects_declarations() {
        let t = "@group(0) @binding(3) var<uniform> pcb3: array<vec4<f32>, 10>;\n@group(1) @binding(2) var pt2: texture_3d<f32>;\n@group(2) @binding(1) var ps1: sampler;\nfn x() {}\n";
        let mut res = vec![];
        let o = rebind(t, Stage::Pixel, 16, &mut res).unwrap();
        assert!(o.contains("@binding(19) var<uniform> pcb3") && o.contains("@binding(18) var pt2") && o.contains("@binding(17) var ps1"), "{o}");
        assert_eq!(res.iter().map(|r| (r.kind, r.slot, r.binding, r.size)).collect::<Vec<_>>(), vec![(ResKind::Uniform, 3, 19, 10), (ResKind::Tex3D, 2, 18, 0), (ResKind::Sampler, 1, 17, 0)]);
    }
}







