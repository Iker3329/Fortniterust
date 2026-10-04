//! Material resolution: `MaterialInstanceConstant` -> parent -> ... -> `Material` (fn-uasset), merging parameter overrides and reading the root material's blend settings and
//! input connectivity. The cooked packages keep only the parameter nodes of the material graph (scalar / vector / texture sample / static switch); the arithmetic nodes between
//! them are stripped, so which texture feeds which input is exact only when an input is wired straight to a texture-sample parameter (`direct`), otherwise it is chosen by
//! parameter name and flagged in `color_source`.
use crate::model::{MaterialDef, TextureRef};
use fn_pak::PakSet;
use fn_uasset::PackageData;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const INPUTS: &[&str] = &["BaseColor", "Metallic", "Specular", "Roughness", "EmissiveColor", "Opacity", "OpacityMask", "Normal", "WorldPositionOffset", "SubsurfaceColor", "AmbientOcclusion", "Refraction"];

fn pname(v: &Value) -> String {
    v.get("ParameterName").and_then(|x| x.as_str()).map(String::from).or_else(|| v.get("ParameterInfo").and_then(|i| i.get("Name")).and_then(|x| x.as_str()).map(String::from)).unwrap_or_default()
}
fn vec4(v: &Value) -> Option<[f32; 4]> {
    if let Some(a) = v.as_array() { let g = |i: usize| a.get(i).and_then(|x| x.as_f64()).map(|x| x as f32); return Some([g(0)?, g(1)?, g(2)?, g(3).unwrap_or(1.0)]); }
    let g = |k: &str| v.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
    Some([g("r").or_else(|| g("R"))?, g("g").or_else(|| g("G"))?, g("b").or_else(|| g("B"))?, g("a").or_else(|| g("A")).unwrap_or(1.0)])
}
fn package_of(path: &str) -> &str { path.split('.').next().unwrap_or(path) }

/// Resolve a material by object path. Texture entries carry the UE path only (`png` is filled in by the exporter).
pub fn resolve(set: &PakSet, path: &str) -> Result<MaterialDef, String> {
    let mut r = MaterialDef { ue_path: path.to_string(), clip: 0.3333, blend_mode: "BLEND_Opaque".into(), shading_model: "MSM_DefaultLit".into(), ..Default::default() };
    r.name = path.rsplit(|c| c == '.' || c == '/').next().unwrap_or(path).to_string();
    let mut layers: Vec<(Map<String, Value>, String)> = vec![];
    let mut cur = package_of(path).to_string();
    let mut root_exprs: Vec<(String, String, Map<String, Value>)> = vec![];
    for _ in 0..10 {
        let pd = PackageData::load(set, &cur).map_err(|e| format!("{cur}: {e}"))?;
        let pkg = pd.package().map_err(|e| format!("{cur}: {e}"))?;
        let idx = (0..pkg.exports.len()).find(|&i| { let c = pkg.export_class_name(i); c.starts_with("MaterialInstance") || c == "Material" }).ok_or_else(|| format!("{cur}: no material export"))?;
        let class = pkg.export_class_name(idx);
        let props = match pkg.read_export(idx).map_err(|e| format!("{cur}: {e}"))?.props.to_json(&pkg) { Value::Object(m) => m, _ => Map::new() };
        if class == "Material" {
            r.root = cur.clone();
            for i in 0..pkg.exports.len() {
                let c = pkg.export_class_name(i);
                if c.starts_with("MaterialExpression") { if let Ok(o) = pkg.read_export(i) { if let Value::Object(m) = o.props.to_json(&pkg) { root_exprs.push((c, pkg.exports[i].name.clone(), m)); } } }
            }
            layers.push((props, class));
            break;
        }
        let parent = props.get("Parent").and_then(|v| v.as_str()).map(String::from);
        layers.push((props, class));
        match parent { Some(p) if !p.is_empty() => cur = package_of(&p).to_string(), _ => { r.notes.push(format!("{cur}: instance without parent")); break; } }
    }
    if r.root.is_empty() { r.notes.push("root Material not reached".into()); }
    let mut expr_by_name: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut direct_expr: BTreeMap<String, String> = BTreeMap::new();
    if let Some((p, _)) = layers.last().filter(|l| l.1 == "Material") {
        if let Some(b) = p.get("BlendMode").and_then(|v| v.as_str()) { r.blend_mode = b.to_string(); }
        if let Some(b) = p.get("ShadingModel").and_then(|v| v.as_str()) { r.shading_model = b.to_string(); }
        r.two_sided = p.get("TwoSided").and_then(|v| v.as_bool()).unwrap_or(false);
        if let Some(c) = p.get("OpacityMaskClipValue").and_then(|v| v.as_f64()) { r.clip = c as f32; }
        for k in INPUTS {
            if let Some(v) = p.get(*k) {
                let name = v.get("ExpressionName").and_then(|x| x.as_str()).unwrap_or("");
                if !name.is_empty() && name != "None" { r.connected.push((*k).to_string()); direct_expr.insert((*k).to_string(), name.to_string()); }
            }
        }
    }
    for (class, name, p) in &root_exprs {
        let mut pn = pname(&Value::Object(p.clone()));
        if p.get("Texture").and_then(|v| v.as_str()).map_or(false, |s| !s.is_empty()) && !class.contains("Parameter") { pn = format!("@{name}"); }
        expr_by_name.insert(name.clone(), (class.clone(), pn.clone()));
        match class.as_str() {
            "MaterialExpressionScalarParameter" => { if let Some(d) = p.get("DefaultValue").and_then(|v| v.as_f64()) { r.scalars.insert(pn, d as f32); } }
            "MaterialExpressionVectorParameter" => { if let Some(d) = p.get("DefaultValue").and_then(vec4) { r.vectors.insert(pn, d); } }
            // `TextureSampleParameter2D`, `...ParameterSubUV` (sprite sheets), `TextureObjectParameter`
            c if (c.contains("TextureSampleParameter") && !c.contains("Cube")) || c == "MaterialExpressionTextureObjectParameter" => {
                if let Some(t) = p.get("Texture").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { r.textures.insert(pn, TextureRef { name: String::new(), ue_path: t.to_string(), png: String::new(), srgb: true, compression: String::new(), width: 0, height: 0, has_alpha: false }); }
            }
            // a plain texture sample node: no parameter name, keyed by its export name
            // a plain texture sample node (`TextureSample`, `ParticleSubUV` sprite-sheet samples, ...): no parameter name, keyed by its export name
            c if c.starts_with("MaterialExpression") && p.get("Texture").and_then(|v| v.as_str()).map_or(false, |s| !s.is_empty()) => {
                if let Some(t) = p.get("Texture").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { r.textures.insert(format!("@{name}"), TextureRef { name: String::new(), ue_path: t.to_string(), png: String::new(), srgb: true, compression: String::new(), width: 0, height: 0, has_alpha: false }); }
            }
            _ => {}
        }
    }
    for (p, class) in layers.iter().rev().filter(|l| l.1 != "Material") {
        let _ = class;
        if let Some(o) = p.get("BasePropertyOverrides") {
            let on = |k: &str| o.get(format!("bOverride_{k}")).and_then(|v| v.as_bool()).unwrap_or(false);
            if on("BlendMode") { if let Some(b) = o.get("BlendMode").and_then(|v| v.as_str()) { r.blend_mode = b.to_string(); } }
            if on("TwoSided") { r.two_sided = o.get("TwoSided").and_then(|v| v.as_bool()).unwrap_or(r.two_sided); }
            if on("ShadingModel") { if let Some(b) = o.get("ShadingModel").and_then(|v| v.as_str()) { r.shading_model = b.to_string(); } }
            if on("OpacityMaskClipValue") { if let Some(c) = o.get("OpacityMaskClipValue").and_then(|v| v.as_f64()) { r.clip = c as f32; } }
        }
        for v in p.get("ScalarParameterValues").and_then(|v| v.as_array()).into_iter().flatten() { if let Some(x) = v.get("ParameterValue").and_then(|x| x.as_f64()) { r.scalars.insert(pname(v), x as f32); } }
        for v in p.get("VectorParameterValues").and_then(|v| v.as_array()).into_iter().flatten() { if let Some(x) = v.get("ParameterValue").and_then(vec4) { r.vectors.insert(pname(v), x); } }
        for v in p.get("TextureParameterValues").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(x) = v.get("ParameterValue").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
                r.textures.insert(pname(v), TextureRef { name: String::new(), ue_path: x.to_string(), png: String::new(), srgb: true, compression: String::new(), width: 0, height: 0, has_alpha: false });
            }
        }
    }
    for (input, expr) in &direct_expr {
        if let Some((class, pn)) = expr_by_name.get(expr) { if (class.contains("TextureSample") || class.contains("ParticleSubUV")) && !pn.is_empty() { r.direct.insert(input.clone(), pn.clone()); } }
    }
    Ok(r)
}

/// Picks the particle's colour texture parameter once the texture facts are known (`has_alpha`, compression): the one wired to BaseColor / EmissiveColor when the graph says so,
/// else by name (diffuse-like names win; normal / mask / noise / gradient names lose; colour compression and alpha break ties).
pub fn choose_color_texture(m: &mut MaterialDef) {
    for input in ["BaseColor", "EmissiveColor", "OpacityMask", "Opacity"] {
        if let Some(p) = m.direct.get(input) { if m.textures.contains_key(p) { m.color_texture = Some(p.clone()); m.color_source = format!("graph: {input} is wired straight to the texture parameter"); return; } }
    }
    let score = |name: &str, t: &TextureRef| -> i32 {
        let l = name.to_lowercase();
        let mut s = 0;
        for (k, w) in [("diffuse", 60), ("albedo", 60), ("base", 40), ("color", 25), ("texture", 25), ("smoke", 25), ("particle", 20), ("sprite", 20), ("main", 20), ("tex", 10)] { if l.contains(k) { s += w; } }
        for (k, w) in [("normal", 90), ("mask", 40), ("noise", 50), ("erode", 40), ("gradient", 45), ("distort", 50), ("refract", 60), ("detail", 20), ("opacity", 10), ("spec", 50), ("rough", 50)] { if l.contains(k) { s -= w; } }
        if t.compression == "TC_Default" { s += 15 } else if t.compression == "TC_Normalmap" { s -= 60 } else if t.compression == "TC_Masks" { s -= 15 }
        if t.has_alpha { s += 5; }
        s - (l.len() as i32) / 10
    };
    let best = m.textures.iter().max_by_key(|(n, t)| score(n, t)).map(|(n, _)| n.clone());
    m.color_source = if best.is_some() { "name heuristic (the material graph arithmetic is not in the cooked package)".into() } else { "no texture parameter".into() };
    m.color_texture = best;
}
