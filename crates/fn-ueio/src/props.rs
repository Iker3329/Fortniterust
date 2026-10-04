//! Generic UE 4.16 tagged-property reader (`FPropertyTag`, file version >= 504).
//!
//! Tag layout: `Name:FName, Type:FName, Size:i32, ArrayIndex:i32`, then type specific
//! (`StructProperty`: StructName FName + FGuid; `BoolProperty`: u8 value; `ByteProperty`/`EnumProperty`:
//! EnumName FName; `ArrayProperty`: InnerType FName; `MapProperty`: Key/Value type FNames, `SetProperty`: InnerType),
//! then `HasPropertyGuid:u8` (+16 bytes). The value occupies exactly `Size` bytes, which is always used to resync.

use crate::{Error, Reader, Result};

#[derive(Debug, Clone)]
pub enum PropValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Name(String),
    /// Package index (positive = export+1, negative = -(import+1)).
    Object(i32),
    Byte(u8),
    Enum(String),
    Array(Vec<PropValue>),
    Struct { name: String, fields: PropList },
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    /// Quat / Vector4 / LinearColor / Plane (xyzw order as stored)
    Vec4([f32; 4]),
    Color([u8; 4]),
    Guid([u8; 16]),
    Raw(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Prop {
    pub name: String,
    pub type_name: String,
    pub array_index: i32,
    pub value: PropValue,
}

#[derive(Debug, Clone, Default)]
pub struct PropList(pub Vec<Prop>);

impl PropList {
    pub fn get(&self, name: &str) -> Option<&PropValue> {
        self.0.iter().find(|p| p.name == name).map(|p| &p.value)
    }
    pub fn int(&self, name: &str) -> Option<i64> {
        match self.get(name)? {
            PropValue::Int(i) => Some(*i),
            PropValue::Byte(b) => Some(*b as i64),
            PropValue::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }
    pub fn float(&self, name: &str) -> Option<f64> {
        match self.get(name)? {
            PropValue::Float(f) => Some(*f),
            PropValue::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
    pub fn bool(&self, name: &str) -> Option<bool> {
        match self.get(name)? {
            PropValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// Enum / name / byte-enum value as text.
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            PropValue::Enum(s) | PropValue::Name(s) | PropValue::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn object(&self, name: &str) -> Option<i32> {
        match self.get(name)? {
            PropValue::Object(i) => Some(*i),
            _ => None,
        }
    }
    pub fn array(&self, name: &str) -> Option<&Vec<PropValue>> {
        match self.get(name)? {
            PropValue::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn fields(&self, name: &str) -> Option<&PropList> {
        match self.get(name)? {
            PropValue::Struct { fields, .. } => Some(fields),
            _ => None,
        }
    }
}

impl PropValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            PropValue::Float(f) => Some(*f),
            PropValue::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
    pub fn as_fields(&self) -> Option<&PropList> {
        match self {
            PropValue::Struct { fields, .. } => Some(fields),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            PropValue::Enum(s) | PropValue::Name(s) | PropValue::Str(s) => Some(s),
            _ => None,
        }
    }
}

pub struct Ctx<'a> {
    pub names: &'a [String],
    pub file_version: i32,
}

fn fname(r: &mut Reader, c: &Ctx) -> Result<String> {
    let idx = r.i32()?;
    let num = r.i32()?;
    let base = c.names.get(idx as usize).cloned().ok_or_else(|| Error::Invalid(format!("bad FName index {idx} at {}", r.tell())))?;
    Ok(if num > 0 { format!("{}_{}", base, num - 1) } else { base })
}

struct Tag {
    name: String,
    type_name: String,
    size: i32,
    array_index: i32,
    struct_name: String,
    bool_val: bool,
    enum_name: String,
    inner_type: String,
    value_type: String,
}

fn read_tag(r: &mut Reader, c: &Ctx) -> Result<Option<Tag>> {
    let name = fname(r, c)?;
    if name == "None" {
        return Ok(None);
    }
    let type_name = fname(r, c)?;
    let size = r.i32()?;
    let array_index = r.i32()?;
    let mut t = Tag { name, type_name, size, array_index, struct_name: String::new(), bool_val: false, enum_name: String::new(), inner_type: String::new(), value_type: String::new() };
    match t.type_name.as_str() {
        "StructProperty" => {
            t.struct_name = fname(r, c)?;
            if c.file_version >= 441 {
                r.guid()?;
            }
        }
        "BoolProperty" => t.bool_val = r.u8()? != 0,
        "ByteProperty" | "EnumProperty" => t.enum_name = fname(r, c)?,
        "ArrayProperty" | "SetProperty" => {
            if c.file_version >= 282 {
                t.inner_type = fname(r, c)?;
            }
        }
        "MapProperty" => {
            t.inner_type = fname(r, c)?;
            t.value_type = fname(r, c)?;
        }
        _ => {}
    }
    if c.file_version >= 503 {
        if r.u8()? != 0 {
            r.guid()?;
        }
    }
    Ok(Some(t))
}

/// Read properties until the `None` terminator. The reader ends right after the terminator.
pub fn read_props(r: &mut Reader, c: &Ctx) -> Result<PropList> {
    let mut out = Vec::new();
    while let Some(t) = read_tag(r, c)? {
        let start = r.tell();
        let end = start + t.size.max(0) as usize;
        let value = match read_value(r, c, &t.type_name, &t, t.size as usize) {
            Ok(v) => v,
            Err(_) => {
                r.seek(start)?;
                PropValue::Raw(r.bytes(t.size as usize)?.to_vec())
            }
        };
        // Always resync on the declared size (BoolProperty has size 0).
        r.seek(end)?;
        out.push(Prop { name: t.name, type_name: t.type_name, array_index: t.array_index, value });
    }
    Ok(PropList(out))
}

fn read_value(r: &mut Reader, c: &Ctx, ty: &str, t: &Tag, size: usize) -> Result<PropValue> {
    Ok(match ty {
        "BoolProperty" => PropValue::Bool(t.bool_val),
        "IntProperty" => PropValue::Int(r.i32()? as i64),
        "UInt32Property" => PropValue::Int(r.u32()? as i64),
        "Int64Property" => PropValue::Int(r.i64()?),
        "UInt64Property" => PropValue::Int(r.u64()? as i64),
        "Int16Property" => PropValue::Int(r.i16()? as i64),
        "UInt16Property" => PropValue::Int(r.u16()? as i64),
        "Int8Property" => PropValue::Int(r.i8()? as i64),
        "FloatProperty" => PropValue::Float(r.f32()? as f64),
        "DoubleProperty" => PropValue::Float(r.f64()?),
        "NameProperty" => PropValue::Name(fname(r, c)?),
        "StrProperty" => PropValue::Str(r.fstring()?),
        "ByteProperty" => {
            if size >= 8 && t.enum_name != "None" {
                PropValue::Enum(fname(r, c)?)
            } else {
                PropValue::Byte(r.u8()?)
            }
        }
        "EnumProperty" => PropValue::Enum(fname(r, c)?),
        "ObjectProperty" | "WeakObjectProperty" | "LazyObjectProperty" | "InterfaceProperty" => PropValue::Object(r.i32()?),
        "AssetObjectProperty" | "SoftObjectProperty" => PropValue::Str(r.fstring()?),
        "ArrayProperty" => {
            let n = r.i32()?;
            if n < 0 || n as usize > r.remaining() + 1 {
                return crate::invalid("bad array count");
            }
            let mut v = Vec::with_capacity(n as usize);
            if t.inner_type == "StructProperty" {
                // inner tag
                let itag = match read_tag_inner(r, c)? {
                    Some(x) => x,
                    None => return crate::invalid("array inner tag"),
                };
                let per = if n > 0 { itag.size as usize / n as usize } else { 0 };
                let atomic = matches!(itag.struct_name.as_str(), "Vector" | "Rotator" | "Vector2D" | "Quat" | "Vector4" | "LinearColor" | "Plane" | "Color" | "Guid" | "IntPoint")
                    || (itag.struct_name == "Transform" && per == 40);
                for _ in 0..n {
                    if atomic {
                        v.push(read_struct(r, c, &itag.struct_name, per)?);
                    } else {
                        let fields = read_props(r, c)?;
                        v.push(PropValue::Struct { name: itag.struct_name.clone(), fields });
                    }
                }
            } else {
                for _ in 0..n {
                    let inner = Tag { type_name: t.inner_type.clone(), enum_name: t.enum_name.clone(), bool_val: false, ..clone_tag(t) };
                    let sz = match t.inner_type.as_str() {
                        "ByteProperty" | "BoolProperty" => 1,
                        "NameProperty" => 8,
                        "IntProperty" | "UInt32Property" | "FloatProperty" | "ObjectProperty" => 4,
                        _ => 0,
                    };
                    let val = match t.inner_type.as_str() {
                        "BoolProperty" => PropValue::Bool(r.u8()? != 0),
                        "ByteProperty" => PropValue::Byte(r.u8()?),
                        other => read_value(r, c, other, &inner, sz)?,
                    };
                    v.push(val);
                }
            }
            PropValue::Array(v)
        }
        "StructProperty" => read_struct(r, c, &t.struct_name, size)?,
        _ => PropValue::Raw(r.bytes(size)?.to_vec()),
    })
}

fn clone_tag(t: &Tag) -> Tag {
    Tag {
        name: t.name.clone(),
        type_name: t.type_name.clone(),
        size: t.size,
        array_index: t.array_index,
        struct_name: t.struct_name.clone(),
        bool_val: t.bool_val,
        enum_name: t.enum_name.clone(),
        inner_type: t.inner_type.clone(),
        value_type: t.value_type.clone(),
    }
}

fn read_tag_inner(r: &mut Reader, c: &Ctx) -> Result<Option<Tag>> {
    // identical layout to a normal tag (name, type, size, index, struct name, guid, has-guid)
    read_tag(r, c)
}

fn read_struct(r: &mut Reader, c: &Ctx, name: &str, size: usize) -> Result<PropValue> {
    Ok(match name {
        "Vector" | "Rotator" if size == 12 => PropValue::Vec3(r.vec3()?),
        "Vector2D" if size == 8 => PropValue::Vec2([r.f32()?, r.f32()?]),
        "Quat" | "Vector4" | "LinearColor" | "Plane" if size == 16 => PropValue::Vec4(r.quat()?),
        "Color" if size == 4 => {
            let b = r.bytes(4)?;
            PropValue::Color([b[0], b[1], b[2], b[3]])
        }
        "Guid" if size == 16 => PropValue::Guid(r.guid()?),
        // native-serialised FTransform (rotation quat, translation, scale3d as floats)
        "Transform" if size == 40 => {
            let q = r.quat()?;
            let t = r.vec3()?;
            let s = r.vec3()?;
            PropValue::Struct {
                name: "Transform".into(),
                fields: PropList(vec![
                    Prop { name: "Rotation".into(), type_name: "Quat".into(), array_index: 0, value: PropValue::Vec4(q) },
                    Prop { name: "Translation".into(), type_name: "Vector".into(), array_index: 0, value: PropValue::Vec3(t) },
                    Prop { name: "Scale3D".into(), type_name: "Vector".into(), array_index: 0, value: PropValue::Vec3(s) },
                ]),
            }
        }
        "IntPoint" if size == 8 => PropValue::Vec2([r.i32()? as f32, r.i32()? as f32]),
        _ => {
            let start = r.tell();
            let sub = Reader::at(&r.data[..(start + size).min(r.data.len())], start);
            let mut sub = sub;
            match read_props(&mut sub, c) {
                Ok(fields) if sub.tell() == start + size || sub.tell() <= start + size => {
                    r.seek(start + size)?;
                    PropValue::Struct { name: name.to_string(), fields }
                }
                _ => {
                    r.seek(start)?;
                    PropValue::Raw(r.bytes(size)?.to_vec())
                }
            }
        }
    })
}

