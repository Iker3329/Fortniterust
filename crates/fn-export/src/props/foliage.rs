//! `props foliage`: decode the ambient ground cover (`FoliageInstancedStaticMeshComponent`, S_WavyGrass_01) of `Athena_Terrain`.
//!
//! Source layout (UE 4.16 cooked `UHierarchicalInstancedStaticMeshComponent`, verified byte-for-byte on the 1.8 data): after the tagged
//! properties and the `bHasGuid` flag the native tail is
//! `i32 LODData count (0)` + `PerInstanceSMData` bulk array (`i32 element size = 80`, `i32 count`, `count x FInstancedStaticMeshInstanceData
//! { FMatrix Transform (4x4 f32, row vectors: rows 0..2 = scaled axes, row 3 = translation); FVector2D LightmapUVBias; FVector2D ShadowmapUVBias }`)
//! followed by the HISM cluster tree (not needed here). The instance matrix is relative to the component; the component hangs off the
//! `InstancedFoliageActor` root (its `RelativeLocation` / `RelativeRotation` / `RelativeScale3D` are applied) and the sublevel's
//! `LevelTransform` is applied last (exactly as the props layout does).
//!
//! Output (`<out>/foliage/`): `foliage.json` (everything recovered about the component / mesh / material), `instances.bin` (compact instance
//! records, see [`write_instances`]), `S_WavyGrass_01/` (glTF LODs + `.mat.json`), `textures/` (PNG).
use super::layout::{compose, levels, rot_to_quat, Xf, IDENT};
use super::material::{self, mat_json, TexMeta};
use super::PropsArgs;
use crate::{write_file, Source};
use fn_mesh::{PropMaterial, StaticMesh};
use fn_texture::{DecodeOptions, Texture2D};
use fn_uasset::PackageData;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn hex(t: &[u8]) -> String {
    t.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

/// One decoded instance in UE world space (cm, left handed, Z up).
#[derive(Clone, Copy, Debug)]
pub struct WorldInst {
    pub t: [f32; 3],
    pub q: [f32; 4],
    pub s: [f32; 3],
    /// `PerInstanceRandom` of the instance buffer (see [`random_fractions`]).
    pub random: f32,
}

/// `FRandomStream::GetFraction` sequence of `count` values for `seed` (what `FInstancedStaticMeshRenderData` feeds to `FStaticMeshInstanceData::SetInstance`
/// as the per-instance random, in instance order).
pub fn random_fractions(seed: i32, count: usize) -> Vec<f32> {
    let mut s = seed;
    (0..count)
        .map(|_| {
            s = s.wrapping_mul(196_314_165).wrapping_add(907_633_515);
            let bits = (1.0f32.to_bits() & 0xff80_0000) | (s as u32 & 0x007f_ffff);
            let f = f32::from_bits(bits);
            f - f.floor()
        })
        .collect()
}

/// Quaternion of a rotation matrix given as columns-are-images-of-axes (`rows` of the UE `FMatrix` are the images of the basis vectors).
fn quat_from_rows(r: [[f64; 3]; 3]) -> [f64; 4] {
    // R_col[i][j] = r[j][i]
    let m = |i: usize, j: usize| r[j][i];
    let tr = m(0, 0) + m(1, 1) + m(2, 2);
    let (x, y, z, w);
    if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        w = 0.25 * s;
        x = (m(2, 1) - m(1, 2)) / s;
        y = (m(0, 2) - m(2, 0)) / s;
        z = (m(1, 0) - m(0, 1)) / s;
    } else if m(0, 0) > m(1, 1) && m(0, 0) > m(2, 2) {
        let s = (1.0 + m(0, 0) - m(1, 1) - m(2, 2)).sqrt() * 2.0;
        w = (m(2, 1) - m(1, 2)) / s;
        x = 0.25 * s;
        y = (m(0, 1) + m(1, 0)) / s;
        z = (m(0, 2) + m(2, 0)) / s;
    } else if m(1, 1) > m(2, 2) {
        let s = (1.0 + m(1, 1) - m(0, 0) - m(2, 2)).sqrt() * 2.0;
        w = (m(0, 2) - m(2, 0)) / s;
        x = (m(0, 1) + m(1, 0)) / s;
        y = 0.25 * s;
        z = (m(1, 2) + m(2, 1)) / s;
    } else {
        let s = (1.0 + m(2, 2) - m(0, 0) - m(1, 1)).sqrt() * 2.0;
        w = (m(1, 0) - m(0, 1)) / s;
        x = (m(0, 2) + m(2, 0)) / s;
        y = (m(1, 2) + m(2, 1)) / s;
        z = 0.25 * s;
    }
    let n = (x * x + y * y + z * z + w * w).sqrt();
    [x / n, y / n, z / n, w / n]
}

/// `FMatrix` (16 f32, row major, row vectors) -> TRS. Handles negative determinants by flipping the X scale.
pub fn matrix_to_xf(m: &[f32; 16]) -> Xf {
    let row = |i: usize| [m[i * 4] as f64, m[i * 4 + 1] as f64, m[i * 4 + 2] as f64];
    let (r0, r1, r2) = (row(0), row(1), row(2));
    let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let det = r0[0] * cross(r1, r2)[0] + r0[1] * cross(r1, r2)[1] + r0[2] * cross(r1, r2)[2];
    let mut s = [len(r0), len(r1), len(r2)];
    if det < 0.0 {
        s[0] = -s[0];
    }
    let n = |v: [f64; 3], l: f64| if l.abs() > 1e-12 { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0; 3] };
    let q = quat_from_rows([n(r0, s[0]), n(r1, s[1]), n(r2, s[2])]);
    Xf { q, t: row(3), s }
}

/// Instances of one component: `(matrices, rest_len)`.
pub fn component_matrices(tail: &[u8]) -> Result<Vec<[f32; 16]>, String> {
    if tail.len() < 12 {
        return Err("tail shorter than the bulk array header".into());
    }
    let u32at = |p: usize| u32::from_le_bytes([tail[p], tail[p + 1], tail[p + 2], tail[p + 3]]);
    let (lod_data, elem, n) = (u32at(0), u32at(4) as usize, u32at(8) as usize);
    if lod_data != 0 || elem != 80 {
        return Err(format!("unexpected component tail head: LODData {lod_data}, element size {elem}"));
    }
    let end = 12 + elem * n;
    if end > tail.len() {
        return Err(format!("instance array ({n} x {elem}) overruns the {} byte tail", tail.len()));
    }
    Ok((0..n)
        .map(|i| {
            let o = 12 + i * elem;
            std::array::from_fn(|k| f32::from_le_bytes([tail[o + k * 4], tail[o + k * 4 + 1], tail[o + k * 4 + 2], tail[o + k * 4 + 3]]))
        })
        .collect())
}

fn arr3(v: Option<&Value>, d: [f64; 3]) -> [f64; 3] {
    v.and_then(|v| v.as_array()).filter(|a| a.len() >= 3).map(|a| [a[0].as_f64().unwrap_or(d[0]), a[1].as_f64().unwrap_or(d[1]), a[2].as_f64().unwrap_or(d[2])]).unwrap_or(d)
}

fn rel_xf(j: &Value) -> Xf {
    let l = arr3(j.get("RelativeLocation"), [0.0; 3]);
    let r = arr3(j.get("RelativeRotation"), [0.0; 3]);
    let s = arr3(j.get("RelativeScale3D"), [1.0; 3]);
    Xf { q: rot_to_quat(r), t: l, s }
}

pub struct Component {
    pub level: String,
    pub export: usize,
    pub props: Value,
    pub seed: i32,
    pub parent: Xf,
    pub level_xf: Xf,
    pub instances: Vec<WorldInst>,
}

/// Every `FoliageInstancedStaticMeshComponent` of the map with its instances in world space.
pub fn gather(src: &Source) -> Result<Vec<Component>, String> {
    let mut out = vec![];
    for lv in levels(src)? {
        let pd = PackageData::load(&src.set, &lv.package).map_err(|e| e.to_string())?;
        let pkg = pd.package().map_err(|e| e.to_string())?;
        for i in 0..pkg.exports.len() {
            if pkg.export_class_name(i) != "FoliageInstancedStaticMeshComponent" {
                continue;
            }
            let o = pkg.read_export(i).map_err(|e| e.to_string())?;
            if let Some(e) = &o.error {
                return Err(format!("{} export {i}: {e}", lv.package));
            }
            let j = o.props.to_json(&pkg);
            let mats = component_matrices(o.trailing).map_err(|e| format!("{} export {i}: {e}", lv.package))?;
            let comp = rel_xf(&j);
            let mut parent = IDENT;
            if let Some(name) = j.get("AttachParent").and_then(|v| v.as_str()).and_then(|ap| ap.rsplit(':').next()) {
                if let Some(pi) = pkg.find_export(name) {
                    parent = rel_xf(&pkg.read_export(pi).map_err(|e| e.to_string())?.props.to_json(&pkg));
                }
            }
            let seed = j.get("InstancingRandomSeed").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let rnd = random_fractions(seed, mats.len());
            // child (instance) -> component -> parent (actor root) -> level
            let to_world = compose(compose(comp, parent), lv.xf);
            let instances = mats
                .iter()
                .zip(&rnd)
                .map(|(m, r)| {
                    let w = compose(matrix_to_xf(m), to_world);
                    WorldInst { t: [w.t[0] as f32, w.t[1] as f32, w.t[2] as f32], q: [w.q[0] as f32, w.q[1] as f32, w.q[2] as f32, w.q[3] as f32], s: [w.s[0] as f32, w.s[1] as f32, w.s[2] as f32], random: *r }
                })
                .collect();
            out.push(Component { level: lv.package.clone(), export: i, props: j, seed, parent, level_xf: lv.xf, instances });
        }
    }
    Ok(out)
}

pub fn inspect(src: &Source, _a: &PropsArgs) -> Result<(), String> {
    let comps = gather(src)?;
    let (mut n, mut max_tilt, mut smin, mut smax, mut aniso) = (0usize, 0f64, f32::MAX, f32::MIN, 0f32);
    let mut tilts = vec![];
    let mut keys: BTreeMap<String, usize> = BTreeMap::new();
    for c in &comps {
        for k in c.props.as_object().unwrap().keys() {
            *keys.entry(k.clone()).or_default() += 1;
        }
        for w in &c.instances {
            n += 1;
            // angle between the rotated +Z axis and world +Z
            let [x, y, _z, _w] = w.q;
            let zz = 1.0 - 2.0 * (x * x + y * y);
            let tilt = (zz.clamp(-1.0, 1.0) as f64).acos().to_degrees();
            tilts.push(tilt as f32);
            max_tilt = max_tilt.max(tilt);
            for s in w.s {
                smin = smin.min(s);
                smax = smax.max(s);
            }
            aniso = aniso.max((w.s[0] - w.s[1]).abs().max((w.s[1] - w.s[2]).abs()) / w.s[0].abs().max(1e-6));
        }
        println!("{} {}: {} instances seed {} parent t={:?} first {:?}", c.level, c.export, c.instances.len(), c.seed, c.parent.t, c.instances.first().map(|w| (w.t, w.s, w.random)));
    }
    tilts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("TOTAL {n} instances; scale {smin}..{smax}; max anisotropy {aniso}; tilt max {max_tilt:.2} p50 {:.2} p90 {:.2} p99 {:.2}", tilts[tilts.len() / 2], tilts[tilts.len() * 9 / 10], tilts[tilts.len() * 99 / 100]);
    println!("property key frequency: {keys:?}");
    // world bounds
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for c in &comps {
        for w in &c.instances {
            for k in 0..3 {
                lo[k] = lo[k].min(w.t[k]);
                hi[k] = hi[k].max(w.t[k]);
            }
        }
    }
    println!("world bounds cm {lo:?} .. {hi:?}");
    let _ = hex(&[]);
    Ok(())
}

pub const MAGIC: &[u8; 4] = b"FNGR";
pub const RECORD_BYTES: usize = 28;

fn snorm16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// `instances.bin`: `b"FNGR"`, `u32 version (1)`, `u32 count`, then `count` records of [`RECORD_BYTES`]:
/// `f32 x, y, z` (UE world cm), `i16 qx, qy, qz, qw` (snorm, UE world rotation, w >= 0), `f32 uniform scale`, `f32 PerInstanceRandom`.
pub fn write_instances(comps: &[Component]) -> Vec<u8> {
    let n: usize = comps.iter().map(|c| c.instances.len()).sum();
    let mut b = Vec::with_capacity(12 + n * RECORD_BYTES);
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&(n as u32).to_le_bytes());
    for c in comps {
        for w in &c.instances {
            for v in w.t {
                b.extend_from_slice(&v.to_le_bytes());
            }
            let sgn = if w.q[3] < 0.0 { -1.0 } else { 1.0 };
            for v in w.q {
                b.extend_from_slice(&snorm16(v * sgn).to_le_bytes());
            }
            b.extend_from_slice(&((w.s[0] + w.s[1] + w.s[2]) / 3.0).to_le_bytes());
            b.extend_from_slice(&w.random.to_le_bytes());
        }
    }
    b
}

fn leaf(p: &str) -> &str {
    p.rsplit(|c| c == '/' || c == '.').next().unwrap_or(p)
}

fn export_texture(src: &Source, path: &str, png_name: &str, dir: &std::path::Path) -> Result<TexMeta, String> {
    let pkg = super::bp::package_of(path);
    let raw = src.load_raw(pkg)?;
    let tex = Texture2D::from_files(&raw.uasset, &raw.uexp, &raw.ubulk).map_err(|e| format!("{path}: {e}"))?;
    let mip = tex.mip_for_max_dimension(2048).ok_or_else(|| format!("{path}: no mip with data"))?;
    let img = tex.decode(mip, DecodeOptions::default()).map_err(|e| format!("{path}: {e}"))?;
    let has_alpha = !img.is_opaque();
    let grey = tex.compression_settings == "TC_Default" && {
        let px = img.to_rgba8();
        let n = (px.len() / 4).max(1);
        let chroma: f64 = px.chunks_exact(4).step_by(7).map(|p| (p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2])) as f64 / 255.0).sum::<f64>() / (n / 7 + 1) as f64;
        chroma < 0.06
    };
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    img.write_png(dir.join(png_name)).map_err(|e| format!("{path}: {e}"))?;
    Ok(TexMeta {
        name: tex.name.clone(),
        png: png_name.to_string(),
        srgb: tex.srgb,
        compression: tex.compression_settings.clone(),
        format: tex.format.name().to_string(),
        width: img.width as u32,
        height: img.height as u32,
        source_width: tex.size_x as u32,
        source_height: tex.size_y as u32,
        address_x: tex.address_x.clone(),
        address_y: tex.address_y.clone(),
        has_alpha,
        grey,
    })
}

pub fn run(src: &Source, a: &PropsArgs) -> Result<(), String> {
    let t0 = std::time::Instant::now();
    let out = a.out.join("foliage");
    let comps = gather(src)?;
    let n: usize = comps.iter().map(|c| c.instances.len()).sum();
    println!("{} foliage components, {n} instances ({:.1}s)", comps.len(), t0.elapsed().as_secs_f32());

    // one mesh / one set of cull distances for the whole map (asserted)
    let first = comps.iter().find(|c| !c.instances.is_empty()).ok_or("no foliage instances")?;
    let mesh_path = first.props.get("StaticMesh").and_then(|v| v.as_str()).ok_or("component has no StaticMesh")?.to_string();
    let (start, end) = (first.props["InstanceStartCullDistance"].as_f64().unwrap_or(0.0), first.props["InstanceEndCullDistance"].as_f64().unwrap_or(0.0));
    for c in &comps {
        let m = c.props.get("StaticMesh").and_then(|v| v.as_str()).unwrap_or("");
        let (s, e) = (c.props["InstanceStartCullDistance"].as_f64().unwrap_or(0.0), c.props["InstanceEndCullDistance"].as_f64().unwrap_or(0.0));
        if m != mesh_path || s != start || e != end {
            return Err(format!("{} uses a different mesh / cull distance ({m}, {s}, {e}): the exporter handles one", c.level));
        }
    }

    // mesh
    let pkg_path = super::bp::package_of(&mesh_path).to_string();
    let pkg = src.package(&pkg_path)?;
    let mesh = StaticMesh::parse(&pkg).map_err(|e| e.to_string())?;
    let mesh_name = mesh.name.clone();
    let mut ext_bounds = Value::Null;
    if let Ok(pd) = PackageData::load(&src.set, &pkg_path) {
        if let Ok(p) = pd.package() {
            if let Some(i) = (0..p.exports.len()).find(|&i| p.export_class_name(i) == "StaticMesh") {
                if let Ok(o) = p.read_export(i) {
                    ext_bounds = o.props.to_json(&p).get("ExtendedBounds").cloned().unwrap_or(Value::Null);
                }
            }
        }
    }

    // materials + textures
    let mut resolved = vec![];
    for sm in &mesh.materials {
        let path = sm.material.clone().ok_or_else(|| format!("mesh slot {} has no material", sm.slot_name))?;
        resolved.push(material::resolve(&src.set, &path)?);
    }
    let tex_dir = out.join("textures");
    let mut tex_meta: BTreeMap<String, TexMeta> = BTreeMap::new();
    for r in &resolved {
        for t in r.textures.values() {
            if tex_meta.contains_key(t) {
                continue;
            }
            match export_texture(src, t, &format!("{}.png", leaf(t)), &tex_dir) {
                Ok(m) => {
                    tex_meta.insert(t.clone(), m);
                }
                Err(e) => eprintln!("texture {t}: {e}"),
            }
        }
    }
    let kind_dir = out.join(&mesh_name);
    let mut prop_mats = vec![];
    let mut mat_entries = vec![];
    for (i, (r, sm)) in resolved.iter().zip(&mesh.materials).enumerate() {
        let rel = format!("materials/{}.mat.json", r.name);
        write_file(&kind_dir.join(&rel), serde_json::to_string_pretty(&mat_json(r, &tex_meta, "../../textures")).unwrap().as_bytes())?;
        let sc = material::choose_slots(r, &tex_meta);
        let uri = |p: &Option<String>| p.as_ref().and_then(|pn| r.textures.get(pn)).and_then(|t| tex_meta.get(t)).map(|m| format!("../textures/{}", m.png));
        let alpha_mode = if r.blend_mode.contains("Masked") { "MASK" } else if r.blend_mode.contains("Translucent") || r.blend_mode.contains("Additive") { "BLEND" } else { "OPAQUE" };
        prop_mats.push(PropMaterial {
            name: r.name.clone(),
            slot_name: sm.slot_name.clone(),
            base_color_uri: uri(&sc.base_color),
            normal_uri: uri(&sc.normal),
            mask_uri: uri(&sc.mask),
            alpha_mode: alpha_mode.into(),
            alpha_cutoff: r.clip as f32,
            double_sided: r.two_sided,
            base_color_factor: [1.0; 4],
            mat_json_uri: Some(rel.clone()),
        });
        mat_entries.push(json!({"slot": i, "slot_name": sm.slot_name, "name": r.name, "ue_path": r.ue_path, "mat_json": rel, "chain": r.chain, "root_material": r.root, "wpo_connected": r.wpo, "blend_mode": r.blend_mode}));
    }
    fn_mesh::export_static_mesh_lods(&mesh, &kind_dir, &mesh_name, &prop_mats).map_err(|e| e.to_string())?;
    let (bmin, bmax) = mesh.bounds.box_min_max();

    // instances
    let bin = write_instances(&comps);
    write_file(&out.join("instances.bin"), &bin)?;
    let per_level: Vec<Value> = comps.iter().map(|c| json!({"level": c.level, "export": c.export, "instances": c.instances.len(), "instancing_random_seed": c.seed, "parent_relative_location_cm": c.parent.t, "level_translation_cm": c.level_xf.t, "level_rotation_quat": c.level_xf.q})).collect();
    let first_props = &first.props;
    let doc = json!({
        "schema": 1,
        "game": "Fortnite 1.8 (UE 4.16, CL 3724489)",
        "source_map": super::layout::MAP,
        "component_class": "FoliageInstancedStaticMeshComponent (HierarchicalInstancedStaticMeshComponent)",
        "mesh": {"name": mesh_name, "ue_path": mesh_path, "gltf": format!("{mesh_name}.gltf"), "dir": mesh_name,
                 "bounds_cm": {"origin": mesh.bounds.origin, "extent": mesh.bounds.extent, "radius": mesh.bounds.radius, "min": bmin, "max": bmax}, "extended_bounds_property": ext_bounds,
                 "lod_screen_size": &mesh.screen_size[..mesh.lods.len().min(8)],
                 "lods": mesh.lods.iter().map(|l| json!({"vertices": l.positions.len(), "triangles": l.indices.len() / 3})).collect::<Vec<_>>()},
        "materials": mat_entries,
        "cull": {"instance_start_cull_distance_cm": start, "instance_end_cull_distance_cm": end,
                 "semantics": "UE `InstanceStartCullDistance` / `InstanceEndCullDistance` of the component: instances are fully visible inside start, fade out between start and end and are culled beyond end"},
        "render_flags": {"cast_shadow": first_props.get("CastShadow"), "collision": first_props.pointer("/BodyInstance/CollisionEnabled"), "receives_decals": first_props.get("bReceivesDecals"), "use_as_occluder": first_props.get("bUseAsOccluder"), "mobility": first_props.get("Mobility"),
                         "enable_density_scaling_property_present": first_props.get("bEnableDensityScaling").is_some()},
        "instances": {"file": "instances.bin", "count": n, "record_bytes": RECORD_BYTES,
                      "format": "b'FNGR', u32 version=1, u32 count, then per instance: f32 x,y,z (UE world cm after parent + LevelTransform), i16 qx,qy,qz,qw (snorm, UE world rotation, w >= 0), f32 uniform scale, f32 PerInstanceRandom",
                      "per_instance_random": "FRandomStream(InstancingRandomSeed).GetFraction() per instance in file order (FInstancedStaticMeshRenderData); engine algorithm, not stored in the asset",
                      "components": per_level},
        "unit_note": "positions in UE world cm (left handed, Z up); the runtime maps (x, y, z) cm -> (x, z, y) m and the quaternion (x, y, z, w) -> (-x, -z, -y, w)",
    });
    write_file(&out.join("foliage.json"), serde_json::to_string_pretty(&doc).unwrap().as_bytes())?;
    println!("wrote {} ({} bytes, {} instances, {:.1}s)", out.display(), bin.len(), n, t0.elapsed().as_secs_f32());
    Ok(())
}
