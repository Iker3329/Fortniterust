//! `collision.json` from the mesh's cooked `UBodySetup` (`AggGeom` tagged struct: boxes, spheres, capsules, convex hulls).
use super::layout::rot_to_quat;
use fn_pak::PakSet;
use fn_uasset::PackageData;
use serde_json::{json, Value};

fn v3(v: Option<&Value>) -> Option<[f64; 3]> {
    let a = v?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

fn quat_of(e: &Value) -> [f64; 4] {
    if let Some(a) = e.get("Orientation").and_then(|v| v.as_array()) {
        if a.len() == 4 {
            return [a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0), a[2].as_f64().unwrap_or(0.0), a[3].as_f64().unwrap_or(1.0)];
        }
    }
    if let Some(r) = v3(e.get("Rotation")) {
        return rot_to_quat(r);
    }
    [0.0, 0.0, 0.0, 1.0]
}

fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let u = [q[0], q[1], q[2]];
    let c = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = c(u, v);
    let t = [2.0 * t[0], 2.0 * t[1], 2.0 * t[2]];
    let w = c(u, t);
    [v[0] + q[3] * t[0] + w[0], v[1] + q[3] * t[1] + w[1], v[2] + q[3] * t[2] + w[2]]
}

struct Bb {
    min: [f64; 3],
    max: [f64; 3],
}

impl Bb {
    fn new() -> Bb {
        Bb { min: [f64::MAX; 3], max: [f64::MIN; 3] }
    }
    fn add(&mut self, p: [f64; 3]) {
        for c in 0..3 {
            self.min[c] = self.min[c].min(p[c]);
            self.max[c] = self.max[c].max(p[c]);
        }
    }
    fn add_box(&mut self, center: [f64; 3], q: [f64; 4], half: [f64; 3]) {
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                for sz in [-1.0, 1.0] {
                    let r = rotate(q, [half[0] * sx, half[1] * sy, half[2] * sz]);
                    self.add([center[0] + r[0], center[1] + r[1], center[2] + r[2]]);
                }
            }
        }
    }
}

fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}
fn r3(a: [f64; 3]) -> Value {
    json!([round(a[0]), round(a[1]), round(a[2])])
}
fn r4(a: [f64; 4]) -> Value {
    json!([a[0], a[1], a[2], a[3]])
}

/// Build the collision document of a mesh package. `body_setup` is `StaticMesh::body_setup` (export name, or import path).
pub fn collision_json(set: &PakSet, mesh_pkg: &str, body_setup: Option<&str>) -> Result<Value, String> {
    let empty = |note: &str| json!({"schema": 1, "units": "cm, mesh local UE space (left handed, Z up)", "shapes": [], "note": note});
    let Some(bs) = body_setup else { return Ok(empty("static mesh has no BodySetup")) };
    // same package export name, or an import path into another package
    let (pkg_path, name) = if bs.starts_with('/') { (super::bp::package_of(bs).to_string(), super::bp::leaf(bs).to_string()) } else { (mesh_pkg.to_string(), bs.to_string()) };
    let pd = PackageData::load(set, &pkg_path).map_err(|e| e.to_string())?;
    let pkg = pd.package().map_err(|e| e.to_string())?;
    let idx = pkg.find_export(&name).or_else(|| (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i) == "BodySetup")).ok_or_else(|| format!("BodySetup {name} not found in {pkg_path}"))?;
    let obj = pkg.read_export(idx).map_err(|e| e.to_string())?;
    let props = obj.props.to_json(&pkg);
    let agg = props.get("AggGeom").cloned().unwrap_or(Value::Null);
    let mut shapes = vec![];
    let mut bb = Bb::new();
    for e in agg.get("BoxElems").and_then(|v| v.as_array()).into_iter().flatten() {
        let c = v3(e.get("Center")).unwrap_or([0.0; 3]);
        let q = quat_of(e);
        let half = [e.get("X").and_then(|v| v.as_f64()).unwrap_or(0.0) * 0.5, e.get("Y").and_then(|v| v.as_f64()).unwrap_or(0.0) * 0.5, e.get("Z").and_then(|v| v.as_f64()).unwrap_or(0.0) * 0.5];
        bb.add_box(c, q, half);
        shapes.push(json!({"type": "box", "center": r3(c), "rotation": r4(q), "half_extent": r3(half)}));
    }
    for e in agg.get("SphereElems").and_then(|v| v.as_array()).into_iter().flatten() {
        let c = v3(e.get("Center")).unwrap_or([0.0; 3]);
        let rad = e.get("Radius").and_then(|v| v.as_f64()).unwrap_or(0.0);
        bb.add([c[0] - rad, c[1] - rad, c[2] - rad]);
        bb.add([c[0] + rad, c[1] + rad, c[2] + rad]);
        shapes.push(json!({"type": "sphere", "center": r3(c), "radius": round(rad)}));
    }
    for e in agg.get("SphylElems").and_then(|v| v.as_array()).into_iter().flatten() {
        let c = v3(e.get("Center")).unwrap_or([0.0; 3]);
        let q = quat_of(e);
        let rad = e.get("Radius").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let half_len = e.get("Length").and_then(|v| v.as_f64()).unwrap_or(0.0) * 0.5;
        bb.add_box(c, q, [rad, rad, half_len + rad]);
        shapes.push(json!({"type": "capsule", "center": r3(c), "rotation": r4(q), "radius": round(rad), "half_length": round(half_len)}));
    }
    for e in agg.get("ConvexElems").and_then(|v| v.as_array()).into_iter().flatten() {
        let verts: Vec<[f64; 3]> = e.get("VertexData").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| v3(Some(x))).collect()).unwrap_or_default();
        let t = e.get("Transform");
        let tq = t.and_then(|t| t.get("Rotation")).and_then(|v| v.as_array()).filter(|a| a.len() == 4).map(|a| [a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0), a[2].as_f64().unwrap_or(0.0), a[3].as_f64().unwrap_or(1.0)]).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let tt = t.and_then(|t| v3(t.get("Translation"))).unwrap_or([0.0; 3]);
        let ts = t.and_then(|t| v3(t.get("Scale3D"))).unwrap_or([1.0; 3]);
        let mut lb = Bb::new();
        for v in &verts {
            lb.add(*v);
            let s = rotate(tq, [v[0] * ts[0], v[1] * ts[1], v[2] * ts[2]]);
            bb.add([s[0] + tt[0], s[1] + tt[1], s[2] + tt[2]]);
        }
        if verts.is_empty() {
            continue;
        }
        shapes.push(json!({
            "type": "convex", "vertices": verts.iter().map(|v| r3(*v)).collect::<Vec<_>>(),
            "transform": {"translation": r3(tt), "rotation": r4(tq), "scale": r3(ts)},
            "bounds_min": r3(lb.min), "bounds_max": r3(lb.max),
        }));
    }
    let di = props.get("DefaultInstance");
    let responses: Vec<Value> = di
        .and_then(|d| d.get("CollisionResponses"))
        .and_then(|c| c.get("ResponseArray"))
        .and_then(|a| a.as_array())
        .map(|a| a.iter().map(|r| json!({"channel": r.get("Channel"), "response": r.get("Response")})).collect())
        .unwrap_or_default();
    let have = !shapes.is_empty();
    Ok(json!({
        "schema": 1,
        "units": "cm, mesh local UE space (left handed, Z up); rotation = quaternion (x, y, z, w)",
        "source": format!("{pkg_path}:{}", pkg.exports[idx].name),
        "trace_flag": props.get("CollisionTraceFlag").and_then(|v| v.as_str()).map(|s| s.trim_start_matches("ECollisionTraceFlag::")).unwrap_or(""),
        "physics_type": props.get("PhysicsType"),
        "profile": di.and_then(|d| d.get("CollisionProfileName")),
        "collision_enabled": di.and_then(|d| d.get("CollisionEnabled")),
        "responses": responses,
        "phys_material": props.get("PhysMaterial"),
        "shapes": shapes,
        "bounds_min": if have { r3(bb.min) } else { json!([0, 0, 0]) },
        "bounds_max": if have { r3(bb.max) } else { json!([0, 0, 0]) },
        "native_bytes_not_decoded": obj.trailing.len(),
        "note": "convex hulls: vertex points only (the cooked PhysX blob that also holds faces is not decoded); rebuild the hull from the points",
    }))
}
