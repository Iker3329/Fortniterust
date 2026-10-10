//! Layouts of structs that UE 4.16 serializes with a native `Serialize()` instead of tagged properties.
//!
//! Every layout here was derived from the engine source *and verified against the Fortnite 1.8 data*
//! (the sweep reports any value that does not consume exactly its byte size as `Unparsed`).
//! Structs not listed here (Sequencer evaluation templates, `FortActorRecord`, ...)
//! are kept as `Value::Unparsed` raw bytes.
//! (`ConnectivityCube` has a native reader since 2026-10-05, see [`read_connectivity_cube`].)

use crate::error::{Error, Result};
use crate::package::{ver, Package};
use crate::reader::Reader;
use crate::value::{Property, PropertyList, Value};

/// How a native struct's byte size behaves (used to decide whether array elements can be split by stride).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NativeLayout {
    /// Every value has the same byte size.
    Fixed,
    /// Size depends on the content (strings, counted arrays, nested tagged data): elements must be read in sequence.
    Variable,
}

/// Struct names with a native layout in [`Package::read_native_struct`].
pub fn native_layout(name: &str) -> Option<NativeLayout> {
    use NativeLayout::*;
    Some(match name {
        "GameplayTag" | "SmartName" | "NavAgentSelector" | "MovieSceneSequenceID" | "MovieSceneTrackIdentifier" | "MovieSceneEvaluationKey" | "FontData" | "FontCharacter" => Fixed,
        "GameplayTagContainer" | "UniqueNetIdRepl" | "MovieSceneEventParameters" | "ExpressionInput" | "ColorMaterialInput" | "ScalarMaterialInput" | "VectorMaterialInput"
        | "Vector2MaterialInput" | "MaterialAttributesInput" | "ShadingModelMaterialInput" | "MovieSceneSegment" | "MovieSceneEvalTemplatePtr" | "MovieSceneTrackImplementationPtr" => Variable,
        // FConnectivityCube (Fortnite, native struct ops): 6 TBitArrays, size depends on NumBits (see read_connectivity_cube)
        "ConnectivityCube" => Variable,
        _ => return None,
    })
}

/// Struct names with a native layout whose values all have the same byte size.
pub fn is_native_struct(name: &str) -> bool {
    native_layout(name) == Some(NativeLayout::Fixed)
}
/// `EFortConnectivityCubeFace` order of the 6 bit arrays of `FConnectivityCube` (Front 0, Left 1, Back 2, Right 3, Upper 4, Lower 5).
pub const CONNECTIVITY_CUBE_FACES: [&str; 6] = ["Front", "Left", "Back", "Right", "Upper", "Lower"];

/// `FConnectivityCube` (Fortnite 1.8, `UBuildingEditModeMetadata` +0x58, in memory 0xC0 = 6 x `TBitArray` of 0x20).
/// Registered with native struct ops (`StaticRegisterNativesFConnectivityCube` @1409b5ec0, `Z_Construct_UScriptStruct_FConnectivityCube`
/// @140d57dd0), so it is not tagged. Serialized form = 6 x UE4.16 `operator<<(FArchive&, TBitArray&)`: `int32 NumBits` followed by
/// `ceil(NumBits / 32)` `uint32` words (all 57 1.8 EMP assets: NumBits 25, one word, 48 bytes). Bit (row, col) of a face is
/// `(word >> (row * 5 + col)) & 1` (`FUN_1404b9a20`). Returns `(NumBits, words)` per face, or `None` when the bytes do not hold
/// exactly that layout.
pub fn read_connectivity_cube(bytes: &[u8]) -> Option<[(i32, Vec<u32>); 6]> {
    let mut r = Reader::new(bytes);
    let mut faces: [(i32, Vec<u32>); 6] = Default::default();
    for f in faces.iter_mut() {
        let n = r.i32().ok()?;
        if !(0..=1 << 20).contains(&n) { return None; }
        let words = ((n as usize) + 31) / 32;
        let mut w = Vec::with_capacity(words);
        for _ in 0..words { w.push(r.u32().ok()?); }
        *f = (n, w);
    }
    if r.remaining() != 0 { return None; }
    Some(faces)
}

fn p(name: &str, ty: &str, value: Value) -> Property {
    Property { name: name.to_string(), index: 0, ty: ty.to_string(), value, offset: 0 }
}
fn st(name: &str, props: Vec<Property>) -> Value {
    Value::Struct { name: name.to_string(), props: PropertyList(props) }
}

impl<'a> Package<'a> {
    fn fname_value(&self, r: &mut Reader<'a>) -> Result<String> {
        let i = r.i32()?;
        let n = r.i32()?;
        self.fname(i, n)
    }

    /// `FExpressionInput` (material pin) + its typed subclasses.
    /// Cooked layout: `OutputIndex i32, InputName FString, Mask i32, MaskR i32, MaskG i32, MaskB i32, MaskA i32,
    /// ExpressionName FName`, then per subclass `UseConstant u32` + the constant.
    fn read_expression_input(&self, name: &str, r: &mut Reader<'a>) -> Result<Value> {
        let mut props = vec![
            p("OutputIndex", "IntProperty", Value::Int(r.i32()? as i64)),
            p("InputName", "StrProperty", Value::Str(r.fstring()?)),
            p("Mask", "IntProperty", Value::Int(r.i32()? as i64)),
            p("MaskR", "IntProperty", Value::Int(r.i32()? as i64)),
            p("MaskG", "IntProperty", Value::Int(r.i32()? as i64)),
            p("MaskB", "IntProperty", Value::Int(r.i32()? as i64)),
            p("MaskA", "IntProperty", Value::Int(r.i32()? as i64)),
            p("ExpressionName", "NameProperty", Value::Name(self.fname_value(r)?)),
        ];
        match name {
            "ColorMaterialInput" => {
                props.push(p("UseConstant", "BoolProperty", Value::Bool(r.u32()? != 0)));
                let (b, g, rr, a) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
                props.push(p("Constant", "StructProperty", Value::Color([rr, g, b, a])));
            }
            "ScalarMaterialInput" => {
                props.push(p("UseConstant", "BoolProperty", Value::Bool(r.u32()? != 0)));
                props.push(p("Constant", "FloatProperty", Value::Float(r.f32()?)));
            }
            "VectorMaterialInput" => {
                props.push(p("UseConstant", "BoolProperty", Value::Bool(r.u32()? != 0)));
                props.push(p("Constant", "StructProperty", Value::Vector([r.f32()?, r.f32()?, r.f32()?])));
            }
            "Vector2MaterialInput" => {
                props.push(p("UseConstant", "BoolProperty", Value::Bool(r.u32()? != 0)));
                props.push(p("ConstantX", "FloatProperty", Value::Float(r.f32()?)));
                props.push(p("ConstantY", "FloatProperty", Value::Float(r.f32()?)));
            }
            _ => {}
        }
        Ok(st(name, props))
    }

    /// Returns `Ok(None)` when `name` has no native layout here (caller falls back to tagged/opaque).
    /// `size` is the exact value size when known; layouts that need it check it and return `Ok(None)` on mismatch.
    pub(crate) fn read_native_struct(&self, name: &str, r: &mut Reader<'a>, size: Option<usize>) -> Result<Option<Value>> {
        let fits = |n: usize| size.map(|s| s == n).unwrap_or(true);
        Ok(Some(match name {
            "GameplayTagContainer" => {
                if let Some(s) = size {
                    if s < 4 || (s - 4) % 8 != 0 {
                        return Ok(None);
                    }
                }
                let n = r.count(8)?;
                let mut tags = Vec::with_capacity(n);
                for _ in 0..n {
                    tags.push(Value::Name(self.fname_value(r)?));
                }
                st(name, vec![p("GameplayTags", "ArrayProperty", Value::Array(tags))])
            }
            "GameplayTag" if fits(8) => st(name, vec![p("TagName", "NameProperty", Value::Name(self.fname_value(r)?))]),
            "SmartName" if fits(10) => {
                let n = self.fname_value(r)?;
                let id = r.u16()?;
                st(name, vec![p("DisplayName", "NameProperty", Value::Name(n)), p("SmartNameID", "UInt16Property", Value::UInt(id as u64))])
            }
            "NavAgentSelector" if fits(4) => st(name, vec![p("PackedBits", "UInt32Property", Value::UInt(r.u32()? as u64))]),
            "MovieSceneTrackIdentifier" if fits(4) => st(name, vec![p("Value", "UInt32Property", Value::UInt(r.u32()? as u64))]),
            "MovieSceneEvaluationKey" if fits(12) => {
                let (seq, track, section) = (r.u32()?, r.u32()?, r.u32()?);
                st(
                    name,
                    vec![
                        p("SequenceID", "UInt32Property", Value::UInt(seq as u64)),
                        p("TrackIdentifier", "UInt32Property", Value::UInt(track as u64)),
                        p("SectionIndex", "UInt32Property", Value::UInt(section as u64)),
                    ],
                )
            }
            "MovieSceneSequenceID" if fits(4) => st(name, vec![p("Value", "UInt32Property", Value::UInt(r.u32()? as u64))]),
            "UniqueNetIdRepl" if size == Some(4) => {
                // empty (invalid) net id: serialised as a zero size
                if r.u32()? != 0 {
                    return Err(Error::unsupported("non-empty UniqueNetIdRepl"));
                }
                st(name, vec![])
            }
            "MovieSceneSegment" => {
                // TRange<float> (lower: u8 bound type + f32, upper: same) then TArray<FSectionEvaluationData> (tagged structs)
                let bound = |r: &mut Reader<'a>| -> Result<Value> {
                    let ty = r.u8()?;
                    let v = r.f32()?;
                    Ok(st("RangeBound", vec![p("Type", "ByteProperty", Value::Byte(ty)), p("Value", "FloatProperty", Value::Float(v))]))
                };
                let lower = bound(r)?;
                let upper = bound(r)?;
                let n = r.count(8)?;
                let mut impls = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut props = PropertyList::new();
                    self.read_tagged_into(r, &mut props)?;
                    impls.push(st("SectionEvaluationData", props.0));
                }
                st(
                    name,
                    vec![
                        p("Range", "StructProperty", st("FloatRange", vec![p("LowerBound", "StructProperty", lower), p("UpperBound", "StructProperty", upper)])),
                        p("Impls", "ArrayProperty", Value::Array(impls)),
                    ],
                )
            }
            "MovieSceneEvalTemplatePtr" | "MovieSceneTrackImplementationPtr" => {
                // inline polymorphic value: UScriptStruct path (FString, empty = null) + tagged properties of that struct
                let path = r.fstring()?;
                let mut props = vec![p("TypePath", "StrProperty", Value::Str(path.clone()))];
                if !path.is_empty() {
                    let mut inner = PropertyList::new();
                    self.read_tagged_into(r, &mut inner)?;
                    let short = path.rsplit(['.', '/']).next().unwrap_or(&path).to_string();
                    props.push(p("Value", "StructProperty", st(&short, inner.0)));
                }
                st(name, props)
            }
            "MovieSceneEventParameters" => {
                let struct_type = r.fstring()?;
                let n = r.count(1)?;
                let bytes = r.bytes(n)?.to_vec();
                st(name, vec![p("StructType", "StrProperty", Value::Str(struct_type)), p("InstanceBytes", "ArrayProperty", Value::Array(bytes.into_iter().map(Value::Byte).collect()))])
            }
            "FontData" if fits(8) => {
                let cooked = r.u32()? != 0;
                let face = Value::Object(crate::package::PackageIndex(r.i32()?));
                st(name, vec![p("bIsCooked", "BoolProperty", Value::Bool(cooked)), p("LocalFontFaceAsset", "ObjectProperty", face)])
            }
            "FontCharacter" if fits(21) => {
                let (su, sv, us, vs) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
                let ti = r.u8()?;
                let vo = r.i32()?;
                st(
                    name,
                    vec![
                        p("StartU", "IntProperty", Value::Int(su as i64)),
                        p("StartV", "IntProperty", Value::Int(sv as i64)),
                        p("USize", "IntProperty", Value::Int(us as i64)),
                        p("VSize", "IntProperty", Value::Int(vs as i64)),
                        p("TextureIndex", "ByteProperty", Value::Byte(ti)),
                        p("VerticalOffset", "IntProperty", Value::Int(vo as i64)),
                    ],
                )
            }
            "ConnectivityCube" => {
                // FConnectivityCube: 6 x (int32 NumBits, ceil(NumBits/32) uint32 words), see read_connectivity_cube. Each face becomes
                // a `BitArray { NumBits, Data: [words] }` named by EFortConnectivityCubeFace.
                let mut props = Vec::with_capacity(6);
                for face in CONNECTIVITY_CUBE_FACES {
                    let n = r.i32()?;
                    if !(0..=1 << 20).contains(&n) {
                        return Err(Error::malformed(format!("ConnectivityCube: {face} NumBits {n}")));
                    }
                    let mut words = Vec::new();
                    for _ in 0..(n as usize + 31) / 32 {
                        words.push(Value::UInt(r.u32()? as u64));
                    }
                    props.push(p(face, "StructProperty", st("BitArray", vec![p("NumBits", "IntProperty", Value::Int(n as i64)), p("Data", "ArrayProperty", Value::Array(words))])));
                }
                st(name, props)
            }
            "ExpressionInput" | "ColorMaterialInput" | "ScalarMaterialInput" | "VectorMaterialInput" | "Vector2MaterialInput" | "MaterialAttributesInput" | "ShadingModelMaterialInput" => {
                // verify the layout consumes exactly `size`
                let start = r.p;
                let v = self.read_expression_input(name, r)?;
                if let Some(s) = size {
                    if r.p - start != s {
                        r.p = start;
                        return Err(Error::malformed(format!("{name}: layout consumed {} of {s} bytes", r.p.max(start) - start)));
                    }
                }
                v
            }
            _ => return Ok(None),
        }))
    }

    // ------------------------------------------------------------------------------------------
    // FText
    // ------------------------------------------------------------------------------------------

    /// `FFormatArgumentValue`: `i8 type` (0 int64, 1 uint64, 2 float, 3 double, 4 text, 5 gender) + payload.
    fn skip_format_argument_value(&self, r: &mut Reader<'a>) -> Result<()> {
        match r.i8()? {
            0 | 1 => r.skip(8),
            2 => r.skip(4),
            3 => r.skip(8),
            4 => self.read_text(r).map(|_| ()),
            5 => r.skip(1),
            t => Err(Error::unsupported(format!("FFormatArgumentValue type {t}"))),
        }
    }

    fn skip_number_format_options(&self, r: &mut Reader<'a>) -> Result<()> {
        // bool32 bHasOptions; if set: UseGrouping(bool32) RoundingMode(u8) MinInt MaxInt MinFrac MaxFrac (i32 each)
        if r.u32()? != 0 {
            r.skip(4 + 1 + 16)?;
        }
        Ok(())
    }

    /// Histories other than none/base/string-table. Only the pattern/source text is kept; arguments are skipped
    /// (consumed exactly, so surrounding data stays aligned).
    pub(crate) fn read_text_history(&self, h: i8, t: &mut crate::value::Text, r: &mut Reader<'a>) -> Result<()> {
        match h {
            1 => {
                let src = self.read_text(r)?;
                t.source = src.source;
                t.key = src.key;
                t.namespace = src.namespace;
                let n = r.count(5)?;
                for _ in 0..n {
                    r.fstring()?;
                    self.skip_format_argument_value(r)?;
                }
            }
            2 => {
                let src = self.read_text(r)?;
                t.source = src.source;
                t.key = src.key;
                t.namespace = src.namespace;
                let n = r.count(1)?;
                for _ in 0..n {
                    self.skip_format_argument_value(r)?;
                }
            }
            3 => {
                let src = self.read_text(r)?;
                t.source = src.source;
                t.key = src.key;
                t.namespace = src.namespace;
                let n = r.count(5)?;
                for _ in 0..n {
                    r.fstring()?;
                    self.skip_format_argument_value(r)?;
                }
            }
            4 | 5 | 6 => {
                self.skip_format_argument_value(r)?;
                self.skip_number_format_options(r)?;
                if h == 6 && self.version() >= 516 {
                    r.fstring()?; // CurrencyCode
                }
                r.fstring()?; // TargetCulture
            }
            7 | 8 | 9 => {
                r.skip(8)?; // FDateTime
                r.skip(if h == 9 { 2 } else { 1 })?; // date/time style(s)
                if self.version() >= 517 {
                    r.fstring()?; // TimeZone
                }
                r.fstring()?; // TargetCulture
            }
            10 => {
                let src = self.read_text(r)?;
                t.source = src.source;
                t.key = src.key;
                t.namespace = src.namespace;
                r.skip(1)?; // transform type
            }
            other => return Err(Error::unsupported(format!("FText history type {other}"))),
        }
        let _ = ver::FTEXT_HISTORY;
        Ok(())
    }
}




