//! A material (or material instance chain) of the original packages: its serialized flags (blend mode, shading model, two-sidedness, depth test, mask clip), the
//! parameter overrides of the instance chain, the textures its uniform expressions reference (in the order the engine's `ReferencedTextures` has), the
//! `FMaterialUniformExpressionSet` recipe of its constant buffer and the cooked shader programs of the root material.
use crate::cooked::Cooked;
use crate::data::{long_package, package_files, PackageJson};
use crate::uniform::{evaluate, parse_set, Expr, Overrides, Packed, UniformSet};
use fn_pak::PakSet;
use serde_json::Value;
use std::collections::HashMap;

/// Serialized material flags (`UMaterial` properties; the MIC `BasePropertyOverrides` replace them).
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialFlags {
    /// `BLEND_Opaque` / `Masked` / `Translucent` / `Additive` / `Modulate` (default `BLEND_Opaque`).
    pub blend: String,
    /// `MSM_DefaultLit` (default) / `MSM_Unlit` / ...
    pub shading: String,
    pub two_sided: bool,
    pub disable_depth_test: bool,
    pub opacity_mask_clip: f32,
    pub translucency_lighting_mode: String,
    pub separate_translucency: bool,
}

pub struct Material {
    /// Object path of the requested material (the instance when one was asked for).
    pub path: String,
    /// Object path of the root `Material` whose shader map is used.
    pub base_path: String,
    pub flags: MaterialFlags,
    /// Effective parameters of the chain (child over parent).
    pub overrides: Overrides,
    pub texture_overrides: HashMap<String, String>,
    pub cooked: Cooked,
    pub uniform: UniformSet,
    /// `UMaterial::ReferencedTextures` of the root (object paths).
    pub referenced_textures: Vec<String>,
    /// Material-function packages walked while collecting the textures (diagnostics).
    pub functions: Vec<String>,
}

fn obj_path(v: &Value) -> Option<String> { v.as_str().map(|s| s.to_owned()).filter(|s| !s.is_empty()) }
fn num(v: &Value) -> Option<f64> { v.as_f64() }

fn flags_from(props: &Value, base: Option<&MaterialFlags>) -> MaterialFlags {
    let mut f = base.cloned().unwrap_or(MaterialFlags { blend: "BLEND_Opaque".into(), shading: "MSM_DefaultLit".into(), two_sided: false, disable_depth_test: false, opacity_mask_clip: 0.3333, translucency_lighting_mode: "TLM_VolumetricNonDirectional".into(), separate_translucency: true });
    let s = |k: &str| props.get(k).and_then(|v| v.as_str()).map(|s| s.to_owned());
    if let Some(v) = s("BlendMode") { f.blend = v; }
    if let Some(v) = s("ShadingModel") { f.shading = v; }
    if let Some(v) = props.get("TwoSided").and_then(|v| v.as_bool()) { f.two_sided = v; }
    if let Some(v) = props.get("bDisableDepthTest").and_then(|v| v.as_bool()) { f.disable_depth_test = v; }
    if let Some(v) = props.get("OpacityMaskClipValue").and_then(num) { f.opacity_mask_clip = v as f32; }
    if let Some(v) = s("TranslucencyLightingMode") { f.translucency_lighting_mode = v; }
    if let Some(v) = props.get("bEnableSeparateTranslucency").and_then(|v| v.as_bool()) { f.separate_translucency = v; }
    f
}

/// Textures of a material in `UMaterial::ReferencedTextures` order: the expressions of the `Expressions` array in order, function calls recursing into the function's
/// `FunctionExpressions` array, duplicates removed (`AddUnique`).
fn collect_textures(set: &PakSet, pkg: &PackageJson, expressions: &[Value], out: &mut Vec<String>, functions: &mut Vec<String>, depth: usize) -> Result<(), String> {
    if depth > 8 { return Err("material function nesting".into()); }
    for e in expressions {
        let Some(path) = obj_path(e) else { continue };
        let Some(ex) = pkg.get(&path) else { continue };
        if ex.class.starts_with("MaterialExpressionTextureSample") || ex.class == "MaterialExpressionTextureObject" || ex.class == "MaterialExpressionTextureObjectParameter" {
            if let Some(t) = ex.props.get("Texture").and_then(obj_path) { if !out.contains(&t) { out.push(t); } }
        } else if ex.class == "MaterialExpressionMaterialFunctionCall" {
            if let Some(f) = ex.props.get("MaterialFunction").and_then(obj_path) {
                let long = long_package(&f).to_owned();
                if !functions.contains(&long) { functions.push(long.clone()); }
                let fp = PackageJson::load(set, &long)?;
                if let Some(fe) = fp.exports.iter().find(|x| x.class == "MaterialFunction") {
                    let list: Vec<Value> = fe.props.get("FunctionExpressions").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                    collect_textures(set, &fp, &list, out, functions, depth + 1)?;
                }
            }
        }
    }
    Ok(())
}

impl Material {
    /// Loads `object_path` (`/Game/.../Name.Name`, a `Material` or `MaterialInstanceConstant`) and everything above it.
    pub fn load(set: &PakSet, object_path: &str) -> Result<Material, String> {
        let mut chain: Vec<(String, PackageJson)> = vec![];
        let mut cur = object_path.to_owned();
        let (root_path, root_pkg) = loop {
            if chain.len() > 16 { return Err(format!("{object_path}: material parent chain too deep")); }
            let long = long_package(&cur).to_owned();
            let pkg = PackageJson::load(set, &long)?;
            let name = cur.rsplit('.').next().unwrap_or(&cur).to_owned();
            let ex = pkg.by_name(&name).ok_or_else(|| format!("{cur}: export {name} missing"))?;
            match ex.class.as_str() {
                "Material" => break (cur, pkg),
                "MaterialInstanceConstant" => {
                    let parent = ex.props.get("Parent").and_then(obj_path).ok_or_else(|| format!("{cur}: instance without parent"))?;
                    chain.push((cur.clone(), pkg));
                    cur = parent;
                }
                other => return Err(format!("{cur}: unsupported material class {other}")),
            }
        };
        let root_name = root_path.rsplit('.').next().unwrap().to_owned();
        let root_ex = root_pkg.by_name(&root_name).unwrap();
        let mut flags = flags_from(&root_ex.props, None);
        let mut overrides = Overrides::default();
        let mut texture_overrides = HashMap::new();
        // parent-most instance first so that children override
        for (path, pkg) in chain.iter().rev() {
            let name = path.rsplit('.').next().unwrap();
            let ex = pkg.by_name(name).unwrap();
            if let Some(o) = ex.props.get("BasePropertyOverrides") { flags = flags_from(o, Some(&flags)); }
            if let Some(a) = ex.props.get("ScalarParameterValues").and_then(|v| v.as_array()) {
                for p in a { if let (Some(n), Some(v)) = (p.get("ParameterName").and_then(|v| v.as_str()), p.get("ParameterValue").and_then(num)) { overrides.scalars.insert(n.to_owned(), v as f32); } }
            }
            if let Some(a) = ex.props.get("VectorParameterValues").and_then(|v| v.as_array()) {
                for p in a {
                    let (Some(n), Some(v)) = (p.get("ParameterName").and_then(|v| v.as_str()), p.get("ParameterValue").and_then(|v| v.as_array())) else { continue };
                    let c: Vec<f32> = v.iter().map(|x| x.as_f64().unwrap_or(0.) as f32).collect();
                    if c.len() == 4 { overrides.vectors.insert(n.to_owned(), [c[0], c[1], c[2], c[3]]); }
                }
            }
            if let Some(a) = ex.props.get("TextureParameterValues").and_then(|v| v.as_array()) {
                for p in a { if let (Some(n), Some(t)) = (p.get("ParameterName").and_then(|v| v.as_str()), p.get("ParameterValue").and_then(obj_path)) { texture_overrides.insert(n.to_owned(), t); } }
            }
        }
        // shader programs and uniform expression set of the root
        let root_long = long_package(&root_path).to_owned();
        let (asset, uexp, ubulk) = package_files(set, &root_long)?;
        let p = fn_ueio::Package::parse(&asset, &uexp, &ubulk).map_err(|e| format!("{root_long}: {e}"))?;
        let cooked = Cooked::parse(&uexp, &p.names, p.summary.total_header_size as usize)?;
        let uniform = parse_set(&uexp, &p.names).map_err(|e| format!("{root_long}: {e}"))?;
        let exprs: Vec<Value> = root_ex.props.get("Expressions").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let (mut referenced_textures, mut functions) = (vec![], vec![]);
        collect_textures(set, &root_pkg, &exprs, &mut referenced_textures, &mut functions, 0)?;
        Ok(Material { path: object_path.to_owned(), base_path: root_path, flags, overrides, texture_overrides, cooked, uniform, referenced_textures, functions })
    }

    /// The packed material constant buffer (`cb[i]` of the `Material` uniform buffer) at `time`.
    pub fn packed(&self, time: f32) -> Packed { evaluate(&self.uniform, &self.overrides, time) }

    /// The object path of 2D texture slot `k` of the material uniform buffer (`Texture2D_k`): the instance's texture parameter when the expression is a parameter
    /// that is overridden, else the referenced texture the expression indexes.
    pub fn texture_slot(&self, k: usize) -> Option<(String, i32)> {
        match self.uniform.textures2d.get(k)? {
            Expr::Texture { index, sampler_source } => self.referenced_textures.get(*index as usize).map(|t| (t.clone(), *sampler_source)),
            Expr::TextureParameter { name, index, sampler_source } => self.texture_overrides.get(name).cloned().or_else(|| self.referenced_textures.get(*index as usize).cloned()).map(|t| (t, *sampler_source)),
            _ => None,
        }
    }
}

