//! `props layout`: placements of harvestable actors in `Athena_Terrain` (persistent level + every always-loaded sublevel,
//! with the sublevel `LevelTransform` applied), class histogram, density statistics, and a 40-instance sample around
//! the densest tree + rock cluster.
//!
//! Output (under `--out`/`layout/`): `all.json` (every harvestable placement, compact arrays), `sample.json`,
//! `stats.json`.
use super::bp::{load_bp, package_of, Bp};
use super::PropsArgs;
use crate::{parallel, write_file, Source};
use fn_uasset::{ObjectRef, PackageData, PackageIndex};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub const MAP: &str = "/Game/Athena/Maps/Athena_Terrain";

#[derive(Clone, Copy, Debug)]
pub struct Xf {
    pub q: [f64; 4],
    pub t: [f64; 3],
    pub s: [f64; 3],
}

pub const IDENT: Xf = Xf { q: [0.0, 0.0, 0.0, 1.0], t: [0.0; 3], s: [1.0; 3] };

fn qmul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn qrot(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    // v' = v + 2w(q x v) + 2 q x (q x v)
    let u = [q[0], q[1], q[2]];
    let c = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = c(u, v);
    let t = [2.0 * t[0], 2.0 * t[1], 2.0 * t[2]];
    let u2 = c(u, t);
    [v[0] + q[3] * t[0] + u2[0], v[1] + q[3] * t[1] + u2[1], v[2] + q[3] * t[2] + u2[2]]
}

/// `FRotator(Pitch, Yaw, Roll)` degrees -> `FQuat` (UE `FRotator::Quaternion`).
pub fn rot_to_quat(r: [f64; 3]) -> [f64; 4] {
    let h = |d: f64| (d.to_radians() * 0.5).sin_cos();
    let (sp, cp) = h(r[0]);
    let (sy, cy) = h(r[1]);
    let (sr, cr) = h(r[2]);
    [cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy]
}

/// `FQuat::Rotator`: (Pitch, Yaw, Roll) degrees.
pub fn quat_to_rot(q: [f64; 4]) -> [f64; 3] {
    let [x, y, z, w] = q;
    let st = z * x - w * y;
    let yaw_y = 2.0 * (w * z + x * y);
    let yaw_x = 1.0 - 2.0 * (y * y + z * z);
    let norm = |a: f64| {
        let mut a = a % 360.0;
        if a > 180.0 {
            a -= 360.0
        } else if a < -180.0 {
            a += 360.0
        }
        a
    };
    let t = 0.4999995;
    let (p, yw, r) = if st < -t {
        let yaw = yaw_y.atan2(yaw_x).to_degrees();
        (-90.0, yaw, norm(-yaw - 2.0 * x.atan2(w).to_degrees()))
    } else if st > t {
        let yaw = yaw_y.atan2(yaw_x).to_degrees();
        (90.0, yaw, norm(yaw - 2.0 * x.atan2(w).to_degrees()))
    } else {
        ((2.0 * st).asin().to_degrees(), yaw_y.atan2(yaw_x).to_degrees(), (-2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y)).to_degrees())
    };
    [p, yw, r]
}

/// `child * parent` in UE FTransform convention (apply child first, then parent).
pub fn compose(child: Xf, parent: Xf) -> Xf {
    let sc = [parent.s[0] * child.t[0], parent.s[1] * child.t[1], parent.s[2] * child.t[2]];
    let r = qrot(parent.q, sc);
    Xf { q: qmul(parent.q, child.q), t: [r[0] + parent.t[0], r[1] + parent.t[1], r[2] + parent.t[2]], s: [child.s[0] * parent.s[0], child.s[1] * parent.s[1], child.s[2] * parent.s[2]] }
}

fn arr(v: Option<&Value>, n: usize) -> Option<Vec<f64>> {
    let a = v?.as_array()?;
    if a.len() < n {
        return None;
    }
    a.iter().take(n).map(|x| x.as_f64()).collect()
}

fn xf_from_json(v: &Value) -> Xf {
    let mut x = IDENT;
    if let Some(r) = arr(v.get("Rotation"), 4) {
        x.q = [r[0], r[1], r[2], r[3]];
    }
    if let Some(t) = arr(v.get("Translation"), 3) {
        x.t = [t[0], t[1], t[2]];
    }
    if let Some(s) = arr(v.get("Scale3D"), 3) {
        x.s = [s[0], s[1], s[2]];
    }
    x
}

#[derive(Clone, Debug)]
pub struct Level {
    pub package: String,
    pub xf: Xf,
}

/// Persistent map + always-loaded sublevels with their transforms.
pub fn levels(src: &Source) -> Result<Vec<Level>, String> {
    let pd = PackageData::load(&src.set, MAP).map_err(|e| e.to_string())?;
    let pkg = pd.package().map_err(|e| e.to_string())?;
    let mut out = vec![Level { package: MAP.to_string(), xf: IDENT }];
    for i in 0..pkg.exports.len() {
        if !pkg.export_class_name(i).starts_with("LevelStreaming") {
            continue;
        }
        let o = pkg.read_export(i).map_err(|e| e.to_string())?;
        let j = o.props.to_json(&pkg);
        let Some(wa) = j.get("WorldAsset").and_then(|v| v.as_str()) else { continue };
        let xf = j.get("LevelTransform").map(xf_from_json).unwrap_or(IDENT);
        out.push(Level { package: package_of(wa).to_string(), xf });
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct Placement {
    pub class: String,
    pub bp: String,
    pub level: String,
    pub xf: Xf,
}

pub struct LevelScan {
    pub placements: Vec<Placement>,
    pub other_classes: BTreeMap<String, usize>,
    pub no_root: usize,
}

/// All Blueprint actors (`*_C`) of a level with their world transform.
pub fn scan_level(src: &Source, lv: &Level) -> Result<LevelScan, String> {
    let pd = PackageData::load(&src.set, &lv.package).map_err(|e| format!("{}: {e}", lv.package))?;
    let pkg = pd.package().map_err(|e| format!("{}: {e}", lv.package))?;
    let mut placements = vec![];
    let mut other = BTreeMap::new();
    let mut no_root = 0;
    for i in 0..pkg.exports.len() {
        let cn = pkg.export_class_name(i);
        let ex = &pkg.exports[i];
        if !cn.ends_with("_C") || ex.name.starts_with("Default__") {
            if matches!(cn.as_str(), "InstancedFoliageActor" | "HierarchicalInstancedStaticMeshComponent" | "InstancedStaticMeshComponent" | "FoliageType_InstancedStaticMesh" | "FoliageInstancedStaticMeshComponent") {
                *other.entry(cn).or_insert(0) += 1;
            }
            continue;
        }
        let bp = match pkg.resolve(ex.class_index) {
            ObjectRef::Import { package, .. } => package,
            _ => continue,
        };
        let o = match pkg.read_export(i) {
            Ok(o) => o,
            Err(_) => continue,
        };
        let root = o.props.get("RootComponent").and_then(|v| v.as_object());
        let Some(ri) = root.and_then(|r: PackageIndex| r.export_index()) else {
            no_root += 1;
            continue;
        };
        let Ok(c) = pkg.read_export(ri) else { continue };
        let j = c.props.to_json(&pkg);
        let loc = arr(j.get("RelativeLocation"), 3).unwrap_or(vec![0.0; 3]);
        let rot = arr(j.get("RelativeRotation"), 3).unwrap_or(vec![0.0; 3]);
        let scl = arr(j.get("RelativeScale3D"), 3).unwrap_or(vec![1.0; 3]);
        let rel = Xf { q: rot_to_quat([rot[0], rot[1], rot[2]]), t: [loc[0], loc[1], loc[2]], s: [scl[0], scl[1], scl[2]] };
        placements.push(Placement { class: cn.trim_end_matches("_C").to_string(), bp, level: lv.package.clone(), xf: compose(rel, lv.xf) });
    }
    Ok(LevelScan { placements, other_classes: other, no_root })
}

pub struct Gathered {
    pub all: Vec<Placement>,
    pub bps: HashMap<String, Option<Bp>>,
    pub per_level: serde_json::Map<String, Value>,
    pub foliage: BTreeMap<String, usize>,
    pub no_root: usize,
}

impl Gathered {
    /// `(attribute category, sub category, resource type, static mesh path)` when the Blueprint is a harvestable prop
    /// (a `Resource*` / `Prop*` attribute category with a wood / stone / metal resource type).
    pub fn kind_of(&self, bp: &str) -> Option<(String, String, String, String)> {
        let b = self.bps.get(bp)?.as_ref()?;
        let (cat, sub) = b.attr_key();
        let rt = b.resource_type();
        if rt.is_empty() || !(cat.starts_with("Resource") || cat.starts_with("Prop")) {
            return None;
        }
        Some((cat, sub, rt, b.static_mesh().unwrap_or_default()))
    }
}

/// Scan every level of the map and load the Blueprint of every placed actor class.
pub fn gather(src: &Source, jobs: usize) -> Result<Gathered, String> {
    let lvls = levels(src)?;
    println!("{} levels", lvls.len());
    let scans = parallel(&lvls, jobs, |l| scan_level(src, l));
    let mut all: Vec<Placement> = vec![];
    let mut per_level = serde_json::Map::new();
    let mut foliage: BTreeMap<String, usize> = BTreeMap::new();
    let mut no_root = 0;
    for (l, s) in lvls.iter().zip(scans) {
        let s = s?;
        per_level.insert(l.package.clone(), json!({"actors": s.placements.len(), "translation": l.xf.t, "rotation": l.xf.q}));
        for (k, v) in s.other_classes {
            *foliage.entry(k).or_insert(0) += v;
        }
        no_root += s.no_root;
        all.extend(s.placements);
    }
    println!("{} blueprint actor placements", all.len());
    let mut bps: HashMap<String, Option<Bp>> = HashMap::new();
    let uniq: Vec<String> = {
        let mut u: Vec<String> = all.iter().map(|p| p.bp.clone()).collect();
        u.sort();
        u.dedup();
        u
    };
    let loaded = parallel(&uniq, jobs, |p| load_bp(&src.set, p).ok());
    for (p, b) in uniq.iter().zip(loaded) {
        bps.insert(p.clone(), b);
    }
    Ok(Gathered { all, bps, per_level, foliage, no_root })
}

pub fn run(src: &Source, a: &PropsArgs) -> Result<(), String> {
    let g = gather(src, a.jobs)?;
    run_with(&g, a)
}

/// Placement statistics + `layout/{all,sample,stats}.json` for an already gathered map.
pub fn run_with(g: &Gathered, a: &PropsArgs) -> Result<(), String> {
    let mut hist: BTreeMap<String, (usize, String, String, String, String)> = BTreeMap::new();
    let mut harv: Vec<&Placement> = vec![];
    for p in &g.all {
        if let Some((cat, sub, rt, mesh)) = g.kind_of(&p.bp) {
            let e = hist.entry(p.class.clone()).or_insert((0, cat, sub, rt, mesh));
            e.0 += 1;
            harv.push(p);
        }
    }
    let (per_level, foliage, no_root, all) = (&g.per_level, &g.foliage, g.no_root, &g.all);
    // compact full list: [class, x,y,z, pitch,yaw,roll, sx,sy,sz]
    let mut classes: Vec<String> = hist.keys().cloned().collect();
    classes.sort();
    let cidx: HashMap<&str, usize> = classes.iter().enumerate().map(|(i, c)| (c.as_str(), i)).collect();
    let inst: Vec<Value> = harv
        .iter()
        .map(|p| {
            let r = quat_to_rot(p.xf.q);
            json!([cidx[p.class.as_str()], round(p.xf.t[0]), round(p.xf.t[1]), round(p.xf.t[2]), round(r[0]), round(r[1]), round(r[2]), round3(p.xf.s[0]), round3(p.xf.s[1]), round3(p.xf.s[2])])
        })
        .collect();
    let all_doc = json!({
        "version": 1, "map": MAP, "units": "cm, UE world space (left handed, Z up); rotation = FRotator (pitch, yaw, roll) degrees",
        "fields": ["class", "x", "y", "z", "pitch", "yaw", "roll", "sx", "sy", "sz"], "classes": classes, "instances": inst,
    });
    write_file(&a.out.join("layout/all.json"), serde_json::to_string(&all_doc).unwrap().as_bytes())?;

    // statistics
    let is_tree = |c: &str| hist.get(c).map_or(false, |h| h.3 == "Wood" && h.1.starts_with("Resource"));
    let is_rock = |c: &str| hist.get(c).map_or(false, |h| h.3 == "Stone" && h.1.starts_with("Resource"));
    let (mut minx, mut maxx, mut miny, mut maxy) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in &harv {
        minx = minx.min(p.xf.t[0]);
        maxx = maxx.max(p.xf.t[0]);
        miny = miny.min(p.xf.t[1]);
        maxy = maxy.max(p.xf.t[1]);
    }
    let cell = 10_000.0;
    let mut grid: HashMap<(i64, i64), (usize, usize)> = HashMap::new();
    for p in &harv {
        let k = ((p.xf.t[0] / cell).floor() as i64, (p.xf.t[1] / cell).floor() as i64);
        let e = grid.entry(k).or_insert((0, 0));
        if is_tree(&p.class) {
            e.0 += 1;
        }
        if is_rock(&p.class) {
            e.1 += 1;
        }
    }
    let mut tree_cells: Vec<usize> = grid.values().map(|v| v.0).filter(|&n| n > 0).collect();
    tree_cells.sort();
    let pct = |v: &Vec<usize>, p: f64| v.get(((v.len() as f64 - 1.0) * p) as usize).copied().unwrap_or(0);

    // sample centre: maximise min(trees, rocks) within 4000 cm (ties by total)
    let resource: Vec<&&Placement> = harv.iter().filter(|p| is_tree(&p.class) || is_rock(&p.class)).collect();
    let mut best = (0usize, 0usize, [0.0f64; 3]);
    let r2 = 4000.0f64 * 4000.0;
    for c in resource.iter().step_by(1) {
        let (mut t, mut r) = (0, 0);
        for q in &resource {
            let d = (q.xf.t[0] - c.xf.t[0]).powi(2) + (q.xf.t[1] - c.xf.t[1]).powi(2);
            if d <= r2 {
                if is_tree(&q.class) {
                    t += 1
                } else {
                    r += 1
                }
            }
        }
        let score = (t.min(r), t + r);
        if score > (best.0, best.1) {
            best = (score.0, score.1, c.xf.t);
        }
    }
    let center = best.2;
    let mut near: Vec<(&Placement, f64)> = harv.iter().map(|p| (*p, ((p.xf.t[0] - center[0]).powi(2) + (p.xf.t[1] - center[1]).powi(2)).sqrt())).collect();
    near.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    let sample: Vec<Value> = near
        .iter()
        .take(40)
        .map(|(p, d)| {
            let r = quat_to_rot(p.xf.q);
            json!({"class": p.class, "pos": [round(p.xf.t[0] - center[0]), round(p.xf.t[1] - center[1]), round(p.xf.t[2] - center[2])],
                   "abs": [round(p.xf.t[0]), round(p.xf.t[1]), round(p.xf.t[2])], "rot": [round3(r[0]), round3(r[1]), round3(r[2])],
                   "scale": [round3(p.xf.s[0]), round3(p.xf.s[1]), round3(p.xf.s[2])], "dist": round(*d), "level": p.level.rsplit('/').next()})
        })
        .collect();
    let sample_radius = near.get(39).map_or(0.0, |x| x.1);
    let sample_doc = json!({"version": 1, "map": MAP, "units": "cm UE (left handed, Z up); pos relative to centre; rot FRotator (pitch,yaw,roll) deg",
        "center_abs": [round(center[0]), round(center[1]), round(center[2])], "radius_cm": round(sample_radius),
        "selection": "centre = harvestable tree/rock maximising min(#trees,#rocks) within 4000 cm; sample = 40 nearest harvestable actors",
        "instances": sample});
    write_file(&a.out.join("layout/sample.json"), serde_json::to_string_pretty(&sample_doc).unwrap().as_bytes())?;

    let area_km2 = (maxx - minx) * (maxy - miny) / 1e10;
    let n_tree: usize = hist.iter().filter(|(c, _)| is_tree(c)).map(|(_, v)| v.0).sum();
    let n_rock: usize = hist.iter().filter(|(c, _)| is_rock(c)).map(|(_, v)| v.0).sum();
    let hist_json: Vec<Value> = {
        let mut v: Vec<_> = hist.iter().collect();
        v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        v.into_iter().map(|(c, h)| json!({"class": c, "count": h.0, "attr": format!("{}.{}", h.1, h.2), "resource": h.3, "mesh": h.4})).collect()
    };
    let stats = json!({
        "map": MAP,
        "levels": per_level,
        "blueprint_actor_placements": all.len(),
        "harvestable_placements": harv.len(),
        "trees_wood_resource": n_tree, "rocks_stone_resource": n_rock,
        "actors_without_root_component": no_root,
        "instanced_foliage_classes_found": foliage,
        "bbox_cm": {"min": [minx, miny], "max": [maxx, maxy], "area_km2": area_km2},
        "density": {
            "harvestables_per_km2_bbox": harv.len() as f64 / area_km2,
            "trees_per_occupied_100m_cell": {"cells": tree_cells.len(), "mean": tree_cells.iter().sum::<usize>() as f64 / tree_cells.len().max(1) as f64, "p50": pct(&tree_cells, 0.5), "p90": pct(&tree_cells, 0.9), "max": tree_cells.last()},
            "sample_centre_trees_rocks_within_4000cm": [best.0, best.1],
        },
        "per_class": hist_json,
    });
    write_file(&a.out.join("layout/stats.json"), serde_json::to_string_pretty(&stats).unwrap().as_bytes())?;
    println!(
        "harvestable placements {} (trees {n_tree}, rocks {n_rock}); bbox area {:.1} km2; sample centre {:?} radius {:.0} cm",
        harv.len(),
        area_km2,
        [center[0].round(), center[1].round(), center[2].round()],
        sample_radius
    );
    Ok(())
}

fn round(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}
fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotator_quat_roundtrip() {
        for r in [[0.0, 45.0, 0.0], [10.0, -120.0, 30.0], [0.0, 179.0, 0.0], [-20.0, 33.0, -45.0]] {
            let q = rot_to_quat(r);
            let r2 = quat_to_rot(q);
            for i in 0..3 {
                assert!((r[i] - r2[i]).abs() < 1e-6, "{r:?} -> {r2:?}");
            }
        }
    }

    #[test]
    fn yaw_90_rotates_x_to_y() {
        // UE: yaw is rotation about +Z, positive yaw turns +X toward +Y
        let q = rot_to_quat([0.0, 90.0, 0.0]);
        let v = qrot(q, [1.0, 0.0, 0.0]);
        assert!((v[0]).abs() < 1e-9 && (v[1] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn compose_applies_child_then_parent() {
        let parent = Xf { q: rot_to_quat([0.0, 90.0, 0.0]), t: [100.0, 0.0, 0.0], s: [1.0; 3] };
        let child = Xf { q: IDENT.q, t: [10.0, 0.0, 5.0], s: [1.0; 3] };
        let w = compose(child, parent);
        assert!((w.t[0] - 100.0).abs() < 1e-9 && (w.t[1] - 10.0).abs() < 1e-9 && (w.t[2] - 5.0).abs() < 1e-9);
    }
}
