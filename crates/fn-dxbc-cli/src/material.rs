//! Single-material export (port of tools/hud-shaders.ps1 + tools/sky-shader-extract.cjs + tools/sky-disasm.ps1 + tools/hud-uniforms.cjs):
//! `<out>/<Name>/shader_<id>.dxbc|.asm|.layout.json`, `<out>/<Name>/index.json` and `<out>/<Name>-uniforms.json`.

use crate::extract::scan_blobs;
use crate::spec::{package_bytes, GLOBAL_CACHE};
use crate::sweep::layout_json;
use crate::uniform::{evaluate, package_names, parse_set_with, Expr, Overrides, UniformSet};
use fn_dxbc::disasm::disassemble_container;
use fn_dxbc::{Container, DisasmOptions, Shader};
use fn_pak::PakSet;
use serde_json::{json, Value};
use std::path::Path;

pub fn expr_json(e: &Expr) -> Value {
    match e {
        Expr::VectorParameter { name, default } => json!({"t": "VectorParameter", "name": name, "default": default}),
        Expr::ScalarParameter { name, default } => json!({"t": "ScalarParameter", "name": name, "default": default}),
        Expr::Constant { value, value_type } => json!({"t": "Constant", "value": value, "valueType": value_type}),
        Expr::ComponentSwizzle { e, idx, count } => json!({"t": "ComponentSwizzle", "e": expr_json(e), "idx": idx, "n": count}),
        Expr::FoldedMath { a, b, op, value_type } => json!({"t": "FoldedMath", "a": expr_json(a), "b": expr_json(b), "op": op, "vt": value_type}),
        Expr::AppendVector { a, b, num_a } => json!({"t": "AppendVector", "a": expr_json(a), "b": expr_json(b), "n": num_a}),
        Expr::TrigMath { x, y, op } => json!({"t": "TrigMath", "x": expr_json(x), "y": expr_json(y), "op": op}),
        Expr::Texture { index, sampler_source } => json!({"t": "Texture", "textureIndex": index, "samplerSource": sampler_source}),
        Expr::TextureParameter { name, index, sampler_source } => json!({"t": "TextureParameter", "name": name, "textureIndex": index, "samplerSource": sampler_source}),
        Expr::Time => json!({"t": "Time"}),
        Expr::RealTime => json!({"t": "RealTime"}),
    }
}

/// The `<Name>-uniforms.json` document (evaluated with `ov`; defaults when empty).
pub fn uniforms_json(source: &str, set: &UniformSet, ov: &Overrides) -> Value {
    let packed = evaluate(set, ov, 0.0);
    let nv = set.vectors.len();
    json!({
        "source": source,
        "note": "cbuffer slots: vectors[i] -> cb[i]; scalars packed 4 per float4 starting at cb[vectors.length].",
        "vectors": set.vectors.iter().enumerate().map(|(i, e)| json!({"slot": i, "expr": expr_json(e), "value": packed.cb[i]})).collect::<Vec<_>>(),
        "scalars": set.scalars.iter().enumerate().map(|(i, e)| json!({"slot": nv + (i >> 2), "comp": comp_name(i & 3), "expr": expr_json(e), "value": packed.cb[nv + (i >> 2)][i & 3]})).collect::<Vec<_>>(),
        "textures2d": set.textures2d.iter().enumerate().map(|(i, e)| match e {
            Expr::Texture { index, .. } => json!({"register": format!("t{i}"), "textureIndex": index, "name": Value::Null}),
            Expr::TextureParameter { name, index, .. } => json!({"register": format!("t{i}"), "textureIndex": index, "name": name}),
            _ => json!({"register": format!("t{i}")}),
        }).collect::<Vec<_>>(),
    })
}

fn comp_name(i: usize) -> &'static str { ["x", "y", "z", "w"][i] }

pub struct Exported { pub shaders: usize, pub uniforms: bool }

/// Slices every shader out of `package` (a `/Game/...` path or `GlobalShaderCache`) into `<out>/<name>/`.
pub fn export_package(set: &PakSet, package: &str, out: &Path, write_uniforms: bool) -> Result<Exported, String> { export_package_with(set, package, out, write_uniforms, true) }

/// `number_suffix = false`: the old hud-uniforms.cjs spelling of parameter names (see [`parse_set_with`]).
pub fn export_package_with(set: &PakSet, package: &str, out: &Path, write_uniforms: bool, number_suffix: bool) -> Result<Exported, String> {
    let name = package.rsplit('/').next().unwrap_or(package).to_string();
    let dir = out.join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bytes = package_bytes(set, package)?;
    let blobs = scan_blobs(&bytes);
    let mut index = vec![];
    for b in &blobs {
        let base = format!("shader_{}", b.id);
        std::fs::write(dir.join(format!("{base}.dxbc")), &b.bytes).map_err(|e| e.to_string())?;
        if let Ok(c) = Container::parse(&b.bytes) {
            if let Ok(t) = disassemble_container(&c, DisasmOptions::default()) { let _ = std::fs::write(dir.join(format!("{base}.asm")), t); }
            if let Ok(sh) = Shader::from_container(&c) { let _ = std::fs::write(dir.join(format!("{base}.layout.json")), serde_json::to_string_pretty(&layout_json(&c, &sh)).unwrap()); }
        }
        index.push(json!({"stream": b.id, "prefixBytes": b.prefix, "dxbcBytes": b.bytes.len(), "file": format!("{base}.dxbc")}));
    }
    std::fs::write(dir.join("index.json"), serde_json::to_string_pretty(&index).unwrap()).map_err(|e| e.to_string())?;
    let mut uniforms = false;
    if write_uniforms && package != "GlobalShaderCache" && package != GLOBAL_CACHE {
        let uasset = set.read_game(package, "uasset").map_err(|e| e.to_string())?.into_owned();
        let names = package_names(&uasset, &bytes)?;
        if let Ok(us) = parse_set_with(&bytes, &names, number_suffix) {
            let doc = uniforms_json(&format!("{package}.uasset"), &us, &Overrides::default());
            std::fs::write(out.join(format!("{name}-uniforms.json")), serde_json::to_string_pretty(&doc).unwrap()).map_err(|e| e.to_string())?;
            uniforms = true;
        }
    }
    Ok(Exported { shaders: blobs.len(), uniforms })
}


