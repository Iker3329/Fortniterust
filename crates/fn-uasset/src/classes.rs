//! Decoding of the *native* payload of reflection exports: `UClass` (and Blueprint/Widget/AnimBlueprint generated
//! classes), `UFunction`, `UProperty` subclasses, `UEnum`, `UScriptStruct`.
//!
//! Layout facts (UE 4.16, cooked, verified against Fortnite 1.8). After the tagged-property `None` and the 4-byte
//! `bHasGuid` flag (both consumed by [`Package::read_export`]) the payload is:
//!
//! ```text
//! UField   : Next                     i32 PackageIndex
//! UStruct  : SuperStruct, Children    2 x PackageIndex   (Children = first property/function export of the chain)
//!            ScriptBytecodeSize       i32 (in-memory size, informational)
//!            ScriptStorageSize        i32, then that many script bytes
//! UClass   : FuncMap                  i32 n, n x { FName, PackageIndex }
//!            ClassFlags u32, ClassWithin PackageIndex, ClassConfigName FName
//!            ClassGeneratedBy PackageIndex (null in cooked data)
//!            Interfaces               i32 n, n x { Class PackageIndex, PointerOffset i32, bImplementedByK2 bool32 }
//!            bDeprecatedForceScriptOrder bool32, Dummy FName (None),
//!            bCooked bool32, ClassDefaultObject PackageIndex
//! UFunction: (UStruct) FunctionFlags u32, [RepOffset u16 if FUNC_Net], EventGraphFunction PackageIndex, EventGraphCallOffset i32
//! UEnum    : Next, Names (i32 n, n x { FName, i64 value }), CppForm u8
//! DataTable/CurveTable: i32 n, n x { RowName FName, tagged row struct }   (see [`read_table`])
//! UProperty: Next, ArrayDim i32, PropertyFlags u64, RepNotifyFunc FName, then per class:
//!            Object/Weak/Lazy/Asset/Soft: PropertyClass | Class: PropertyClass, MetaClass | Interface: InterfaceClass |
//!            Struct: Struct | Byte: Enum | Enum: UnderlyingProp, Enum | Bool: 2 raw bytes | Array: Inner |
//!            Set: ElementProp | Map: KeyProp, ValueProp | (Multicast)Delegate: SignatureFunction
//! ```

use crate::error::{Error, Result};
use crate::object::ExportObject;
use crate::package::{Package, PackageIndex};
use crate::reader::Reader;
use serde_json::{json, Value as Json};

pub const FUNC_NET: u32 = 0x40;

#[derive(Clone, Debug)]
pub struct StructBase<'a> {
    pub next: PackageIndex,
    pub super_struct: PackageIndex,
    /// First child (property or function export); in 1.8 follow each child's `next` for the chain.
    pub children: PackageIndex,
    /// 1.10 / >= 4.18 layout serializes the children as an explicit array (no `UField::Next`); empty for 1.8.
    pub child_list: Vec<PackageIndex>,
    pub script_bytecode_size: i32,
    pub script: &'a [u8],
}

#[derive(Clone, Debug)]
pub struct ImplementedInterface {
    pub class: PackageIndex,
    pub pointer_offset: i32,
    pub implemented_by_k2: bool,
}

#[derive(Clone, Debug)]
pub struct ClassData<'a> {
    pub base: StructBase<'a>,
    pub func_map: Vec<(String, PackageIndex)>,
    pub class_flags: u32,
    pub class_within: PackageIndex,
    pub config_name: String,
    pub interfaces: Vec<ImplementedInterface>,
    pub class_generated_by: PackageIndex,
    pub cooked: bool,
    pub default_object: PackageIndex,
    /// Bytes after `ClassDefaultObject` (native subclass data; empty for plain classes).
    pub extra: &'a [u8],
}

#[derive(Clone, Debug)]
pub struct FunctionData<'a> {
    pub next: PackageIndex,
    pub super_struct: PackageIndex,
    pub children: PackageIndex,
    pub script_bytecode_size: i32,
    pub script: &'a [u8],
    pub function_flags: u32,
    pub rep_offset: Option<u16>,
    pub event_graph_function: PackageIndex,
    pub event_graph_call_offset: i32,
}

#[derive(Clone, Debug)]
pub struct PropertyDecl {
    pub class: String,
    pub name: String,
    pub next: PackageIndex,
    pub array_dim: i32,
    pub property_flags: u64,
    pub rep_notify_func: String,
    /// Type-specific references, labelled (`PropertyClass`, `Struct`, `Inner`, `KeyProp`, ...).
    pub refs: Vec<(&'static str, PackageIndex)>,
    /// `BoolProperty` only: the two raw bytes.
    pub bool_bytes: Option<[u8; 2]>,
}

#[derive(Clone, Debug)]
pub struct EnumData {
    /// `(enumerator name incl. `EFoo::` prefix, value)`
    pub names: Vec<(String, i64)>,
    pub cpp_form: Option<u8>,
    pub remaining: usize,
}

#[derive(Clone, Debug)]
pub struct ScriptStructData<'a> {
    pub base: StructBase<'a>,
    pub struct_flags: u32,
    pub extra: &'a [u8],
}

fn idx(r: &mut Reader) -> Result<PackageIndex> {
    Ok(PackageIndex(r.i32()?))
}

fn read_fname(pkg: &Package, r: &mut Reader) -> Result<String> {
    let i = r.i32()?;
    let n = r.i32()?;
    pkg.fname(i, n)
}

/// `UField::Next` is serialized in 1.8 (UE 4.16) data but not in 1.10 (>= 4.18 layout, `ParseOptions::assumed_ue4_version >= 514`).
fn read_next(pkg: &Package, r: &mut Reader) -> Result<PackageIndex> {
    if pkg.version() >= crate::package::ver::ADDED_SOFT_OBJECT_PATH {
        Ok(PackageIndex(0))
    } else {
        idx(r)
    }
}

fn read_struct_base<'a>(pkg: &Package, r: &mut Reader<'a>) -> Result<StructBase<'a>> {
    let next = read_next(pkg, r)?;
    let super_struct = idx(r)?;
    let mut child_list = Vec::new();
    let children = if pkg.version() >= crate::package::ver::ADDED_SOFT_OBJECT_PATH {
        let n = r.count(4)?;
        for _ in 0..n {
            child_list.push(idx(r)?);
        }
        child_list.first().copied().unwrap_or_default()
    } else {
        idx(r)?
    };
    let script_bytecode_size = r.i32()?;
    let storage = r.i32()?;
    if storage < 0 {
        return Err(Error::malformed("negative script storage size"));
    }
    let script = r.bytes(storage as usize)?;
    Ok(StructBase { next, super_struct, children, child_list, script_bytecode_size, script })
}

fn is_class_like(class: &str) -> bool {
    class == "Class" || class.ends_with("GeneratedClass")
}

/// Decode a class export (`Class`, `BlueprintGeneratedClass`, `WidgetBlueprintGeneratedClass`,
/// `AnimBlueprintGeneratedClass`, ...).
pub fn read_class<'a>(pkg: &Package<'a>, i: usize) -> Result<ClassData<'a>> {
    let obj = pkg.read_export(i)?;
    if let Some(e) = &obj.error {
        return Err(e.clone());
    }
    class_from_object(pkg, &obj)
}

fn class_from_object<'a>(pkg: &Package<'a>, obj: &ExportObject<'a>) -> Result<ClassData<'a>> {
    let mut r = Reader::new(obj.trailing);
    let base = read_struct_base(pkg, &mut r)?;
    let n = r.count(12)?;
    let mut func_map = Vec::with_capacity(n);
    for _ in 0..n {
        let name = read_fname(pkg, &mut r)?;
        func_map.push((name, idx(&mut r)?));
    }
    let class_flags = r.u32()?;
    let class_within = idx(&mut r)?;
    let config_name = read_fname(pkg, &mut r)?;
    let class_generated_by = idx(&mut r)?;
    let n = r.count(12)?;
    let mut interfaces = Vec::with_capacity(n);
    for _ in 0..n {
        interfaces.push(ImplementedInterface { class: idx(&mut r)?, pointer_offset: r.i32()?, implemented_by_k2: r.bool32()? });
    }
    let _deprecated_force_script_order = r.bool32()?;
    let _dummy = read_fname(pkg, &mut r)?;
    let cooked = r.bool32()?;
    let default_object = idx(&mut r)?;
    let extra = &obj.trailing[r.pos()..];
    Ok(ClassData { base, func_map, class_flags, class_within, config_name, interfaces, class_generated_by, cooked, default_object, extra })
}

/// Decode a `Function` / `DelegateFunction` export.
pub fn read_function<'a>(pkg: &Package<'a>, i: usize) -> Result<FunctionData<'a>> {
    let obj = pkg.read_export(i)?;
    if let Some(e) = &obj.error {
        return Err(e.clone());
    }
    function_from_object(pkg, &obj)
}

fn function_from_object<'a>(pkg: &Package, obj: &ExportObject<'a>) -> Result<FunctionData<'a>> {
    let mut r = Reader::new(obj.trailing);
    let b = read_struct_base(pkg, &mut r)?;
    let function_flags = r.u32()?;
    let rep_offset = if function_flags & FUNC_NET != 0 { Some(r.u16()?) } else { None };
    let event_graph_function = idx(&mut r)?;
    let event_graph_call_offset = r.i32()?;
    if r.remaining() != 0 {
        return Err(Error::malformed(format!("function: {} unread byte(s) after EventGraphCallOffset", r.remaining())));
    }
    Ok(FunctionData {
        next: b.next,
        super_struct: b.super_struct,
        children: b.children,
        script_bytecode_size: b.script_bytecode_size,
        script: b.script,
        function_flags,
        rep_offset,
        event_graph_function,
        event_graph_call_offset,
    })
}

/// Decode a `*Property` export (a Blueprint/UserDefinedStruct member declaration).
pub fn read_property_decl(pkg: &Package, i: usize) -> Result<PropertyDecl> {
    let obj = pkg.read_export(i)?;
    if let Some(e) = &obj.error {
        return Err(e.clone());
    }
    property_from_object(pkg, &obj)
}

fn property_from_object(pkg: &Package, obj: &ExportObject) -> Result<PropertyDecl> {
    let mut r = Reader::new(obj.trailing);
    let next = read_next(pkg, &mut r)?;
    let array_dim = r.i32()?;
    let property_flags = r.u64()?;
    let rep_notify_func = read_fname(pkg, &mut r)?;
    let mut refs: Vec<(&'static str, PackageIndex)> = Vec::new();
    let mut bool_bytes = None;
    match obj.class.as_str() {
        "ObjectProperty" | "WeakObjectProperty" | "LazyObjectProperty" | "AssetObjectProperty" | "SoftObjectProperty" => refs.push(("PropertyClass", idx(&mut r)?)),
        "ClassProperty" | "AssetClassProperty" | "SoftClassProperty" => {
            refs.push(("PropertyClass", idx(&mut r)?));
            refs.push(("MetaClass", idx(&mut r)?));
        }
        "InterfaceProperty" => refs.push(("InterfaceClass", idx(&mut r)?)),
        "StructProperty" => refs.push(("Struct", idx(&mut r)?)),
        "ByteProperty" => refs.push(("Enum", idx(&mut r)?)),
        "EnumProperty" => {
            refs.push(("UnderlyingProp", idx(&mut r)?));
            refs.push(("Enum", idx(&mut r)?));
        }
        "BoolProperty" => {
            let b = r.bytes(2)?;
            bool_bytes = Some([b[0], b[1]]);
        }
        "ArrayProperty" => refs.push(("Inner", idx(&mut r)?)),
        "SetProperty" => refs.push(("ElementProp", idx(&mut r)?)),
        "MapProperty" => {
            refs.push(("KeyProp", idx(&mut r)?));
            refs.push(("ValueProp", idx(&mut r)?));
        }
        "DelegateProperty" | "MulticastDelegateProperty" => refs.push(("SignatureFunction", idx(&mut r)?)),
        _ => {}
    }
    if r.remaining() != 0 {
        return Err(Error::malformed(format!("{}: {} unread byte(s)", obj.class, r.remaining())));
    }
    Ok(PropertyDecl { class: obj.class.clone(), name: obj.name.clone(), next, array_dim, property_flags, rep_notify_func, refs, bool_bytes })
}

/// Decode an `Enum` / `UserDefinedEnum` export.
pub fn read_enum(pkg: &Package, i: usize) -> Result<EnumData> {
    let obj = pkg.read_export(i)?;
    if let Some(e) = &obj.error {
        return Err(e.clone());
    }
    enum_from_object(pkg, &obj)
}

fn enum_from_object(pkg: &Package, obj: &ExportObject) -> Result<EnumData> {
    let mut r = Reader::new(obj.trailing);
    let _next = read_next(pkg, &mut r)?;
    let n = r.count(16)?;
    let mut names = Vec::with_capacity(n);
    for _ in 0..n {
        let name = read_fname(pkg, &mut r)?;
        let v = r.i64()?;
        names.push((name, v));
    }
    let cpp_form = if r.remaining() >= 1 { Some(r.u8()?) } else { None };
    Ok(EnumData { names, cpp_form, remaining: r.remaining() })
}

/// Decode a `ScriptStruct` / `UserDefinedStruct` export header (UStruct part + flags).
pub fn read_script_struct<'a>(pkg: &Package<'a>, i: usize) -> Result<ScriptStructData<'a>> {
    let obj = pkg.read_export(i)?;
    if let Some(e) = &obj.error {
        return Err(e.clone());
    }
    let mut r = Reader::new(obj.trailing);
    let base = read_struct_base(pkg, &mut r)?;
    let struct_flags = r.u32()?;
    let extra = &obj.trailing[r.pos()..];
    Ok(ScriptStructData { base, struct_flags, extra })
}

/// Rows of a `DataTable` / `CurveTable` export: `i32 n`, then `n x { RowName FName, tagged property list of the row
/// struct (a `FRichCurve` for curve tables) }`.
#[derive(Clone, Debug)]
pub struct TableData {
    /// `RowStruct` of a DataTable (null for curve tables).
    pub row_struct: PackageIndex,
    pub rows: Vec<(String, crate::value::PropertyList)>,
}

/// Decode a `DataTable` or `CurveTable` export.
pub fn read_table(pkg: &Package, i: usize) -> Result<TableData> {
    let obj = pkg.read_export(i)?;
    if let Some(e) = &obj.error {
        return Err(e.clone());
    }
    table_from_object(pkg, &obj)
}

fn table_from_object<'a>(pkg: &Package<'a>, obj: &ExportObject<'a>) -> Result<TableData> {
    let row_struct = obj.props.get("RowStruct").and_then(|v| v.as_object()).unwrap_or_default();
    let mut r = Reader::new(obj.trailing);
    let n = r.count(8)?;
    let mut rows = Vec::with_capacity(n);
    for _ in 0..n {
        let name = read_fname(pkg, &mut r)?;
        let mut props = crate::value::PropertyList::new();
        pkg.read_tagged_into(&mut r, &mut props)?;
        rows.push((name, props));
    }
    if r.remaining() != 0 {
        return Err(Error::malformed(format!("table: {} unread byte(s) after the last row", r.remaining())));
    }
    Ok(TableData { row_struct, rows })
}

/// Used by sweeps: decode the export according to its class and verify the layout is consumed exactly.
/// Classes without a native decoder here are accepted (their payload stays raw in `ExportObject::trailing`).
pub fn check_export(pkg: &Package, i: usize, obj: &ExportObject) -> Result<()> {
    let class = obj.class.as_str();
    if is_class_like(class) {
        let c = class_from_object(pkg, obj)?;
        if !c.extra.is_empty() {
            return Err(Error::malformed(format!("class: {} byte(s) of extra native data", c.extra.len())));
        }
        let _ = i;
        return Ok(());
    }
    match class {
        "Function" | "DelegateFunction" => {
            let f = function_from_object(pkg, obj)?;
            let d = crate::kismet::disassemble(pkg, f.script);
            if let Some(e) = d.error {
                return Err(Error::malformed(format!("kismet: {e}")));
            }
            if !f.script.is_empty() && (!d.reached_end || d.end != f.script.len()) {
                return Err(Error::malformed(format!("kismet: script ends at {} of {} (reached END: {})", d.end, f.script.len(), d.reached_end)));
            }
            Ok(())
        }
        "BoolProperty" | "ByteProperty" | "IntProperty" | "UInt32Property" | "Int64Property" | "UInt64Property" | "Int16Property" | "UInt16Property" | "Int8Property" | "FloatProperty"
        | "DoubleProperty" | "NameProperty" | "StrProperty" | "TextProperty" | "ObjectProperty" | "WeakObjectProperty" | "LazyObjectProperty" | "AssetObjectProperty" | "SoftObjectProperty"
        | "ClassProperty" | "AssetClassProperty" | "SoftClassProperty" | "InterfaceProperty" | "StructProperty" | "EnumProperty" | "ArrayProperty" | "SetProperty" | "MapProperty"
        | "DelegateProperty" | "MulticastDelegateProperty" => property_from_object(pkg, obj).map(|_| ()),
        "DataTable" | "CurveTable" => table_from_object(pkg, obj).map(|_| ()),
        "Enum" | "UserDefinedEnum" => {
            let e = enum_from_object(pkg, obj)?;
            if e.remaining != 0 {
                return Err(Error::malformed(format!("enum: {} unread byte(s)", e.remaining)));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// JSON summary of every reflection export in the package (classes, functions with disassembly, properties, enums).
pub fn package_classes_json(pkg: &Package) -> Json {
    let mut out = Vec::new();
    for i in 0..pkg.exports.len() {
        let class = pkg.export_class_name(i);
        let name = &pkg.exports[i].name;
        let j = if is_class_like(&class) {
            match read_class(pkg, i) {
                Ok(c) => json!({
                    "index": i, "kind": "class", "name": name, "class": class,
                    "super": pkg.object_path(c.base.super_struct), "children": pkg.object_path(c.base.children),
                    "classFlags": c.class_flags, "within": pkg.object_path(c.class_within), "config": c.config_name,
                    "defaultObject": pkg.object_path(c.default_object), "cooked": c.cooked,
                    "funcMap": c.func_map.iter().map(|(n, p)| json!({"name": n, "function": pkg.object_path(*p)})).collect::<Vec<_>>(),
                    "interfaces": c.interfaces.iter().map(|x| json!({"class": pkg.object_path(x.class), "k2": x.implemented_by_k2})).collect::<Vec<_>>(),
                    "scriptBytes": c.base.script.len(), "extraBytes": c.extra.len(),
                }),
                Err(e) => json!({"index": i, "kind": "class", "name": name, "error": e.to_string()}),
            }
        } else if class == "Function" || class == "DelegateFunction" {
            match read_function(pkg, i) {
                Ok(f) => {
                    let d = crate::kismet::disassemble(pkg, f.script);
                    json!({
                        "index": i, "kind": "function", "name": name, "functionFlags": f.function_flags,
                        "super": pkg.object_path(f.super_struct), "children": pkg.object_path(f.children),
                        "eventGraphFunction": pkg.object_path(f.event_graph_function), "eventGraphCallOffset": f.event_graph_call_offset,
                        "scriptBytes": f.script.len(), "disassembly": d.lines.iter().map(|(o, t)| format!("{o:x}: {t}")).collect::<Vec<_>>(),
                        "disassemblyError": d.error,
                    })
                }
                Err(e) => json!({"index": i, "kind": "function", "name": name, "error": e.to_string()}),
            }
        } else if class.ends_with("Property") {
            match read_property_decl(pkg, i) {
                Ok(p) => json!({
                    "index": i, "kind": "property", "class": class, "name": name, "flags": format!("{:#x}", p.property_flags), "arrayDim": p.array_dim,
                    "refs": p.refs.iter().map(|(k, v)| json!({"role": k, "object": pkg.object_path(*v)})).collect::<Vec<_>>(),
                }),
                Err(e) => json!({"index": i, "kind": "property", "name": name, "error": e.to_string()}),
            }
        } else if class == "DataTable" || class == "CurveTable" {
            match read_table(pkg, i) {
                Ok(t) => json!({"index": i, "kind": "table", "class": class, "name": name, "rowStruct": pkg.object_path(t.row_struct),
                                "rows": t.rows.iter().map(|(n, p)| json!({"name": n, "values": p.to_json(pkg)})).collect::<Vec<_>>()}),
                Err(e) => json!({"index": i, "kind": "table", "name": name, "error": e.to_string()}),
            }
        } else if class == "Enum" || class == "UserDefinedEnum" {
            match read_enum(pkg, i) {
                Ok(e) => json!({"index": i, "kind": "enum", "name": name, "names": e.names.iter().map(|(n, v)| json!([n, v])).collect::<Vec<_>>()}),
                Err(e) => json!({"index": i, "kind": "enum", "name": name, "error": e.to_string()}),
            }
        } else {
            continue;
        };
        out.push(j);
    }
    Json::Array(out)
}





