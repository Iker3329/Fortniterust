//! Uploads the exported real props (`fn-propdata`: `assets/props/`, written by `fn-export props`) as textured static meshes.
//!
//! ```no_run
//! # fn demo(renderer: &mut fn_render::Renderer) -> Result<(), String> {
//! let set = fn_render::static_loader::load_dir(renderer, std::path::Path::new(r"..\FortniteRuntime\assets\props"))?;
//! let pine = set.get("Athena_Tree_Pine_3").unwrap();
//! let inst = fn_render::StaticMeshInstance::new(pine.mesh, glam::Vec3::new(10., 0., -20.)).with_yaw(1.0);
//! renderer.set_static_instances(&[inst]);
//! # Ok(()) }
//! ```
//! Conversions (everything not exported is ASSUMED and labelled in the README):
//! * LOD switch distance of LOD `i` = `LOD_SCREEN_MULTIPLE * sphere_radius / screen_size_i` (UE 4.16 `ComputeBoundsScreenSize`, recalled);
//!   `RenderInfo.cull_distance_cm` is the cull distance (0 = none).
//! * Opacity comes from the colour texture's alpha (`slots.opacity_from == base_color`), otherwise from the alpha (or red) channel of the
//!   named mask texture. `BLEND_Masked` -> masked at `opacity_mask_clip_value`; translucent blend modes are drawn as masked at 0.5.
//! * Wind: when the material graph drives `WorldPositionOffset` the assumed sway / flutter of `StaticMaterialData` is enabled; the exported
//!   wind parameter values are not used for the motion (the node arithmetic is not recoverable), see the README.
//! * The mesh winding is detected from the signed volume of the one-sided sections and flipped if the export came out clockwise.
use crate::renderer::Renderer;
use crate::static_mesh::*;
use fn_propdata::{LodMesh, Material, PropCatalog, PropKind};
use glam::Vec3;
use image::RgbaImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// `ScreenMultiple` of 4.16's `ComputeBoundsScreenSize`: max(0.5 * Proj[0][0], 0.5 * Proj[1][1]) for the 80 degree (16:9, Hor+) camera. ASSUMED.
pub const LOD_SCREEN_MULTIPLE: f32 = 1.0605;

#[derive(Clone, Debug)]
pub struct LoadedProp {
    pub id: String,
    pub display: String,
    /// `tree`, `rock`, `bush`, ...
    pub category: String,
    pub mesh: StaticMeshHandle,
    /// Relative frequency for scatters (tree 3, rock 2, bush 1).
    pub weight: f32,
    pub height_m: f32,
    pub cull_distance_m: f32,
    pub random_scale: Option<(f32, f32)>,
    pub random_yaw: bool,
    /// Triangles of LOD 0 / of the last LOD.
    pub triangles: (usize, usize),
    pub lod_switch_m: Vec<f32>,
    /// The exported triangles wound clockwise and were flipped on load.
    pub flipped_winding: bool,
}
pub struct PropSet {
    pub props: Vec<LoadedProp>,
    pub material_count: usize,
    /// Kinds that failed to load (id, error); the rest is usable.
    pub skipped: Vec<(String, String)>,
}
impl PropSet {
    pub fn get(&self, id: &str) -> Option<&LoadedProp> { self.props.iter().find(|p| p.id == id) }
}

pub fn looks_like_rock(name: &str) -> bool { let n = name.to_ascii_lowercase(); n.contains("rock") || n.contains("stone") || n.contains("boulder") }

/// Categories `load_dir` uploads.
pub const DEFAULT_CATEGORIES: [&str; 4] = ["tree", "rock", "bush", "hedge"];

/// Loads every tree / rock / bush / hedge kind of `<dir>/catalog.json`.
pub fn load_dir(renderer: &mut Renderer, dir: &Path) -> Result<PropSet, String> {
    let cat = PropCatalog::load(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    load_catalog(renderer, &cat, |k| DEFAULT_CATEGORIES.contains(&k.category.as_str()))
}

/// Uploads the kinds of `cat` accepted by `keep`. Materials / textures shared by several kinds are uploaded once.
pub fn load_catalog(renderer: &mut Renderer, cat: &PropCatalog, keep: impl Fn(&PropKind) -> bool) -> Result<PropSet, String> {
    let mut cache: HashMap<PathBuf, StaticMaterialHandle> = HashMap::new();
    let (mut props, mut skipped) = (vec![], vec![]);
    for k in cat.kinds.iter().filter(|k| keep(k)) {
        match load_kind(renderer, k, &mut cache) { Ok(p) => props.push(p), Err(e) => skipped.push((k.id.clone(), e)) }
    }
    if props.is_empty() && !skipped.is_empty() { return Err(format!("no prop could be loaded: {}: {}", skipped[0].0, skipped[0].1)); }
    Ok(PropSet { props, material_count: cache.len(), skipped })
}

/// The CPU half of one kind (everything `load_kind` does that does not need the GPU: glTF LODs, material + texture PNG decode, LOD switch distances,
/// winding detection). Built on worker threads by `prepare_kind`, uploaded by `upload_prepared` once the renderer exists (`fn-app`'s loader does the
/// decode of every kind on a pool while the renderer's pipelines compile, then uploads kind by kind so only a few kinds' textures are in memory at once).
pub struct PreparedProp {
    pub id: String,
    pub display: String,
    pub category: String,
    /// (slot, .mat.json path, the material unless already uploaded / prepared elsewhere, its mip chains built on the loading thread)
    mats: Vec<(usize, PathBuf, Option<StaticMaterialData>, Option<crate::statics::PrebuiltMips>)>,
    n_slots: usize,
    mesh: StaticMeshData,
    weight: f32,
    height_m: f32,
    cull_distance_m: f32,
    random_scale: Option<(f32, f32)>,
    random_yaw: bool,
    triangles: (usize, usize),
    lod_switch_m: Vec<f32>,
    flipped_winding: bool,
}
impl PreparedProp {
    /// Approximate bytes of decoded texture data held (for budgeting).
    pub fn texture_bytes(&self) -> usize {
        let chain = |c: &Option<Vec<image::RgbaImage>>| c.as_ref().map_or(0, |l| l.iter().map(|i| i.as_raw().len()).sum::<usize>());
        self.mats.iter().map(|m| m.2.as_ref().map_or(0, |d| d.base_color.as_ref().map_or(0, |i| i.as_raw().len()) + d.normal.as_ref().map_or(0, |i| i.as_raw().len())) + m.3.as_ref().map_or(0, |(b, n)| chain(b) + chain(n))).sum()
    }
}

/// VRAM (2026-10-07): set when the props are drawn by the real-shader path (fn-app statics_render.rs, the only one that shades them then): the textured
/// static-mesh copy of each kind is then needed only for the custom-depth outline (fn-app outline_hook.rs: a highlighted prop's silhouette, masked alpha)
/// and as the fallback of a kind the real path cannot build. [`prepare_kind`] then keeps only what the custom depth reads (the base colour of MASKED
/// materials, for its alpha clip) and drops the normal maps and the opaque base colours; fn-app re-prepares a kind in full ([`prepare_kind_ext`]) if the real
/// path fails it. Native and browser loaders alike (both go through [`prepare_kind`]).
static DEPTH_ONLY_TEXTURES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_prop_textures_depth_only(on: bool) { DEPTH_ONLY_TEXTURES.store(on, std::sync::atomic::Ordering::Relaxed); }
pub fn prop_textures_depth_only() -> bool { DEPTH_ONLY_TEXTURES.load(std::sync::atomic::Ordering::Relaxed) }

/// Decodes one kind without touching the GPU. Materials whose `.mat.json` path is in `known` (already uploaded / being prepared elsewhere) are not decoded again.
pub fn prepare_kind(k: &PropKind, known: &std::collections::HashSet<PathBuf>) -> Result<PreparedProp, String> { prepare_kind_ext(k, known, prop_textures_depth_only()) }
/// [`prepare_kind`] with the VRAM choice explicit: `depth_only` = only the textures the custom depth reads (see [`set_prop_textures_depth_only`]).
pub fn prepare_kind_ext(k: &PropKind, known: &std::collections::HashSet<PathBuf>, depth_only: bool) -> Result<PreparedProp, String> {
    let lods = k.load_lods().map_err(|e| e.to_string())?;
    if lods.is_empty() { return Err("no LODs".into()); }
    let max_slot = k.materials.iter().map(|m| m.slot).max().unwrap_or(0).max(lods.iter().flat_map(|l| l.sections.iter().map(|s| s.material_slot)).max().unwrap_or(0));
    let n_slots = max_slot + 1;
    let mut two_sided_slots = vec![false; n_slots];
    let mut mats = vec![];
    for m in &k.materials {
        let path = k.dir().join(&m.mat_json);
        let mat = Material::load(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        two_sided_slots[m.slot] = mat.two_sided;
        let data = if known.contains(&path) { None } else { Some(convert_material_ext(&mat, depth_only)) };
        let mips = data.as_ref().map(crate::statics::prebuilt_mips); // LOAD TIME: built here, on the loading thread, not at upload
        mats.push((m.slot, path, data, mips));
    }
    let flip = detect_flipped_winding(&lods[0], &two_sided_slots);
    let sphere = {
        let e = Vec3::from(k.bounds_extent_cm) * 0.01;
        if e.length() > 0.01 { e.length() } else { 0. }
    };
    let (mut mesh, switch) = convert_mesh(&k.id, &lods, sphere, k.render.cull_distance_cm * 0.01, k.render.cast_shadow, flip);
    // FORTGLOBALS CULL CURVE: every catalogue prop is an ABuildingActor (exe ABuildingActor__vfunc_253 @ 0x140498440, see quality::fort_cull)
    mesh.fort_default_cull = true;
    let tris = (lods[0].triangle_count(), lods.last().map_or(0, |l| l.triangle_count()));
    let weight = match k.category.as_str() { "tree" => 3., "rock" => 2., _ => 1. };
    Ok(PreparedProp {
        id: k.id.clone(), display: k.display.clone(), category: k.category.clone(), mats, n_slots, mesh, weight, height_m: k.height_m(), cull_distance_m: k.render.cull_distance_cm * 0.01,
        random_scale: k.render.random_scale.then_some(k.render.random_scale_range).flatten(), random_yaw: k.render.random_yaw, triangles: tris, lod_switch_m: switch, flipped_winding: flip,
    })
}

/// The GPU half: creates the materials (shared ones once, through `cache`) and the mesh.
pub fn upload_prepared(renderer: &mut Renderer, p: PreparedProp, cache: &mut HashMap<PathBuf, StaticMaterialHandle>) -> LoadedProp {
    let PreparedProp { id, display, category, mats, n_slots, mesh, weight, height_m, cull_distance_m, random_scale, random_yaw, triangles, lod_switch_m, flipped_winding } = p;
    let mut handles: Vec<Option<StaticMaterialHandle>> = vec![None; n_slots];
    for (slot, path, data, mips) in mats {
        let h = match cache.get(&path) {
            Some(h) => Some(*h),
            None => data.map(|d| { let h = crate::statics::with_prebuilt_mips(mips.unwrap_or((None, None)), || renderer.create_static_material(&d)); cache.insert(path, h); h }),
        };
        handles[slot] = h;
    }
    let fallback = handles.iter().flatten().next().copied().unwrap_or_else(|| renderer.create_static_material(&StaticMaterialData { name: format!("{id} fallback"), vertex_color_albedo: true, ..Default::default() }));
    let handles: Vec<StaticMaterialHandle> = handles.into_iter().map(|h| h.unwrap_or(fallback)).collect();
    let mesh = renderer.create_static_mesh(&mesh, &handles);
    LoadedProp { id, display, category, mesh, weight, height_m, cull_distance_m, random_scale, random_yaw, triangles, lod_switch_m, flipped_winding }
}

/// FOLIAGE HOOK: what `prepare_ground_cover` needs (the exported ambient ground cover, `assets/props/foliage/foliage.json`; no catalog entry / `props.json`).
pub struct GroundCoverSpec<'a> {
    pub name: &'a str,
    pub gltf: &'a Path,
    /// `(material slot, path of its .mat.json)`
    pub materials: &'a [(usize, PathBuf)],
    /// `FBoxSphereBounds.BoxExtent` of the mesh (cm), for the LOD / cull sphere.
    pub bounds_extent_cm: [f32; 3],
    /// `InstanceEndCullDistance` (cm): instances are culled beyond it.
    pub cull_cm: f32,
    /// `InstanceEndCullDistance - InstanceStartCullDistance` (cm): the fade-out length before the cull distance.
    pub fade_cm: f32,
    pub cast_shadow: bool,
}

/// FOLIAGE HOOK: decodes the ground-cover mesh + materials without the prop catalog (same conversions as [`prepare_kind`]). The material's opacity is NOT taken from the
/// colour texture's alpha (for `M_FieldGrassMatching` that texture is the terrain grass tile whose alpha is not the blade opacity; the real opacity graph is not recoverable),
/// so the blades are drawn opaque and two-sided; wind stays off (the material's WorldPositionOffset graph is not decoded).
pub fn prepare_ground_cover(s: &GroundCoverSpec) -> Result<PreparedProp, String> {
    let lods = fn_propdata::load_lods(s.gltf).map_err(|e| e.to_string())?;
    if lods.is_empty() { return Err("no LODs".into()); }
    let max_slot = s.materials.iter().map(|m| m.0).max().unwrap_or(0).max(lods.iter().flat_map(|l| l.sections.iter().map(|x| x.material_slot)).max().unwrap_or(0));
    let n_slots = max_slot + 1;
    let mut two_sided_slots = vec![false; n_slots];
    let mut mats = vec![];
    for (slot, path) in s.materials {
        let mat = Material::load(path).map_err(|e| format!("{}: {e}", path.display()))?;
        two_sided_slots[*slot] = mat.two_sided;
        let mut d = convert_material(&mat);
        d.alpha_mode = AlphaMode::Opaque;
        let mips = crate::statics::prebuilt_mips(&d); // LOAD TIME: built here, on the loading thread, not at upload
        mats.push((*slot, path.clone(), Some(d), Some(mips)));
    }
    let flip = detect_flipped_winding(&lods[0], &two_sided_slots);
    let sphere = { let e = Vec3::from(s.bounds_extent_cm) * 0.01; if e.length() > 0.01 { e.length() } else { 0. } };
    let (mut mesh, switch) = convert_mesh(s.name, &lods, sphere, s.cull_cm * 0.01, s.cast_shadow, flip);
    mesh.fade_range = s.fade_cm * 0.01;
    let tris = (lods[0].triangle_count(), lods.last().map_or(0, |l| l.triangle_count()));
    let (lo, hi) = lods[0].bounds();
    Ok(PreparedProp { id: s.name.to_owned(), display: s.name.to_owned(), category: "ground_cover".into(), mats, n_slots, mesh, weight: 1., height_m: hi[1].max(lo[1]), cull_distance_m: s.cull_cm * 0.01, random_scale: None, random_yaw: true, triangles: tris, lod_switch_m: switch, flipped_winding: flip })
}

fn load_kind(renderer: &mut Renderer, k: &PropKind, cache: &mut HashMap<PathBuf, StaticMaterialHandle>) -> Result<LoadedProp, String> {
    let known: std::collections::HashSet<PathBuf> = cache.keys().cloned().collect();
    let p = prepare_kind(k, &known)?;
    Ok(upload_prepared(renderer, p, cache))
}
/// True when the signed volume of the one-sided sections of `lod` is negative (triangles wind clockwise seen from outside). Meshes with no
/// one-sided closed geometry (pure foliage cards) return false.
pub fn detect_flipped_winding(lod: &LodMesh, two_sided_slot: &[bool]) -> bool {
    let mut vol = 0f64;
    for s in &lod.sections {
        if two_sided_slot.get(s.material_slot).copied().unwrap_or(false) { continue; }
        for t in s.indices.chunks_exact(3) {
            let p = |i: u32| { let v = lod.positions[i as usize]; Vec3::new(v[0], v[1], v[2]).as_dvec3() };
            vol += p(t[0]).dot(p(t[1]).cross(p(t[2])));
        }
    }
    vol < 0.
}

/// LOD switch distances (metres) from the exported `screen_size` of every LOD (LOD 0 starts at 0; entry `i-1` = where LOD `i` takes over).
pub fn lod_switch_distances(screen_sizes: &[f32], sphere_radius_m: f32) -> Vec<f32> {
    screen_sizes.iter().skip(1).map(|&s| if s > 1e-4 { LOD_SCREEN_MULTIPLE * sphere_radius_m / s } else { f32::MAX }).collect()
}

fn convert_mesh(name: &str, lods: &[LodMesh], sphere_m: f32, cull_m: f32, cast_shadow: bool, flip: bool) -> (StaticMeshData, Vec<f32>) {
    let mut radius = sphere_m;
    if radius <= 0. {
        let (lo, hi) = lods[0].bounds();
        radius = (Vec3::from(hi) - Vec3::from(lo)).length() * 0.5;
    }
    let screen: Vec<f32> = lods.iter().map(|l| l.screen_size).collect();
    let mut switch = lod_switch_distances(&screen, radius);
    // distances must ascend for the hysteresis logic
    for i in 1..switch.len() { if switch[i] < switch[i - 1] { switch[i] = switch[i - 1]; } }
    let mut out = StaticMeshData { name: name.into(), lods: vec![], cull_distance: cull_m, fade_range: 0., cast_shadow, fort_default_cull: false };
    for (li, l) in lods.iter().enumerate().take(MAX_LODS) {
        let n = l.positions.len();
        let vertices: Vec<StaticVertex> = (0..n).map(|i| StaticVertex {
            pos: l.positions[i],
            normal: l.normals.get(i).copied().unwrap_or([0., 1., 0.]),
            uv: l.uvs.first().and_then(|u| u.get(i)).copied().unwrap_or([0., 0.]),
            color: l.colors.get(i).copied().unwrap_or([255; 4]),
        }).collect();
        let (mut indices, mut sections) = (vec![], vec![]);
        for s in &l.sections {
            let first = indices.len() as u32;
            for t in s.indices.chunks_exact(3) { if flip { indices.extend_from_slice(&[t[0], t[2], t[1]]) } else { indices.extend_from_slice(t) } }
            sections.push(StaticSection { first_index: first, index_count: indices.len() as u32 - first, material: s.material_slot });
        }
        out.lods.push(StaticLod { vertices, indices, sections, max_distance: switch.get(li).copied().unwrap_or(f32::MAX) });
    }
    switch.truncate(out.lods.len().saturating_sub(1));
    (out, switch)
}

fn load_rgba(path: &Path) -> Result<RgbaImage, String> { Ok(std::fs::read(path).map_err(image::ImageError::IoError).and_then(|b| image::load_from_memory(&b)).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8()) }

/// The texture of the first parameter whose name equals (case-insensitively) one of `names`, else the exporter's heuristic slot. The slot
/// heuristic picks by parameter NAME fragments and mislabels material instances (e.g. `Branch Thicken Diffuse Map` over `Diffuse`), so
/// the conventional parameter names win.
fn pick_texture<'a>(m: &'a Material, names: &[&str], slot: Option<&String>) -> Option<&'a fn_propdata::TextureRef> {
    for n in names { if let Some((_, t)) = m.textures.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)) { return Some(t); } }
    slot.and_then(|s| m.textures.get(s))
}

fn scalar(m: &Material, names: &[&str]) -> Option<f32> {
    m.scalars.iter().find(|(k, _)| names.iter().any(|n| k.eq_ignore_ascii_case(n))).map(|(_, v)| *v)
}

/// The value a shader reads from a stored 8-bit channel: an `SRGB` texture's RGB goes through the hardware sRGB decode (UE 4.16 creates the SRV of every
/// `SRGB` B8G8R8A8 / DXT1 / DXT3 / DXT5 / BC7 texture with the `*_UNORM_SRGB` format: Texture2D.cpp:1219 `TexCreate_SRGB`, D3D11RHIPrivate.h:912
/// `FindShaderResourceDXGIFormat`); alpha and non-sRGB textures are read as stored. The exported PNGs are those formats (BC4 / BC5 / G8 / float are never sRGB-decoded
/// and are not used through this path with `srgb` set).
fn sampled_channel(v: u8, channel: usize, srgb: bool) -> f32 {
    let x = v as f32 / 255.;
    if !srgb || channel == 3 { return x; }
    if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
}

/// Mean of each channel of a mask texture as the shader samples it (`srgb`: the texture's `SRGB` flag, RGB decoded to linear first).
fn channel_means(img: &RgbaImage, srgb: bool) -> [f32; 4] {
    let mut s = [0f64; 4];
    for p in img.pixels() { for c in 0..4 { s[c] += sampled_channel(p[c], c, srgb) as f64; } }
    let n = (img.width() as f64 * img.height() as f64).max(1.);
    s.map(|v| (v / n) as f32)
}

/// REFLECTION APPLY (docs/spec/reflection_gaps.md section 3): the GBuffer Specular / Roughness the material's compiled base-pass PS writes
/// (`TBasePassPSFNoLightMapPolicySkylight` of each root material, disassembled with `fn-dxbc disasm`, GBufferB = (Metallic, Specular, Roughness, ShadingModel)):
/// * Fortress_BaseMaterial: Specular = saturate(SpecularMasks.r * SpecularMult), Roughness = saturate(lerp(SpecRoughnessMin, SpecRoughnessMax, SpecularMasks.b))
///   (material uniforms cb[7].x / .y / .z = SpecularMult / SpecRoughnessMax / SpecRoughnessMin);
/// * FortniteFoliage_MasterMaterial: Specular = saturate(S.r * (1 - lerp(Branch Spec AO Level, Leaf Spec AO Level, mask) * AO)), Roughness = saturate(S.b);
/// * DEPRECATED_AnimatedTreeMaterial: Specular = saturate(S.r), Roughness = saturate(lerp(min, max, S.b)) (its uniform set is not decoded: ASSUMED
///   SpecRoughnessMin / Max);
/// * M_FieldGrassMatching: Specular = saturate(Spec) (0.05), Roughness = saturate(Rough) (1);
/// * CarShader: Specular / Roughness come from several texture terms (damage, colour picker) that are not evaluated: ASSUMED SpecLevel (else 0.5) and
///   lerp(SpecRoughnessMin, SpecRoughnessMax, S.b).
/// The static pipeline has one Specular / Roughness per material (no per-pixel mask texture), so a texture-driven input takes the MEAN of the texture
/// channel through the same (linear) formula: ASSUMED, logged. The formulas are those of the root material's default static-switch permutation.
/// Other roots: a `Specular` / `Roughness` parameter of the chain (MI or base material default, `scalars` holds both), else UE's unconnected 0.5 / 0.5.
pub fn resolve_specular_roughness(m: &Material) -> (f32, f32) {
    let root = m.root_material.rsplit('/').next().unwrap_or("");
    let sat = |x: f32| x.clamp(0., 1.);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let tex_means = |names: &[&str]| -> Option<[f32; 4]> {
        let t = names.iter().find_map(|n| m.textures.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, t)| t))?;
        static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, [f32; 4]>>> = std::sync::OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        if let Some(v) = cache.lock().ok().and_then(|c| c.get(&t.png).copied()) { return Some(v); }
        let v = load_rgba(&t.png).map(|i| channel_means(&i, t.srgb)).map_err(|e| eprintln!("[static] {}: {e}", m.name)).ok()?;
        if let Ok(mut c) = cache.lock() { c.insert(t.png.clone(), v); }
        Some(v)
    };
    let log = |what: &str, s: f32, r: f32| eprintln!("[static] {} ({root}): Specular {s:.3} / Roughness {r:.3} {what}", m.name);
    let (s, r) = match root {
        "Fortress_BaseMaterial" => {
            let mean = tex_means(&["SpecularMasks"]).unwrap_or([0.5, 0., 0.5, 1.]);
            let (s, r) = (sat(mean[0] * scalar(m, &["SpecularMult"]).unwrap_or(0.5)), sat(lerp(scalar(m, &["SpecRoughnessMin"]).unwrap_or(0.1), scalar(m, &["SpecRoughnessMax"]).unwrap_or(0.9), mean[2])));
            log("(compiled PS, SpecularMasks channel means: ASSUMED per-material constant)", s, r);
            (s, r)
        }
        "FortniteFoliage_MasterMaterial" => {
            let mean = tex_means(&["SpecularMasks"]).unwrap_or([0.5, 0., 0.5, 1.]);
            let ao_levels = (scalar(m, &["Leaf Spec AO Level"]).unwrap_or(1.), scalar(m, &["Branch Spec AO Level"]).unwrap_or(1.));
            let (s, r) = (sat(mean[0]), sat(mean[2]));
            log(&format!("(compiled PS, SpecularMasks means; Spec AO levels {ao_levels:?}{}: ASSUMED)", if ao_levels != (0., 0.) { ", AO term not evaluated" } else { "" }), s, r);
            (s, r)
        }
        "DEPRECATED_AnimatedTreeMaterial" => {
            let mean = tex_means(&["SpecTexture", "SpecularMasks"]).unwrap_or([0.5, 0., 0.5, 1.]);
            let (s, r) = (sat(mean[0]), sat(lerp(scalar(m, &["SpecRoughnessMin"]).unwrap_or(0.1), scalar(m, &["SpecRoughnessMax"]).unwrap_or(1.), mean[2])));
            log("(compiled PS, spec texture means, roughness min / max by name: ASSUMED)", s, r);
            (s, r)
        }
        "M_FieldGrassMatching" => (sat(scalar(m, &["Spec"]).unwrap_or(0.05)), sat(scalar(m, &["Rough"]).unwrap_or(1.))),
        "CarShader" => {
            let mean = tex_means(&["SpecularMasks"]).unwrap_or([0.5, 0., 0.5, 1.]);
            let (s, r) = (sat(scalar(m, &["SpecLevel"]).unwrap_or(0.5)), sat(lerp(scalar(m, &["SpecRoughnessMin"]).unwrap_or(0.25), scalar(m, &["SpecRoughnessMax"]).unwrap_or(1.), mean[2])));
            log("(texture maths not evaluated: ASSUMED)", s, r);
            (s, r)
        }
        _ => (scalar(m, &["Specular", "SpecularValue", "SpecularAmount"]).map_or(0.5, sat), scalar(m, &["Roughness", "SpecRoughness", "RoughnessValue"]).map_or(0.5, sat)),
    };
    (s, r)
}

/// Converts an exported material. Missing / unreadable textures degrade to an untextured material instead of failing the whole prop.
pub fn convert_material(m: &Material) -> StaticMaterialData { convert_material_ext(m, false) }
/// VRAM (2026-10-07): [`convert_material`]; `depth_only` = only the base colour of a MASKED material (the custom depth's alpha clip), no normal map, no opaque
/// base colour (see [`set_prop_textures_depth_only`]).
pub fn convert_material_ext(m: &Material, depth_only: bool) -> StaticMaterialData {
    let mut d = StaticMaterialData { name: m.name.clone(), two_sided: m.two_sided, ..Default::default() };
    d.alpha_mode = match m.alpha_mode { fn_propdata::AlphaMode::Opaque => AlphaMode::Opaque, _ => AlphaMode::Masked };
    d.alpha_cutoff = if m.alpha_mode == fn_propdata::AlphaMode::Blend { 0.5 } else { m.alpha_cutoff.clamp(0.02, 0.98) };
    let base_tex = pick_texture(m, &["Diffuse", "BaseColor", "Base Color", "Albedo", "DiffuseMap", "Diffuse Map"], m.slots.base_color.as_ref()).filter(|_| !depth_only || d.alpha_mode == AlphaMode::Masked);
    let normal_tex = pick_texture(m, &["NormalMap", "Normals", "Normal", "Normal Map"], m.slots.normal.as_ref()).filter(|_| !depth_only);
    let mask_tex = pick_texture(m, &["MaskTexture", "Mask Texture", "OpacityMask", "Mask"], m.slots.mask.as_ref());
    d.base_color = base_tex.and_then(|t| load_rgba(&t.png).map_err(|e| eprintln!("[static] {e}")).ok());
    d.normal = normal_tex.and_then(|t| load_rgba(&t.png).map_err(|e| eprintln!("[static] {e}")).ok());
    if d.alpha_mode == AlphaMode::Masked {
        // opacity from a separate texture: merge it into the alpha channel of the base colour
        // opacity: the alpha of the colour texture when the exporter says so, otherwise the R channel (alpha when present) of the mask texture
        if let (Some(from), Some(base)) = (m.slots.opacity_from.as_deref(), d.base_color.as_mut()) {
            if from != "base_color" {
                if let Some(t) = mask_tex.or_else(|| m.textures.get(from)) {
                    if let Ok(mask) = load_rgba(&t.png) {
                        let (bw, bh) = (base.width(), base.height());
                        for (x, y, p) in base.enumerate_pixels_mut() {
                            let mp = mask.get_pixel((x * mask.width() / bw).min(mask.width() - 1), (y * mask.height() / bh).min(mask.height() - 1));
                            // an SRGB mask (HeroTrees0_M, Shrub_*_M, ... are SRGB DXT1) is sampled through an sRGB view: its R is decoded to linear before the
                            // opacity clip; the alpha channel never is (see `sampled_channel`)
                            p[3] = if t.has_alpha { mp[3] } else { (sampled_channel(mp[0], 0, t.srgb) * 255. + 0.5) as u8 };
                        }
                    }
                }
            }
        }
        if d.base_color.is_none() { d.alpha_mode = AlphaMode::Opaque; }
    }
    let (specular, roughness) = resolve_specular_roughness(m);
    d.specular = specular;
    d.roughness = roughness;
    // ASSUMED wind: the material graph animates WorldPositionOffset; the values of its parameters are not used for the motion.
    if m.uses_world_position_offset {
        d.sway = 0.03;
        // LeafAnimationAmount is a real exported parameter (0.05 on the pines); the metres-per-unit mapping is ASSUMED
        if d.alpha_mode == AlphaMode::Masked { d.flutter = scalar(m, &["LeafAnimationAmount"]).map_or(0.03, |a| (a * 0.6).clamp(0., 0.1)); }
    }
    d.vertex_color_albedo = d.base_color.is_none();
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lod_distances_follow_the_screen_size_law_and_ascend() {
        // radius 10 m, LOD 1 at screen size 0.5 -> 21.2 m, LOD 2 at 0.25 -> 42.4 m
        let d = lod_switch_distances(&[1., 0.5, 0.25], 10.);
        assert!((d[0] - 21.21).abs() < 0.01 && (d[1] - 42.42).abs() < 0.01, "{d:?}");
        assert!(lod_switch_distances(&[1.], 10.).is_empty());
        assert_eq!(lod_switch_distances(&[1., 0.], 10.)[0], f32::MAX);
    }
    fn tetra(sign: f32) -> LodMesh {
        let positions = vec![[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [0., 0., 0.]];
        let mut idx = vec![0, 1, 2, 0, 3, 1, 1, 3, 2, 2, 3, 0];
        if sign < 0. { for t in idx.chunks_exact_mut(3) { t.swap(1, 2); } }
        LodMesh { positions, sections: vec![fn_propdata::MeshSection { material_slot: 0, indices: idx }], ..Default::default() }
    }
    #[test]
    fn winding_detection_uses_the_signed_volume_of_one_sided_sections() {
        let (a, b) = (detect_flipped_winding(&tetra(1.), &[false]), detect_flipped_winding(&tetra(-1.), &[false]));
        assert_ne!(a, b);
        // two-sided sections (foliage cards) never vote
        assert!(!detect_flipped_winding(&tetra(-1.), &[true]) && !detect_flipped_winding(&tetra(1.), &[true]));
    }
    #[test]
    fn rock_names() { assert!(looks_like_rock("Athena_Rock_Boulder_2")); assert!(!looks_like_rock("Athena_Tree_Pine")); }
}
