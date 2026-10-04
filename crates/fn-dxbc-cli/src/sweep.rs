//! Install sweep: find every material package (and the global shader cache), slice out all shaders, run them through the
//! whole fn-dxbc pipeline (container -> decode -> disassemble -> Rust / WGSL translation) and aggregate coverage numbers.

use crate::extract::{program_hash, scan_blobs, Blob};
use crate::spec::{package_bytes, GLOBAL_CACHE};
use fn_dxbc::analysis;
use fn_dxbc::codegen::{translate_rust, translate_wgsl};
use fn_dxbc::disasm::disassemble_container;
use fn_dxbc::{Container, DisasmOptions, Opcode, ProgramType, Shader};
use fn_pak::PakSet;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Default)]
pub struct SweepOptions {
    pub out: Option<PathBuf>,
    /// also write the disassembly next to each extracted shader
    pub write_asm: bool,
    /// stop after this many packages (smoke runs)
    pub limit: Option<usize>,
    /// only packages whose game path contains this (case-insensitive)
    pub filter: Option<String>,
    /// validate every generated WGSL module with naga
    pub validate_wgsl: bool,
    /// skip the (slow) full scan of every material and only do the global shader cache
    pub global_only: bool,
}

#[derive(Clone, Default, Debug)]
pub struct Counts {
    pub shaders: usize,
    pub parsed: usize,
    pub disassembled: usize,
    pub rust_ok: usize,
    pub wgsl_ok: usize,
}

#[derive(Default, Debug)]
pub struct Stats {
    pub packages_considered: usize,
    pub packages_with_shaders: usize,
    pub blobs: usize,
    pub unique: usize,
    pub container_errors: usize,
    pub total: Counts,
    pub by_type: BTreeMap<String, Counts>,
    /// reason -> number of unique shaders (opcode names, "program type gs", ...)
    pub unsupported: BTreeMap<String, usize>,
    pub unknown_opcodes: BTreeMap<u16, usize>,
    pub decode_errors: BTreeMap<String, usize>,
    pub checksum_zeroed: usize,
    pub bytes_written: u64,
    /// opcode mnemonic -> unique shaders using it (whole install)
    pub opcode_usage: BTreeMap<&'static str, usize>,
    /// opcodes the CPU interpreter cannot execute -> unique shaders using them
    pub interp_unsupported: BTreeMap<&'static str, usize>,
    pub interp_runnable: usize,
    pub naga_checked: usize,
    pub naga_ok: usize,
    pub naga_failures: BTreeMap<String, usize>,
    /// first shader (program hash) of every naga failure key, for debugging
    pub naga_examples: BTreeMap<String, String>,
}

struct Rec {
    key: u64,
    kind: String,
    parsed: bool,
    disasm: bool,
    rust: Result<(), String>,
    wgsl: Result<(), String>,
    unknown: Vec<u16>,
    decode_error: Option<String>,
    zeroed: bool,
    layout: Option<serde_json::Value>,
    asm: Option<String>,
    bytes: Vec<u8>,
    id: usize,
    opcodes: Vec<&'static str>,
    interp_missing: Vec<&'static str>,
    naga: Option<Result<(), String>>,
}

pub fn layout_json(c: &Container, sh: &Shader) -> serde_json::Value {
    use serde_json::json;
    let usage = analysis::cbuffer_usage(sh);
    let (tex, samp) = analysis::resource_usage(sh);
    let sig = |s: Option<fn_dxbc::Signature>| -> serde_json::Value {
        s.map(|s| s.elements.iter().map(|e| json!({"name": e.name, "index": e.semantic_index, "register": e.register, "mask": e.mask, "sysvalue": e.sysvalue_name()})).collect::<Vec<_>>()).into()
    };
    let rdef = c.rdef().map(|r| {
        r.cbuffers.iter().map(|cb| json!({
            "name": cb.name, "size": cb.size,
            "variables": cb.vars.iter().map(|v| json!({"name": v.name, "offset": v.offset, "size": v.size, "type": v.ty.hlsl_name()})).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    });
    json!({
        "shader": sh.version_string(),
        "cbuffers": sh.decls.cbuffers.iter().map(|d| {
            let u = usage.iter().find(|u| u.slot == d.slot);
            json!({"slot": d.slot, "size_float4": d.size, "dynamic_indexed": d.dynamic,
                   "used": u.map(|u| u.elements.iter().map(|(e, m)| json!({"element": e, "mask": comp_str(*m)})).collect::<Vec<_>>()).unwrap_or_default(),
                   "relative_read": u.map_or(false, |u| u.dynamic)})
        }).collect::<Vec<_>>(),
        "rdef": rdef,
        "textures": tex, "samplers": samp,
        "resources": sh.decls.resources.iter().map(|r| json!({"slot": r.slot, "dim": fn_dxbc::disasm::dim_name(r.dim)})).collect::<Vec<_>>(),
        "inputs": sig(c.input_signature()), "outputs": sig(c.output_signature()),
        "immediate_constant_buffer": sh.decls.icb.as_ref().map(|v| v.len()),
        "temps": sh.decls.temps,
    })
}
fn comp_str(m: u8) -> String { "xyzw".chars().enumerate().filter(|(i, _)| m & (1 << i) != 0).map(|(_, c)| c).collect() }

pub fn validate_wgsl(src: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(src).map_err(|e| e.emit_to_string(src))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).map(|_| ()).map_err(|e| format!("{e:?}"))
}

fn analyse(b: &Blob, want_asm: bool, validate: bool) -> Rec {
    let key = program_hash(&b.bytes);
    let mut rec = Rec { key, kind: "?".into(), parsed: false, disasm: false, rust: Err("not parsed".into()), wgsl: Err("not parsed".into()), unknown: vec![], decode_error: None, zeroed: false, layout: None, asm: None, bytes: b.bytes.clone(), id: b.id, opcodes: vec![], interp_missing: vec![], naga: None };
    let Ok(c) = Container::parse(&b.bytes) else { rec.decode_error = Some("container".into()); return rec };
    rec.zeroed = b.hash_was_zero;
    match Shader::from_container(&c) {
        Ok(sh) => {
            rec.parsed = true;
            rec.kind = sh.version_string();
            rec.unknown = analysis::unknown_opcodes(&sh);
            rec.layout = Some(layout_json(&c, &sh));
            rec.opcodes = analysis::opcode_histogram(&sh).keys().copied().collect();
            let mut miss: Vec<&'static str> = sh.body().filter(|i| !fn_dxbc::interp::supports(i.op)).map(|i| i.op.name()).collect();
            miss.sort();
            miss.dedup();
            rec.interp_missing = miss;
            if let Ok(t) = disassemble_container(&c, DisasmOptions::default()) {
                rec.disasm = rec.unknown.is_empty();
                if want_asm { rec.asm = Some(t); }
            }
            rec.rust = translate_rust(&sh, "shader").map(|_| ()).map_err(|e| e.0);
            match translate_wgsl(&sh, "shader") {
                Ok(w) => { rec.wgsl = Ok(()); if validate { rec.naga = Some(validate_wgsl(&w)); } }
                Err(e) => rec.wgsl = Err(e.0),
            }
        }
        Err(e) => rec.decode_error = Some(e.to_string()),
    }
    rec
}

fn is_material_class(c: &str) -> bool { matches!(c, "Material" | "MaterialInstanceConstant" | "MaterialInstance" | "MaterialInstanceDynamic") }

/// `(game path, pak path of the uasset)` of every cooked material-ish package.
pub fn material_packages(set: &PakSet, opts: &SweepOptions) -> Vec<(String, String)> {
    let cands: Vec<String> = set
        .effective()
        .filter(|(_, e)| e.path.to_ascii_lowercase().ends_with(".uasset"))
        .map(|(_, e)| fn_pak::normalize_path(&e.path))
        .filter(|p| match &opts.filter { Some(f) => fn_pak::pak_path_to_game(p).unwrap_or_default().to_ascii_lowercase().contains(&f.to_ascii_lowercase()), None => true })
        .collect();
    let mut v: Vec<(String, String)> = cands
        .par_iter()
        .filter_map(|p| {
            let bytes = set.read(p).ok()?;
            let pkg = fn_uasset::package::Package::parse(&bytes, &[]).ok()?;
            let has = pkg.imports.iter().any(|_| false) || (0..pkg.exports.len()).any(|i| is_material_class(&pkg.export_class_name(i)));
            if !has { return None; }
            Some((fn_pak::pak_path_to_game(p).unwrap_or_else(|| p.clone()), p.clone()))
        })
        .collect();
    v.sort();
    v
}

pub fn sweep(install: &Path, opts: &SweepOptions) -> Result<Stats, String> {
    let set = PakSet::open_install(install).map_err(|e| e.to_string())?;
    let mut sources: Vec<(String, String)> = vec![("GlobalShaderCache".into(), GLOBAL_CACHE.into())];
    if !opts.global_only {
        let mut m = material_packages(&set, opts);
        if let Some(l) = opts.limit { m.truncate(l); }
        sources.extend(m);
    }
    let mut stats = Stats { packages_considered: sources.len(), ..Default::default() };
    // unique shaders by program hash, first source wins
    let mut seen: HashMap<u64, usize> = HashMap::new();
    let mut index: Vec<serde_json::Value> = vec![];
    let mut layouts: Vec<String> = vec![];
    if let Some(out) = &opts.out { std::fs::create_dir_all(out.join("shaders")).map_err(|e| e.to_string())?; }
    // process in chunks to bound memory
    for chunk in sources.chunks(256) {
        let results: Vec<(String, Vec<Rec>)> = chunk
            .par_iter()
            .map(|(game, pak)| {
                let bytes = if game == "GlobalShaderCache" { set.read(GLOBAL_CACHE).map(|c| c.into_owned()).ok() } else { package_bytes(&set, game).ok().or_else(|| set.read(pak).ok().map(|c| c.into_owned())) };
                let recs = bytes.map(|b| scan_blobs(&b).iter().map(|bl| analyse(bl, opts.write_asm, opts.validate_wgsl)).collect()).unwrap_or_default();
                (game.clone(), recs)
            })
            .collect();
        for (game, recs) in results {
            if !recs.is_empty() { stats.packages_with_shaders += 1; }
            stats.blobs += recs.len();
            let mut ids = vec![];
            for r in recs {
                ids.push(serde_json::json!({"id": r.id, "hash": format!("{:016x}", r.key), "type": r.kind}));
                if seen.contains_key(&r.key) { continue; }
                seen.insert(r.key, stats.unique);
                stats.unique += 1;
                if r.decode_error.as_deref() == Some("container") { stats.container_errors += 1; }
                if r.zeroed { stats.checksum_zeroed += 1; }
                let t = stats.by_type.entry(r.kind.clone()).or_default();
                t.shaders += 1;
                stats.total.shaders += 1;
                if r.parsed { t.parsed += 1; stats.total.parsed += 1; }
                if r.disasm { t.disassembled += 1; stats.total.disassembled += 1; }
                if r.rust.is_ok() { t.rust_ok += 1; stats.total.rust_ok += 1; }
                if r.wgsl.is_ok() { t.wgsl_ok += 1; stats.total.wgsl_ok += 1; }
                if let Some(n) = &r.naga {
                    stats.naga_checked += 1;
                    match n { Ok(()) => stats.naga_ok += 1, Err(e) => { let key: String = e.lines().next().unwrap_or("").chars().take(140).collect(); stats.naga_examples.entry(key.clone()).or_insert(format!("{:016x}", r.key)); *stats.naga_failures.entry(key).or_default() += 1; } }
                }
                if let Some(e) = &r.decode_error { *stats.decode_errors.entry(e.clone()).or_default() += 1; }
                for u in &r.unknown { *stats.unknown_opcodes.entry(*u).or_default() += 1; }
                for o in &r.opcodes { *stats.opcode_usage.entry(o).or_default() += 1; }
                if r.parsed && r.interp_missing.is_empty() { stats.interp_runnable += 1; }
                for o in &r.interp_missing { *stats.interp_unsupported.entry(o).or_default() += 1; }
                let mut reasons: Vec<String> = vec![];
                for e in [&r.rust, &r.wgsl] { if let Err(m) = e { if r.parsed { reasons.push(m.clone()); } } }
                reasons.sort();
                reasons.dedup();
                for m in reasons { *stats.unsupported.entry(m).or_default() += 1; }
                if let Some(out) = &opts.out {
                    let base = out.join("shaders").join(format!("{:016x}", r.key));
                    if std::fs::write(base.with_extension("dxbc"), &r.bytes).is_ok() { stats.bytes_written += r.bytes.len() as u64; }
                    if let Some(a) = &r.asm { let _ = std::fs::write(base.with_extension("asm"), a); }
                    if let Some(l) = &r.layout { layouts.push(serde_json::json!({"hash": format!("{:016x}", r.key), "layout": l}).to_string()); }
                }
            }
            if opts.out.is_some() { index.push(serde_json::json!({"package": game, "shaders": ids})); }
        }
    }
    if let Some(out) = &opts.out {
        let _ = std::fs::write(out.join("index.json"), serde_json::to_string(&index).unwrap_or_default());
        let _ = std::fs::write(out.join("layouts.jsonl"), layouts.join("\n"));
    }
    let _ = (ProgramType::Pixel, Opcode::Add);
    Ok(stats)
}

pub fn report(s: &Stats) -> String {
    let mut o = String::new();
    let pc = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f64 * 100.0 / b as f64 };
    o += &format!("packages considered      : {}\npackages with shaders    : {}\nshader blobs (with dups) : {}\nunique shaders           : {}\n", s.packages_considered, s.packages_with_shaders, s.blobs, s.unique);
    o += &format!("container errors         : {}\nUE-zeroed checksums fixed: {}\n", s.container_errors, s.checksum_zeroed);
    let t = &s.total;
    o += &format!("parsed                   : {} ({:.2}%)\ndisassembled (no unknown): {} ({:.2}%)\ntranslated to Rust       : {} ({:.2}%)\ntranslated to WGSL       : {} ({:.2}%)\n", t.parsed, pc(t.parsed, t.shaders), t.disassembled, pc(t.disassembled, t.shaders), t.rust_ok, pc(t.rust_ok, t.shaders), t.wgsl_ok, pc(t.wgsl_ok, t.shaders));
    if s.naga_checked > 0 {
        o += &format!("naga validation of the generated WGSL: {} of {} modules valid\n", s.naga_ok, s.naga_checked);
        for (k, n) in &s.naga_failures { o += &format!("  {n:>7}  {k}   (e.g. {})\n", s.naga_examples.get(k).map(|s| s.as_str()).unwrap_or("?")); }
    }
    o += "by program type:\n";
    for (k, c) in &s.by_type { o += &format!("  {k:<8} shaders {:>7} parsed {:>7} disasm {:>7} rust {:>7} wgsl {:>7}\n", c.shaders, c.parsed, c.disassembled, c.rust_ok, c.wgsl_ok); }
    o += "translation blockers (unique shaders affected):\n";
    let mut v: Vec<_> = s.unsupported.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in v { o += &format!("  {n:>7}  {k}\n"); }
    o += &format!("CPU interpreter can execute {} of {} unique shaders; opcodes it cannot execute (unique shaders using them):\n", s.interp_runnable, s.total.parsed);
    let mut iv: Vec<_> = s.interp_unsupported.iter().collect();
    iv.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in iv { o += &format!("  {n:>7}  {k}\n"); }
    o += &format!("opcodes seen in the install ({} distinct): {}\n", s.opcode_usage.len(), s.opcode_usage.iter().map(|(k, n)| format!("{k}:{n}")).collect::<Vec<_>>().join(" "));
    if !s.unknown_opcodes.is_empty() {
        o += "unknown opcode numbers:\n";
        for (k, n) in &s.unknown_opcodes { o += &format!("  opcode {k}: {n} shaders\n"); }
    }
    if !s.decode_errors.is_empty() {
        o += "decode errors:\n";
        for (k, n) in &s.decode_errors { o += &format!("  {n:>7}  {k}\n"); }
    }
    o
}



