//! `FMaterialUniformExpressionSet` of a cooked 4.16 material (the recipe of the `Material` constant buffer), parsed from the shader map in the `.uexp` and evaluated.
//! The byte layout is the one recovered by `fn-dxbc-cli::uniform` (vectors list, scalars list, 2D textures list, trailing count); this parser additionally accepts the
//! node kinds `Max` / `Min` / `Clamp` / unary maths that the weak-point and damage-number materials contain and sets without vector expressions.
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    VectorParameter { name: String, default: [f32; 4] },
    ScalarParameter { name: String, default: f32 },
    Constant { value: [f32; 4], value_type: u8 },
    ComponentSwizzle { e: Box<Expr>, idx: [i8; 4], count: u8 },
    /// `op`: 0 add, 1 sub, 2 mul, 3 div, 4 dot.
    FoldedMath { a: Box<Expr>, b: Box<Expr>, op: u8, value_type: u32 },
    AppendVector { a: Box<Expr>, b: Box<Expr>, num_a: u32 },
    /// `op`: 0 sin, 1 cos, 2 tan.
    TrigMath { x: Box<Expr>, y: Box<Expr>, op: u8 },
    Max(Box<Expr>, Box<Expr>),
    Min(Box<Expr>, Box<Expr>),
    Clamp { x: Box<Expr>, lo: Box<Expr>, hi: Box<Expr> },
    /// Unary maths: "Abs" "Ceil" "Floor" "Frac" "Sign" "SquareRoot" "Saturate".
    Unary(&'static str, Box<Expr>),
    Texture { index: i32, sampler_source: i32 },
    TextureParameter { name: String, index: i32, sampler_source: i32 },
    Time,
    RealTime,
}
#[derive(Debug, Clone, PartialEq)]
pub struct UniformSet { pub vectors: Vec<Expr>, pub scalars: Vec<Expr>, pub textures2d: Vec<Expr> }
#[derive(Debug, Clone, Default)]
pub struct Overrides { pub vectors: HashMap<String, [f32; 4]>, pub scalars: HashMap<String, f32> }
#[derive(Debug, Clone)]
pub struct Packed { pub registers: usize, pub vectors: usize, pub scalars: usize, pub cb: Vec<[f32; 4]> }

type PR<T> = Result<T, String>;
struct R<'a> { d: &'a [u8], o: usize, names: &'a [String] }
impl R<'_> {
    fn take(&mut self, n: usize) -> PR<&[u8]> { let s = self.d.get(self.o..self.o + n).ok_or_else(|| format!("read past end at {}", self.o))?; self.o += n; Ok(s) }
    fn i32(&mut self) -> PR<i32> { Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn f32(&mut self) -> PR<f32> { Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u8(&mut self) -> PR<u8> { Ok(self.take(1)?[0]) }
    fn i8(&mut self) -> PR<i8> { Ok(self.take(1)?[0] as i8) }
    fn fname(&mut self) -> PR<String> {
        let (i, n) = (self.i32()?, self.i32()?);
        if i < 0 || i as usize >= self.names.len() || !(0..=4).contains(&n) { return Err(format!("bad FName at {}", self.o - 8)); }
        let base = &self.names[i as usize];
        Ok(if n > 0 { format!("{base}_{}", n - 1) } else { base.clone() })
    }
    fn vec4(&mut self) -> PR<[f32; 4]> { Ok([self.f32()?, self.f32()?, self.f32()?, self.f32()?]) }
    fn bx(&mut self, depth: u32) -> PR<Box<Expr>> { Ok(Box::new(self.expr(depth)?)) }
    fn expr(&mut self, depth: u32) -> PR<Expr> {
        if depth > 32 { return Err("expression nesting".into()); }
        let t = self.fname()?;
        let t = t.strip_prefix("FMaterialUniformExpression").unwrap_or(&t).to_string();
        Ok(match t.as_str() {
            "VectorParameter" => { let name = self.fname()?; Expr::VectorParameter { name, default: self.vec4()? } }
            "ScalarParameter" => { let name = self.fname()?; Expr::ScalarParameter { name, default: self.f32()? } }
            "Constant" => { let value = self.vec4()?; Expr::Constant { value, value_type: self.u8()? } }
            "ComponentSwizzle" => { let e = self.bx(depth + 1)?; let idx = [self.i8()?, self.i8()?, self.i8()?, self.i8()?]; Expr::ComponentSwizzle { e, idx, count: self.u8()? } }
            "FoldedMath" => { let a = self.bx(depth + 1)?; let b = self.bx(depth + 1)?; let op = self.u8()?; Expr::FoldedMath { a, b, op, value_type: self.i32()? as u32 } }
            "AppendVector" => { let a = self.bx(depth + 1)?; let b = self.bx(depth + 1)?; Expr::AppendVector { a, b, num_a: self.i32()? as u32 } }
            "TrigMath" => { let x = self.bx(depth + 1)?; let y = self.bx(depth + 1)?; Expr::TrigMath { x, y, op: self.u8()? } }
            "Max" => { let a = self.bx(depth + 1)?; Expr::Max(a, self.bx(depth + 1)?) }
            "Min" => { let a = self.bx(depth + 1)?; Expr::Min(a, self.bx(depth + 1)?) }
            "Clamp" => { let x = self.bx(depth + 1)?; let lo = self.bx(depth + 1)?; Expr::Clamp { x, lo, hi: self.bx(depth + 1)? } }
            "Abs" => Expr::Unary("Abs", self.bx(depth + 1)?), "Ceil" => Expr::Unary("Ceil", self.bx(depth + 1)?), "Floor" => Expr::Unary("Floor", self.bx(depth + 1)?),
            "Frac" => Expr::Unary("Frac", self.bx(depth + 1)?), "Sign" => Expr::Unary("Sign", self.bx(depth + 1)?), "SquareRoot" => Expr::Unary("SquareRoot", self.bx(depth + 1)?),
            "Saturate" => Expr::Unary("Saturate", self.bx(depth + 1)?),
            "Texture" => { let index = self.i32()?; Expr::Texture { index, sampler_source: self.i32()? } }
            "TextureParameter" => { let name = self.fname()?; let index = self.i32()?; Expr::TextureParameter { name, index, sampler_source: self.i32()? } }
            "Time" => Expr::Time,
            "RealTime" => Expr::RealTime,
            other => return Err(format!("unhandled uniform expression {other} at {}", self.o)),
        })
    }
    fn list(&mut self) -> PR<Vec<Expr>> {
        let n = self.i32()?;
        if !(0..=256).contains(&n) { return Err(format!("bad count {n}")); }
        (0..n).map(|_| self.expr(0)).collect()
    }
}

/// Finds and parses the uniform expression set of a cooked material (`uexp` = the `.uexp` bytes, `names` the package name table).
pub fn parse_set(uexp: &[u8], names: &[String]) -> Result<UniformSet, String> {
    let is_type = |i: i32| i >= 0 && (i as usize) < names.len() && names[i as usize].starts_with("FMaterialUniformExpression");
    let mut last = String::from("no candidate");
    let mut best: Option<UniformSet> = None;
    let mut h = 4usize;
    while h + 8 <= uexp.len() {
        let idx = i32::from_le_bytes(uexp[h..h + 4].try_into().unwrap());
        let num = i32::from_le_bytes(uexp[h + 4..h + 8].try_into().unwrap());
        // a list starts with its count, the first expression with its type FName: the first list may be empty, so a candidate also starts at an empty vector list
        if is_type(idx) && num == 0 {
            let cnt = i32::from_le_bytes(uexp[h - 4..h].try_into().unwrap());
            if (1..=256).contains(&cnt) {
                let mut r = R { d: uexp, o: h - 4, names };
                let attempt = (|| -> PR<UniformSet> {
                    let vectors = r.list()?; let scalars = r.list()?; let textures2d = r.list()?; let tc = r.i32()?;
                    if (vectors.is_empty() && scalars.is_empty() && textures2d.is_empty()) || !(0..64).contains(&tc) { return Err("trailing count".into()); }
                    Ok(UniformSet { vectors, scalars, textures2d })
                })();
                match attempt { Ok(s) => { if s.vectors.len() >= 1 { return Ok(s); } if best.is_none() { best = Some(s); } } Err(e) => last = e }
            }
        }
        h += 1;
    }
    // sets without vector expressions start with a zero vector count right before the scalar list: search for `00 00 00 00, count, FName`
    if best.is_none() {
        for h in 8..uexp.len().saturating_sub(8) {
            let idx = i32::from_le_bytes(uexp[h..h + 4].try_into().unwrap());
            let num = i32::from_le_bytes(uexp[h + 4..h + 8].try_into().unwrap());
            if is_type(idx) && num == 0 && i32::from_le_bytes(uexp[h - 8..h - 4].try_into().unwrap()) == 0 {
                let cnt = i32::from_le_bytes(uexp[h - 4..h].try_into().unwrap());
                if (1..=256).contains(&cnt) {
                    let mut r = R { d: uexp, o: h - 8, names };
                    let attempt = (|| -> PR<UniformSet> { let vectors = r.list()?; let scalars = r.list()?; let textures2d = r.list()?; let tc = r.i32()?; if !(0..64).contains(&tc) { return Err("trailing count".into()); } Ok(UniformSet { vectors, scalars, textures2d }) })();
                    if let Ok(s) = attempt { best = Some(s); break; }
                }
            }
        }
    }
    best.ok_or_else(|| format!("no uniform expression set found ({last})"))
}

fn map4(a: [f32; 4], b: [f32; 4], f: impl Fn(f32, f32) -> f32) -> [f32; 4] { std::array::from_fn(|k| f(a[k], b[k])) }
pub fn eval_vec(e: &Expr, ov: &Overrides, time: f32) -> [f32; 4] {
    match e {
        Expr::VectorParameter { name, default } => ov.vectors.get(name).copied().unwrap_or(*default),
        Expr::ScalarParameter { name, default } => [ov.scalars.get(name).copied().unwrap_or(*default); 4],
        Expr::Constant { value, .. } => *value,
        Expr::ComponentSwizzle { e, idx, .. } => { let v = eval_vec(e, ov, time); std::array::from_fn(|k| if idx[k] >= 0 { v[idx[k] as usize & 3] } else { 0.0 }) }
        Expr::AppendVector { a, b, num_a } => { let (a, b) = (eval_vec(a, ov, time), eval_vec(b, ov, time)); std::array::from_fn(|k| if (k as u32) < *num_a { a[k] } else { b.get(k - *num_a as usize).copied().unwrap_or(0.0) }) }
        Expr::FoldedMath { a, b, op, .. } => {
            let (a, b) = (eval_vec(a, ov, time), eval_vec(b, ov, time));
            if *op == 4 { return [a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]; 4]; }
            map4(a, b, |x, y| match op { 0 => x + y, 1 => x - y, 2 => x * y, _ => if y == 0.0 { 0.0 } else { x / y } })
        }
        Expr::TrigMath { x, op, .. } => eval_vec(x, ov, time).map(|v| match op { 0 => v.sin(), 1 => v.cos(), _ => v.tan() }),
        Expr::Max(a, b) => map4(eval_vec(a, ov, time), eval_vec(b, ov, time), f32::max),
        Expr::Min(a, b) => map4(eval_vec(a, ov, time), eval_vec(b, ov, time), f32::min),
        Expr::Clamp { x, lo, hi } => { let (x, lo, hi) = (eval_vec(x, ov, time), eval_vec(lo, ov, time), eval_vec(hi, ov, time)); std::array::from_fn(|k| x[k].clamp(lo[k], hi[k])) }
        Expr::Unary(op, x) => eval_vec(x, ov, time).map(|v| match *op { "Abs" => v.abs(), "Ceil" => v.ceil(), "Floor" => v.floor(), "Frac" => v - v.floor(), "Sign" => if v > 0. { 1. } else if v < 0. { -1. } else { 0. }, "SquareRoot" => v.max(0.).sqrt(), _ => v.clamp(0., 1.) }),
        Expr::Time | Expr::RealTime => [time; 4],
        Expr::Texture { .. } | Expr::TextureParameter { .. } => [0.0; 4],
    }
}
/// The packed material constant buffer: vector expressions fill `cb[i]`, scalar expressions are packed four per float4 after them.
pub fn evaluate(set: &UniformSet, ov: &Overrides, time: f32) -> Packed {
    let vec: Vec<[f32; 4]> = set.vectors.iter().map(|e| eval_vec(e, ov, time)).collect();
    let sc: Vec<f32> = set.scalars.iter().map(|e| eval_vec(e, ov, time)[0]).collect();
    let nv = vec.len();
    let regs = nv + sc.len().div_ceil(4);
    let mut cb = vec![[0.0f32; 4]; regs];
    cb[..nv].copy_from_slice(&vec);
    for (i, v) in sc.iter().enumerate() { cb[nv + (i >> 2)][i & 3] = *v; }
    Packed { registers: regs, vectors: nv, scalars: sc.len(), cb }
}
