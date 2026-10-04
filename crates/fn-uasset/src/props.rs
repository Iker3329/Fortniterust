//! Reflection-light `FPropertyTag` reader (UE 4.16 tagged property serialization).
//!
//! The reader never needs class reflection: every property carries its own type tag and byte size, so any value
//! that cannot be decoded is returned as [`Value::Unparsed`] and reading continues at the next tag.
//! Structs are decoded as binary for the engine's "atomic" structs (Vector, Rotator, Quat, LinearColor, Color,
//! IntPoint, Guid, Transform, ...) when the byte size matches the known layout, and as nested tagged property
//! lists otherwise.

use crate::error::{Error, Result};
use crate::package::{ver, Package, PackageIndex};
use crate::reader::Reader;
use crate::value::{Property, PropertyList, Text, Value};

/// One decoded `FPropertyTag` header.
#[derive(Clone, Debug, Default)]
pub struct Tag {
    pub name: String,
    pub ty: String,
    pub size: usize,
    pub array_index: u32,
    pub struct_name: String,
    pub struct_guid: [u8; 16],
    pub bool_val: bool,
    pub enum_name: String,
    pub inner_type: String,
    pub value_type: String,
    pub prop_guid: Option<[u8; 16]>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ByteMode {
    Name,
    U8,
    I32,
}

const MAX_UNPARSED_BYTES: usize = 4096;

fn fixed_width(ty: &str) -> Option<usize> {
    Some(match ty {
        "NameProperty" => 8,
        "IntProperty" | "UInt32Property" | "FloatProperty" | "ObjectProperty" | "ClassProperty" => 4,
        "Int64Property" | "UInt64Property" | "DoubleProperty" => 8,
        _ => return None,
    })
}

fn prop(name: &str, ty: &str, value: Value) -> Property {
    Property { name: name.to_string(), index: 0, ty: ty.to_string(), value, offset: 0 }
}

fn binary_struct_size(name: &str) -> Option<usize> {
    Some(match name {
        "Vector" | "Rotator" | "IntVector" => 12,
        "Vector2D" | "IntPoint" | "DateTime" | "Timespan" => 8,
        "Vector4" | "Quat" | "LinearColor" | "Guid" | "Plane" | "IntRect" => 16,
        "Color" => 4,
        "Transform" => 40,
        "Box" => 25,
        "Box2D" => 17,
        "Matrix" => 64,
        "RichCurveKey" => 27,
        _ => return None,
    })
}

impl<'a> Package<'a> {
    // ------------------------------------------------------------------------------------------
    // public entry points
    // ------------------------------------------------------------------------------------------

    /// Read a tagged property list starting at `r.p` up to and including the `None` terminator.
    /// On error, properties read so far are left in `out`.
    pub fn read_tagged_into(&self, r: &mut Reader<'a>, out: &mut PropertyList) -> Result<()> {
        for _ in 0..200_000 {
            let tag_offset = self.abs_offset(r);
            let tag = match self.read_tag(r)? {
                None => return Ok(()),
                Some(t) => t,
            };
            let data = r.bytes(tag.size).map_err(|_| Error::malformed(format!("property '{}' ({}) size {} runs past end of data", tag.name, tag.ty, tag.size)))?;
            let value = match self.read_property_value(&tag, data) {
                Ok(v) => v,
                Err(e) => Value::Unparsed { ty: if tag.ty == "StructProperty" { format!("Struct<{}>", tag.struct_name) } else { tag.ty.clone() }, bytes: data[..data.len().min(MAX_UNPARSED_BYTES)].to_vec(), reason: e.to_string() },
            };
            out.0.push(Property { name: tag.name, index: tag.array_index, ty: tag.ty, value, offset: tag_offset });
        }
        Err(Error::malformed("property list has no None terminator"))
    }

    /// Strict variant of [`Package::read_tagged_into`].
    pub fn read_tagged(&self, r: &mut Reader<'a>) -> Result<PropertyList> {
        let mut l = PropertyList::new();
        self.read_tagged_into(r, &mut l)?;
        Ok(l)
    }

    /// Offset of the reader position in `uasset ++ uexp` (0 if the reader is over foreign memory).
    pub(crate) fn abs_offset(&self, r: &Reader<'a>) -> u32 {
        let pos = r.b.as_ptr() as usize + r.p;
        let (a0, a1) = (self.asset.as_ptr() as usize, self.asset.as_ptr() as usize + self.asset.len());
        let (u0, u1) = (self.uexp.as_ptr() as usize, self.uexp.as_ptr() as usize + self.uexp.len());
        if pos >= u0 && pos < u1 + 1 && !self.uexp.is_empty() {
            (self.asset.len() + (pos - u0)) as u32
        } else if pos >= a0 && pos < a1 + 1 {
            (pos - a0) as u32
        } else {
            0
        }
    }

    // ------------------------------------------------------------------------------------------
    // tag header
    // ------------------------------------------------------------------------------------------

    fn read_fname(&self, r: &mut Reader<'a>) -> Result<String> {
        let i = r.i32()?;
        let n = r.i32()?;
        self.fname(i, n)
    }

    /// Returns `None` for the `None` terminator.
    pub fn read_tag(&self, r: &mut Reader<'a>) -> Result<Option<Tag>> {
        let ni = r.i32()?;
        let nn = r.i32()?;
        let base = self.name_base(ni).ok_or_else(|| Error::malformed(format!("tag name index {ni} out of range at {}", r.p - 8)))?;
        if base == "None" && nn == 0 {
            return Ok(None);
        }
        let name = self.fname(ni, nn)?;
        let ty = self.read_fname(r)?;
        let size = r.i32()?;
        let array_index = r.i32()?;
        if size < 0 || array_index < 0 {
            return Err(Error::malformed(format!("tag '{name}' has negative size/index ({size}, {array_index})")));
        }
        let v = self.version();
        let mut t = Tag { name, ty, size: size as usize, array_index: array_index as u32, ..Default::default() };
        match t.ty.as_str() {
            "StructProperty" => {
                t.struct_name = self.read_fname(r)?;
                if v >= ver::STRUCT_GUID_IN_PROPERTY_TAG {
                    t.struct_guid = r.guid()?;
                }
            }
            "BoolProperty" => t.bool_val = r.u8()? != 0,
            "ByteProperty" | "EnumProperty" => t.enum_name = self.read_fname(r)?,
            "ArrayProperty" => {
                if v >= ver::ARRAY_PROPERTY_INNER_TAGS {
                    t.inner_type = self.read_fname(r)?;
                }
            }
            "SetProperty" => {
                if v >= ver::PROPERTY_TAG_SET_MAP_SUPPORT {
                    t.inner_type = self.read_fname(r)?;
                }
            }
            "MapProperty" => {
                if v >= ver::PROPERTY_TAG_SET_MAP_SUPPORT {
                    t.inner_type = self.read_fname(r)?;
                    t.value_type = self.read_fname(r)?;
                }
            }
            _ => {}
        }
        if v >= ver::PROPERTY_GUID_IN_PROPERTY_TAG && r.u8()? != 0 {
            t.prop_guid = Some(r.guid()?);
        }
        Ok(Some(t))
    }

    // ------------------------------------------------------------------------------------------
    // values
    // ------------------------------------------------------------------------------------------

    fn read_property_value(&self, tag: &Tag, data: &'a [u8]) -> Result<Value> {
        if tag.ty == "BoolProperty" {
            return Ok(Value::Bool(tag.bool_val));
        }
        let mut r = Reader::new(data);
        let v = self.read_value(&tag.ty, tag, &mut r, false, ByteMode::Name, None)?;
        if r.remaining() != 0 {
            return Err(Error::malformed(format!("{} trailing byte(s) after {} value", r.remaining(), tag.ty)));
        }
        Ok(v)
    }

    fn read_object_ref(&self, r: &mut Reader<'a>) -> Result<PackageIndex> {
        let i = r.i32()?;
        let idx = PackageIndex(i);
        if let Some(n) = idx.import_index() {
            if n >= self.imports.len() {
                return Err(Error::malformed(format!("object ref import {n} out of range")));
            }
        } else if let Some(n) = idx.export_index() {
            if n >= self.exports.len() {
                return Err(Error::malformed(format!("object ref export {n} out of range")));
            }
        }
        Ok(idx)
    }

    fn read_soft_path(&self, r: &mut Reader<'a>) -> Result<String> {
        if self.version() >= ver::ADDED_SOFT_OBJECT_PATH {
            let asset = self.read_fname(r)?;
            let sub = r.fstring()?;
            Ok(if sub.is_empty() { asset } else { format!("{asset}:{sub}") })
        } else {
            r.fstring()
        }
    }

    /// Decode a scalar/container value of property type `ty`.
    /// `size_hint` = remaining byte budget for this value when the caller knows it (used for byte/enum width).
    fn read_value(&self, ty: &str, tag: &Tag, r: &mut Reader<'a>, in_array: bool, bm: ByteMode, size_hint: Option<usize>) -> Result<Value> {
        Ok(match ty {
            "BoolProperty" => {
                if in_array {
                    Value::Bool(r.u8()? != 0)
                } else {
                    Value::Bool(tag.bool_val)
                }
            }
            "IntProperty" => Value::Int(r.i32()? as i64),
            "UInt32Property" => Value::UInt(r.u32()? as u64),
            "Int64Property" => Value::Int(r.i64()?),
            "UInt64Property" => Value::UInt(r.u64()?),
            "Int16Property" => Value::Int(r.i16()? as i64),
            "UInt16Property" => Value::UInt(r.u16()? as u64),
            "Int8Property" => Value::Int(r.i8()? as i64),
            "FloatProperty" => Value::Float(r.f32()?),
            "DoubleProperty" => Value::Double(r.f64()?),
            "NameProperty" => Value::Name(self.read_fname(r)?),
            "StrProperty" => Value::Str(r.fstring()?),
            "TextProperty" => Value::Text(self.read_text(r)?),
            "ObjectProperty" | "ClassProperty" | "InterfaceProperty" | "WeakObjectProperty" => Value::Object(self.read_object_ref(r)?),
            "LazyObjectProperty" => Value::LazyObject(r.guid()?),
            "AssetObjectProperty" | "AssetClassProperty" | "SoftObjectProperty" | "SoftClassProperty" => Value::SoftObject(self.read_soft_path(r)?),
            "DelegateProperty" => {
                let object = self.read_object_ref(r)?;
                let function = self.read_fname(r)?;
                Value::Delegate { object, function }
            }
            "MulticastDelegateProperty" => {
                let n = r.count(12)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let o = self.read_object_ref(r)?;
                    let f = self.read_fname(r)?;
                    v.push((o, f));
                }
                Value::MulticastDelegate(v)
            }
            "ByteProperty" | "EnumProperty" => {
                // standalone tag: width is given by the tag size (1 = plain byte, 4 = int-backed enum, 8 = FName)
                let mode = if in_array || size_hint.is_some() {
                    bm
                } else {
                    match tag.size {
                        1 => ByteMode::U8,
                        4 => ByteMode::I32,
                        _ => ByteMode::Name,
                    }
                };
                match mode {
                    ByteMode::Name => Value::Enum { enum_name: tag.enum_name.clone(), value: self.read_fname(r)? },
                    ByteMode::U8 => Value::Byte(r.u8()?),
                    ByteMode::I32 => Value::Int(r.i32()? as i64),
                }
            }
            "StructProperty" => self.read_struct(&tag.struct_name, r, size_hint.or(if in_array { None } else { Some(tag.size) }))?,
            "ArrayProperty" => self.read_array(tag, r)?,
            "SetProperty" => self.read_set(tag, r)?,
            "MapProperty" => self.read_map(tag, r)?,
            other => return Err(Error::unsupported(format!("property type {other}"))),
        })
    }

    pub(crate) fn read_text(&self, r: &mut Reader<'a>) -> Result<Text> {
        let flags = r.u32()?;
        let mut t = Text { flags, history_type: -1, ..Default::default() };
        if self.version() < ver::FTEXT_HISTORY {
            // legacy: just a source string
            t.history_type = 0;
            t.source = Some(r.fstring()?);
            return Ok(t);
        }
        let h = r.i8()?;
        t.history_type = h;
        match h {
            -1 => {
                // UE 4.16: nothing follows a none-history (later engines add a culture-invariant string flag)
            }
            0 => {
                t.namespace = Some(r.fstring()?);
                t.key = Some(r.fstring()?);
                t.source = Some(r.fstring()?);
            }
            11 => {
                let table = self.read_fname(r)?;
                let key = r.fstring()?;
                t.namespace = Some(table);
                t.key = Some(key);
            }
            other => self.read_text_history(other, &mut t, r)?,
        }
        Ok(t)
    }

    // ------------------------------------------------------------------------------------------
    // structs
    // ------------------------------------------------------------------------------------------

    fn read_binary_struct(&self, name: &str, r: &mut Reader<'a>) -> Result<Option<Value>> {
        Ok(Some(match name {
            "Vector" => Value::Vector([r.f32()?, r.f32()?, r.f32()?]),
            "Rotator" => Value::Rotator([r.f32()?, r.f32()?, r.f32()?]),
            "Vector2D" => Value::Vector2D([r.f32()?, r.f32()?]),
            "Vector4" | "Plane" => Value::Vector4([r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
            "Quat" => Value::Quat([r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
            "LinearColor" => Value::LinearColor([r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
            "Color" => {
                let (b, g, rr, a) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
                Value::Color([rr, g, b, a])
            }
            "IntPoint" => Value::IntPoint([r.i32()?, r.i32()?]),
            "IntVector" => Value::IntVector([r.i32()?, r.i32()?, r.i32()?]),
            "Guid" => Value::Guid(r.guid()?),
            "DateTime" => Value::DateTime(r.i64()?),
            "Timespan" => Value::Timespan(r.i64()?),
            "Transform" => {
                let rotation = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
                let translation = [r.f32()?, r.f32()?, r.f32()?];
                let scale = [r.f32()?, r.f32()?, r.f32()?];
                Value::Transform { rotation, translation, scale }
            }
            "IntRect" => {
                let v = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
                Value::Struct {
                    name: name.into(),
                    props: PropertyList(vec![
                        prop("Min", "StructProperty", Value::IntPoint([v[0], v[1]])),
                        prop("Max", "StructProperty", Value::IntPoint([v[2], v[3]])),
                    ]),
                }
            }
            "Box" => {
                let min = Value::Vector([r.f32()?, r.f32()?, r.f32()?]);
                let max = Value::Vector([r.f32()?, r.f32()?, r.f32()?]);
                let valid = r.u8()? != 0;
                Value::Struct {
                    name: name.into(),
                    props: PropertyList(vec![prop("Min", "StructProperty", min), prop("Max", "StructProperty", max), prop("IsValid", "BoolProperty", Value::Bool(valid))]),
                }
            }
            "Box2D" => {
                let min = Value::Vector2D([r.f32()?, r.f32()?]);
                let max = Value::Vector2D([r.f32()?, r.f32()?]);
                let valid = r.u8()? != 0;
                Value::Struct {
                    name: name.into(),
                    props: PropertyList(vec![prop("Min", "StructProperty", min), prop("Max", "StructProperty", max), prop("bIsValid", "BoolProperty", Value::Bool(valid))]),
                }
            }
            "Matrix" => {
                let mut m = Vec::with_capacity(16);
                for _ in 0..16 {
                    m.push(Value::Float(r.f32()?));
                }
                Value::Struct { name: name.into(), props: PropertyList(vec![prop("M", "ArrayProperty", Value::Array(m))]) }
            }
            "RichCurveKey" => {
                let interp = r.u8()?;
                let tangent_mode = r.u8()?;
                let weight_mode = r.u8()?;
                let f = |r: &mut Reader<'a>| r.f32();
                let (time, value, at, atw, lt, ltw) = (f(r)?, f(r)?, f(r)?, f(r)?, f(r)?, f(r)?);
                Value::Struct {
                    name: name.into(),
                    props: PropertyList(vec![
                        prop("InterpMode", "ByteProperty", Value::Byte(interp)),
                        prop("TangentMode", "ByteProperty", Value::Byte(tangent_mode)),
                        prop("TangentWeightMode", "ByteProperty", Value::Byte(weight_mode)),
                        prop("Time", "FloatProperty", Value::Float(time)),
                        prop("Value", "FloatProperty", Value::Float(value)),
                        prop("ArriveTangent", "FloatProperty", Value::Float(at)),
                        prop("ArriveTangentWeight", "FloatProperty", Value::Float(atw)),
                        prop("LeaveTangent", "FloatProperty", Value::Float(lt)),
                        prop("LeaveTangentWeight", "FloatProperty", Value::Float(ltw)),
                    ]),
                }
            }
            _ => return Ok(None),
        }))
    }

    /// Read one struct value. `size` is the exact byte size of this value when known (single property tag,
    /// or array payload divided by element count).
    pub(crate) fn read_struct(&self, name: &str, r: &mut Reader<'a>, size: Option<usize>) -> Result<Value> {
        if let Some(fixed) = binary_struct_size(name) {
            let use_binary = match size {
                Some(s) => s == fixed,
                None => true,
            };
            if use_binary {
                if let Some(v) = self.read_binary_struct(name, r)? {
                    return Ok(v);
                }
            }
        }
        let native_start = r.p;
        if let Some(v) = self.read_native_struct(name, r, size)? {
            if let Some(s) = size {
                if r.p - native_start != s {
                    let used = r.p - native_start;
                    r.p = native_start;
                    return Err(Error::malformed(format!("native struct {name}: layout consumed {used} of {s} bytes")));
                }
            }
            return Ok(v);
        }
        if size == Some(0) {
            return Ok(Value::Struct { name: name.to_string(), props: PropertyList::new() });
        }
        if matches!(name, "SoftObjectPath" | "StringAssetReference" | "StringClassReference" | "SoftClassPath") {
            // binary string form when it parses exactly; otherwise tagged form
            let save = r.p;
            if let Ok(s) = self.read_soft_path(r) {
                if size.map(|sz| r.p - save == sz).unwrap_or(true) {
                    return Ok(Value::SoftObject(s));
                }
            }
            r.p = save;
        }
        match size {
            Some(sz) => {
                let data = r.bytes(sz)?;
                let mut sub = Reader::new(data);
                let mut props = PropertyList::new();
                self.read_tagged_into(&mut sub, &mut props)?;
                if sub.remaining() != 0 {
                    return Err(Error::malformed(format!("struct {name}: tagged list ended with {} byte(s) left of {sz}", sub.remaining())));
                }
                Ok(Value::Struct { name: name.to_string(), props })
            }
            None => {
                let mut props = PropertyList::new();
                self.read_tagged_into(r, &mut props)?;
                Ok(Value::Struct { name: name.to_string(), props })
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // containers
    // ------------------------------------------------------------------------------------------

    fn read_array(&self, tag: &Tag, r: &mut Reader<'a>) -> Result<Value> {
        let n = r.i32()?;
        if n < 0 {
            return Err(Error::malformed(format!("negative array count {n}")));
        }
        let n = n as usize;
        let inner = tag.inner_type.as_str();
        if n == 0 {
            let rest = r.remaining();
            r.skip(rest)?; // an (unused) inner tag header may follow an empty struct array
            return Ok(Value::Array(vec![]));
        }
        if n > r.remaining() + 1 {
            return Err(Error::malformed(format!("array count {n} exceeds payload {}", r.remaining())));
        }
        let mut out = Vec::with_capacity(n);
        match inner {
            "StructProperty" => {
                let itag = self.read_tag(r)?.ok_or_else(|| Error::malformed("array inner tag is None"))?;
                if itag.ty != "StructProperty" {
                    return Err(Error::malformed(format!("array inner tag type {}", itag.ty)));
                }
                let payload = itag.size;
                if payload > r.remaining() {
                    return Err(Error::malformed("array struct payload larger than property"));
                }
                let start = r.p;
                let stride = if payload % n == 0 { Some(payload / n) } else { None };
                let mut ok = true;
                let mut err: Option<Error> = None;
                let fixed = binary_struct_size(&itag.struct_name).is_some() || crate::native::is_native_struct(&itag.struct_name);
                for _ in 0..n {
                    let res = if fixed && stride.is_some() {
                        self.read_struct(&itag.struct_name, r, stride)
                    } else {
                        self.read_struct(&itag.struct_name, r, None)
                    };
                    match res {
                        Ok(v) => out.push(v),
                        Err(e) => {
                            ok = false;
                            err = Some(e);
                            break;
                        }
                    }
                }
                if ok && r.p - start != payload {
                    ok = false;
                    err = Some(Error::malformed(format!("struct array consumed {} of {} payload bytes", r.p - start, payload)));
                }
                if !ok {
                    // fallback: equal-size opaque elements
                    r.p = start;
                    out.clear();
                    match stride {
                        Some(s) if s > 0 => {
                            for _ in 0..n {
                                let b = r.bytes(s)?;
                                out.push(Value::Unparsed { ty: format!("Struct<{}>", itag.struct_name), bytes: b.to_vec(), reason: err.as_ref().map(|e| e.to_string()).unwrap_or_default() });
                            }
                        }
                        _ => return Err(err.unwrap_or_else(|| Error::malformed("struct array unreadable"))),
                    }
                }
            }
            "ByteProperty" | "EnumProperty" => {
                let rem = r.remaining();
                if rem % n != 0 {
                    return Err(Error::malformed(format!("byte/enum array payload {rem} not divisible by count {n}")));
                }
                let stride = rem / n;
                let mode = match stride {
                    1 => ByteMode::U8,
                    4 => ByteMode::I32,
                    8 => ByteMode::Name,
                    s => return Err(Error::unsupported(format!("{inner} array element stride {s}"))),
                };
                let t = Tag { enum_name: tag.enum_name.clone(), ..Default::default() };
                for _ in 0..n {
                    out.push(self.read_value(inner, &t, r, true, mode, Some(stride))?);
                }
            }
            _ => {
                let t = Tag { ty: inner.to_string(), ..Default::default() };
                for _ in 0..n {
                    out.push(self.read_value(inner, &t, r, true, ByteMode::Name, None)?);
                }
            }
        }
        if r.remaining() != 0 {
            return Err(Error::malformed(format!("array<{inner}> left {} unread byte(s)", r.remaining())));
        }
        Ok(Value::Array(out))
    }

    fn read_elem(&self, ty: &str, struct_hint: &str, r: &mut Reader<'a>, bm: ByteMode) -> Result<Value> {
        let t = Tag { ty: ty.to_string(), struct_name: struct_hint.to_string(), ..Default::default() };
        if ty == "StructProperty" {
            return self.read_struct(struct_hint, r, None);
        }
        self.read_value(ty, &t, r, true, bm, None)
    }

    fn read_set(&self, tag: &Tag, r: &mut Reader<'a>) -> Result<Value> {
        let modes: &[ByteMode] = if matches!(tag.inner_type.as_str(), "ByteProperty" | "EnumProperty") { &[ByteMode::Name, ByteMode::U8, ByteMode::I32] } else { &[ByteMode::Name] };
        let start = r.p;
        let mut last_err = Error::malformed("set unreadable");
        for &bm in modes {
            r.p = start;
            let res: Result<Vec<Value>> = (|| {
                let removed = r.count(1)?;
                for _ in 0..removed {
                    self.read_elem(&tag.inner_type, "", r, bm)?;
                }
                let n = r.count(1)?;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    out.push(self.read_elem(&tag.inner_type, "", r, bm)?);
                }
                Ok(out)
            })();
            match res {
                Ok(v) if r.remaining() == 0 => return Ok(Value::Set(v)),
                Ok(_) => last_err = Error::malformed("set left unread bytes"),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    fn read_map(&self, tag: &Tag, r: &mut Reader<'a>) -> Result<Value> {
        let amb = |t: &str| matches!(t, "ByteProperty" | "EnumProperty");
        let modes: &[ByteMode] = if amb(&tag.inner_type) || amb(&tag.value_type) { &[ByteMode::Name, ByteMode::U8, ByteMode::I32] } else { &[ByteMode::Name] };
        let start = r.p;
        let mut last_err = Error::malformed("map unreadable");
        for &bm in modes {
            r.p = start;
            let res: Result<Vec<(Value, Value)>> = (|| {
                let removed = r.count(1)?;
                for _ in 0..removed {
                    self.read_elem(&tag.inner_type, "", r, bm)?;
                }
                let n = r.count(1)?;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    let k = self.read_elem(&tag.inner_type, "", r, bm)?;
                    let v = self.read_elem(&tag.value_type, "", r, bm)?;
                    out.push((k, v));
                }
                Ok(out)
            })();
            match res {
                Ok(v) if r.remaining() == 0 => return Ok(Value::Map(v)),
                Ok(_) => last_err = Error::malformed("map left unread bytes"),
                Err(e) => last_err = e,
            }
        }
        // Struct values whose type is not named in the tag (needs reflection): split entries by stride and keep
        // each value as raw bytes so the map structure stays usable.
        if tag.value_type == "StructProperty" {
            let cands: Vec<(usize, ByteMode)> = match fixed_width(&tag.inner_type) {
                Some(w) => vec![(w, ByteMode::Name)],
                None if amb(&tag.inner_type) => vec![(8, ByteMode::Name), (1, ByteMode::U8), (4, ByteMode::I32)],
                None => vec![],
            };
            for (kw, bm) in cands {
                r.p = start;
                let res: Result<Vec<(Value, Value)>> = (|| {
                    if r.i32()? != 0 {
                        return Err(Error::unsupported("map with removals"));
                    }
                    let n = r.count(1)?;
                    if n == 0 || r.remaining() % n != 0 || r.remaining() / n < kw {
                        return Err(Error::malformed("map entry stride"));
                    }
                    let vw = r.remaining() / n - kw;
                    let mut out = Vec::with_capacity(n);
                    for _ in 0..n {
                        let k = self.read_elem(&tag.inner_type, "", r, bm)?;
                        let b = r.bytes(vw)?;
                        out.push((k, Value::Unparsed { ty: "Struct<?>".into(), bytes: b.to_vec(), reason: "struct type not named in map tag (needs class reflection)".into() }));
                    }
                    Ok(out)
                })();
                if let Ok(v) = res {
                    return Ok(Value::Map(v));
                }
            }
        }
        Err(last_err)
    }
}








