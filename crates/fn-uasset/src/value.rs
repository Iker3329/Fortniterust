//! Generic property value tree + JSON export.

use crate::package::{Package, PackageIndex};
use serde_json::{json, Map, Number, Value as Json};

/// `FText` as stored in packages.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Text {
    pub flags: u32,
    /// `-1` none, `0` base (namespace/key/source), `11` string table entry, others are not decoded.
    pub history_type: i8,
    pub namespace: Option<String>,
    pub key: Option<String>,
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    /// Static-array index (`ArrayIndex` of the tag); 0 for ordinary properties.
    pub index: u32,
    /// Property type name from the tag (`FloatProperty`, `StructProperty`, ...).
    pub ty: String,
    pub value: Value,
    /// Byte offset of this property's tag in the concatenation `uasset ++ uexp` (subtract the `.uasset` length
    /// for a `.uexp` offset). 0 for synthesized properties.
    pub offset: u32,
}

/// Ordered list of tagged properties (duplicate names allowed for static arrays).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PropertyList(pub Vec<Property>);

impl PropertyList {
    pub fn new() -> Self {
        PropertyList(Vec::new())
    }
    pub fn iter(&self) -> std::slice::Iter<'_, Property> {
        self.0.iter()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// First property with this exact name and static index 0.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.iter().find(|p| p.name == name && p.index == 0).map(|p| &p.value)
    }
    /// Case-insensitive variant.
    pub fn get_ci(&self, name: &str) -> Option<&Value> {
        self.0.iter().find(|p| p.index == 0 && p.name.eq_ignore_ascii_case(name)).map(|p| &p.value)
    }
    /// All entries named `name` (static arrays).
    pub fn get_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Property> + 'a {
        self.0.iter().filter(move |p| p.name == name)
    }
    pub fn to_json(&self, pkg: &Package) -> Json {
        let mut m = Map::new();
        for p in &self.0 {
            let key = if p.index > 0 { format!("{}[{}]", p.name, p.index) } else { p.name.clone() };
            m.insert(key, p.value.to_json(pkg));
        }
        Json::Object(m)
    }
}

/// A decoded property value. `Unparsed` is the safe fallback: raw bytes plus the reason decoding was not possible.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f32),
    Double(f64),
    /// ByteProperty without enum.
    Byte(u8),
    /// ByteProperty/EnumProperty with enum: the enumerator name (e.g. `EFoo::Bar`).
    Enum { enum_name: String, value: String },
    Name(String),
    Str(String),
    Text(Text),
    /// Object / class / weak / interface reference (resolve with [`Package::object_path`]).
    Object(PackageIndex),
    /// Soft object / asset reference: full asset path string (empty = null).
    SoftObject(String),
    /// LazyObjectProperty guid.
    LazyObject([u8; 16]),
    Delegate { object: PackageIndex, function: String },
    MulticastDelegate(Vec<(PackageIndex, String)>),
    Array(Vec<Value>),
    Set(Vec<Value>),
    Map(Vec<(Value, Value)>),
    /// Tagged struct (properties follow the property-tag format).
    Struct { name: String, props: PropertyList },
    Vector([f32; 3]),
    Vector2D([f32; 2]),
    Vector4([f32; 4]),
    Rotator([f32; 3]),
    Quat([f32; 4]),
    LinearColor([f32; 4]),
    /// `FColor`, stored as the on-disk byte order B,G,R,A -> here normalised to `[r, g, b, a]`.
    Color([u8; 4]),
    IntPoint([i32; 2]),
    IntVector([i32; 3]),
    Guid([u8; 16]),
    /// Binary `FTransform`: rotation quat, translation, scale.
    Transform { rotation: [f32; 4], translation: [f32; 3], scale: [f32; 3] },
    /// `FDateTime` / `FTimespan` ticks (100ns).
    DateTime(i64),
    Timespan(i64),
    /// Not decoded (unknown type, native-serialized struct without a layout, or a decode error).
    Unparsed { ty: String, bytes: Vec<u8>, reason: String },
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f as f64),
            Value::Double(d) => Some(*d),
            Value::Int(i) => Some(*i as f64),
            Value::UInt(u) => Some(*u as f64),
            Value::Byte(b) => Some(*b as f64),
            _ => None,
        }
    }
    pub fn as_f32(&self) -> Option<f32> {
        self.as_f64().map(|x| x as f32)
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::UInt(u) => Some(*u as i64),
            Value::Byte(b) => Some(*b as i64),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }
    /// Name / string / enumerator / soft path / text source as `&str`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Name(s) | Value::Str(s) | Value::SoftObject(s) => Some(s),
            Value::Enum { value, .. } => Some(value),
            Value::Text(t) => t.source.as_deref(),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<PackageIndex> {
        if let Value::Object(i) = self {
            Some(*i)
        } else {
            None
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) | Value::Set(a) => Some(a),
            _ => None,
        }
    }
    /// Properties of a tagged struct.
    pub fn as_struct(&self) -> Option<&PropertyList> {
        if let Value::Struct { props, .. } = self {
            Some(props)
        } else {
            None
        }
    }
    /// `[x, y, z]` for Vector / Rotator (pitch, yaw, roll).
    pub fn as_vec3(&self) -> Option<[f32; 3]> {
        match self {
            Value::Vector(v) | Value::Rotator(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_linear_color(&self) -> Option<[f32; 4]> {
        if let Value::LinearColor(c) = self {
            Some(*c)
        } else {
            None
        }
    }
    /// Struct field lookup (`value.field("Settings")`).
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.as_struct().and_then(|p| p.get(name))
    }
    /// Dotted-path lookup through nested structs: `"Settings.WhiteTemp"`.
    pub fn path(&self, path: &str) -> Option<&Value> {
        let mut cur = self;
        for part in path.split('.') {
            cur = cur.field(part)?;
        }
        Some(cur)
    }

    /// JSON export. Floats are exported as their exact f32 value widened to f64 (same as the Node tools).
    /// Vectors/rotators/quats/colors are arrays, structs are objects with a `"_struct"` key, objects are
    /// path strings (`null` for none), maps are `[{"key":..,"value":..}]`.
    pub fn to_json(&self, pkg: &Package) -> Json {
        match self {
            Value::Bool(b) => json!(b),
            Value::Int(i) => json!(i),
            Value::UInt(u) => json!(u),
            Value::Float(f) => f64_json(*f as f64),
            Value::Double(d) => f64_json(*d),
            Value::Byte(b) => json!(b),
            Value::Enum { value, .. } => json!(value),
            Value::Name(s) | Value::Str(s) | Value::SoftObject(s) => json!(s),
            Value::Text(t) => match t.history_type {
                0 => json!({"text": t.source, "key": t.key, "namespace": t.namespace}),
                _ => Json::Null,
            },
            Value::Object(i) => {
                if i.is_null() {
                    Json::Null
                } else {
                    json!(pkg.object_path(*i))
                }
            }
            Value::LazyObject(g) | Value::Guid(g) => json!(hex(g)),
            Value::Delegate { object, function } => json!({"object": pkg.object_path(*object), "function": function}),
            Value::MulticastDelegate(v) => Json::Array(v.iter().map(|(o, f)| json!({"object": pkg.object_path(*o), "function": f})).collect()),
            Value::Array(a) | Value::Set(a) => Json::Array(a.iter().map(|v| v.to_json(pkg)).collect()),
            Value::Map(m) => Json::Array(m.iter().map(|(k, v)| json!({"key": k.to_json(pkg), "value": v.to_json(pkg)})).collect()),
            Value::Struct { name, props } => {
                let mut m = Map::new();
                m.insert("_struct".into(), json!(name));
                if let Json::Object(o) = props.to_json(pkg) {
                    for (k, v) in o {
                        m.insert(k, v);
                    }
                }
                Json::Object(m)
            }
            Value::Vector(v) | Value::Rotator(v) => Json::Array(v.iter().map(|x| f64_json(*x as f64)).collect()),
            Value::Vector2D(v) => Json::Array(v.iter().map(|x| f64_json(*x as f64)).collect()),
            Value::Vector4(v) | Value::Quat(v) | Value::LinearColor(v) => Json::Array(v.iter().map(|x| f64_json(*x as f64)).collect()),
            Value::Color(c) => json!({"r": c[0], "g": c[1], "b": c[2], "a": c[3]}),
            Value::IntPoint(v) => json!(v),
            Value::IntVector(v) => json!(v),
            Value::Transform { rotation, translation, scale } => json!({
                "_struct": "Transform",
                "Rotation": rotation.iter().map(|x| f64_json(*x as f64)).collect::<Vec<_>>(),
                "Translation": translation.iter().map(|x| f64_json(*x as f64)).collect::<Vec<_>>(),
                "Scale3D": scale.iter().map(|x| f64_json(*x as f64)).collect::<Vec<_>>(),
            }),
            Value::DateTime(t) | Value::Timespan(t) => json!(t),
            Value::Unparsed { ty, bytes, reason } => {
                json!({"_unparsed": ty, "size": bytes.len(), "hex": hex(&bytes[..bytes.len().min(64)]), "error": reason})
            }
        }
    }
}

fn f64_json(x: f64) -> Json {
    match Number::from_f64(x) {
        Some(n) => Json::Number(n),
        None => json!(format!("{x}")),
    }
}

pub fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

