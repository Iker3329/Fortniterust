//! Offline tests: a tiny cooked-style package (unversioned, legacy -7, `.uasset` + `.uexp`) is assembled in memory
//! and read back, covering the header, every common property type, nested/array/map structs, class defaults and
//! Kismet bytecode. No game install is needed.

use fn_uasset::kismet;
use fn_uasset::{Package, Value};
use serde_json::json;

// ---------------------------------------------------------------------------------------------
// builder
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct B {
    names: Vec<String>,
}

impl B {
    fn n(&mut self, s: &str) -> i32 {
        if let Some(i) = self.names.iter().position(|x| x == s) {
            return i as i32;
        }
        self.names.push(s.to_string());
        (self.names.len() - 1) as i32
    }
    fn fname(&mut self, s: &str) -> Vec<u8> {
        let i = self.n(s);
        let mut v = i.to_le_bytes().to_vec();
        v.extend(0i32.to_le_bytes());
        v
    }
    /// tag header (+type specific extras) followed by the value bytes
    fn tag(&mut self, name: &str, ty: &str, extra: Vec<u8>, value: Vec<u8>) -> Vec<u8> {
        let mut v = self.fname(name);
        v.extend(self.fname(ty));
        v.extend((value.len() as i32).to_le_bytes());
        v.extend(0i32.to_le_bytes());
        v.extend(extra);
        v.push(0); // no property guid
        v.extend(value);
        v
    }
    fn none(&mut self) -> Vec<u8> {
        self.fname("None")
    }
}

fn fstr(s: &str) -> Vec<u8> {
    let mut v = ((s.len() + 1) as i32).to_le_bytes().to_vec();
    v.extend(s.as_bytes());
    v.push(0);
    v
}
fn f32s(x: &[f32]) -> Vec<u8> {
    x.iter().flat_map(|f| f.to_le_bytes()).collect()
}

struct Built {
    asset: Vec<u8>,
    uexp: Vec<u8>,
}

/// One import (a native class) + exports: [0] a UObject with many properties, [1] a Function with a script.
fn build() -> Built {
    let mut b = B::default();
    b.n("None");
    // ---- export 0 payload (tagged props)
    let mut p0: Vec<u8> = Vec::new();
    p0.extend(b.tag("Count", "IntProperty", vec![], 7i32.to_le_bytes().to_vec()));
    p0.extend(b.tag("Speed", "FloatProperty", vec![], 1.5f32.to_le_bytes().to_vec()));
    {
        // BoolProperty stores the value in the tag, size 0
        let mut v = b.fname("bFlag");
        v.extend(b.fname("BoolProperty"));
        v.extend(0i32.to_le_bytes());
        v.extend(0i32.to_le_bytes());
        v.push(1); // value
        v.push(0); // no guid
        p0.extend(v);
    }
    let tagname = b.fname("Hello");
    p0.extend(b.tag("Tag", "NameProperty", vec![], tagname));
    p0.extend(b.tag("Label", "StrProperty", vec![], fstr("Hi there")));
    let sname = b.fname("Vector");
    let mut ex = sname.clone();
    ex.extend([0u8; 16]);
    p0.extend(b.tag("Offset", "StructProperty", ex, f32s(&[1.0, 2.0, 3.0])));
    let mut arr = 3i32.to_le_bytes().to_vec();
    for i in [1i32, 2, 3] {
        arr.extend(i.to_le_bytes());
    }
    let inner = b.fname("IntProperty");
    p0.extend(b.tag("Nums", "ArrayProperty", inner, arr));
    let en = b.fname("EMode");
    let val = b.fname("EMode::B");
    p0.extend(b.tag("Mode", "ByteProperty", en, val));
    p0.extend(b.tag("Ref", "ObjectProperty", vec![], (-1i32).to_le_bytes().to_vec()));
    // text: flags, history 0 (base), namespace, key, source
    let mut txt = 0u32.to_le_bytes().to_vec();
    txt.push(0);
    txt.extend(fstr("NS"));
    txt.extend(fstr("KEY"));
    txt.extend(fstr("Source text"));
    p0.extend(b.tag("Title", "TextProperty", vec![], txt));
    // tagged struct "Inner" { Level: int }
    let mut inner_props = b.tag("Level", "IntProperty", vec![], 3i32.to_le_bytes().to_vec());
    inner_props.extend(b.none());
    let mut ex = b.fname("InnerStruct");
    ex.extend([0u8; 16]);
    p0.extend(b.tag("Inner", "StructProperty", ex, inner_props.clone()));
    // array of tagged structs: count, inner tag header (name,type,size,idx,structname,guid,hasguid), elements
    {
        let mut elems = Vec::new();
        for lvl in [10i32, 20] {
            elems.extend(b.tag("Level", "IntProperty", vec![], lvl.to_le_bytes().to_vec()));
            elems.extend(b.none());
        }
        let mut payload = 2i32.to_le_bytes().to_vec();
        let mut h = b.fname("Items");
        h.extend(b.fname("StructProperty"));
        h.extend((elems.len() as i32).to_le_bytes());
        h.extend(0i32.to_le_bytes());
        h.extend(b.fname("InnerStruct"));
        h.extend([0u8; 16]);
        h.push(0);
        payload.extend(h);
        payload.extend(elems);
        let inner = b.fname("StructProperty");
        p0.extend(b.tag("Items", "ArrayProperty", inner, payload));
    }
    // map<Name,Int>: removed=0, count=2, (key,value)*
    {
        let mut m = 0i32.to_le_bytes().to_vec();
        m.extend(2i32.to_le_bytes());
        for (k, v) in [("A", 1i32), ("B", 2)] {
            m.extend(b.fname(k));
            m.extend(v.to_le_bytes());
        }
        let mut ex = b.fname("NameProperty");
        ex.extend(b.fname("IntProperty"));
        p0.extend(b.tag("Lookup", "MapProperty", ex, m));
    }
    // unknown property type is kept raw
    p0.extend(b.tag("Weird", "FuturePropertyType", vec![], vec![1, 2, 3, 4, 5]));
    p0.extend(b.none());
    p0.extend(0i32.to_le_bytes()); // bHasGuid = 0

    // ---- export 1: Function. payload = None + guid flag + Next, Super, Children, size, storage, script, flags, graph fn, offset
    let call = b.fname("Foo"); // EX_VirtualFunction name
    let mut script: Vec<u8> = Vec::new();
    script.push(0x1B); // vcall Foo(int 5)
    script.extend(call);
    script.push(0x1D);
    script.extend(5i32.to_le_bytes());
    script.push(0x16); // EndFunctionParms
    script.push(0x0F); // let[prop] V:x = float 0.5
    script.extend(1i32.to_le_bytes()); // property ptr -> export 0
    script.push(0x01);
    script.extend(1i32.to_le_bytes());
    script.push(0x1E);
    script.extend(0.5f32.to_le_bytes());
    script.push(0x07); // jumpIfNot(true) -> 77
    script.extend(77u32.to_le_bytes());
    script.push(0x27);
    script.push(0x31); // setarray[L:x](int 1, int 2)
    script.push(0x00);
    script.extend((-1i32).to_le_bytes());
    script.push(0x25);
    script.push(0x26);
    script.push(0x32);
    script.push(0x04); // return nop
    script.push(0x0B);
    script.push(0x53); // END
    let mut p1 = b.none();
    p1.extend(0i32.to_le_bytes()); // guid flag
    p1.extend(0i32.to_le_bytes()); // next
    p1.extend(0i32.to_le_bytes()); // super
    p1.extend(0i32.to_le_bytes()); // children
    p1.extend((script.len() as i32).to_le_bytes());
    p1.extend((script.len() as i32).to_le_bytes());
    p1.extend(&script);
    p1.extend(0x0802_0800u32.to_le_bytes()); // flags
    p1.extend(0i32.to_le_bytes());
    p1.extend(0i32.to_le_bytes());

    // ---- header
    let class_pkg = b.n("/Script/CoreUObject");
    let class_name = b.n("Class");
    let obj_name = b.n("Object");
    let pkg_name = b.n("/Script/Engine");
    let mypkg = b.n("MyObject");
    let myfn = b.n("MyFunction");
    let class_function = b.n("Function");
    let _ = (class_pkg, obj_name);
    // imports: [0] package /Script/Engine, [1] class Class (outer pkg)
    let mut imports = Vec::new();
    for (cp, cn, outer, nm) in [(class_pkg, class_name, 0i32, pkg_name), (class_pkg, class_name, -1, class_name)] {
        for x in [cp, 0, cn, 0] {
            imports.extend(x.to_le_bytes());
        }
        imports.extend(outer.to_le_bytes());
        imports.extend(nm.to_le_bytes());
        imports.extend(0i32.to_le_bytes());
    }
    let _ = class_function;

    // names section
    let mut names_sec = Vec::new();
    for n in &b.names {
        names_sec.extend(fstr(n));
        names_sec.extend([0u8; 4]);
    }
    let header_fixed = 193usize;
    let names_off = header_fixed;
    let imports_off = names_off + names_sec.len();
    let exports_off = imports_off + imports.len();
    let exports_len = 104 * 2;
    let depends_off = exports_off + exports_len;
    let total = depends_off + 8;

    let mut exports = Vec::new();
    let mut off = total as i64;
    for (idx, (class_index, nm, flags, payload)) in [(-2i32, mypkg, 0x0000_0008u32, &p0), (-2, myfn, 0x0000_0009, &p1)].iter().enumerate() {
        let _ = idx;
        exports.extend(class_index.to_le_bytes());
        exports.extend(0i32.to_le_bytes()); // super
        exports.extend(0i32.to_le_bytes()); // template
        exports.extend(0i32.to_le_bytes()); // outer
        exports.extend(nm.to_le_bytes());
        exports.extend(0i32.to_le_bytes());
        exports.extend(flags.to_le_bytes());
        exports.extend((payload.len() as i64).to_le_bytes());
        exports.extend(off.to_le_bytes());
        exports.extend([0u8; 12]); // forced export / not for client / not for server
        exports.extend([0u8; 16]); // package guid
        exports.extend(0u32.to_le_bytes()); // package flags
        exports.extend(0i32.to_le_bytes()); // not always loaded for editor game
        exports.extend(1i32.to_le_bytes()); // is asset
        exports.extend((-1i32).to_le_bytes());
        exports.extend([0u8; 16]);
        off += payload.len() as i64;
    }
    assert_eq!(exports.len(), exports_len);

    let mut h: Vec<u8> = Vec::new();
    h.extend(0x9E2A83C1u32.to_le_bytes());
    h.extend((-7i32).to_le_bytes());
    h.extend(0i32.to_le_bytes()); // ue3
    h.extend(0i32.to_le_bytes()); // ue4 (unversioned)
    h.extend(0i32.to_le_bytes()); // licensee
    h.extend(0i32.to_le_bytes()); // custom versions
    h.extend((total as i32).to_le_bytes());
    h.extend(fstr("None"));
    h.extend(0x8000_0000u32.to_le_bytes());
    h.extend((b.names.len() as i32).to_le_bytes());
    h.extend((names_off as i32).to_le_bytes());
    h.extend([0u8; 8]); // gatherable text
    h.extend(2i32.to_le_bytes());
    h.extend((exports_off as i32).to_le_bytes());
    h.extend(2i32.to_le_bytes());
    h.extend((imports_off as i32).to_le_bytes());
    h.extend((depends_off as i32).to_le_bytes());
    h.extend([0u8; 8]); // string asset refs
    h.extend(0i32.to_le_bytes()); // searchable names
    h.extend(0i32.to_le_bytes()); // thumbnails
    h.extend([7u8; 16]); // guid
    h.extend(1i32.to_le_bytes());
    h.extend([0u8; 8]); // generation
    for _ in 0..2 {
        h.extend([0u8; 6]);
        h.extend(0u32.to_le_bytes());
        h.extend(fstr("").iter().copied().take(4)); // empty FString (len 0)
    }
    h.extend(0u32.to_le_bytes()); // compression flags
    h.extend(0i32.to_le_bytes()); // compressed chunks
    h.extend(0u32.to_le_bytes()); // package source
    h.extend(0i32.to_le_bytes()); // additional packages
    h.extend(0i32.to_le_bytes()); // asset registry offset
    h.extend(0i64.to_le_bytes()); // bulk data start
    h.extend(0i32.to_le_bytes()); // world tile info
    h.extend(0i32.to_le_bytes()); // chunk ids
    h.extend(0i32.to_le_bytes()); // preload deps count
    h.extend(0i32.to_le_bytes()); // preload deps offset
    assert_eq!(h.len(), header_fixed, "header size model");
    h.extend(&names_sec);
    h.extend(&imports);
    h.extend(&exports);
    h.extend([0u8; 8]); // depends: 2 x empty array
    assert_eq!(h.len(), total);

    let mut uexp = p0.clone();
    uexp.extend(&p1);
    uexp.extend(0x9E2A83C1u32.to_le_bytes());
    Built { asset: h, uexp }
}

// ---------------------------------------------------------------------------------------------

#[test]
fn header_and_names() {
    let built = build();
    let pkg = Package::parse(&built.asset, &built.uexp).unwrap();
    assert_eq!(pkg.summary.legacy_file_version, -7);
    assert_eq!(pkg.summary.file_version_ue4, 0);
    assert_eq!(pkg.effective_version, fn_uasset::package::DEFAULT_ASSUMED_UE4_VERSION);
    assert_eq!(pkg.exports.len(), 2);
    assert_eq!(pkg.imports.len(), 2);
    assert_eq!(pkg.exports[0].name, "MyObject");
    assert_eq!(pkg.object_path(fn_uasset::PackageIndex(-2)), "/Script/Engine.Class");
    assert_eq!(pkg.export_class_name(0), "Class");
    assert!(pkg.warnings.is_empty(), "{:?}", pkg.warnings);
    match pkg.resolve(fn_uasset::PackageIndex(-2)) {
        fn_uasset::ObjectRef::Import { package, name, .. } => {
            assert_eq!(package, "/Script/Engine");
            assert_eq!(name, "Class");
        }
        r => panic!("{r:?}"),
    }
    assert!(Package::parse(&built.asset[..40], &[]).is_err());
}

#[test]
fn property_values() {
    let built = build();
    let pkg = Package::parse(&built.asset, &built.uexp).unwrap();
    let o = pkg.read_export(0).unwrap();
    assert!(o.error.is_none(), "{:?}", o.error);
    assert!(o.had_guid_flag);
    assert_eq!(o.trailing.len(), 0);
    assert_eq!(o.get("Count"), Some(&Value::Int(7)));
    assert_eq!(o.get("Speed"), Some(&Value::Float(1.5)));
    assert_eq!(o.get("bFlag"), Some(&Value::Bool(true)));
    assert_eq!(o.get("Tag").and_then(|v| v.as_str()), Some("Hello"));
    assert_eq!(o.get("Label").and_then(|v| v.as_str()), Some("Hi there"));
    assert_eq!(o.get("Offset"), Some(&Value::Vector([1.0, 2.0, 3.0])));
    assert_eq!(o.get("Nums"), Some(&Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)])));
    assert_eq!(o.get("Mode").and_then(|v| v.as_str()), Some("EMode::B"));
    assert_eq!(o.get("Ref").and_then(|v| v.as_object()), Some(fn_uasset::PackageIndex(-1)));
    let Some(Value::Text(t)) = o.get("Title") else { panic!() };
    assert_eq!((t.namespace.as_deref(), t.key.as_deref(), t.source.as_deref()), (Some("NS"), Some("KEY"), Some("Source text")));
    assert_eq!(o.get("Inner").and_then(|v| v.field("Level")), Some(&Value::Int(3)));
    assert_eq!(o.get("Inner").and_then(|v| v.path("Level")).and_then(|v| v.as_i64()), Some(3));
    let items = o.get("Items").and_then(|v| v.as_array()).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].field("Level"), Some(&Value::Int(20)));
    let Some(Value::Map(m)) = o.get("Lookup") else { panic!() };
    assert_eq!(m.len(), 2);
    assert_eq!(m[1].0, Value::Name("B".into()));
    assert_eq!(m[1].1, Value::Int(2));
    // unknown type: safe fallback, reading continues past it
    assert!(matches!(o.get("Weird"), Some(Value::Unparsed { ty, bytes, .. }) if ty == "FuturePropertyType" && bytes == &vec![1, 2, 3, 4, 5]));
    assert_eq!(o.unparsed_count(), 1);

    // JSON export
    let j = o.to_json(&pkg);
    assert_eq!(j["properties"]["Count"], json!(7));
    assert_eq!(j["properties"]["Offset"], json!([1.0, 2.0, 3.0]));
    assert_eq!(j["properties"]["Inner"]["Level"], json!(3));
    assert_eq!(j["properties"]["Inner"]["_struct"], json!("InnerStruct"));
    assert_eq!(j["properties"]["Ref"], json!("/Script/Engine"));
    assert_eq!(j["properties"]["Lookup"][0], json!({"key": "A", "value": 1}));
    // byte offsets of tags are exposed (uasset ++ uexp)
    assert_eq!(o.props.iter().next().unwrap().offset as usize, built.asset.len());
}

#[test]
fn kismet_function() {
    let built = build();
    let pkg = Package::parse(&built.asset, &built.uexp).unwrap();
    let f = fn_uasset::classes::read_function(&pkg, 1).unwrap();
    assert_eq!(f.function_flags, 0x0802_0800);
    let d = kismet::disassemble(&pkg, f.script);
    assert!(d.error.is_none(), "{:?}", d.error);
    assert!(d.reached_end && d.end == f.script.len());
    let text: Vec<&str> = d.lines.iter().map(|l| l.1.as_str()).collect();
    assert_eq!(
        text,
        [
            "vcall Foo(int 5)",
            "let[MyObject] V:MyObject = f0.5",
            "jumpIfNot(true) -> 77",
            "setarray[L:/Script/Engine](0, 1)",
            "return(nop)",
            "END"
        ]
    );
    assert!(kismet::parse(&pkg, f.script).is_ok());
    // truncated scripts report an error instead of panicking
    let cut = &f.script[..f.script.len() - 5];
    let d2 = kismet::disassemble(&pkg, cut);
    assert!(!d2.reached_end);
    assert!(kismet::parse(&pkg, cut).is_err());
    // unknown opcode
    let d3 = kismet::disassemble(&pkg, &[0xFE]);
    assert!(d3.error.as_deref().unwrap().contains("opcode 0xfe"));
    // exports are classified for the sweep decoders
    let obj = pkg.read_export(1).unwrap();
    let _ = obj;
}

#[test]
fn bad_exports_do_not_panic() {
    let built = build();
    // corrupt the first export's first tag size so it runs past the end of the data
    let mut uexp = built.uexp.clone();
    uexp[16..20].copy_from_slice(&0x7fff_0000i32.to_le_bytes());
    let pkg = Package::parse(&built.asset, &uexp).unwrap();
    let o = pkg.read_export(0).unwrap();
    assert!(o.error.is_some());
    // out-of-range export index
    assert!(pkg.read_export(99).is_err());
}


/// Deterministic mutation fuzzing: no byte pattern may make the parser panic, over-allocate or hang.
#[test]
fn fuzz_mutations_never_panic() {
    let built = build();
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut parsed = 0;
    for iter in 0..30_000 {
        let mut asset = built.asset.clone();
        let mut uexp = built.uexp.clone();
        let flips = 1 + (rnd() % 6) as usize;
        for _ in 0..flips {
            let which = rnd() % 3;
            let target = if which == 0 { &mut asset } else { &mut uexp };
            let i = (rnd() as usize) % target.len();
            match rnd() % 4 {
                0 => target[i] = rnd() as u8,
                1 => target[i] ^= 1 << (rnd() % 8),
                2 => target[i] = 0xff,
                _ => {
                    // overwrite a whole i32 with an extreme value
                    let v = [0x7fff_ffffi32, -1, 0x4000_0000, i32::MIN][(rnd() % 4) as usize];
                    for (k, b) in v.to_le_bytes().iter().enumerate() {
                        if i + k < target.len() {
                            target[i + k] = *b;
                        }
                    }
                }
            }
        }
        if iter % 7 == 0 {
            let cut = (rnd() as usize) % uexp.len();
            uexp.truncate(cut.max(1));
        }
        let Ok(pkg) = Package::parse(&asset, &uexp) else { continue };
        parsed += 1;
        for i in 0..pkg.exports.len() {
            if let Ok(o) = pkg.read_export(i) {
                let _ = o.to_json(&pkg);
                let _ = fn_uasset::classes::check_export(&pkg, i, &o);
                let _ = fn_uasset::classes::read_class(&pkg, i);
                let _ = fn_uasset::classes::read_table(&pkg, i);
                if let Ok(f) = fn_uasset::classes::read_function(&pkg, i) {
                    let _ = kismet::disassemble(&pkg, f.script);
                }
            }
        }
        let _ = pkg.resolve(fn_uasset::PackageIndex(rnd() as i32));
        let _ = pkg.object_path(fn_uasset::PackageIndex(rnd() as i32));
    }
    assert!(parsed > 1000, "too few mutated packages still parsed ({parsed}); fuzz is not exercising the readers");
}

#[test]
fn fuzz_kismet_random_bytes() {
    let built = build();
    let pkg = Package::parse(&built.asset, &built.uexp).unwrap();
    let mut seed = 99u64;
    for _ in 0..50_000 {
        let n = 1 + (seed % 64) as usize;
        let bytes: Vec<u8> = (0..n)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        let _ = kismet::disassemble(&pkg, &bytes);
    }
}