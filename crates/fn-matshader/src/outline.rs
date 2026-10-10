//! OUTLINE: the build preview's highlight as the original 1.8 data draws it (renderer side: `fn_render::outline_pass`).
//!
//! RECOVERED:
//! * `AFortPlayerController` vfunc_672 @140849310 highlights a list of building actors: each gets custom depth with the custom stencil value of
//!   `Stencil_ID_Definitions.SelectionOutlineID_CodeReferenced` (`ABuildingActor` vfunc_259 @1404823b0, applied by vfunc_261 @1404894c0), and the
//!   time-of-day manager's `PostProcessMaterialMID` (a MID of `TODM_A.BasePostProcessMaterial` = `SkinAndMeshOutlinePostProcess`) gets the vector
//!   parameters `OutlineColor`, `SceneModulation1`, `SceneModulation2` = `UFortGameData::HighlightColors[mode]` (`FUN_140588150`: `ValidHighlight` when the
//!   last argument is set, else `InvalidHighlight`) and `OutlinePostProcessOpacity` 1 (a scalar this material does not have).
//! * The build preview update (FUN_140835750): an Obstructed placement highlights the obstructing actors with mode 0 / Invalid; otherwise the actors the
//!   placement would destroy (`bDestroyOnPlayerBuildingPlacement`) with mode 2 / Valid.
//! * `HighlightColors` = the `UFortGameData` ctor @1405751d0 defaults (Valid: Outline (1,1,1,1), SceneModulation1 (1,1,1,1), SceneModulation2 (2,2,2,2);
//!   Invalid: Outline (0.65,0.07,0.03,1), SceneModulation1 (0.65,0.07,0.03,1), SceneModulation2 (1.65,1.07,1.03,2)) with the `DefaultGameData` overrides.
//! * The post program is the material's cooked `FFPostProcessMaterialPS_HighEnd`, translated from DXBC and run unchanged. Its four textures are not in the
//!   material's tables (scene textures); they are told apart by how the program reads them: `textureLoad` = CustomStencil (`.y`, the X24_G8 view), the
//!   sampled-once depth = SceneDepth, the multi-tap depth (the edge filter) = CustomDepth, the only `textureSample` = PostprocessInput0.
use crate::material::ResolvedMaterial;
use crate::view::ViewInput;
use fn_fx::data::{long_package, PackageJson};
use fn_pak::PakSet;
use fn_render::outline_pass::{OutlineBind, OutlineDesc, OutlineFrame};
use std::collections::HashMap;

pub const MATERIAL: &str = "/Game/Packages/Fortnite_UniqueMaterials_1/SkinAndMeshOutlinePostProcess.SkinAndMeshOutlinePostProcess";
pub const PROGRAM: &str = "FFPostProcessMaterialPS_HighEnd";
pub const GAME_DATA: &str = "/Game/Balance/DefaultGameData";
/// `View` float4 slots the program reads that `ViewInput::view_buffer` does not fill.
pub const BUFFER_SIZE_AND_INV_SIZE: usize = 122;
pub const BUFFER_BILINEAR_UV_MIN_MAX: usize = 154;

const FULLSCREEN_VS: &str = r#"
@vertex fn outline_vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

/// `FFortHighlightColors`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighlightColors { pub outline: [f32; 4], pub scene1: [f32; 4], pub scene2: [f32; 4] }

/// `UFortGameData::HighlightColors[3]` as (Valid, Invalid) pairs: the ctor defaults with the `DefaultGameData` values on top.
pub fn highlight_colors(set: &PakSet) -> Result<[[HighlightColors; 2]; 3], String> {
    let valid = HighlightColors { outline: [1.; 4], scene1: [1.; 4], scene2: [2.; 4] };
    let invalid = HighlightColors { outline: [0.65, 0.07, 0.03, 1.], scene1: [0.65, 0.07, 0.03, 1.], scene2: [1.65, 1.07, 1.03, 2.] };
    let mut out = [[valid, invalid]; 3];
    let pkg = PackageJson::load(set, GAME_DATA)?;
    let cdo = pkg.exports.iter().find(|e| e.props.get("HighlightColors").is_some() || e.props.get("HighlightColors[1]").is_some() || e.props.get("HighlightColors[2]").is_some());
    let Some(e) = cdo else { return Ok(out) };
    let c4 = |v: &serde_json::Value| -> Option<[f32; 4]> { let a = v.as_array()?; Some([0, 1, 2, 3].map(|i| a.get(i).and_then(|x| x.as_f64()).unwrap_or(0.) as f32)) };
    for i in 0..3 {
        let key = if i == 0 { "HighlightColors".to_owned() } else { format!("HighlightColors[{i}]") };
        let Some(c) = e.props.get(&key) else { continue };
        for (j, side) in ["ValidHighlight", "InvalidHighlight"].iter().enumerate() {
            let Some(h) = c.get(*side) else { continue };
            if let Some(v) = h.get("OutlineColor").and_then(c4) { out[i][j].outline = v; }
            if let Some(v) = h.get("SceneModulationColor1").and_then(c4) { out[i][j].scene1 = v; }
            if let Some(v) = h.get("SceneModulationColor2").and_then(c4) { out[i][j].scene2 = v; }
        }
    }
    Ok(out)
}

/// A material parameter collection's uniform buffer defaults (scalars packed 4 per float4, then the vectors) + the scalar and vector names.
pub fn collection(set: &PakSet, object_path: &str) -> Result<(Vec<[f32; 4]>, Vec<String>, Vec<String>), String> {
    let pkg = PackageJson::load(set, long_package(object_path))?;
    let e = pkg.of_class("MaterialParameterCollection").next().ok_or_else(|| format!("{object_path}: no MaterialParameterCollection export"))?;
    let num = |v: &serde_json::Value| v.as_f64().unwrap_or(0.) as f32;
    let list = |k: &str| e.props.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let (scalars, vectors) = (list("ScalarParameters"), list("VectorParameters"));
    let mut out = vec![[0f32; 4]; scalars.len().div_ceil(4)];
    let name = |p: &serde_json::Value| p.get("ParameterName").and_then(|v| v.as_str()).unwrap_or("").to_owned();
    for (i, p) in scalars.iter().enumerate() { out[i / 4][i % 4] = p.get("DefaultValue").map(num).unwrap_or(0.); }
    for p in &vectors {
        let d = p.get("DefaultValue");
        let c = |k: &str, i: usize| d.and_then(|d| d.get(k).or_else(|| d.as_array().and_then(|a| a.get(i)))).map(num).unwrap_or(0.);
        out.push([c("R", 0), c("G", 1), c("B", 2), c("A", 3)]);
    }
    Ok((out, scalars.iter().map(name).collect(), vectors.iter().map(name).collect()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role { View, Collection(usize), Material }

/// Everything decoded from the packages; `desc` goes to `fn_render::outline_pass::OutlineRenderer::create`.
pub struct OutlineAssets {
    pub desc: OutlineDesc,
    roles: Vec<(Role, usize)>,
    mat: ResolvedMaterial,
    collections: Vec<Vec<[f32; 4]>>,
    /// float4 slot of `FortniteActiveTimeOfDay` in each collection buffer (None = not in that collection).
    tod_slots: Vec<Option<usize>>,
    pub colors: [[HighlightColors; 2]; 3],
    /// `SelectionOutlineID_CodeReferenced`.
    pub selection_stencil: f32,
    pub notes: Vec<String>,
}

fn declarations(wgsl: &str) -> Vec<(u32, u32, String)> {
    wgsl.lines().filter_map(|l| {
        let r = l.strip_prefix("@group(")?;
        let (g, r) = r.split_once(") @binding(")?;
        let (b, r) = r.split_once(") ")?;
        Some((g.parse().ok()?, b.parse().ok()?, r.to_owned()))
    }).collect()
}
fn var_name(decl: &str) -> Option<String> { Some(decl.split("var").nth(1)?.split(':').next()?.trim().rsplit(' ').next()?.to_owned()) }

impl OutlineAssets {
    pub fn load(set: &PakSet) -> Result<OutlineAssets, String> {
        let mat = ResolvedMaterial::load(set, MATERIAL)?;
        let prog = mat.maps.find(mat.map_index, "", PROGRAM).ok_or_else(|| format!("{MATERIAL}: no {PROGRAM}"))?;
        let mut body = fn_dxbc::codegen::translate_wgsl(&prog.shader, "ps_fx").map_err(|e| e.0)?;
        // texture roles from how the program reads them
        let mut uses: HashMap<String, (usize, usize, usize, Option<String>)> = HashMap::new(); // (sample, sample_level, load, sampler)
        for (kind, pat) in [(0usize, "textureSample("), (1, "textureSampleLevel("), (2, "textureLoad(")] {
            for part in body.split(pat).skip(1) {
                let args: Vec<&str> = part.splitn(3, ',').collect();
                let t = args[0].trim().to_owned();
                let e = uses.entry(t).or_default();
                match kind { 0 => e.0 += 1, 1 => e.1 += 1, _ => e.2 += 1 }
                if kind < 2 { if let Some(s) = args.get(1) { e.3 = Some(s.trim().to_owned()); } }
            }
        }
        let mut role_of: HashMap<String, OutlineBind> = HashMap::new();
        let loads: Vec<&String> = uses.iter().filter(|(_, u)| u.2 > 0).map(|(t, _)| t).collect();
        let samples: Vec<&String> = uses.iter().filter(|(_, u)| u.0 > 0).map(|(t, _)| t).collect();
        let mut levels: Vec<(&String, usize)> = uses.iter().filter(|(_, u)| u.1 > 0 && u.0 == 0 && u.2 == 0).map(|(t, u)| (t, u.1)).collect();
        levels.sort_by_key(|x| x.1);
        let ([stencil], [color], [(scene_depth, 1), (custom_depth, n)]) = (loads.as_slice(), samples.as_slice(), levels.as_slice()) else {
            return Err(format!("{PROGRAM}: unexpected scene texture reads {uses:?}"));
        };
        if *n < 3 { return Err(format!("{PROGRAM}: the custom depth is read {n} times (expected the edge filter's taps)")); }
        role_of.insert((*stencil).clone(), OutlineBind::CustomStencil);
        role_of.insert((*color).clone(), OutlineBind::SceneColor);
        role_of.insert((*scene_depth).clone(), OutlineBind::SceneDepth);
        role_of.insert((*custom_depth).clone(), OutlineBind::CustomDepth);
        let mut sampler_of: HashMap<String, OutlineBind> = HashMap::new();
        for (t, u) in &uses { if let Some(s) = &u.3 { sampler_of.insert(s.clone(), if role_of.get(t) == Some(&OutlineBind::SceneColor) { OutlineBind::LinearSampler } else { OutlineBind::PointSampler }); } }
        // the stencil is unsigned: declare it so and hand the program the bits it expects (it bitcasts the loaded value back to u32)
        body = body.lines().map(|l| {
            if l.starts_with("@group(") && var_name(l).as_deref() == Some(stencil.as_str()) { return l.replace("texture_2d<f32>", "texture_2d<u32>"); }
            let call = format!("textureLoad({stencil},");
            if let Some(i) = l.find(&call) {
                if let Some(end) = l.rfind(");") { return format!("{}bitcast<vec4<f32>>({}){}", &l[..i], &l[i..end + 1], &l[end + 1..]); }
            }
            l.to_owned()
        }).collect::<Vec<_>>().join("\n");
        let mut entries = vec![];
        let mut sizes = vec![];
        let mut roles = vec![];
        for (g, b, decl) in declarations(&body) {
            let name = var_name(&decl).ok_or("declaration name")?;
            if decl.starts_with("var<uniform>") {
                let n: usize = name.strip_prefix("cb").and_then(|x| x.parse().ok()).ok_or("uniform name")?;
                let size: usize = decl.split("vec4<f32>, ").nth(1).and_then(|r| r.split('>').next()).and_then(|x| x.parse().ok()).ok_or("uniform size")?;
                let role = match prog.ubo_names.get(n).map(String::as_str) {
                    Some("View") => Role::View, Some("Material") => Role::Material,
                    Some(c) if c.starts_with("MaterialCollection") => Role::Collection(c["MaterialCollection".len()..].parse().map_err(|_| format!("collection name {c}"))?),
                    other => return Err(format!("{PROGRAM}: uniform buffer {n} is {other:?}")),
                };
                entries.push((g, b, OutlineBind::Uniform(sizes.len())));
                roles.push((role, size));
                sizes.push(size * 16);
            } else if decl.contains(": sampler;") {
                entries.push((g, b, *sampler_of.get(&name).ok_or_else(|| format!("sampler {name} is not used"))?));
            } else {
                entries.push((g, b, *role_of.get(&name).ok_or_else(|| format!("texture {name} has no role"))?));
            }
        }
        // collections in MaterialCollectionN order (the uniform set's ParameterCollections, resolved by fn-fx)
        let mut collections = vec![];
        let mut tod_slots = vec![];
        let mut selection_stencil = None;
        let mut notes = vec![];
        for (i, c) in mat.fx.collections.iter().enumerate() {
            let Some(path) = c else { collections.push(vec![]); tod_slots.push(None); continue };
            let (buf, scalars, vectors) = collection(set, path)?;
            let base = scalars.len().div_ceil(4);
            tod_slots.push(vectors.iter().position(|n| n == "FortniteActiveTimeOfDay").map(|j| base + j));
            if let Some(k) = scalars.iter().position(|n| n == "SelectionOutlineID_CodeReferenced") { selection_stencil = Some(buf[k / 4][k % 4]); }
            notes.push(format!("MaterialCollection{i} = {path} ({} float4)", buf.len()));
            collections.push(buf);
        }
        let selection_stencil = selection_stencil.ok_or("no collection has SelectionOutlineID_CodeReferenced")?;
        let colors = highlight_colors(set)?;
        let desc = OutlineDesc { label: "SkinAndMeshOutlinePostProcess".into(), wgsl: format!("{body}\n{FULLSCREEN_VS}"), vs_entry: "outline_vs".into(), fs_entry: "ps_fx_main".into(), entries, buffer_sizes: sizes };
        Ok(OutlineAssets { desc, roles, mat, collections, tod_slots, colors, selection_stencil, notes })
    }

    /// The frame's uniform buffers: `View` of the camera, the collections (with `FortniteActiveTimeOfDay` = `tod_weights`), the material with the highlight colours.
    /// `highlight` = None: the pass does not run.
    pub fn frame(&self, view: &ViewInput, tod_weights: [f32; 4], highlight: Option<HighlightColors>) -> OutlineFrame {
        let Some(h) = highlight else { return OutlineFrame { active: false, stencil: self.selection_stencil, buffer_writes: vec![] } };
        let (w, ht) = (view.width.max(1) as f32, view.height.max(1) as f32);
        let mut v = view.view_buffer([0.; 3]);
        if v.len() <= BUFFER_BILINEAR_UV_MIN_MAX { v.resize(BUFFER_BILINEAR_UV_MIN_MAX + 1, [0.; 4]); }
        v[BUFFER_SIZE_AND_INV_SIZE] = [w, ht, 1. / w, 1. / ht];
        // BufferBilinearUVMinMax: the view rect's texel centres in buffer UV (SceneRendering.cpp)
        v[BUFFER_BILINEAR_UV_MIN_MAX] = [0.5 / w, 0.5 / ht, (w - 0.5) / w, (ht - 0.5) / ht];
        let mut ov = fn_dxbc_cli::uniform::Overrides { vectors: self.mat.fx.overrides.vectors.clone(), scalars: self.mat.fx.overrides.scalars.clone() };
        ov.vectors.insert("OutlineColor".into(), h.outline);
        ov.vectors.insert("SceneModulation1".into(), h.scene1);
        ov.vectors.insert("SceneModulation2".into(), h.scene2);
        let material = fn_dxbc_cli::uniform::evaluate(&self.mat.map().uniform, &ov, view.time).cb;
        let bytes = |mut x: Vec<[f32; 4]>, n: usize| { x.resize(n.max(1), [0.; 4]); crate::view::bytes(&x) };
        let writes = self.roles.iter().enumerate().map(|(i, &(role, size))| (i, match role {
            Role::View => bytes(v.clone(), size),
            Role::Material => bytes(material.clone(), size),
            Role::Collection(c) => {
                let mut buf = self.collections.get(c).cloned().unwrap_or_default();
                if let Some(Some(s)) = self.tod_slots.get(c) { if *s < buf.len() { buf[*s] = tod_weights; } }
                bytes(buf, size)
            }
        })).collect();
        OutlineFrame { active: true, stencil: self.selection_stencil, buffer_writes: writes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set() -> Option<PakSet> { PakSet::open_install(fn_fx::default_install()).ok() }

    /// The data: DefaultGameData's overrides on the ctor defaults, the stencil ID, the program's bindings (one of each scene texture), and the translated
    /// program + bindings pass WebGPU validation.
    #[test] fn outline_material_loads_binds_and_validates() {
        let Some(set) = set() else { eprintln!("no 1.8 install: skipped"); return };
        let a = OutlineAssets::load(&set).unwrap();
        assert_eq!(a.selection_stencil, 2.);
        assert_eq!(a.colors[0][1].outline, [0.65, 0.07, 0.03, 1.], "mode 0 Invalid outline = ctor default");
        assert_eq!(a.colors[0][1].scene2, [1., 0.8, 0.8, 1.], "mode 0 Invalid SceneModulationColor2 = DefaultGameData");
        assert_eq!(a.colors[2][0].outline, [1., 0.875397, 0., 1.], "mode 2 Valid outline = DefaultGameData");
        for r in [OutlineBind::SceneDepth, OutlineBind::CustomDepth, OutlineBind::SceneColor, OutlineBind::CustomStencil] { assert_eq!(a.desc.entries.iter().filter(|e| e.2 == r).count(), 1, "{r:?}"); }
        let gpu = match fn_render::Gpu::headless() { Ok(g) => g, Err(e) => { eprintln!("no GPU: {e}"); return } };
        let mut r = fn_render::outline_pass::OutlineRenderer::new(&gpu.device, &gpu.queue);
        r.create(a.desc.clone(), fn_render::renderer::HDR_FORMAT).unwrap();
        let view = ViewInput { eye: glam::Vec3::new(0., 2., 5.), forward: glam::Vec3::NEG_Z, up: glam::Vec3::Y, tan_half: (1., 0.6), near_m: 0.1, far_m: 1000., width: 640, height: 360, time: 1.5 };
        let f = a.frame(&view, [0., 1., 0., 0.], Some(a.colors[2][0]));
        assert!(f.active && f.buffer_writes.len() == a.desc.buffer_sizes.len());
    }
}
