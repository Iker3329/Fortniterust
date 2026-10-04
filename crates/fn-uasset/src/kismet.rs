//! Kismet bytecode reader / disassembler for UE 4.16 cooked scripts (port of `tools/kismet-disasm.cjs`).
//!
//! Cooked scripts serialize object pointers as 4-byte `FPackageIndex`es and names as 8-byte `FName`s.
//! [`parse`] decodes a function's script into an [`Expr`] tree per top-level statement; [`Expr::render`]
//! prints the exact text format of the Node tool (`let[..] V:x = ...`, `call Foo(..)`, `jumpIfNot(..) -> 123`),
//! so existing evidence (e.g. `animation-state-machines.json`) can be compared line for line.
//!
//! Offsets are byte offsets inside the script (`FunctionData::script`).

use crate::package::{Package, PackageIndex};

/// A pointer operand: the raw package index and its display name (import/export object name).
#[derive(Clone, Debug, PartialEq)]
pub struct Ref {
    pub index: PackageIndex,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextLit {
    Empty,
    Localized(Box<Expr>, Box<Expr>, Box<Expr>),
    Invariant(Box<Expr>),
    Literal(Box<Expr>),
    StringTable { table: Ref, id: Box<Expr>, key: Box<Expr> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    LocalVariable(Ref),
    InstanceVariable(Ref),
    DefaultVariable(Ref),
    LocalOutVariable(Ref),
    Return(Box<Expr>),
    Jump(u32),
    JumpIfNot { target: u32, cond: Box<Expr> },
    Assert { line: u16, debug: bool, cond: Box<Expr> },
    Nothing,
    Let { prop: Ref, var: Box<Expr>, value: Box<Expr> },
    LetBool { var: Box<Expr>, value: Box<Expr> },
    LetObj { var: Box<Expr>, value: Box<Expr> },
    LetWeakObjPtr { var: Box<Expr>, value: Box<Expr> },
    LetDelegate { var: Box<Expr>, value: Box<Expr> },
    LetMulticastDelegate { var: Box<Expr>, value: Box<Expr> },
    LetValueOnPersistentFrame { prop: Ref, value: Box<Expr> },
    ClassContext { object: Box<Expr>, skip: u32, prop: Ref, context: Box<Expr> },
    Context { object: Box<Expr>, skip: u32, prop: Ref, context: Box<Expr> },
    ContextFailSilent { object: Box<Expr>, skip: u32, prop: Ref, context: Box<Expr> },
    InterfaceContext(Box<Expr>),
    MetaCast { class: Ref, expr: Box<Expr> },
    DynamicCast { class: Ref, expr: Box<Expr> },
    ObjToInterfaceCast { class: Ref, expr: Box<Expr> },
    CrossInterfaceCast { class: Ref, expr: Box<Expr> },
    InterfaceToObjCast { class: Ref, expr: Box<Expr> },
    PrimitiveCast { kind: u8, expr: Box<Expr> },
    EndParmValue,
    EndFunctionParms,
    SelfRef,
    Skip { skip: u32, expr: Box<Expr> },
    VirtualFunction { name: String, args: Vec<Expr> },
    FinalFunction { func: Ref, args: Vec<Expr> },
    LocalVirtualFunction { name: String, args: Vec<Expr> },
    LocalFinalFunction { func: Ref, args: Vec<Expr> },
    CallMath { func: Ref, args: Vec<Expr> },
    CallMulticastDelegate { func: Ref, args: Vec<Expr> },
    IntConst(i32),
    FloatConst(f32),
    StringConst(String),
    UnicodeStringConst(String),
    ObjectConst(Ref),
    NameConst(String),
    RotationConst([f32; 3]),
    VectorConst([f32; 3]),
    ByteConst(u8),
    IntZero,
    IntOne,
    True,
    False,
    TextConst(TextLit),
    NoObject,
    TransformConst([f32; 10]),
    IntConstByte(u8),
    NoInterface,
    Int64Const(i64),
    UInt64Const(u64),
    PropertyConst(Ref),
    SoftObjectConst(Box<Expr>),
    StructConst { struct_ref: Ref, serialized_size: i32, fields: Vec<Expr> },
    SetArray { array: Box<Expr>, elements: Vec<Expr> },
    ArrayConst { inner: Ref, count: i32, elements: Vec<Expr> },
    SetSet { set: Box<Expr>, count: i32, elements: Vec<Expr> },
    SetMap { map: Box<Expr>, count: i32, elements: Vec<Expr> },
    StructMemberContext { prop: Ref, expr: Box<Expr> },
    InstanceDelegate(String),
    BindDelegate { name: String, delegate: Box<Expr>, object: Box<Expr> },
    AddMulticastDelegate(Box<Expr>, Box<Expr>),
    RemoveMulticastDelegate(Box<Expr>, Box<Expr>),
    ClearMulticastDelegate(Box<Expr>),
    PushExecutionFlow(u32),
    PopExecutionFlow,
    PopExecutionFlowIfNot(Box<Expr>),
    ComputedJump(Box<Expr>),
    Breakpoint,
    WireTracepoint,
    Tracepoint,
    SkipOffsetConst(u32),
    Switch { index: Box<Expr>, cases: Vec<(Expr, Expr)>, default: Box<Expr> },
    /// `EX_ArrayGetByRef` (0x6b): array expression, index expression.
    ArrayGetByRef { array: Box<Expr>, index: Box<Expr> },
    InstrumentationEvent { kind: u8, name: Option<String> },
    EndOfScript,
}

/// One top-level statement with its byte offset in the script.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    pub offset: usize,
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub struct Disassembly {
    pub statements: Vec<Statement>,
    /// `(offset, rendered text)` per statement (Node tool format).
    pub lines: Vec<(usize, String)>,
    /// Bytes consumed.
    pub end: usize,
    /// First decode error (statements before it are valid). `None` = script parsed cleanly.
    pub error: Option<String>,
    /// True if the last statement was `EX_EndOfScript`.
    pub reached_end: bool,
}

struct P<'p, 'a> {
    pkg: &'p Package<'a>,
    b: &'p [u8],
    p: usize,
}

type R<T> = std::result::Result<T, String>;

impl<'p, 'a> P<'p, 'a> {
    fn need(&self, n: usize) -> R<()> {
        if self.p + n > self.b.len() {
            Err(format!("eof reading {n} bytes at {}", self.p))
        } else {
            Ok(())
        }
    }
    fn u8(&mut self) -> R<u8> {
        self.need(1)?;
        let v = self.b[self.p];
        self.p += 1;
        Ok(v)
    }
    fn skip(&mut self, n: usize) -> R<()> {
        self.need(n)?;
        self.p += n;
        Ok(())
    }
    fn u16(&mut self) -> R<u16> {
        self.need(2)?;
        let v = u16::from_le_bytes([self.b[self.p], self.b[self.p + 1]]);
        self.p += 2;
        Ok(v)
    }
    fn u32(&mut self) -> R<u32> {
        self.need(4)?;
        let v = u32::from_le_bytes(self.b[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        Ok(v)
    }
    fn i32(&mut self) -> R<i32> {
        Ok(self.u32()? as i32)
    }
    fn f32(&mut self) -> R<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn i64(&mut self) -> R<i64> {
        self.need(8)?;
        let v = i64::from_le_bytes(self.b[self.p..self.p + 8].try_into().unwrap());
        self.p += 8;
        Ok(v)
    }
    fn ptr(&mut self) -> R<Ref> {
        let i = self.i32()?;
        let idx = PackageIndex(i);
        Ok(Ref { index: idx, name: ref_name(self.pkg, i) })
    }
    fn fname(&mut self) -> R<String> {
        let i = self.i32()?;
        let n = self.i32()?;
        Ok(match self.pkg.names.get(i as usize) {
            Some(base) => {
                if n != 0 {
                    format!("{}_{}", base, (n as u32).wrapping_sub(1))
                } else {
                    base.clone()
                }
            }
            None => return Err(format!("bad name index {i} at {}", self.p - 8)),
        })
    }
    fn list(&mut self, end: u8) -> R<Vec<Expr>> {
        let mut v = Vec::new();
        loop {
            self.need(1)?;
            if self.b[self.p] == end {
                self.p += 1;
                return Ok(v);
            }
            v.push(self.expr()?);
        }
    }
    fn bx(&mut self) -> R<Box<Expr>> {
        Ok(Box::new(self.expr()?))
    }
    fn cstr(&mut self) -> R<String> {
        let mut s = Vec::new();
        loop {
            let c = self.u8()?;
            if c == 0 {
                break;
            }
            s.push(c);
        }
        Ok(s.iter().map(|&c| c as char).collect())
    }
    fn wstr(&mut self) -> R<String> {
        let mut u = Vec::new();
        loop {
            let c = self.u16()?;
            if c == 0 {
                break;
            }
            u.push(c);
        }
        Ok(String::from_utf16_lossy(&u))
    }

    fn expr(&mut self) -> R<Expr> {
        let at = self.p;
        let op = self.u8()?;
        Ok(match op {
            0x00 => Expr::LocalVariable(self.ptr()?),
            0x01 => Expr::InstanceVariable(self.ptr()?),
            0x02 => Expr::DefaultVariable(self.ptr()?),
            0x04 => Expr::Return(self.bx()?),
            0x06 => Expr::Jump(self.u32()?),
            0x07 => {
                let target = self.u32()?;
                Expr::JumpIfNot { target, cond: self.bx()? }
            }
            0x09 => {
                let line = self.u16()?;
                let debug = self.u8()? != 0;
                Expr::Assert { line, debug, cond: self.bx()? }
            }
            0x0B => Expr::Nothing,
            0x0F => {
                let prop = self.ptr()?;
                let var = self.bx()?;
                let value = self.bx()?;
                Expr::Let { prop, var, value }
            }
            0x12 => {
                let object = self.bx()?;
                let skip = self.u32()?;
                let prop = self.ptr()?;
                let context = self.bx()?;
                Expr::ClassContext { object, skip, prop, context }
            }
            0x13 => {
                let class = self.ptr()?;
                Expr::MetaCast { class, expr: self.bx()? }
            }
            0x14 => {
                let var = self.bx()?;
                Expr::LetBool { var, value: self.bx()? }
            }
            0x15 => Expr::EndParmValue,
            0x16 => Expr::EndFunctionParms,
            0x17 => Expr::SelfRef,
            0x18 => {
                let skip = self.u32()?;
                Expr::Skip { skip, expr: self.bx()? }
            }
            0x19 | 0x1A => {
                let object = self.bx()?;
                let skip = self.u32()?;
                let prop = self.ptr()?;
                let context = self.bx()?;
                if op == 0x19 {
                    Expr::Context { object, skip, prop, context }
                } else {
                    Expr::ContextFailSilent { object, skip, prop, context }
                }
            }
            0x1B => {
                let name = self.fname()?;
                Expr::VirtualFunction { name, args: self.list(0x16)? }
            }
            0x1C => {
                let func = self.ptr()?;
                Expr::FinalFunction { func, args: self.list(0x16)? }
            }
            0x1D => Expr::IntConst(self.i32()?),
            0x1E => Expr::FloatConst(self.f32()?),
            0x1F => Expr::StringConst(self.cstr()?),
            0x20 => Expr::ObjectConst(self.ptr()?),
            0x21 => Expr::NameConst(self.fname()?),
            0x22 => Expr::RotationConst([self.f32()?, self.f32()?, self.f32()?]),
            0x23 => Expr::VectorConst([self.f32()?, self.f32()?, self.f32()?]),
            0x24 => Expr::ByteConst(self.u8()?),
            0x25 => Expr::IntZero,
            0x26 => Expr::IntOne,
            0x27 => Expr::True,
            0x28 => Expr::False,
            0x29 => {
                let k = self.u8()?;
                Expr::TextConst(match k {
                    0 => TextLit::Empty,
                    1 => TextLit::Localized(self.bx()?, self.bx()?, self.bx()?),
                    2 => TextLit::Invariant(self.bx()?),
                    3 => TextLit::Literal(self.bx()?),
                    4 => {
                        let table = self.ptr()?;
                        TextLit::StringTable { table, id: self.bx()?, key: self.bx()? }
                    }
                    k => return Err(format!("unknown text literal kind {k} at {at}")),
                })
            }
            0x2A => Expr::NoObject,
            0x2B => {
                let mut v = [0f32; 10];
                for x in v.iter_mut() {
                    *x = self.f32()?;
                }
                Expr::TransformConst(v)
            }
            0x2C => Expr::IntConstByte(self.u8()?),
            0x2D => Expr::NoInterface,
            0x2E => {
                let class = self.ptr()?;
                Expr::DynamicCast { class, expr: self.bx()? }
            }
            0x2F => {
                let struct_ref = self.ptr()?;
                let serialized_size = self.i32()?;
                Expr::StructConst { struct_ref, serialized_size, fields: self.list(0x30)? }
            }
            0x31 => {
                // 4.16: the array variable is an expression (not a property pointer), then the elements
                let array = self.bx()?;
                Expr::SetArray { array, elements: self.list(0x32)? }
            }
            0x33 => Expr::PropertyConst(self.ptr()?),
            0x34 => Expr::UnicodeStringConst(self.wstr()?),
            0x35 => Expr::Int64Const(self.i64()?),
            0x36 => Expr::UInt64Const(self.i64()? as u64),
            0x38 => {
                let kind = self.u8()?;
                Expr::PrimitiveCast { kind, expr: self.bx()? }
            }
            0x39 => {
                let set = self.bx()?;
                let count = self.i32()?;
                Expr::SetSet { set, count, elements: self.list(0x3A)? }
            }
            0x3B => {
                let map = self.bx()?;
                let count = self.i32()?;
                Expr::SetMap { map, count, elements: self.list(0x3C)? }
            }
            0x42 => {
                let prop = self.ptr()?;
                Expr::StructMemberContext { prop, expr: self.bx()? }
            }
            0x43 => {
                let var = self.bx()?;
                Expr::LetMulticastDelegate { var, value: self.bx()? }
            }
            0x44 => {
                let var = self.bx()?;
                Expr::LetDelegate { var, value: self.bx()? }
            }
            0x45 => {
                let name = self.fname()?;
                Expr::LocalVirtualFunction { name, args: self.list(0x16)? }
            }
            0x46 => {
                let func = self.ptr()?;
                Expr::LocalFinalFunction { func, args: self.list(0x16)? }
            }
            0x48 => Expr::LocalOutVariable(self.ptr()?),
            0x4B => Expr::InstanceDelegate(self.fname()?),
            0x4C => Expr::PushExecutionFlow(self.u32()?),
            0x4D => Expr::PopExecutionFlow,
            0x4E => Expr::ComputedJump(self.bx()?),
            0x4F => Expr::PopExecutionFlowIfNot(self.bx()?),
            0x50 => Expr::Breakpoint,
            0x51 => Expr::InterfaceContext(self.bx()?),
            0x52 => {
                let class = self.ptr()?;
                Expr::ObjToInterfaceCast { class, expr: self.bx()? }
            }
            0x53 => Expr::EndOfScript,
            0x54 => {
                let class = self.ptr()?;
                Expr::CrossInterfaceCast { class, expr: self.bx()? }
            }
            0x55 => {
                let class = self.ptr()?;
                Expr::InterfaceToObjCast { class, expr: self.bx()? }
            }
            0x5A => Expr::WireTracepoint,
            0x5B => Expr::SkipOffsetConst(self.u32()?),
            0x5C => {
                let a = self.bx()?;
                Expr::AddMulticastDelegate(a, self.bx()?)
            }
            0x5D => Expr::ClearMulticastDelegate(self.bx()?),
            0x5E => Expr::Tracepoint,
            0x5F => {
                let var = self.bx()?;
                Expr::LetObj { var, value: self.bx()? }
            }
            0x60 => {
                let var = self.bx()?;
                Expr::LetWeakObjPtr { var, value: self.bx()? }
            }
            0x61 => {
                let name = self.fname()?;
                let delegate = self.bx()?;
                Expr::BindDelegate { name, delegate, object: self.bx()? }
            }
            0x62 => {
                let a = self.bx()?;
                Expr::RemoveMulticastDelegate(a, self.bx()?)
            }
            0x63 => {
                let func = self.ptr()?;
                Expr::CallMulticastDelegate { func, args: self.list(0x16)? }
            }
            0x64 => {
                let prop = self.ptr()?;
                Expr::LetValueOnPersistentFrame { prop, value: self.bx()? }
            }
            0x65 => {
                let inner = self.ptr()?;
                let count = self.i32()?;
                Expr::ArrayConst { inner, count, elements: self.list(0x66)? }
            }
            0x67 => Expr::SoftObjectConst(self.bx()?),
            0x68 => {
                let func = self.ptr()?;
                Expr::CallMath { func, args: self.list(0x16)? }
            }
            0x69 => {
                let n = self.u16()?;
                self.skip(4)?; // EndGotoOffset
                let index = self.bx()?;
                let mut cases = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    let k = self.expr()?;
                    self.skip(4)?; // NextOffset
                    let r = self.expr()?;
                    cases.push((k, r));
                }
                let default = self.bx()?;
                Expr::Switch { index, cases, default }
            }
            0x6B => {
                let array = self.bx()?;
                Expr::ArrayGetByRef { array, index: self.bx()? }
            }
            0x6A => {
                let kind = self.u8()?;
                let name = if kind == 4 { Some(self.fname()?) } else { None };
                Expr::InstrumentationEvent { kind, name }
            }
            _ => return Err(format!("opcode 0x{op:x} at {at}")),
        })
    }
}

fn ref_name(pkg: &Package, i: i32) -> String {
    if i < 0 {
        pkg.imports.get((-(i as i64) - 1) as usize).map(|x| x.name.clone()).unwrap_or_else(|| format!("I{i}"))
    } else if i > 0 {
        pkg.exports.get(i as usize - 1).map(|x| x.name.clone()).unwrap_or_else(|| format!("E{i}"))
    } else {
        "null".to_string()
    }
}

/// Decode statements until `EX_EndOfScript` or the end of `script`. Never panics; on a decode error the
/// statements before it are returned with `error` set.
pub fn disassemble(pkg: &Package, script: &[u8]) -> Disassembly {
    let mut p = P { pkg, b: script, p: 0 };
    let mut statements = Vec::new();
    let mut lines = Vec::new();
    let mut error = None;
    let mut reached_end = false;
    while p.p < script.len() {
        let at = p.p;
        match p.expr() {
            Ok(e) => {
                let end = e == Expr::EndOfScript;
                lines.push((at, e.render()));
                statements.push(Statement { offset: at, expr: e });
                if end {
                    reached_end = true;
                    break;
                }
            }
            Err(m) => {
                error = Some(m);
                break;
            }
        }
    }
    Disassembly { statements, lines, end: p.p, error, reached_end }
}

/// Like [`disassemble`] but returns an error unless the whole script decodes and ends with `EX_EndOfScript`
/// exactly at its last byte.
pub fn parse(pkg: &Package, script: &[u8]) -> crate::error::Result<Vec<Statement>> {
    let d = disassemble(pkg, script);
    if let Some(e) = d.error {
        return Err(crate::error::Error::malformed(format!("kismet: {e}")));
    }
    if !d.reached_end {
        return Err(crate::error::Error::malformed("kismet: script has no EX_EndOfScript"));
    }
    if d.end != script.len() {
        return Err(crate::error::Error::malformed(format!("kismet: {} byte(s) after EX_EndOfScript", script.len() - d.end)));
    }
    Ok(d.statements)
}

// ---------------------------------------------------------------------------------------------------
// rendering (Node tool text format)
// ---------------------------------------------------------------------------------------------------

/// JavaScript `Number.prototype.toString` for finite doubles (and NaN/Infinity).
pub fn js_number(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x == 0.0 {
        return "0".into();
    }
    let a = x.abs();
    if !(1e-6..1e21).contains(&a) {
        // exponent form: d.ddde+XX
        let s = format!("{x:e}"); // Rust: 1.2345e-7 / 1e21
        let (mant, exp) = s.split_once('e').unwrap();
        let exp: i32 = exp.parse().unwrap();
        return format!("{mant}e{}{}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    format!("{x}")
}

/// JS `+v.toFixed(d)` (round to `d` decimals, then back to a number string).
fn js_fixed(v: f32, d: usize) -> String {
    let x = v as f64;
    if !x.is_finite() || x.abs() >= 1e21 {
        return js_number(x);
    }
    let s = format!("{x:.d$}");
    js_number(s.parse::<f64>().unwrap_or(x))
}

fn jstr(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

fn args(v: &[Expr]) -> String {
    v.iter().map(|e| e.render()).collect::<Vec<_>>().join(", ")
}

impl Expr {
    /// Text in the Node tool's format.
    pub fn render(&self) -> String {
        use Expr::*;
        match self {
            LocalVariable(r) => format!("L:{}", r.name),
            InstanceVariable(r) => format!("V:{}", r.name),
            DefaultVariable(r) => format!("D:{}", r.name),
            LocalOutVariable(r) => format!("O:{}", r.name),
            Return(e) => format!("return({})", e.render()),
            Jump(t) => format!("jump {t}"),
            JumpIfNot { target, cond } => format!("jumpIfNot({}) -> {target}", cond.render()),
            Assert { cond, .. } => format!("assert({})", cond.render()),
            Nothing => "nop".into(),
            Let { prop, var, value } => format!("let[{}] {} = {}", prop.name, var.render(), value.render()),
            LetBool { var, value } => format!("letbool {} = {}", var.render(), value.render()),
            LetObj { var, value } => format!("letobj {} = {}", var.render(), value.render()),
            LetWeakObjPtr { var, value } => format!("letweak {} = {}", var.render(), value.render()),
            LetDelegate { var, value } => format!("letdel {} = {}", var.render(), value.render()),
            LetMulticastDelegate { var, value } => format!("letmcdel {} = {}", var.render(), value.render()),
            LetValueOnPersistentFrame { prop, value } => format!("letpf[{}] = {}", prop.name, value.render()),
            ClassContext { object, prop, context, .. } => format!("classctx({},{},{})", object.render(), prop.name, context.render()),
            Context { object, prop, context, .. } => format!("ctx({} . {} : {})", object.render(), prop.name, context.render()),
            ContextFailSilent { object, prop, context, .. } => format!("ctxsilent({} . {} : {})", object.render(), prop.name, context.render()),
            InterfaceContext(e) => format!("ifacectx({})", e.render()),
            MetaCast { class, expr } => format!("metacast({},{})", class.name, expr.render()),
            DynamicCast { class, expr } => format!("dyncast({},{})", class.name, expr.render()),
            ObjToInterfaceCast { class, expr } => format!("obj2iface({},{})", class.name, expr.render()),
            CrossInterfaceCast { class, expr } => format!("xiface({},{})", class.name, expr.render()),
            InterfaceToObjCast { class, expr } => format!("iface2obj({},{})", class.name, expr.render()),
            PrimitiveCast { kind, expr } => format!("cast{kind}({})", expr.render()),
            EndParmValue => "endparmvalue".into(),
            EndFunctionParms => "endparms".into(),
            SelfRef => "self".into(),
            Skip { expr, .. } => format!("skip({})", expr.render()),
            VirtualFunction { name, args: a } => format!("vcall {name}({})", args(a)),
            FinalFunction { func, args: a } => format!("call {}({})", func.name, args(a)),
            LocalVirtualFunction { name, args: a } => format!("lvcall {name}({})", args(a)),
            LocalFinalFunction { func, args: a } => format!("lfcall {}({})", func.name, args(a)),
            CallMath { func, args: a } => format!("math {}({})", func.name, args(a)),
            CallMulticastDelegate { func, args: a } => format!("callmc {}({})", func.name, args(a)),
            IntConst(v) => format!("int {v}"),
            FloatConst(v) => format!("f{}", js_fixed(*v, 5)),
            StringConst(s) | UnicodeStringConst(s) => jstr(s),
            ObjectConst(r) => format!("obj {}", r.name),
            NameConst(n) => format!("name {n}"),
            RotationConst(v) => format!("rot[{}]", v.iter().map(|x| js_number(*x as f64)).collect::<Vec<_>>().join(",")),
            VectorConst(v) => format!("vec[{}]", v.iter().map(|x| js_fixed(*x, 4)).collect::<Vec<_>>().join(",")),
            ByteConst(b) => format!("byte {b}"),
            IntZero => "0".into(),
            IntOne => "1".into(),
            True => "true".into(),
            False => "false".into(),
            TextConst(t) => match t {
                TextLit::Empty => "text(empty)".into(),
                TextLit::Localized(a, b, c) => format!("text({}{}{})", a.render(), b.render(), c.render()),
                TextLit::Invariant(e) => format!("text(inv {})", e.render()),
                TextLit::Literal(e) => format!("text(lit {})", e.render()),
                TextLit::StringTable { table, id, key } => format!("text(strtbl {} {} {})", table.name, id.render(), key.render()),
            },
            NoObject => "noobj".into(),
            TransformConst(v) => format!("xform[{}]", v.iter().map(|x| js_fixed(*x, 3)).collect::<Vec<_>>().join(",")),
            IntConstByte(b) => format!("int8 {b}"),
            NoInterface => "nointerface".into(),
            Int64Const(v) => format!("i64 {v}"),
            UInt64Const(v) => format!("u64 {v}"),
            PropertyConst(r) => format!("propconst {}", r.name),
            SoftObjectConst(e) => format!("softobj({})", e.render()),
            StructConst { struct_ref, fields, .. } => format!("struct {}{{{}}}", struct_ref.name, args(fields)),
            SetArray { array, elements } => format!("setarray[{}]({})", array.render(), args(elements)),
            ArrayConst { elements, .. } => format!("arrayconst({})", args(elements)),
            SetSet { set, elements, .. } => format!("setset[{}]({})", set.render(), args(elements)),
            SetMap { map, elements, .. } => format!("setmap[{}]({})", map.render(), args(elements)),
            StructMemberContext { prop, expr } => format!("smember[{}]({})", prop.name, expr.render()),
            InstanceDelegate(n) => format!("instdel {n}"),
            BindDelegate { name, delegate, object } => format!("binddel {name} {} {}", delegate.render(), object.render()),
            AddMulticastDelegate(a, b) => format!("addmcdel {} {}", a.render(), b.render()),
            RemoveMulticastDelegate(a, b) => format!("rmmcdel {} {}", a.render(), b.render()),
            ClearMulticastDelegate(e) => format!("clrmcdel {}", e.render()),
            PushExecutionFlow(t) => format!("pushflow {t}"),
            PopExecutionFlow => "popflow".into(),
            PopExecutionFlowIfNot(e) => format!("popflowifnot({})", e.render()),
            ComputedJump(e) => format!("computedjump({})", e.render()),
            Breakpoint => "breakpoint".into(),
            WireTracepoint => "wiretp".into(),
            Tracepoint => "tracepoint".into(),
            SkipOffsetConst(v) => format!("skipoffset {v}"),
            Switch { index, cases, default } => {
                let cs: Vec<String> = cases.iter().map(|(k, r)| format!("{}:{}", k.render(), r.render())).collect();
                format!("switch({}){{{} default:{}}}", index.render(), cs.join(";"), default.render())
            }
            ArrayGetByRef { array, index } => format!("arrayget({},{})", array.render(), index.render()),
            InstrumentationEvent { kind, name } => format!("instrument{kind}{}", name.as_ref().map(|n| format!(" {n}")).unwrap_or_default()),
            EndOfScript => "END".into(),
        }
    }

    /// Direct sub-expressions (operands, arguments, elements) in evaluation order.
    pub fn children(&self) -> Vec<&Expr> {
        use Expr::*;
        match self {
            Return(e) | InterfaceContext(e) | SoftObjectConst(e) | PopExecutionFlowIfNot(e) | ComputedJump(e) | ClearMulticastDelegate(e) => vec![e],
            JumpIfNot { cond, .. } | Assert { cond, .. } => vec![cond],
            Let { var, value, .. } | LetBool { var, value } | LetObj { var, value } | LetWeakObjPtr { var, value } | LetDelegate { var, value } | LetMulticastDelegate { var, value } => vec![var, value],
            LetValueOnPersistentFrame { value, .. } => vec![value],
            ClassContext { object, context, .. } | Context { object, context, .. } | ContextFailSilent { object, context, .. } => vec![object, context],
            MetaCast { expr, .. } | DynamicCast { expr, .. } | ObjToInterfaceCast { expr, .. } | CrossInterfaceCast { expr, .. } | InterfaceToObjCast { expr, .. } | PrimitiveCast { expr, .. } | Skip { expr, .. } | StructMemberContext { expr, .. } => vec![expr],
            VirtualFunction { args, .. } | FinalFunction { args, .. } | LocalVirtualFunction { args, .. } | LocalFinalFunction { args, .. } | CallMath { args, .. } | CallMulticastDelegate { args, .. } => args.iter().collect(),
            StructConst { fields, .. } => fields.iter().collect(),
            ArrayConst { elements, .. } => elements.iter().collect(),
            SetArray { array, elements } => std::iter::once(&**array).chain(elements.iter()).collect(),
            SetSet { set, elements, .. } => std::iter::once(&**set).chain(elements.iter()).collect(),
            SetMap { map, elements, .. } => std::iter::once(&**map).chain(elements.iter()).collect(),
            BindDelegate { delegate, object, .. } => vec![delegate, object],
            ArrayGetByRef { array, index } => vec![array, index],
            AddMulticastDelegate(a, b) | RemoveMulticastDelegate(a, b) => vec![a, b],
            TextConst(t) => match t {
                TextLit::Empty => vec![],
                TextLit::Localized(a, b, c) => vec![a, b, c],
                TextLit::Invariant(e) | TextLit::Literal(e) => vec![e],
                TextLit::StringTable { id, key, .. } => vec![id, key],
            },
            Switch { index, cases, default } => {
                let mut v: Vec<&Expr> = vec![index];
                for (k, r) in cases {
                    v.push(k);
                    v.push(r);
                }
                v.push(default);
                v
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn js_numbers() {
        assert_eq!(js_number(1.0), "1");
        assert_eq!(js_number(0.5), "0.5");
        assert_eq!(js_number(-0.0), "0");
        assert_eq!(js_number(1e21), "1e+21");
        assert_eq!(js_number(1.5e-7), "1.5e-7");
        assert_eq!(js_number(0.30000001192092896), "0.30000001192092896");
        assert_eq!(js_fixed(0.4, 5), "0.4");
        assert_eq!(js_fixed(1.0, 5), "1");
        assert_eq!(js_fixed(0.01, 5), "0.01");
    }
}


