//! `props backdrop-*`: everything of `Athena_Terrain` that is not a harvestable prop but part of how the map looks beyond the player:
//! the landscape height field (every `LandscapeComponent` of the persistent level + the always-loaded `Athena_Terrain_LS_xx` sublevels),
//! the ocean planes, the far skybox rocks / terraces (placed `StaticMeshActor`s of the persistent level).
//!
//! Output (`<out>/backdrop/`): `terrain.bin` (see [`write_terrain`]), `backdrop.json`, `meshes/*.bin`, `textures/*.png`.
use super::layout::{compose, levels, rot_to_quat, Xf, IDENT};
use super::PropsArgs;
use crate::{write_file, Source};
use fn_texture::{DecodeOptions, Texture2D};
use fn_uasset::PackageData;
use image::ImageEncoder;
use serde_json::Value;

pub struct TerrainComp {
    pub level: String,
    pub name: String,
    /// UE world cm of vertex (0,0) at raw height 32768.
    pub origin: [f64; 3],
    /// cm per quad in x / y, cm per raw height unit.
    pub step: [f64; 3],
    pub heights: Vec<u16>,
    pub size: usize,
    /// Landscape layer weights per vertex: (layer info name, size*size weights).
    pub layers: Vec<(String, Vec<u8>)>,
}

fn arr3(v: Option<&Value>, d: [f64; 3]) -> [f64; 3] {
    v.and_then(|v| v.as_array()).filter(|a| a.len() >= 3).map(|a| [a[0].as_f64().unwrap_or(d[0]), a[1].as_f64().unwrap_or(d[1]), a[2].as_f64().unwrap_or(d[2])]).unwrap_or(d)
}
pub fn rel_xf(j: &Value) -> Xf {
    Xf { q: rot_to_quat(arr3(j.get("RelativeRotation"), [0.0; 3])), t: arr3(j.get("RelativeLocation"), [0.0; 3]), s: arr3(j.get("RelativeScale3D"), [1.0; 3]) }
}
pub fn leaf_name(p: &str) -> &str {
    p.rsplit(|c| c == '.' || c == ':').next().unwrap_or(p)
}

fn level_files(src: &Source, long: &str) -> Result<fn_ueio::Package, String> {
    let ap = fn_pak::game_path_to_pak(long, "umap").ok_or("bad path")?;
    let a = src.set.read(&ap).map_err(|e| format!("{ap}: {e}"))?;
    let e = src.set.read(&ap.replace(".umap", ".uexp")).map(|c| c.into_owned()).unwrap_or_default();
    let b = src.set.read(&ap.replace(".umap", ".ubulk")).map(|c| c.into_owned()).unwrap_or_default();
    fn_ueio::Package::parse(&a, &e, &b).map_err(|e| format!("{long}: {e}"))
}

fn decode_tex(pk: &fn_ueio::Package, name: &str) -> Result<(usize, usize, Vec<u8>), String> {
    let idx = (0..pk.exports.len()).find(|&i| pk.exports[i].object_name == name && pk.export_class(i) == "Texture2D").ok_or_else(|| format!("no Texture2D {name}"))?;
    let t = Texture2D::parse_export(pk, idx).map_err(|e| e.to_string())?;
    let mip = t.best_mip().ok_or("no mip")?;
    let img = t.decode(mip, DecodeOptions::default()).map_err(|e| e.to_string())?;
    Ok((img.width, img.height, img.to_rgba8()))
}

pub fn gather_terrain(src: &Source) -> Result<Vec<TerrainComp>, String> {
    let mut out = vec![];
    for lv in levels(src)? {
        let pd = PackageData::load(&src.set, &lv.package).map_err(|e| e.to_string())?;
        let pkg = pd.package().map_err(|e| e.to_string())?;
        let mut tex_pkg: Option<fn_ueio::Package> = None;
        for ai in 0..pkg.exports.len() {
            let cn = pkg.export_class_name(ai);
            if cn != "Landscape" && cn != "LandscapeStreamingProxy" {
                continue;
            }
            // the actor's root component: the "RootComponent*" whose outer is this actor
            let root = (0..pkg.exports.len()).find(|&i| pkg.exports[i].name.starts_with("RootComponent") && pkg.exports[i].outer_index.export_index() == Some(ai)).ok_or("landscape without root")?;
            let rj = pkg.read_export(root).map_err(|e| e.to_string())?.props.to_json(&pkg);
            let root_xf = rel_xf(&rj);
            for ci in 0..pkg.exports.len() {
                if pkg.export_class_name(ci) != "LandscapeComponent" || pkg.exports[ci].outer_index.export_index() != Some(ai) {
                    continue;
                }
                let o = pkg.read_export(ci).map_err(|e| e.to_string())?;
                let j = o.props.to_json(&pkg);
                let comp = Xf { q: IDENT.q, t: arr3(j.get("RelativeLocation"), [0.0; 3]), s: [1.0; 3] };
                let w = compose(compose(comp, root_xf), lv.xf);
                if w.q[0].abs() > 1e-6 || w.q[1].abs() > 1e-6 || w.q[2].abs() > 1e-6 {
                    return Err(format!("{} {}: rotated landscape not supported", lv.package, pkg.exports[ci].name));
                }
                let hm = j.get("HeightmapTexture").and_then(|v| v.as_str()).ok_or("no HeightmapTexture")?;
                if tex_pkg.is_none() {
                    tex_pkg = Some(level_files(src, &lv.package)?);
                }
                let tp = tex_pkg.as_ref().unwrap();
                let (wd, ht, px) = decode_tex(tp, leaf_name(hm))?;
                let heights: Vec<u16> = px.chunks_exact(4).map(|p| ((p[0] as u16) << 8) | p[1] as u16).collect();
                let mut layers: Vec<(String, Vec<u8>)> = vec![];
                if let Some(allocs) = j.get("WeightmapLayerAllocations").and_then(|v| v.as_array()) {
                    let wts: Vec<String> = j.get("WeightmapTextures").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| leaf_name(s).to_string())).collect()).unwrap_or_default();
                    let sb = j.get("WeightmapScaleBias").and_then(|v| v.as_array()).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect::<Vec<_>>()).unwrap_or(vec![0.0078125, 0.0078125, 0.0, 0.0]);
                    let mut cache: Vec<Option<(usize, usize, Vec<u8>)>> = vec![None; wts.len()];
                    for a in allocs {
                        let li = a.get("LayerInfo").and_then(|v| v.as_str()).map(|s| leaf_name(s).to_string()).unwrap_or_default();
                        let ti = a.get("WeightmapTextureIndex").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let ch = a.get("WeightmapTextureChannel").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        if ti >= wts.len() {
                            continue;
                        }
                        if cache[ti].is_none() {
                            cache[ti] = decode_tex(tp, &wts[ti]).ok();
                        }
                        let Some((ww, wh, wpx)) = cache[ti].as_ref() else { continue };
                        let (offx, offy) = ((sb.get(2).copied().unwrap_or(0.0) * *ww as f64).round() as usize, (sb.get(3).copied().unwrap_or(0.0) * *wh as f64).round() as usize);
                        let mut v = vec![0u8; wd * ht];
                        for y in 0..ht {
                            for x in 0..wd {
                                let (sx, sy) = ((x + offx).min(ww - 1), (y + offy).min(wh - 1));
                                v[y * wd + x] = wpx[(sy * ww + sx) * 4 + ch];
                            }
                        }
                        layers.push((li, v));
                    }
                }
                out.push(TerrainComp { level: lv.package.clone(), name: pkg.exports[ci].name.clone(), origin: [w.t[0], w.t[1], w.t[2]], step: [w.s[0], w.s[1], w.s[2] / 128.0], heights, size: wd, layers });
            }
        }
    }
    Ok(out)
}

pub const TERRAIN_MAGIC: &[u8; 4] = b"FNTH";
/// `terrain.bin`: magic, u32 version 1, u32 component count, u32 layer-name count, names (u8 len + bytes); per component: u32 size, f32 origin xyz (UE cm),
/// f32 step x y z (z = cm per raw unit), `size*size` u16 heights, u8 layer count, then per layer: u8 layer-name index + `size*size` u8 weights.
pub fn write_terrain(comps: &[TerrainComp]) -> Vec<u8> {
    let mut names: Vec<String> = vec![];
    for c in comps {
        for (n, _) in &c.layers {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
    }
    let mut b = vec![];
    b.extend_from_slice(TERRAIN_MAGIC);
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&(comps.len() as u32).to_le_bytes());
    b.extend_from_slice(&(names.len() as u32).to_le_bytes());
    for n in &names {
        b.push(n.len() as u8);
        b.extend_from_slice(n.as_bytes());
    }
    for c in comps {
        b.extend_from_slice(&(c.size as u32).to_le_bytes());
        for v in c.origin.iter().chain(c.step.iter()) {
            b.extend_from_slice(&(*v as f32).to_le_bytes());
        }
        for h in &c.heights {
            b.extend_from_slice(&h.to_le_bytes());
        }
        b.push(c.layers.len() as u8);
        for (n, w) in &c.layers {
            b.push(names.iter().position(|x| x == n).unwrap() as u8);
            b.extend_from_slice(w);
        }
    }
    b
}

pub fn terrain_cmd(src: &Source, a: &PropsArgs) -> Result<(), String> {
    let t0 = std::time::Instant::now();
    let comps = gather_terrain(src)?;
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for c in &comps {
        let n = c.size as f64 - 1.0;
        let (mn, mx) = (*c.heights.iter().min().unwrap() as f64, *c.heights.iter().max().unwrap() as f64);
        for (k, v) in [(0, c.origin[0]), (0, c.origin[0] + n * c.step[0]), (1, c.origin[1]), (1, c.origin[1] + n * c.step[1]), (2, c.origin[2] + (mn - 32768.0) * c.step[2]), (2, c.origin[2] + (mx - 32768.0) * c.step[2])] {
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    println!("{} landscape components, bounds cm {lo:?} .. {hi:?}; step {:?} ({:.1}s)", comps.len(), comps.first().map(|c| c.step), t0.elapsed().as_secs_f32());
    let mut layer_names: Vec<String> = vec![];
    for c in &comps {
        for (n, _) in &c.layers {
            if !layer_names.contains(n) {
                layer_names.push(n.clone());
            }
        }
    }
    println!("layers: {layer_names:?}");
    let bytes = write_terrain(&comps);
    write_file(&a.out.join("backdrop/terrain.bin"), &bytes)?;
    // top-down debug image (1 px = 4 m)
    let (w, h) = (((hi[0] - lo[0]) / 400.0) as usize + 1, ((hi[1] - lo[1]) / 400.0) as usize + 1);
    let mut img = vec![0u8; w * h * 3];
    for c in &comps {
        for y in 0..c.size {
            for x in 0..c.size {
                let (px, py) = (((c.origin[0] + x as f64 * c.step[0] - lo[0]) / 400.0) as usize, ((c.origin[1] + y as f64 * c.step[1] - lo[1]) / 400.0) as usize);
                if px < w && py < h {
                    let z = c.origin[2] + (c.heights[y * c.size + x] as f64 - 32768.0) * c.step[2];
                    let v = (((z + 6000.0) / 12000.0).clamp(0.0, 1.0) * 255.0) as u8;
                    let o = (py * w + px) * 3;
                    img[o] = v;
                    img[o + 1] = if z < -5312.0 { 80 } else { v };
                    img[o + 2] = v;
                }
            }
        }
    }
    let mut png = vec![];
    image::codecs::png::PngEncoder::new(&mut png).write_image(&img, w as u32, h as u32, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
    write_file(&a.out.join("backdrop/terrain-debug.png"), &png)?;
    println!("wrote terrain.bin ({} bytes) + terrain-debug.png {w}x{h}", bytes.len());
    Ok(())
}
