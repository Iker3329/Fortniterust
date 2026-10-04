//! Material resolution for props: walks `MaterialInstanceConstant` -> parent -> ... -> `Material` (fn-uasset), merges the
//! parameter overrides and reads the root material's blend settings, parameter defaults and input connectivity.
//!
//! The cooked packages keep only the *parameter* nodes of the material graph (scalar / vector / texture-sample / static
//! switch); the arithmetic nodes between them are stripped (an input names its source as `ExpressionName`, e.g.
//! `MaterialExpressionMultiply_9`, which has no export). So the texture roles are exact only when an input is wired
//! directly to a texture-sample parameter; otherwise they are chosen from parameter names and texture compression
//! settings and flagged as a guess. The per-instance static switch values live in the native tail of the instance
//! (cooked shader permutation) and are not decoded: only the parent's defaults are reported.
use fn_pak::PakSet;
use fn_uasset::PackageData;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

use super::bp::package_of;

pub const INPUTS: &[&str] = &[
    "BaseColor",
    "Metallic",
    "Specular",
    "Roughness",
    "EmissiveColor",
    "Opacity",
    "OpacityMask",
    "Normal",
    "WorldPositionOffset",
    "SubsurfaceColor",
    "AmbientOcclusion",
    "Refraction",
];

#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub name: String,
    pub ue_path: String,
    pub chain: Vec<String>,
    pub root: String,
    pub blend_mode: String,
    pub shading_model: String,
    pub two_sided: bool,
    pub clip: f64,
    /// parameter name -> texture object path
    pub textures: BTreeMap<String, String>,
    pub scalars: BTreeMap<String, f64>,
    pub vectors: BTreeMap<String, [f64; 4]>,
    pub switches: BTreeMap<String, bool>,
    pub inputs_connected: BTreeMap<String, bool>,
    /// input name -> texture parameter name, when wired directly
    pub direct: BTreeMap<String, String>,
    pub wpo: bool,
    pub notes: Vec<String>,
}

fn vec4(v: &Value) -> Option<[f64; 4]> {
    if let Some(a) = v.as_array() {
        let g = |i: usize| a.get(i).and_then(|x| x.as_f64());
        return Some([g(0)?, g(1)?, g(2)?, g(3).unwrap_or(1.0)]);
    }
    let g = |k: &str| v.get(k).and_then(|x| x.as_f64());
    Some([g("r").or_else(|| g("R"))?, g("g").or_else(|| g("G"))?, g("b").or_else(|| g("B"))?, g("a").or_else(|| g("A")).unwrap_or(1.0)])
}

fn pname(v: &Value) -> String {
    v.get("ParameterName").and_then(|x| x.as_str()).map(String::from).or_else(|| v.get("ParameterInfo").and_then(|i| i.get("Name")).and_then(|x| x.as_str()).map(String::from)).unwrap_or_default()
}

struct Layer {
    props: Map<String, Value>,
    class: String,
}

/// Resolve one material (instance or plain material) by object path (`/Game/..../M_X.M_X`).
pub fn resolve(set: &PakSet, path: &str) -> Result<Resolved, String> {
    let mut r = Resolved { ue_path: path.to_string(), clip: 0.3333, blend_mode: "BLEND_Opaque".into(), shading_model: "MSM_DefaultLit".into(), ..Default::default() };
    r.name = path.rsplit(|c| c == '.' || c == '/').next().unwrap_or(path).to_string();
    let mut layers: Vec<Layer> = vec![];
    let mut cur = package_of(path).to_string();
    let mut root_exprs: Vec<(String, String, Map<String, Value>)> = vec![]; // (class, name, props)
    for _ in 0..10 {
        let pd = PackageData::load(set, &cur).map_err(|e| format!("{cur}: {e}"))?;
        let pkg = pd.package().map_err(|e| format!("{cur}: {e}"))?;
        r.chain.push(cur.clone());
        let idx = (0..pkg.exports.len())
            .find(|&i| {
                let c = pkg.export_class_name(i);
                c.starts_with("MaterialInstance") || c == "Material"
            })
            .ok_or_else(|| format!("{cur}: no material export"))?;
        let class = pkg.export_class_name(idx);
        let obj = pkg.read_export(idx).map_err(|e| format!("{cur}: {e}"))?;
        let props = match obj.props.to_json(&pkg) {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        if class == "Material" {
            r.root = cur.clone();
            for i in 0..pkg.exports.len() {
                let c = pkg.export_class_name(i);
                if c.starts_with("MaterialExpression") {
                    if let Ok(o) = pkg.read_export(i) {
                        if let Value::Object(m) = o.props.to_json(&pkg) {
                            root_exprs.push((c, pkg.exports[i].name.clone(), m));
                        }
                    }
                }
            }
            layers.push(Layer { props, class });
            break;
        }
        let parent = props.get("Parent").and_then(|v| v.as_str()).map(String::from);
        layers.push(Layer { props, class });
        match parent {
            Some(p) if !p.is_empty() => cur = package_of(&p).to_string(),
            _ => {
                r.notes.push(format!("{cur}: instance without parent"));
                break;
            }
        }
    }
    if r.root.is_empty() {
        r.notes.push("root Material not reached".into());
    }
    // root material: blend settings, inputs, parameter defaults
    let mut expr_by_name: BTreeMap<String, (String, String)> = BTreeMap::new(); // expr name -> (class, param name)
    if let Some(root) = layers.last().filter(|l| l.class == "Material") {
        let p = &root.props;
        if let Some(b) = p.get("BlendMode").and_then(|v| v.as_str()) {
            r.blend_mode = b.to_string();
        }
        if let Some(b) = p.get("ShadingModel").and_then(|v| v.as_str()) {
            r.shading_model = b.to_string();
        }
        r.two_sided = p.get("TwoSided").and_then(|v| v.as_bool()).unwrap_or(false);
        if let Some(c) = p.get("OpacityMaskClipValue").and_then(|v| v.as_f64()) {
            r.clip = c;
        }
        for k in INPUTS {
            if let Some(v) = p.get(*k) {
                let name = v.get("ExpressionName").and_then(|x| x.as_str()).unwrap_or("");
                let used = !name.is_empty() && name != "None";
                r.inputs_connected.insert((*k).to_string(), used);
                if used {
                    r.direct.insert((*k).to_string(), name.to_string());
                }
            }
        }
        r.wpo = r.inputs_connected.get("WorldPositionOffset").copied().unwrap_or(false);
    }
    for (class, name, p) in &root_exprs {
        let pn = pname(&Value::Object(p.clone()));
        expr_by_name.insert(name.clone(), (class.clone(), pn.clone()));
        match class.as_str() {
            "MaterialExpressionScalarParameter" => {
                if let Some(d) = p.get("DefaultValue").and_then(|v| v.as_f64()) {
                    r.scalars.insert(pn, d);
                }
            }
            "MaterialExpressionVectorParameter" => {
                if let Some(d) = p.get("DefaultValue").and_then(vec4) {
                    r.vectors.insert(pn, d);
                }
            }
            "MaterialExpressionTextureSampleParameter2D" | "MaterialExpressionTextureObjectParameter" | "MaterialExpressionTextureSampleParameterCube" => {
                if let Some(t) = p.get("Texture").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                    r.textures.insert(pn, t.to_string());
                }
            }
            "MaterialExpressionStaticSwitchParameter" | "MaterialExpressionStaticBoolParameter" => {
                if let Some(d) = p.get("DefaultValue").and_then(|v| v.as_bool()) {
                    r.switches.insert(pn, d);
                }
            }
            _ => {}
        }
    }
    // instance layers, parent first (layers is leaf-first; the last entry is the root material)
    for l in layers.iter().rev().filter(|l| l.class != "Material") {
        let p = &l.props;
        if let Some(o) = p.get("BasePropertyOverrides") {
            let on = |k: &str| o.get(format!("bOverride_{k}")).and_then(|v| v.as_bool()).unwrap_or(false);
            if on("BlendMode") {
                if let Some(b) = o.get("BlendMode").and_then(|v| v.as_str()) {
                    r.blend_mode = b.to_string();
                }
            }
            if on("TwoSided") {
                r.two_sided = o.get("TwoSided").and_then(|v| v.as_bool()).unwrap_or(r.two_sided);
            }
            if on("ShadingModel") {
                if let Some(b) = o.get("ShadingModel").and_then(|v| v.as_str()) {
                    r.shading_model = b.to_string();
                }
            }
            if on("OpacityMaskClipValue") {
                if let Some(c) = o.get("OpacityMaskClipValue").and_then(|v| v.as_f64()) {
                    r.clip = c;
                }
            }
        }
        for v in p.get("ScalarParameterValues").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(x) = v.get("ParameterValue").and_then(|x| x.as_f64()) {
                r.scalars.insert(pname(v), x);
            }
        }
        for v in p.get("VectorParameterValues").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(x) = v.get("ParameterValue").and_then(vec4) {
                r.vectors.insert(pname(v), x);
            }
        }
        for v in p.get("TextureParameterValues").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(x) = v.get("ParameterValue").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
                r.textures.insert(pname(v), x.to_string());
            }
        }
    }
    // inputs wired straight to a texture-sample parameter
    let mut direct = BTreeMap::new();
    for (input, expr) in &r.direct {
        if let Some((class, pn)) = expr_by_name.get(expr) {
            if class.contains("TextureSampleParameter") && !pn.is_empty() {
                direct.insert(input.clone(), pn.clone());
            }
        }
    }
    r.direct = direct;
    r.notes.push("static switch values of material instances are in the cooked native tail and not decoded: `static_switches` holds the root material's defaults".into());
    Ok(r)
}

fn wind_like(name: &str) -> bool {
    let n = name.to_lowercase();
    ["wind", "leafanim", "leaf anim", "sway", "flutter", "bend", "trunk", "branchanim", "animation", "foliage"].iter().any(|k| n.contains(k))
}

/// Texture facts needed to finish the `.mat.json`.
#[derive(Debug, Clone)]
pub struct TexMeta {
    pub name: String,
    pub png: String, // path relative to the textures dir, e.g. `T_X.png`
    pub srgb: bool,
    pub compression: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub address_x: String,
    pub address_y: String,
    pub has_alpha: bool,
    /// Nearly colourless texture (mean chroma < 6 %): foliage canopies are authored grey and tinted by material vectors.
    pub grey: bool,
}

/// Role guess for the texture parameters.
pub struct SlotChoice {
    pub base_color: Option<String>,
    pub normal: Option<String>,
    pub mask: Option<String>,
    pub opacity_from: Option<String>,
    pub source: &'static str,
}

const NOT_A_SURFACE: &[&str] = &["damage", "thicken", "trunk", "noise", "smoothstep", "smooth step", "panning", "position", "axis", "color change", "chart", "storm", "gradient", "reveal", "index", "refraction", "bounce"];

fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x.eq_ignore_ascii_case(y)).count()
}

pub fn choose_slots(r: &Resolved, tex: &BTreeMap<String, TexMeta>) -> SlotChoice {
    let meta = |param: &str| r.textures.get(param).and_then(|p| tex.get(p));
    let masked = r.blend_mode.contains("Masked");
    let mut s = SlotChoice { base_color: None, normal: None, mask: None, opacity_from: None, source: "name-heuristic" };
    let mut exact = false;
    for (input, field) in [("BaseColor", 0), ("Normal", 1)] {
        if let Some(p) = r.direct.get(input) {
            if r.textures.contains_key(p) {
                exact = true;
                if field == 0 {
                    s.base_color = Some(p.clone());
                } else {
                    s.normal = Some(p.clone());
                }
            }
        }
    }
    let params: Vec<&String> = r.textures.keys().filter(|k| meta(k).is_some()).collect();
    let surface = |p: &String| {
        let l = p.to_lowercase();
        !NOT_A_SURFACE.iter().any(|w| l.contains(w))
    };
    if s.base_color.is_none() {
        const EXACT: &[&str] = &["diffuse", "basecolor", "base color", "albedo", "diffuse map", "diffusemap", "color", "diffuse texture", "main texture", "texture"];
        let score = |p: &&String| -> i32 {
            let l = p.to_lowercase();
            let m = meta(p).unwrap();
            let mut sc = if let Some(i) = EXACT.iter().position(|e| *e == l) {
                100 - i as i32
            } else if l.contains("diffuse") || l.contains("albedo") || l.contains("basecolor") || l.contains("base color") {
                50
            } else if l.contains("color") {
                20
            } else {
                0
            };
            if l.contains("normal") || l.contains("spec") || (l.contains("mask") && !l.contains("diffuse")) {
                sc -= 60;
            }
            sc += if m.compression == "TC_Default" && m.srgb { 10 } else { -40 };
            sc - (l.len() as i32) / 8
        };
        s.base_color = params.iter().filter(|p| surface(p)).max_by_key(|p| score(p)).filter(|p| score(p) > 0).map(|p| (*p).clone());
    }
    if s.normal.is_none() {
        let score = |p: &&String| -> i32 {
            let l = p.to_lowercase();
            let m = meta(p).unwrap();
            let mut sc = if l == "normalmap" || l == "normal" || l == "normals" || l == "normal map" { 100 } else if l.contains("normal") { 40 } else { 0 };
            if m.compression == "TC_Normalmap" {
                sc += 50;
            }
            sc - (l.len() as i32) / 8
        };
        s.normal = params.iter().filter(|p| surface(p)).max_by_key(|p| score(p)).filter(|p| score(p) >= 40).map(|p| (*p).clone());
    }
    let base_tex = s.base_color.as_ref().and_then(|p| meta(p)).map(|m| m.name.clone()).unwrap_or_default();
    let mask_score = |p: &&String| -> i32 {
        let l = p.to_lowercase();
        let m = meta(p).unwrap();
        let mut sc = if l.starts_with("mask") { 60 } else if l.contains("mask") { 30 } else if l.contains("spec") || l.contains("orm") || l.contains("rough") { 20 } else { 0 };
        if l.contains("opacity") {
            sc -= 15;
        }
        if m.compression == "TC_Masks" {
            sc += 15;
        }
        // same texture family as the colour texture (T_Tree_Pine01_D / T_Tree_Pine01_M)
        sc += common_prefix(&m.name, &base_tex).min(24) as i32;
        sc
    };
    s.mask = params
        .iter()
        .filter(|p| surface(p) && Some((**p).clone()) != s.base_color && Some((**p).clone()) != s.normal)
        .max_by_key(|p| mask_score(p))
        .filter(|p| mask_score(p) >= 20)
        .map(|p| (*p).clone());
    if masked {
        let bc_alpha = s.base_color.as_ref().and_then(|p| meta(p)).map_or(false, |m| m.has_alpha);
        s.opacity_from = if bc_alpha { Some("base_color".into()) } else if s.mask.is_some() { Some("mask".into()) } else { None };
    }
    if exact {
        s.source = "graph-direct (colour / normal) + name-heuristic (rest)";
    }
    s
}

/// Final `.mat.json` document. `tex_rel` is the relative path from the `.mat.json` to the shared textures folder.
pub fn mat_json(r: &Resolved, tex: &BTreeMap<String, TexMeta>, tex_rel: &str) -> Value {
    let slots = choose_slots(r, tex);
    let mut textures = Map::new();
    for (param, path) in &r.textures {
        let Some(m) = tex.get(path) else { continue };
        textures.insert(
            param.clone(),
            json!({
                "texture": m.name, "ue_path": path, "png": format!("{tex_rel}/{}", m.png), "srgb": m.srgb, "compression": m.compression, "format": m.format,
                "width": m.width, "height": m.height, "source_width": m.source_width, "source_height": m.source_height,
                "address_x": m.address_x, "address_y": m.address_y, "has_alpha": m.has_alpha, "grey": m.grey,
            }),
        );
    }
    let alpha_mode = if r.blend_mode.contains("Masked") {
        "MASK"
    } else if r.blend_mode.contains("Translucent") || r.blend_mode.contains("Additive") || r.blend_mode.contains("Modulate") {
        "BLEND"
    } else {
        "OPAQUE"
    };
    let scalars: Map<String, Value> = r.scalars.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    let vectors: Map<String, Value> = r.vectors.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    let switches: Map<String, Value> = r.switches.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    let mut wind = Map::new();
    for (k, v) in &r.scalars {
        if wind_like(k) {
            wind.insert(k.clone(), json!(v));
        }
    }
    // greyscale canopy textures are tinted by the material's gradient / subsurface colour vectors (approximation for renderers)
    let base_grey = slots.base_color.as_ref().and_then(|p| r.textures.get(p)).and_then(|t| tex.get(t)).map_or(false, |m| m.grey);
    let (tint_guess, tint_source) = if base_grey {
        let pick = ["Gradient_TopColor", "SimpleSSColor", "Leaf Color", "Tint"].iter().find_map(|k| r.vectors.get(*k).map(|v| (*k, *v)));
        match pick {
            Some((k, v)) => {
                let m = v[0].max(v[1]).max(v[2]).max(1e-6);
                (json!([v[0] / m, v[1] / m, v[2] / m]), json!(format!("{k} normalised to max 1 (guess)")))
            }
            None => (Value::Null, Value::Null),
        }
    } else {
        (Value::Null, Value::Null)
    };
    json!({
        "tint_guess": tint_guess, "tint_source": tint_source,
        "schema": 1, "name": r.name, "ue_path": r.ue_path, "chain": r.chain, "root_material": r.root,
        "blend_mode": r.blend_mode, "alpha_mode": alpha_mode, "opacity_mask_clip_value": r.clip, "two_sided": r.two_sided, "shading_model": r.shading_model,
        "textures": textures,
        "slots": {"base_color": slots.base_color, "normal": slots.normal, "mask": slots.mask, "opacity_from": slots.opacity_from, "source": slots.source},
        "inputs_connected": r.inputs_connected,
        "direct_texture_inputs": r.direct,
        "scalars": scalars, "vectors": vectors, "static_switches": switches,
        "wind": {"uses_world_position_offset": r.wpo, "parameters": wind},
        "notes": r.notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str, comp: &str, srgb: bool, alpha: bool) -> TexMeta {
        TexMeta { name: name.into(), png: format!("{name}.png"), srgb, compression: comp.into(), format: "DXT1".into(), width: 4, height: 4, source_width: 4, source_height: 4, address_x: "TA_Wrap".into(), address_y: "TA_Wrap".into(), has_alpha: alpha, grey: false }
    }

    #[test]
    fn slots_by_name_and_compression() {
        let mut r = Resolved { blend_mode: "BLEND_Masked".into(), ..Default::default() };
        r.textures.insert("Diffuse".into(), "/g/T_D".into());
        r.textures.insert("NormalMap".into(), "/g/T_N".into());
        r.textures.insert("MaskTexture".into(), "/g/T_M".into());
        r.textures.insert("Damage reveal texture".into(), "/g/T_X".into());
        let mut t = BTreeMap::new();
        t.insert("/g/T_D".into(), meta("T_D", "TC_Default", true, true));
        t.insert("/g/T_N".into(), meta("T_N", "TC_Normalmap", false, false));
        t.insert("/g/T_M".into(), meta("T_M", "TC_Masks", false, false));
        t.insert("/g/T_X".into(), meta("T_X", "TC_Default", true, false));
        let s = choose_slots(&r, &t);
        assert_eq!(s.base_color.as_deref(), Some("Diffuse"));
        assert_eq!(s.normal.as_deref(), Some("NormalMap"));
        assert_eq!(s.mask.as_deref(), Some("MaskTexture"));
        assert_eq!(s.opacity_from.as_deref(), Some("base_color"));
    }

    #[test]
    fn direct_wiring_wins() {
        let mut r = Resolved::default();
        r.textures.insert("Tex A".into(), "/g/A".into());
        r.textures.insert("Diffuse".into(), "/g/B".into());
        r.direct.insert("BaseColor".into(), "Tex A".into());
        let mut t = BTreeMap::new();
        t.insert("/g/A".into(), meta("A", "TC_Default", true, false));
        t.insert("/g/B".into(), meta("B", "TC_Default", true, false));
        let s = choose_slots(&r, &t);
        assert_eq!(s.base_color.as_deref(), Some("Tex A"));
        assert!(s.source.starts_with("graph-direct"));
    }

    #[test]
    fn wind_names() {
        assert!(wind_like("LeafAnimationAmount"));
        assert!(wind_like("WindIntensity"));
        assert!(!wind_like("SpecRoughnessMin"));
    }

    #[test]
    fn vec4_both_shapes() {
        assert_eq!(vec4(&json!([1.0, 0.5, 0.25, 1.0])), Some([1.0, 0.5, 0.25, 1.0]));
        assert_eq!(vec4(&json!({"r": 0.1, "g": 0.2, "b": 0.3, "a": 0.4})), Some([0.1, 0.2, 0.3, 0.4]));
    }
}
