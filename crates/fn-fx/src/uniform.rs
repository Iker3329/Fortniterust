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
    /// `FMaterialUniformExpressionFmod`: component-wise `fmod(a, b)`.
    Fmod(Box<Expr>, Box<Expr>),
    Texture { index: i32, sampler_source: i32 },
    TextureParameter { name: String, index: i32, sampler_source: i32 },
    /// FOOT FX (2026-10-07): `FMaterialUniformExpressionTextureProperty` (UE4.16 MaterialUniformExpressions.h:1630): `Ar << TextureExpression << TextureProperty` (int8,
    /// `EMaterialExposedTextureProperty`: 0 TMTM_TextureSize = (SizeX, SizeY), 1 TMTM_TexelSize = (1 / SizeX, 1 / SizeY); B / A stay 0). The size is not in the set:
    /// `fn_fx::material::Material::load` puts it into the overrides under [`texture_property_key`]; without it the value is 0 (no texture resource, as the engine).
    TextureProperty { tex: Box<Expr>, property: i8 },
    Time,
    RealTime,
}
impl Expr {
    pub fn uses_time(&self) -> bool {
        match self {
            Expr::Time | Expr::RealTime => true,
            Expr::VectorParameter { .. } | Expr::ScalarParameter { .. } | Expr::Constant { .. } | Expr::Texture { .. } | Expr::TextureParameter { .. } => false,
            Expr::ComponentSwizzle { e, .. } | Expr::Unary(_, e) => e.uses_time(),
            Expr::FoldedMath { a, b, .. } | Expr::AppendVector { a, b, .. } | Expr::Max(a, b) | Expr::Min(a, b) | Expr::Fmod(a, b) => a.uses_time() || b.uses_time(),
            Expr::TrigMath { x, y, .. } => x.uses_time() || y.uses_time(),
            Expr::Clamp { x, lo, hi } => x.uses_time() || lo.uses_time() || hi.uses_time(),
            Expr::TextureProperty { tex, .. } => tex.uses_time(),
        }
    }
}
impl UniformSet {
    pub fn is_time_invariant(&self) -> bool {
        self.per_frame_scalars.is_empty() && self.per_frame_vectors.is_empty() && self.per_frame_prev_scalars.is_empty() && self.per_frame_prev_vectors.is_empty()
            && !self.vectors.iter().chain(self.scalars.iter()).any(Expr::uses_time)
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct UniformSet { pub vectors: Vec<Expr>, pub scalars: Vec<Expr>, pub textures2d: Vec<Expr>,
    /// `PerFrameUniformScalarExpressions` / `PerFrameUniformVectorExpressions` (UE 4.16: time dependent expressions, evaluated every frame into loose pixel / vertex shader globals `UE_Material_PerFrameScalarExpressionN`)
    pub per_frame_scalars: Vec<Expr>, pub per_frame_vectors: Vec<Expr>,
    /// `PerFramePrevUniformScalarExpressions` / `PerFramePrevUniformVectorExpressions` (the same expressions at the previous frame's time, ShaderBaseClasses.cpp:317; kept so the
    /// `UE_Material_PerFramePrev*ExpressionN` globals can be filled instead of left at zero)
    pub per_frame_prev_scalars: Vec<Expr>, pub per_frame_prev_vectors: Vec<Expr>,
    /// MATERIAL PARAMETERS (2026-10-06): `ParameterCollections` = the StateId of the collection behind each `MaterialCollectionN` uniform buffer, in N order (hex of the
    /// 16 serialized bytes, the form the cooked JSON prints a GUID in; `crate::mpc`).
    pub parameter_collections: Vec<String> }
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
        if i < 0 || i as usize >= self.names.len() || !(0..=512).contains(&n) { return Err(format!("bad FName at {}", self.o - 8)); }
        let base = &self.names[i as usize];
        Ok(if n > 0 { format!("{base}_{}", n - 1) } else { base.clone() })
    }
    fn vec4(&mut self) -> PR<[f32; 4]> { Ok([self.f32()?, self.f32()?, self.f32()?, self.f32()?]) }
    fn bx(&mut self, depth: u32) -> PR<Box<Expr>> { Ok(Box::new(self.expr(depth)?)) }
    fn expr(&mut self, depth: u32) -> PR<Expr> {
        if depth > 32 { return Err("expression nesting".into()); }
        let at = self.o; let t = self.fname()?; if std::env::var_os("FX_UNIFORM_TRACE").is_some() { eprintln!("  expr at {at} depth {depth}: {t}"); }
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
            "Fmod" => { let a = self.bx(depth + 1)?; Expr::Fmod(a, self.bx(depth + 1)?) }
            "Clamp" => { let x = self.bx(depth + 1)?; let lo = self.bx(depth + 1)?; Expr::Clamp { x, lo, hi: self.bx(depth + 1)? } }
            "Abs" => Expr::Unary("Abs", self.bx(depth + 1)?), "Ceil" => Expr::Unary("Ceil", self.bx(depth + 1)?), "Floor" => Expr::Unary("Floor", self.bx(depth + 1)?),
            "Frac" => Expr::Unary("Frac", self.bx(depth + 1)?), "Sign" => Expr::Unary("Sign", self.bx(depth + 1)?), "SquareRoot" => Expr::Unary("SquareRoot", self.bx(depth + 1)?),
            "Saturate" => Expr::Unary("Saturate", self.bx(depth + 1)?),
            "Texture" => { let index = self.i32()?; Expr::Texture { index, sampler_source: self.i32()? } }
            "TextureParameter" => { let name = self.fname()?; let index = self.i32()?; Expr::TextureParameter { name, index, sampler_source: self.i32()? } }
            "TextureProperty" => { let tex = self.bx(depth + 1)?; Expr::TextureProperty { tex, property: self.i8()? } } // FOOT FX
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

/// What follows the 2D texture list (`tc` = the cube texture expression count already read): the cube expressions, `ParameterCollections` (i32 n + n GUIDs) and the four per-frame lists.
/// Tolerant: a set without them (or whose tail does not parse) keeps empty per-frame lists.
// (now also returns the two PerFramePrev lists: the material shaders bind them too, shader_params.rs)
fn per_frame_tail(r: &mut R, tc: i32) -> (Vec<Expr>, Vec<Expr>, Vec<Expr>, Vec<Expr>, Vec<String>) {
    let save = r.o;
    let res = (|| -> PR<(Vec<Expr>, Vec<Expr>, Vec<Expr>, Vec<Expr>, Vec<String>)> {
        for _ in 0..tc { r.expr(0)?; }
        let n = r.i32()?; if !(0..32).contains(&n) { return Err("collections".into()); }
        let guids: Vec<String> = (0..n).map(|_| r.take(16).map(|b| b.iter().map(|x| format!("{x:02x}")).collect::<String>())).collect::<PR<_>>()?;
        let s = r.list()?; let v = r.list()?; let ps = r.list()?; let pv = r.list()?;
        Ok((s, v, ps, pv, guids))
    })();
    match res { Ok(x) => x, Err(_) => { r.o = save; (vec![], vec![], vec![], vec![], vec![]) } }
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
                    let (per_frame_scalars, per_frame_vectors, per_frame_prev_scalars, per_frame_prev_vectors, parameter_collections) = per_frame_tail(&mut r, tc);
                    Ok(UniformSet { vectors, scalars, textures2d, per_frame_scalars, per_frame_vectors, per_frame_prev_scalars, per_frame_prev_vectors, parameter_collections })
                })();
                match attempt.and_then(|s| if plausible(&s) { Ok(s) } else { Err("implausible lists".into()) }) { Ok(s) => { if s.vectors.len() >= 1 { return Ok(s); } if best.is_none() { best = Some(s); } } Err(e) => { if std::env::var_os("FX_UNIFORM_DEBUG").is_some() { eprintln!("candidate at {h}: {e}"); } last = e } }
            }
        }
        h += 1;
    }
    // sets without vector expressions start with a zero vector count right before the scalar list: search for `00 00 00 00, count, FName`
    // PP2 FIX (water agent, 2026-10-07): a set with NEITHER vector nor scalar expressions (the PivotPainter2 `MI_*_Ambient` instances of Fortress_BaseMaterial:
    // `00 00 00 00, 00 00 00 00, count, FName`) was read one list early (its texture expressions as the scalars, no Texture2D_k: "texture slot N unresolved");
    // the textures-only alignment is tried first and every candidate must be `plausible` (FUniformExpressionSet::Serialize order, UE4.16
    // MaterialUniformExpressions.cpp: vectors, scalars, 2D textures, cube textures)
    if best.is_none() {
        for h in 8..uexp.len().saturating_sub(8) {
            let idx = i32::from_le_bytes(uexp[h..h + 4].try_into().unwrap());
            let num = i32::from_le_bytes(uexp[h + 4..h + 8].try_into().unwrap());
            if is_type(idx) && num == 0 && i32::from_le_bytes(uexp[h - 8..h - 4].try_into().unwrap()) == 0 {
                let cnt = i32::from_le_bytes(uexp[h - 4..h].try_into().unwrap());
                if (1..=256).contains(&cnt) {
                    let starts: Vec<usize> = if h >= 12 && i32::from_le_bytes(uexp[h - 12..h - 8].try_into().unwrap()) == 0 { vec![h - 12, h - 8] } else { vec![h - 8] };
                    for o in starts {
                        let mut r = R { d: uexp, o, names };
                        let attempt = (|| -> PR<UniformSet> { let vectors = r.list()?; let scalars = r.list()?; let textures2d = r.list()?; let tc = r.i32()?; if !(0..64).contains(&tc) { return Err("trailing count".into()); } let (per_frame_scalars, per_frame_vectors, per_frame_prev_scalars, per_frame_prev_vectors, parameter_collections) = per_frame_tail(&mut r, tc); Ok(UniformSet { vectors, scalars, textures2d, per_frame_scalars, per_frame_vectors, per_frame_prev_scalars, per_frame_prev_vectors, parameter_collections }) })();
                        if let Ok(s) = attempt { if plausible(&s) { best = Some(s); break; } }
                    }
                    if best.is_some() { break; }
                }
            }
        }
    }
    best.ok_or_else(|| format!("no uniform expression set found ({last})"))
}

/// PP2 FIX: a parsed set is only taken when its lists hold the right kinds: no texture expression among the vectors / scalars, only texture expressions among
/// the 2D textures (FMaterialUniformExpressionTexture / ...TextureParameter, UE4.16 MaterialUniformExpressions.h).
fn plausible(s: &UniformSet) -> bool {
    let tex = |e: &Expr| matches!(e, Expr::Texture { .. } | Expr::TextureParameter { .. });
    !s.vectors.iter().chain(&s.scalars).any(tex) && s.textures2d.iter().all(tex)
}

fn map4(a: [f32; 4], b: [f32; 4], f: impl Fn(f32, f32) -> f32) -> [f32; 4] { std::array::from_fn(|k| f(a[k], b[k])) }
/// [`eval_vec_t`] with `RealTime` = `Time` (the old single-clock evaluation, kept for the existing callers).
pub fn eval_vec(e: &Expr, ov: &Overrides, time: f32) -> [f32; 4] { eval_vec_t(e, ov, time, time) }
/// Evaluates an expression with UE4.16 FMaterialRenderContext times: `Time` = world time (pauses / dilates), `RealTime` = real time (MaterialUniformExpressions.h `FMaterialUniformExpressionTime` / `RealTime`).
pub fn eval_vec_t(e: &Expr, ov: &Overrides, time: f32, real: f32) -> [f32; 4] {
    match e {
        Expr::VectorParameter { name, default } => ov.vectors.get(name).copied().unwrap_or(*default),
        Expr::ScalarParameter { name, default } => [ov.scalars.get(name).copied().unwrap_or(*default); 4],
        Expr::Constant { value, .. } => *value,
        Expr::ComponentSwizzle { e, idx, .. } => { let v = eval_vec_t(e, ov, time, real); std::array::from_fn(|k| if idx[k] >= 0 { v[idx[k] as usize & 3] } else { 0.0 }) }
        Expr::AppendVector { a, b, num_a } => { let (a, b) = (eval_vec_t(a, ov, time, real), eval_vec_t(b, ov, time, real)); std::array::from_fn(|k| if (k as u32) < *num_a { a[k] } else { b.get(k - *num_a as usize).copied().unwrap_or(0.0) }) }
        Expr::FoldedMath { a, b, op, .. } => {
            let (a, b) = (eval_vec_t(a, ov, time, real), eval_vec_t(b, ov, time, real));
            if *op == 4 { return [a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]; 4]; }
            map4(a, b, |x, y| match op { 0 => x + y, 1 => x - y, 2 => x * y, _ => if y == 0.0 { 0.0 } else { x / y } })
        }
        Expr::TrigMath { x, op, .. } => eval_vec_t(x, ov, time, real).map(|v| match op { 0 => v.sin(), 1 => v.cos(), _ => v.tan() }),
        Expr::Max(a, b) => map4(eval_vec_t(a, ov, time, real), eval_vec_t(b, ov, time, real), f32::max),
        Expr::Min(a, b) => map4(eval_vec_t(a, ov, time, real), eval_vec_t(b, ov, time, real), f32::min),
        Expr::Fmod(a, b) => map4(eval_vec_t(a, ov, time, real), eval_vec_t(b, ov, time, real), |x, y| if y == 0.0 { 0.0 } else { x % y }),
        Expr::Clamp { x, lo, hi } => { let (x, lo, hi) = (eval_vec_t(x, ov, time, real), eval_vec_t(lo, ov, time, real), eval_vec_t(hi, ov, time, real)); std::array::from_fn(|k| x[k].clamp(lo[k], hi[k])) }
        Expr::Unary(op, x) => eval_vec_t(x, ov, time, real).map(|v| match *op { "Abs" => v.abs(), "Ceil" => v.ceil(), "Floor" => v.floor(), "Frac" => v - v.floor(), "Sign" => if v > 0. { 1. } else if v < 0. { -1. } else { 0. }, "SquareRoot" => v.max(0.).sqrt(), _ => v.clamp(0., 1.) }),
        Expr::Time => [time; 4],
        Expr::RealTime => [real; 4], // separated from Time: RealTime does not pause / dilate (FMaterialRenderContext::RealTime)
        Expr::Texture { .. } | Expr::TextureParameter { .. } => [0.0; 4],
        Expr::TextureProperty { tex, property } => texture_property_key(tex, *property).and_then(|k| ov.vectors.get(&k).copied()).unwrap_or([0.0; 4]), // FOOT FX
    }
}
/// FOOT FX: the overrides key that carries the value of a `TextureProperty` expression on `tex` (a `Texture` / `TextureParameter` expression).
pub fn texture_property_key(tex: &Expr, property: i8) -> Option<String> {
    match tex { Expr::Texture { index, .. } => Some(format!("@TextureProperty #{index} {property}")), Expr::TextureParameter { name, index, .. } => Some(format!("@TextureProperty {name} #{index} {property}")), _ => None }
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

/// Values of the per-frame expressions at `time`: (scalars, vectors).
pub fn evaluate_per_frame(set: &UniformSet, ov: &Overrides, time: f32) -> (Vec<f32>, Vec<[f32; 4]>) {
    (set.per_frame_scalars.iter().map(|e| eval_vec(e, ov, time)[0]).collect(), set.per_frame_vectors.iter().map(|e| eval_vec(e, ov, time)).collect())
}

/// Material time inputs of one frame (UE4.16 ShaderBaseClasses.cpp:287: `Time = View.Family->CurrentWorldTime` (pauses / dilates), `RealTime = CurrentRealTime`; the
/// `PerFramePrev*` expressions are evaluated at both minus `DeltaWorldTime`, :317).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MaterialTime { pub world: f32, pub real: f32, pub delta_world: f32 }
impl MaterialTime {
    /// One clock for both (callers that only have one time).
    pub fn single(t: f32) -> MaterialTime { MaterialTime { world: t, real: t, delta_world: 0.0 } }
}

/// Writes the per-frame expressions into `floats` (the unnamed constant buffer in slot `slot` of one program) at the offsets the program serialized
/// (`crate::shader_params`): `FMaterialShader::SetParameters` (ShaderBaseClasses.cpp:279) sets each bound parameter with `SetShaderValue`, which copies
/// `min(sizeof(value), NumBytes)` bytes: the scalar (4 bytes) or the `FLinearColor` (16).
pub fn write_per_frame(floats: &mut [f32], set: &UniformSet, ov: &Overrides, params: &crate::shader_params::MaterialShaderParams, slot: u16, t: MaterialTime) {
    let lists: [(&Vec<crate::shader_params::ShaderParam>, &Vec<Expr>, bool, bool); 4] = [
        (&params.per_frame_scalars, &set.per_frame_scalars, false, false), (&params.per_frame_vectors, &set.per_frame_vectors, true, false),
        (&params.per_frame_prev_scalars, &set.per_frame_prev_scalars, false, true), (&params.per_frame_prev_vectors, &set.per_frame_prev_vectors, true, true)];
    for (ps, es, vector, prev) in lists {
        let (w, r) = if prev { (t.world - t.delta_world, t.real - t.delta_world) } else { (t.world, t.real) };
        for (p, e) in ps.iter().zip(es.iter()) {
            if !p.is_bound() || p.buffer_index != slot { continue; }
            let v = eval_vec_t(e, ov, w, r);
            let n = (p.num_bytes as usize / 4).min(if vector { 4 } else { 1 });
            for k in 0..n { if let Some(f) = floats.get_mut(p.float_index() + k) { *f = v[k]; } }
        }
    }
}

/// STATIC MATERIALS (2026-10-05): the booleans of `FMaterialCompilationOutput` that follow the uniform expression set in a cooked shader map (UE4.16
/// MaterialShared.cpp:321-333 `Serialize`: the set, then `bRequiresSceneColorCopy`, `bNeedsSceneTextures`, `bUsesEyeAdaptation`, `bModifiesMeshPosition`,
/// `bUsesWorldPositionOffset`, `bNeedsGBuffer`, `bUsesGlobalDistanceField`, `bUsesPixelDepthOffset`, `bUsesSceneDepthLookup`, each a 4-byte UBOOL).
/// `FMaterial::MaterialModifiesMeshPosition_RenderThread` reads `bModifiesMeshPosition` (the base-pass velocity rule, VelocityRendering.cpp:465).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompilationFlags {
    pub requires_scene_color_copy: bool, pub needs_scene_textures: bool, pub uses_eye_adaptation: bool, pub modifies_mesh_position: bool,
    pub uses_world_position_offset: bool, pub needs_gbuffer: bool, pub uses_global_distance_field: bool, pub uses_pixel_depth_offset: bool, pub uses_scene_depth_lookup: bool,
}
/// The first shader map's [`CompilationFlags`] of a cooked material package: the candidate scan of [`parse_set`]; a candidate counts only when the WHOLE set
/// (cube textures, parameter collections, the four per-frame lists, FUniformExpressionSet::Serialize order, MaterialUniformExpressions.cpp:87) parses and nine
/// UBOOLs (0 / 1) follow it. `None` when no such set is found.
pub fn compilation_flags(uexp: &[u8], names: &[String]) -> Option<CompilationFlags> {
    let is_type = |i: i32| i >= 0 && (i as usize) < names.len() && names[i as usize].starts_with("FMaterialUniformExpression");
    let rd = |o: usize| uexp.get(o..o + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    for h in 8..uexp.len().saturating_sub(8) {
        let (Some(idx), Some(num)) = (rd(h), rd(h + 4)) else { break };
        if !(is_type(idx) && num == 0) { continue; }
        // the set starts at the vector count (h - 4), or one list earlier when the vector list is empty (count 0 at h - 8, the scalar count at h - 4)
        let starts = [Some(h - 4), (rd(h - 8) == Some(0)).then(|| h - 8)];
        for start in starts.into_iter().flatten() {
            if !rd(h - 4).map_or(false, |c| (1..=256).contains(&c)) { continue; }
            let mut r = R { d: uexp, o: start, names };
            let ok = (|| -> PR<()> {
                let (v, s, t) = (r.list()?, r.list()?, r.list()?);
                if v.is_empty() && s.is_empty() && t.is_empty() { return Err("empty".into()); }
                let tc = r.i32()?; if !(0..64).contains(&tc) { return Err("cube count".into()); }
                for _ in 0..tc { r.expr(0)?; }
                let n = r.i32()?; if !(0..32).contains(&n) { return Err("collections".into()); }
                let _guids: Vec<String> = (0..n).map(|_| r.take(16).map(|b| b.iter().map(|x| format!("{x:02x}")).collect::<String>())).collect::<PR<_>>()?;
                for _ in 0..4 { r.list()?; }
                Ok(())
            })();
            if ok.is_err() { continue; }
            let mut b = [false; 9];
            let mut valid = true;
            for k in 0..9 { match r.i32() { Ok(0) => b[k] = false, Ok(1) => b[k] = true, _ => { valid = false; break } } }
            if !valid { continue; }
            return Some(CompilationFlags { requires_scene_color_copy: b[0], needs_scene_textures: b[1], uses_eye_adaptation: b[2], modifies_mesh_position: b[3], uses_world_position_offset: b[4], needs_gbuffer: b[5], uses_global_distance_field: b[6], uses_pixel_depth_offset: b[7], uses_scene_depth_lookup: b[8] });
        }
    }
    None
}
