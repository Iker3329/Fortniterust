//! Material instance export: `<package>.mat.json` with the parameters the runtime shaders need.
//!
//! The old UEViewer export wrote a heuristic `.mat` (Diffuse=/Normal=/Other[n]=) plus a `.props.txt` dump.
//! The new runtime wants the real data, so we write JSON built from the `MaterialInstanceConstant` tagged
//! properties: parent, scalar/vector/texture parameter values, static switch parameters, base property overrides.
//! (For parent `Material` assets only the referenced textures and the class are recorded; graph evaluation is
//! out of scope here.)
use fn_ueio::props::{read_props, Ctx};
use fn_ueio::{Package, PropList, PropValue, Reader};
use serde_json::{json, Map, Value};

fn object_path(pkg: &Package, idx: i32) -> String {
    if idx < 0 {
        pkg.import_path(idx)
    } else if idx > 0 {
        pkg.index_name(idx)
    } else {
        String::new()
    }
}

fn param_name(fields: &PropList) -> String {
    if let Some(n) = fields.text("ParameterName") {
        return n.to_string();
    }
    fields.fields("ParameterInfo").and_then(|i| i.text("Name")).unwrap_or("").to_string()
}

fn prop_json(pkg: &Package, v: &PropValue) -> Value {
    match v {
        PropValue::Bool(b) => json!(b),
        PropValue::Int(i) => json!(i),
        PropValue::Float(f) => json!(f),
        PropValue::Str(s) | PropValue::Name(s) | PropValue::Enum(s) => json!(s),
        PropValue::Object(i) => json!(object_path(pkg, *i)),
        PropValue::Byte(b) => json!(b),
        PropValue::Vec2(a) => json!(a),
        PropValue::Vec3(a) => json!(a),
        PropValue::Vec4(a) => json!(a),
        PropValue::Color(a) => json!(a),
        PropValue::Guid(_) => Value::Null,
        PropValue::Raw(b) => json!({ "raw_bytes": b.len() }),
        PropValue::Array(a) => Value::Array(a.iter().map(|x| prop_json(pkg, x)).collect()),
        PropValue::Struct { fields, .. } => list_json(pkg, fields),
    }
}

fn list_json(pkg: &Package, l: &PropList) -> Value {
    let mut m = Map::new();
    for p in &l.0 {
        m.insert(p.name.clone(), prop_json(pkg, &p.value));
    }
    Value::Object(m)
}

/// Build the JSON description of the first material-like export of `pkg`.
pub fn material_json(pkg: &Package, name: &str) -> Result<Value, String> {
    let idx = (0..pkg.exports.len())
        .find(|&i| {
            let c = pkg.export_class(i);
            c.starts_with("MaterialInstance") || c == "Material"
        })
        .ok_or("no material export")?;
    let class = pkg.export_class(idx);
    let (s, e) = pkg.export_range(idx);
    let mut r = Reader::at(&pkg.data[..e], s);
    let c = Ctx { names: &pkg.names, file_version: pkg.summary.file_version };
    let props = read_props(&mut r, &c).map_err(|e| e.to_string())?;
    let mut scalars = Map::new();
    let mut vectors = Map::new();
    let mut textures = Map::new();
    if let Some(a) = props.array("ScalarParameterValues") {
        for x in a {
            if let Some(f) = x.as_fields() {
                if let Some(v) = f.float("ParameterValue") {
                    scalars.insert(param_name(f), json!(v));
                }
            }
        }
    }
    if let Some(a) = props.array("VectorParameterValues") {
        for x in a {
            if let Some(f) = x.as_fields() {
                if let Some(v) = f.get("ParameterValue") {
                    vectors.insert(param_name(f), prop_json(pkg, v));
                }
            }
        }
    }
    if let Some(a) = props.array("TextureParameterValues") {
        for x in a {
            if let Some(f) = x.as_fields() {
                if let Some(i) = f.object("ParameterValue") {
                    textures.insert(param_name(f), json!(object_path(pkg, i)));
                }
            }
        }
    }
    // textures imported by the package (covers plain Material assets)
    let imported_textures: Vec<Value> =
        pkg.imports.iter().enumerate().filter(|(_, i)| i.class_name == "Texture2D").map(|(n, _)| json!(pkg.import_path(-(n as i32) - 1))).collect();
    let mut out = Map::new();
    out.insert("name".into(), json!(name));
    out.insert("class".into(), json!(class));
    out.insert("parent".into(), props.object("Parent").map(|i| json!(object_path(pkg, i))).unwrap_or(Value::Null));
    out.insert("scalars".into(), Value::Object(scalars));
    out.insert("vectors".into(), Value::Object(vectors));
    out.insert("textures".into(), Value::Object(textures));
    out.insert("imported_textures".into(), Value::Array(imported_textures));
    for key in ["StaticParameters", "BasePropertyOverrides", "PhysMaterial", "bUseMobileSpecular"] {
        if let Some(v) = props.get(key) {
            out.insert(key.into(), prop_json(pkg, v));
        }
    }
    Ok(Value::Object(out))
}
