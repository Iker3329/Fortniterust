//! IR -> source translators: readable Rust and WGSL, one statement per written component.
//!
//! Register model of the generated code: every register is a vector of four `f32` holding raw 32-bit patterns (`r0`, `o0`, `x0[i]`);
//! float instructions use them directly, integer / bit instructions reinterpret them with `to_bits` / `bitcast`, exactly like the
//! interpreter in [`crate::interp`].
//!
//! Coverage: vertex, pixel and compute shaders (Rust and WGSL; geometry shaders Rust only, WebGPU has no geometry stage), structured flow
//! control (if / loop / switch), all arithmetic / integer / bit instructions of the corpus, the sample family, `ld`, `ld_ms`, `gather4`,
//! `resinfo`, derivatives, raw / structured / typed UAV access (typed textures -> WGSL storage textures) and atomics (typed buffers /
//! textures with atomics -> `array<atomic<u32>>`).  Hull / domain shaders (tessellation) are not translated.

use crate::disasm::{special_name, Ty};
use crate::ir::*;
use crate::opcode::Opcode;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenError(pub String);
impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}
impl std::error::Error for CodegenError {}
type Res<T> = Result<T, CodegenError>;
fn unsup<T>(s: impl Into<String>) -> Res<T> { Err(CodegenError(s.into())) }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend { Rust, Wgsl }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum T { F, I, U, B }

#[derive(Clone, Debug)]
struct Ex { s: String, t: T }
fn ex(s: impl Into<String>, t: T) -> Ex { Ex { s: s.into(), t } }

fn optype_name(t: u8) -> String { special_name(t).map(|s| s.to_string()).unwrap_or_else(|| format!("operand type {t}")) }

/// Support code every generated Rust shader function relies on (put it once in the module that holds the functions).
pub fn rust_prelude() -> &'static str {
    r#"/// Kind of a resource access requested by a shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op { Sample, SampleB, SampleL, SampleD, SampleC, SampleCLz, Gather4, Ld, LdMs, Lod, Resinfo, Ddx, Ddy, Uav }
/// One resource access requested by a shader.  Texture ops: `coord` is the float coordinate (`icoord` the integer one for `Ld`),
/// `arg` = lod / bias / compare reference / sample index / mip level / gather component, `Ddx`/`Ddy` carry the value in `coord`.
/// `Op::Uav` (UAV / raw / structured access and atomics): `kind` = DXBC opcode number (ld_uav_typed 163, store_uav_typed 164,
/// ld_raw 165, store_raw 166, ld_structured 167, store_structured 168, atomic_* 169..177, imm_atomic_* 178..189), `texture` = slot,
/// `sampler` = 1 for a UAV and 0 for an SRV, `icoord` = address operand lanes (structured: element, byte offset), `coord` = value
/// (raw bits), `ddx` = second value (compare value of cmp_exch), `offset[0]` = component write mask of stores.
/// The callback returns the raw (un-swizzled) result: texel / loaded words / the previous value of an atomic in `[0]`.
#[derive(Clone, Debug)]
pub struct Fetch { pub op: Op, pub texture: u32, pub sampler: u32, pub coord: [f32; 4], pub icoord: [i32; 4], pub offset: [i32; 3], pub arg: f32, pub ddx: [f32; 4], pub ddy: [f32; 4], pub kind: u32 }
/// One `emit` / `cut` of a geometry shader (`o` = the output registers at that moment).
#[derive(Clone, Debug)]
pub struct Emit { pub stream: u32, pub cut: bool, pub o: Vec<[f32; 4]> }
pub fn sat(x: f32) -> f32 { if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) } }
/// Dynamically indexed register read: out-of-range reads return 0 (D3D semantics).
pub fn at(a: &[[f32; 4]], i: i32) -> [f32; 4] { if i >= 0 && (i as usize) < a.len() { a[i as usize] } else { [0.0; 4] } }
pub fn to_f16(f: f32) -> u32 {
    let x = f.to_bits();
    let sign = (x >> 16) & 0x8000;
    let exp = ((x >> 23) & 0xff) as i32;
    let man = x & 0x7f_ffff;
    if exp == 0xff { return sign | 0x7c00 | if man != 0 { 0x200 | (man >> 13) } else { 0 }; }
    let e = exp - 127 + 15;
    if e >= 0x1f { return sign | 0x7c00; }
    if e <= 0 {
        if e < -10 { return sign; }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let mut h = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && h & 1 == 1) { h += 1; }
        return sign | h;
    }
    let mut h = sign | ((e as u32) << 10) | (man >> 13);
    let rem = man & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && h & 1 == 1) { h += 1; }
    h
}
pub fn from_f16(h: u32) -> f32 {
    let sign = (h & 0x8000) << 16;
    let exp = (h >> 10) & 0x1f;
    let man = h & 0x3ff;
    let bits = if exp == 0 {
        if man == 0 { sign } else { let mut e = 127 - 15 + 1; let mut m = man; while m & 0x400 == 0 { m <<= 1; e -= 1; } sign | ((e as u32) << 23) | ((m & 0x3ff) << 13) }
    } else if exp == 0x1f { sign | 0x7f80_0000 | (man << 13) } else { sign | ((exp + 127 - 15) << 23) | (man << 13) };
    f32::from_bits(bits)
}
"#
}

fn comp_name(c: usize) -> char { ['x', 'y', 'z', 'w'][c] }

fn f_lit(be: Backend, bits: u32) -> String {
    let e = (bits >> 23) & 0xff;
    if bits == 0 { return "0.0".into(); }
    if bits == 0x8000_0000 { return match be { Backend::Rust => "(-0.0f32)".into(), Backend::Wgsl => "-0.0".into() }; }
    if (1..=254).contains(&e) {
        let f = f32::from_bits(bits);
        let mut s = format!("{f:?}");
        if !s.contains('.') && !s.contains('e') && !s.contains("inf") { s.push_str(".0"); }
        if s.contains('e') && !s.contains('.') { s = s.replacen('e', ".0e", 1); }
        return match be { Backend::Rust => if f < 0.0 { format!("({s}f32)") } else { format!("{s}f32") }, Backend::Wgsl => if f < 0.0 { format!("({s})") } else { s } };
    }
    match be { Backend::Rust => format!("f32::from_bits({bits:#010x})"), Backend::Wgsl => format!("bitcast<f32>({bits:#010x}u)") }
}

struct Gen<'a> {
    be: Backend,
    sh: &'a Shader,
    out: String,
    ind: usize,
    sw_depth: Vec<usize>,
    sw_count: usize,
    nout: usize,
    nin: usize,
    nvert: usize,
    has_depth: bool,
    ret_expr: String,
    tmp: usize,
    /// UAV slots that are accessed with atomics (WGSL needs `atomic<u32>` storage for them)
    atomic_uavs: HashSet<u32>,
    /// UAV slots whose hidden counter is used
    counter_uavs: HashSet<u32>,
    /// Rust name of the immediate constant buffer constant
    icb_name: String,
    /// texture slots sampled with a comparison sampler (WGSL: depth textures)
    depth_slots: HashSet<u32>,
    /// groupshared memory slots accessed with atomics
    tg_atomic: HashSet<u32>,
    /// WGSL binding of every typed UAV slot (decided once by [`uav_modes`]; the declarations and the accesses both read it)
    uav_modes: HashMap<u32, UavMode>,
    /// typed-buffer SRV slots bound one word per element ([`WgslOptions::r32_srv_buffers`])
    r32_srv: HashSet<u32>,
}

/// WGSL binding of a typed UAV (`dcl_uav_typed_*`).  Raw / structured UAVs keep the `array<u32>` storage buffer and are not listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UavMode {
    /// typed buffer: `array<vec4<T>>` (one vec4 per element), or one 32-bit word per element when the format is single-channel R32
    /// (`r32`: an atomic touches it, or [`WgslOptions::r32_buffers`] lists it): `array<atomic<u32>>` with atomics, `array<u32>` without
    Buffer { atomic: bool, r32: bool },
    /// typed texture: `texture_storage_{1d,2d,2d_array,3d}<format, access>`; stores outside the texture are dropped like D3D11 does
    Texture { format: String, access: &'static str },
    /// typed texture used with atomics (WGSL has no texture atomics): `array<atomic<u32>>` of the texels in x-fastest order plus the size
    /// `u<slot>_size: vec4<u32>` (width, height, depth / layers, 0) as a uniform at group 3, binding 32 + slot
    AtomicTexture,
}

/// Options of [`translate_wgsl_with`].
#[derive(Clone, Debug, Default)]
pub struct WgslOptions {
    /// storage-texture format per typed-UAV slot (WGSL texel format name, e.g. `rgba16float`); slots not listed get [`default_uav_format`]
    pub uav_formats: Vec<(u32, String)>,
    /// typed-buffer UAV slots whose format is R32_UINT / R32_SINT / R32_FLOAT: bound one word per element (`array<u32>`), the layout the
    /// atomic passes and indirect dispatches see for the same GPU buffer.  The shader does not carry the format (the C++ side picks it).
    pub r32_buffers: Vec<u32>,
    /// the same for typed-buffer SRV slots (`Buffer<uint>` of an R32 format read with `ld`)
    pub r32_srv_buffers: Vec<u32>,
}

/// CHOSEN default storage format of a typed texture UAV: the DXBC declares only the return type of a typed UAV, the texel format is
/// picked by the C++ side when it creates the texture, so it is not in the shader.  A slot that is LOADED (`ld_uav_typed`) must be
/// R32_FLOAT / R32_UINT / R32_SINT (D3D11 typed-UAV-load rule, which is also the only `read_write` set of core WebGPU).  Otherwise:
/// float -> `rgba16float` (UE 4.16 PF_FloatRGBA, the format of the volumetric-fog VBuffers / light scattering / integrated scattering
/// and the histogram target; PF_G16R16F / PF_R16F targets such as the DF-shadow output are not storage formats in core WebGPU, so they
/// also get rgba16float = the same per-channel precision), unorm -> `rgba8unorm`, snorm -> `rgba8snorm`, uint -> `rgba32uint`,
/// sint -> `rgba32sint`.  Override per slot with [`WgslOptions::uav_formats`] to match the texture the renderer allocates.
pub fn default_uav_format(ret: u8, loaded: bool) -> &'static str {
    match (ret, loaded) {
        (4, true) => "r32uint", (3, true) => "r32sint", (_, true) => "r32float",
        (1, _) => "rgba8unorm", (2, _) => "rgba8snorm", (4, _) => "rgba32uint", (3, _) => "rgba32sint", _ => "rgba16float",
    }
}

/// How every typed UAV slot of `sh` is bound in WGSL (see [`UavMode`]).
pub fn uav_modes(sh: &Shader, opts: &WgslOptions) -> HashMap<u32, UavMode> {
    use Opcode::*;
    let (mut atomic, mut loaded, mut stored) = (HashSet::new(), HashSet::new(), HashSet::new());
    for i in sh.body() {
        let slot = |k: usize| i.operands.get(k).filter(|o| o.is(OperandType::UAV)).and_then(|o| o.reg_index());
        match i.op {
            AtomicAnd | AtomicOr | AtomicXor | AtomicCmpStore | AtomicIadd | AtomicImax | AtomicImin | AtomicUmax | AtomicUmin => { if let Some(s) = slot(0) { atomic.insert(s); } }
            ImmAtomicIadd | ImmAtomicAnd | ImmAtomicOr | ImmAtomicXor | ImmAtomicExch | ImmAtomicCmpExch | ImmAtomicImax | ImmAtomicImin | ImmAtomicUmax | ImmAtomicUmin => { if let Some(s) = slot(1) { atomic.insert(s); } }
            LdUavTyped => { if let Some(s) = slot(2) { loaded.insert(s); } }
            StoreUavTyped => { if let Some(s) = slot(0) { stored.insert(s); } }
            _ => {}
        }
    }
    let mut m = HashMap::new();
    for u in &sh.decls.uavs {
        if u.kind != ResKind::Typed { continue; }
        let mode = if u.dim == 1 { UavMode::Buffer { atomic: atomic.contains(&u.slot), r32: atomic.contains(&u.slot) || opts.r32_buffers.contains(&u.slot) } } else if atomic.contains(&u.slot) { UavMode::AtomicTexture } else {
            let (l, s) = (loaded.contains(&u.slot), stored.contains(&u.slot));
            let format = opts.uav_formats.iter().find(|(k, _)| *k == u.slot).map(|(_, f)| f.clone()).unwrap_or_else(|| default_uav_format(u.ret[0], l).to_string());
            UavMode::Texture { format, access: match (l, s) { (true, true) => "read_write", (true, false) => "read", _ => "write" } }
        };
        m.insert(u.slot, mode);
    }
    m
}

/// the immediate constant buffer holds a word that has no WGSL f32 literal (WGSL keeps it as `icb_bits: array<vec4<u32>>`)
fn icb_needs_bits(sh: &Shader) -> bool { sh.decls.icb.as_ref().is_some_and(|icb| icb.iter().flatten().any(|&b| f_lit(Backend::Wgsl, b).starts_with("bitcast"))) }

/// scalar type of a WGSL texel format: `f32`, `u32` or `i32`
fn format_scalar(f: &str) -> &'static str { if f.ends_with("uint") { "u32" } else if f.ends_with("sint") { "i32" } else { "f32" } }

impl<'a> Gen<'a> {
    fn line(&mut self, s: &str) { for _ in 0..self.ind { self.out.push_str("    "); } self.out.push_str(s); self.out.push('\n'); }
    fn r(&self) -> bool { self.be == Backend::Rust }
    fn pt(&self) -> ProgramType { self.sh.program_type }
    fn tg(&self) -> [u32; 3] { self.sh.decls.thread_group.unwrap_or([1, 1, 1]) }
    fn ret_stmt(&self) -> String { if self.ret_expr.is_empty() { "return;".into() } else { format!("return {};", self.ret_expr) } }

    // ---------------------------------------------------------------- conversions
    fn conv(&self, e: Ex, want: T) -> Ex {
        use T::*;
        if e.t == want { return e; }
        let rs = self.r();
        let s = e.s;
        // BROWSER PORT: an integer immediate typed as a float lane and converted back is `bitcast<u32>(bitcast<f32>(K))` == K. For NaN / Inf bit
        // patterns (e.g. 0xffffffff) Chrome's WGSL compiler rejects the inner const-expression ("value -nan cannot be represented as 'f32'"; naga
        // accepts it), so the web build folds the round trip back to the integer literal. Native text is unchanged.
        if cfg!(target_arch = "wasm32") && !rs && matches!(want, U | I) {
            if let Some(k) = s.strip_prefix("bitcast<f32>(").and_then(|r| r.strip_suffix("u)")).filter(|k| k.starts_with("0x") && k[2..].chars().all(|c| c.is_ascii_hexdigit())) {
                return if want == U { ex(format!("{k}u"), U) } else { ex(format!("bitcast<i32>({k}u)"), I) };
            }
        }
        match (e.t, want) {
            (F, I) => ex(if rs { format!("({s}.to_bits() as i32)") } else { format!("bitcast<i32>({s})") }, I),
            (F, U) => ex(if rs { format!("{s}.to_bits()") } else { format!("bitcast<u32>({s})") }, U),
            (I, F) => ex(if rs { format!("f32::from_bits({s} as u32)") } else { format!("bitcast<f32>({s})") }, F),
            (U, F) => ex(if rs { format!("f32::from_bits({s})") } else { format!("bitcast<f32>({s})") }, F),
            (I, U) => ex(if rs { format!("({s} as u32)") } else { format!("bitcast<u32>({s})") }, U),
            (U, I) => ex(if rs { format!("({s} as i32)") } else { format!("bitcast<i32>({s})") }, I),
            (B, F) => ex(if rs { format!("f32::from_bits(if {s} {{ 0xffff_ffffu32 }} else {{ 0 }})") } else { format!("bitcast<f32>(select(0u, 0xffffffffu, {s}))") }, F),
            (B, I) => { let f = self.conv(ex(s, B), F); self.conv(f, I) }
            (B, U) => { let f = self.conv(ex(s, B), F); self.conv(f, U) }
            _ => ex(s, want),
        }
    }
    fn lit(&self, bits: u32, t: T) -> Ex {
        match t {
            T::F => ex(f_lit(self.be, bits), T::F),
            T::I if bits as i32 == i32::MIN => ex(if self.r() { "i32::MIN".to_string() } else { "(-2147483647i - 1i)".to_string() }, T::I),
            T::I => ex(if self.r() { format!("({}i32)", bits as i32) } else { format!("({}i)", bits as i32) }, T::I),
            T::U => ex(if self.r() { format!("{bits}u32") } else { format!("{bits}u") }, T::U),
            T::B => ex(if bits != 0 { "true" } else { "false" }, T::B),
        }
    }

    // ---------------------------------------------------------------- operands
    fn index_expr(&self, op: &Operand, k: usize, len: usize) -> Res<String> {
        let Some(i) = op.index.get(k) else { return Ok("0".into()) };
        match &i.rel {
            None => Ok(format!("{}", i.imm)),
            Some(rel) => {
                let e = self.conv(self.lane(rel, 0, Ty::Int)?, T::I).s;
                let max = len.saturating_sub(1);
                Ok(if self.r() { format!("({e} + {}).clamp(0, {max}) as usize", i.imm as i32) } else { format!("u32(clamp({e} + {}i, 0i, {max}i))", i.imm as i32) })
            }
        }
    }
    /// Rust only: the `i32` expression (`relative + immediate`) of a dynamically indexed dimension, for reads that must return 0 when out of range
    fn rel_i32(&self, op: &Operand, k: usize) -> Res<Option<String>> {
        let Some(i) = op.index.get(k) else { return Ok(None) };
        let Some(rel) = &i.rel else { return Ok(None) };
        let e = self.conv(self.lane(rel, 0, Ty::Int)?, T::I).s;
        Ok(Some(format!("{e}.wrapping_add({})", i.imm as i32)))
    }
    fn comp_sfx(&self, c: usize) -> String { if self.r() { format!("[{c}]") } else { format!(".{}", comp_name(c)) } }

    /// container (f32) expression of component `k` of a register operand
    fn reg_comp(&self, op: &Operand, k: usize) -> Res<String> {
        let sfx = self.comp_sfx(k);
        let rs = self.r();
        Ok(match op.ty.0 {
            OperandType::TEMP => format!("r{}{sfx}", op.reg_index().ok_or(CodegenError("relative temp index".into()))?),
            OperandType::OUTPUT => format!("o{}{sfx}", op.reg_index().ok_or(CodegenError("relative output index".into()))?),
            OperandType::INPUT => {
                if self.pt() == ProgramType::Geometry {
                    if op.index.len() != 2 { return unsup("geometry shader input without vertex index"); }
                    let vi = self.index_expr(op, 0, self.nvert.max(1))?;
                    let ri = self.index_expr(op, 1, self.nin.max(1))?;
                    return Ok(format!("v[{vi}][{ri}]{sfx}"));
                }
                if op.index.len() != 1 { return unsup("2-dimensional input (hs/ds)"); }
                if rs { if let Some(i) = self.rel_i32(op, 0)? { return Ok(format!("at(v, {i}){sfx}")); } }
                let ix = self.index_expr(op, 0, self.nin.max(1))?;
                format!("v[{ix}]{sfx}")
            }
            OperandType::CONSTANT_BUFFER => {
                let slot = op.index.first().map(|i| i.imm as u32).unwrap_or(0);
                if op.index.first().is_some_and(|i| i.rel.is_some()) { return unsup("relative constant buffer slot"); }
                if rs { if let Some(i) = self.rel_i32(op, 1)? { return Ok(format!("at(cb[{slot}], {i}){sfx}")); } }
                let size = self.sh.cbuffer(slot).map(|c| c.size as usize).unwrap_or(4096);
                let ix = self.index_expr(op, 1, size)?;
                if rs { format!("cb[{slot}][{ix}]{sfx}") } else { format!("cb{slot}[{ix}]{sfx}") }
            }
            OperandType::IMMEDIATE_CONSTANT_BUFFER => {
                if rs { if let Some(i) = self.rel_i32(op, 0)? { return Ok(format!("at(&{}, {i}){sfx}", self.icb_name)); } }
                let n = self.sh.decls.icb.as_ref().map_or(0, |v| v.len());
                let ix = self.index_expr(op, 0, n)?;
                if rs { format!("{}[{ix}]{sfx}", self.icb_name) } else if icb_needs_bits(self.sh) { format!("bitcast<vec4<f32>>(icb_bits[{ix}]){sfx}") } else { format!("icb[{ix}]{sfx}") }
            }
            OperandType::INDEXABLE_TEMP => {
                let r = op.index.first().map(|i| i.imm as u32).unwrap_or(0);
                if rs { if let Some(i) = self.rel_i32(op, 1)? { return Ok(format!("at(&x{r}, {i}){sfx}")); } }
                let n = self.sh.decls.indexable_temps.iter().find(|x| x.0 == r).map_or(1, |x| x.1 as usize);
                let ix = self.index_expr(op, 1, n)?;
                format!("x{r}[{ix}]{sfx}")
            }
            OperandType::INPUT_PRIMITIVE_ID if self.pt() == ProgramType::Geometry => {
                if rs { "f32::from_bits(prim_id)".into() } else { return unsup("geometry shader"); }
            }
            OperandType::INPUT_THREAD_ID | OperandType::INPUT_THREAD_GROUP_ID | OperandType::INPUT_THREAD_ID_IN_GROUP if self.pt() == ProgramType::Compute => {
                let name = match op.ty.0 { OperandType::INPUT_THREAD_ID => "tid", OperandType::INPUT_THREAD_GROUP_ID => "gid", _ => "ltid" };
                let k = k.min(2);
                if rs { format!("f32::from_bits({name}[{k}])") } else { format!("bitcast<f32>({name}.{})", comp_name(k)) }
            }
            OperandType::INPUT_THREAD_ID_IN_GROUP_FLATTENED if self.pt() == ProgramType::Compute => {
                let g = self.tg();
                if rs { format!("f32::from_bits(ltid[0] + ltid[1] * {} + ltid[2] * {})", g[0], g[0] * g[1]) } else { format!("bitcast<f32>(ltid.x + ltid.y * {}u + ltid.z * {}u)", g[0], g[0] * g[1]) }
            }
            t => return unsup(optype_name(t)),
        })
    }

    /// typed scalar for lane `c` of a source operand
    fn lane(&self, op: &Operand, c: usize, ty: Ty) -> Res<Ex> {
        let want = match ty { Ty::Float | Ty::Untyped => T::F, Ty::Int => T::I, Ty::Uint | Ty::Bits => T::U };
        if op.ty.0 == OperandType::IMMEDIATE32 {
            let bits = if op.imm.len() == 1 { op.imm[0] } else { op.imm[c.min(3)] };
            return Ok(self.apply_mod_imm(bits, op.modifier, want));
        }
        let k = op.src_comp(c);
        let e = ex(self.reg_comp(op, k)?, T::F);
        let e = self.conv(e, want);
        Ok(self.apply_mod(e, op.modifier))
    }
    fn apply_mod_imm(&self, bits: u32, m: Modifier, t: T) -> Ex {
        let b = match (m, t) {
            (Modifier::None, _) => bits,
            (_, T::F) => match m { Modifier::Neg => bits ^ 0x8000_0000, Modifier::Abs => bits & 0x7fff_ffff, _ => bits | 0x8000_0000 },
            (_, _) => { let v = bits as i32; (match m { Modifier::Neg => v.wrapping_neg(), Modifier::Abs => v.wrapping_abs(), _ => v.wrapping_abs().wrapping_neg() }) as u32 }
        };
        self.lit(b, t)
    }
    fn apply_mod(&self, e: Ex, m: Modifier) -> Ex {
        if m == Modifier::None { return e; }
        let rs = self.r();
        let s = e.s;
        let t = e.t;
        let abs = |s: &str| match (rs, t) { (true, T::F) => format!("{s}.abs()"), (true, T::I) => format!("{s}.wrapping_abs()"), (true, _) => s.to_string(), (false, _) => format!("abs({s})") };
        let neg = |s: &str| match (rs, t) { (true, T::F) => format!("(-{s})"), (true, _) => format!("{s}.wrapping_neg()"), (false, T::U) => format!("(0u - {s})"), (false, _) => format!("(-{s})") };
        let s2 = match m { Modifier::Neg => neg(&s), Modifier::Abs => abs(&s), _ => neg(&abs(&s)) };
        ex(s2, t)
    }

    fn dst_target(&self, op: &Operand, c: usize) -> Res<String> {
        let sfx = self.comp_sfx(c);
        Ok(match op.ty.0 {
            OperandType::TEMP => format!("r{}{sfx}", op.reg_index().ok_or(CodegenError("relative temp dst".into()))?),
            OperandType::OUTPUT => format!("o{}{sfx}", op.reg_index().ok_or(CodegenError("relative output dst".into()))?),
            OperandType::OUTPUT_DEPTH | OperandType::OUTPUT_DEPTH_GE | OperandType::OUTPUT_DEPTH_LE => format!("od{}", self.comp_sfx(0)),
            OperandType::INDEXABLE_TEMP => {
                let r = op.index.first().map(|i| i.imm as u32).unwrap_or(0);
                let n = self.sh.decls.indexable_temps.iter().find(|x| x.0 == r).map_or(1, |x| x.1 as usize);
                let ix = self.index_expr(op, 1, n)?;
                format!("x{r}[{ix}]{sfx}")
            }
            t => return unsup(format!("destination {}", optype_name(t))),
        })
    }

    // ---------------------------------------------------------------- expression builders
    fn call1(&self, rust: &str, wgsl: &str, a: &Ex, t: T) -> Ex {
        ex(if self.r() { format!("{}.{}()", paren(&a.s), rust) } else { format!("{wgsl}({})", a.s) }, t)
    }
    fn call2(&self, rust: &str, wgsl: &str, a: &Ex, b: &Ex, t: T) -> Ex {
        ex(if self.r() { format!("{}.{}({})", paren(&a.s), rust, b.s) } else { format!("{wgsl}({}, {})", a.s, b.s) }, t)
    }
    fn bin(&self, op: &str, a: &Ex, b: &Ex, t: T) -> Ex { ex(format!("({} {op} {})", a.s, b.s), t) }

    /// Result lanes of a component-wise instruction: one expression per destination.
    fn lanes(&self, ins: &Instruction, c: usize) -> Res<Vec<Ex>> {
        use Opcode::*;
        let ops = &ins.operands;
        let s = |i: usize, ty: Ty| self.lane(&ops[i], c, ty);
        let f = |i: usize| s(i, Ty::Float);
        let iv = |i: usize| s(i, Ty::Int);
        let uv = |i: usize| s(i, Ty::Uint);
        let rs = self.r();
        let one = |e: Ex| Ok(vec![e]);
        match ins.op {
            Mov => one(s(1, Ty::Untyped)?),
            Movc => {
                let cnd = self.conv(s(1, Ty::Untyped)?, T::U);
                let (a, b) = (s(2, Ty::Untyped)?, s(3, Ty::Untyped)?);
                one(ex(if rs { format!("if {} != 0 {{ {} }} else {{ {} }}", cnd.s, a.s, b.s) } else { format!("select({}, {}, {} != 0u)", b.s, a.s, cnd.s) }, T::F))
            }
            Swapc => {
                // swapc dst0, dst1, cond, a, b: dst0 = cond ? b : a ; dst1 = cond ? a : b
                let cnd = self.conv(s(2, Ty::Untyped)?, T::U);
                let (a, b) = (s(3, Ty::Untyped)?, s(4, Ty::Untyped)?);
                let mk = |x: &Ex, y: &Ex| ex(if rs { format!("if {} != 0 {{ {} }} else {{ {} }}", cnd.s, y.s, x.s) } else { format!("select({}, {}, {} != 0u)", x.s, y.s, cnd.s) }, T::F);
                Ok(vec![mk(&a, &b), mk(&b, &a)])
            }
            Add => one(self.bin("+", &f(1)?, &f(2)?, T::F)),
            Mul => one(self.bin("*", &f(1)?, &f(2)?, T::F)),
            Div => one(self.bin("/", &f(1)?, &f(2)?, T::F)),
            Mad => { let p = self.bin("*", &f(1)?, &f(2)?, T::F); one(self.bin("+", &p, &f(3)?, T::F)) }
            Min => one(self.call2("min", "min", &f(1)?, &f(2)?, T::F)),
            Max => one(self.call2("max", "max", &f(1)?, &f(2)?, T::F)),
            Rsq => { let a = f(1)?; one(if rs { ex(format!("(1.0 / {}.sqrt())", paren(&a.s)), T::F) } else { ex(format!("inverseSqrt({})", a.s), T::F) }) }
            Sqrt => one(self.call1("sqrt", "sqrt", &f(1)?, T::F)),
            Rcp => { let a = f(1)?; one(ex(format!("(1.0 / {})", a.s), T::F)) }
            Exp => one(self.call1("exp2", "exp2", &f(1)?, T::F)),
            Log => one(self.call1("log2", "log2", &f(1)?, T::F)),
            Frc => { let a = f(1)?; one(if rs { ex(format!("({} - {}.floor())", a.s, paren(&a.s)), T::F) } else { ex(format!("fract({})", a.s), T::F) }) }
            RoundNe => one(self.call1("round_ties_even", "round", &f(1)?, T::F)),
            RoundNi => one(self.call1("floor", "floor", &f(1)?, T::F)),
            RoundPi => one(self.call1("ceil", "ceil", &f(1)?, T::F)),
            RoundZ => one(self.call1("trunc", "trunc", &f(1)?, T::F)),
            Dp2 | Dp3 | Dp4 => {
                let n = match ins.op { Dp2 => 2, Dp3 => 3, _ => 4 };
                let mut acc: Option<Ex> = None;
                for k in 0..n {
                    let p = self.bin("*", &self.lane(&ops[1], k, Ty::Float)?, &self.lane(&ops[2], k, Ty::Float)?, T::F);
                    acc = Some(match acc { None => p, Some(a) => self.bin("+", &a, &p, T::F) });
                }
                one(acc.unwrap())
            }
            Sincos => {
                let a = self.lane(&ops[2], c, Ty::Float)?;
                Ok(vec![self.call1("sin", "sin", &a, T::F), self.call1("cos", "cos", &a, T::F)])
            }
            Lt => one(self.bin("<", &f(1)?, &f(2)?, T::B)),
            Ge => one(self.bin(">=", &f(1)?, &f(2)?, T::B)),
            Eq => one(self.bin("==", &f(1)?, &f(2)?, T::B)),
            Ne => one(self.bin("!=", &f(1)?, &f(2)?, T::B)),
            Ieq => one(self.bin("==", &iv(1)?, &iv(2)?, T::B)),
            Ine => one(self.bin("!=", &iv(1)?, &iv(2)?, T::B)),
            Ilt => one(self.bin("<", &iv(1)?, &iv(2)?, T::B)),
            Ige => one(self.bin(">=", &iv(1)?, &iv(2)?, T::B)),
            Ult => one(self.bin("<", &uv(1)?, &uv(2)?, T::B)),
            Uge => one(self.bin(">=", &uv(1)?, &uv(2)?, T::B)),
            Itof => { let a = iv(1)?; one(ex(if rs { format!("({} as f32)", a.s) } else { format!("f32({})", a.s) }, T::F)) }
            Utof => { let a = uv(1)?; one(ex(if rs { format!("({} as f32)", a.s) } else { format!("f32({})", a.s) }, T::F)) }
            Ftoi => { let a = f(1)?; one(ex(if rs { format!("({} as i32)", a.s) } else { format!("i32({})", a.s) }, T::I)) }
            Ftou => { let a = f(1)?; one(ex(if rs { format!("({} as u32)", a.s) } else { format!("u32({})", a.s) }, T::U)) }
            And => one(self.bin("&", &s(1, Ty::Bits)?, &s(2, Ty::Bits)?, T::U)),
            Or => one(self.bin("|", &s(1, Ty::Bits)?, &s(2, Ty::Bits)?, T::U)),
            Xor => one(self.bin("^", &s(1, Ty::Bits)?, &s(2, Ty::Bits)?, T::U)),
            Not => { let a = s(1, Ty::Bits)?; one(ex(if rs { format!("(!{})", a.s) } else { format!("(~{})", a.s) }, T::U)) }
            Iadd => { let (a, b) = (iv(1)?, iv(2)?); one(if rs { ex(format!("{}.wrapping_add({})", paren(&a.s), b.s), T::I) } else { self.bin("+", &a, &b, T::I) }) }
            Ineg => { let a = iv(1)?; one(if rs { ex(format!("{}.wrapping_neg()", paren(&a.s)), T::I) } else { ex(format!("(-{})", a.s), T::I) }) }
            Imax => one(self.call2("max", "max", &iv(1)?, &iv(2)?, T::I)),
            Imin => one(self.call2("min", "min", &iv(1)?, &iv(2)?, T::I)),
            Umax => one(self.call2("max", "max", &uv(1)?, &uv(2)?, T::U)),
            Umin => one(self.call2("min", "min", &uv(1)?, &uv(2)?, T::U)),
            Ishl => { let (a, b) = (self.conv(s(1, Ty::Bits)?, T::U), uv(2)?); one(ex(if rs { format!("({} << ({} & 31))", a.s, b.s) } else { format!("({} << ({} & 31u))", a.s, b.s) }, T::U)) }
            Ishr => { let (a, b) = (iv(1)?, uv(2)?); one(ex(if rs { format!("({} >> ({} & 31))", a.s, b.s) } else { format!("({} >> ({} & 31u))", a.s, b.s) }, T::I)) }
            Ushr => { let (a, b) = (uv(1)?, uv(2)?); one(ex(if rs { format!("({} >> ({} & 31))", a.s, b.s) } else { format!("({} >> ({} & 31u))", a.s, b.s) }, T::U)) }
            Imad => {
                let (a, b, cc) = (iv(1)?, iv(2)?, iv(3)?);
                one(if rs { ex(format!("{}.wrapping_mul({}).wrapping_add({})", paren(&a.s), b.s, cc.s), T::I) } else { ex(format!("({} * {} + {})", a.s, b.s, cc.s), T::I) })
            }
            Umad => {
                let (a, b, cc) = (uv(1)?, uv(2)?, uv(3)?);
                one(if rs { ex(format!("{}.wrapping_mul({}).wrapping_add({})", paren(&a.s), b.s, cc.s), T::U) } else { ex(format!("({} * {} + {})", a.s, b.s, cc.s), T::U) })
            }
            Imul | Umul => {
                let signed = ins.op == Imul;
                let t = if signed { T::I } else { T::U };
                let ty = if signed { Ty::Int } else { Ty::Uint };
                let (a, b) = (s(2, ty)?, s(3, ty)?);
                let hi_used = !ops[0].is(OperandType::NULL);
                let lo = if rs { ex(format!("{}.wrapping_mul({})", paren(&a.s), b.s), t) } else { ex(format!("({} * {})", a.s, b.s), t) };
                let hi = if !hi_used { ex("0u", T::U) } else if rs {
                    if signed { ex(format!("((({} as i64) * ({} as i64)) >> 32) as i32", a.s, b.s), T::I) } else { ex(format!("((({} as u64) * ({} as u64)) >> 32) as u32", a.s, b.s), T::U) }
                } else { return unsup("imul/umul high word in WGSL"); };
                Ok(vec![hi, lo])
            }
            Udiv => {
                let (a, b) = (s(2, Ty::Uint)?, s(3, Ty::Uint)?);
                let (q, r) = if rs {
                    (ex(format!("if {} == 0 {{ u32::MAX }} else {{ {} / {} }}", b.s, a.s, b.s), T::U), ex(format!("if {} == 0 {{ u32::MAX }} else {{ {} % {} }}", b.s, a.s, b.s), T::U))
                } else {
                    (ex(format!("select({} / {}, 0xffffffffu, {} == 0u)", a.s, b.s, b.s), T::U), ex(format!("select({} % {}, 0xffffffffu, {} == 0u)", a.s, b.s, b.s), T::U))
                };
                Ok(vec![q, r])
            }
            Bfi => {
                let (w, o, ins_v, base) = (uv(1)?, uv(2)?, uv(3)?, uv(4)?);
                one(if rs {
                    ex(format!("{{ let m = (((1u64 << ({w} & 31)) - 1) as u32) << ({o} & 31); (({ins_v} << ({o} & 31)) & m) | ({base} & !m) }}", w = w.s, o = o.s, ins_v = ins_v.s, base = base.s), T::U)
                } else {
                    ex(format!("insertBits({}, {}, {} & 31u, {} & 31u)", base.s, ins_v.s, o.s, w.s), T::U)
                })
            }
            Ubfe => {
                let (w, o, v) = (uv(1)?, uv(2)?, uv(3)?);
                one(if rs {
                    ex(format!("{{ let (w, o) = ({} & 31, {} & 31); if w == 0 {{ 0 }} else if w + o < 32 {{ ({} << (32 - w - o)) >> (32 - w) }} else {{ {} >> o }} }}", w.s, o.s, v.s, v.s), T::U)
                } else {
                    ex(format!("extractBits({}, {} & 31u, {} & 31u)", v.s, o.s, w.s), T::U)
                })
            }
            Countbits => { let a = uv(1)?; one(if rs { ex(format!("{}.count_ones()", paren(&a.s)), T::U) } else { ex(format!("countOneBits({})", a.s), T::U) }) }
            FirstbitHi => { let a = uv(1)?; one(if rs { ex(format!("if {0} == 0 {{ u32::MAX }} else {{ {0}.leading_zeros() }}", a.s), T::U) } else { ex(format!("select(31u - u32(firstLeadingBit({0})), 0xffffffffu, {0} == 0u)", a.s), T::U) }) }
            FirstbitLo => { let a = uv(1)?; one(if rs { ex(format!("if {0} == 0 {{ u32::MAX }} else {{ {0}.trailing_zeros() }}", a.s), T::U) } else { ex(format!("select(u32(firstTrailingBit({0})), 0xffffffffu, {0} == 0u)", a.s), T::U) }) }
            Bfrev => { let a = uv(1)?; one(if rs { ex(format!("{}.reverse_bits()", paren(&a.s)), T::U) } else { ex(format!("reverseBits({})", a.s), T::U) }) }
            F32toF16 => { let a = f(1)?; one(if rs { ex(format!("to_f16({})", a.s), T::U) } else { ex(format!("(pack2x16float(vec2<f32>({}, 0.0)) & 0xffffu)", a.s), T::U) }) }
            F16toF32 => { let a = uv(1)?; one(if rs { ex(format!("from_f16({})", a.s), T::F) } else { ex(format!("unpack2x16float({} & 0xffffu).x", a.s), T::F) }) }
            op => unsup(op.name()),
        }
    }

    fn reads_reg(&self, ins: &Instruction, ty: u8, idx: u32) -> bool {
        let (nd, _) = ins.op.arity();
        ins.operands.iter().skip(nd).any(|o| o.ty.0 == ty && (o.reg_index() == Some(idx) || o.reg_index().is_none() && ty == OperandType::INDEXABLE_TEMP))
            || ins.operands.iter().any(|o| o.index.iter().any(|i| i.rel.as_ref().is_some_and(|r| r.ty.0 == ty && r.reg_index() == Some(idx))))
    }

    fn write_dst(&mut self, ins: &Instruction, dsts: &[(usize, [Option<Ex>; 4])]) -> Res<()> {
        let sat = ins.saturate();
        let mut stmts: Vec<(String, Ex)> = vec![];
        let mut hazard = false;
        for (di, lanes) in dsts {
            let dst = &ins.operands[*di];
            if dst.is(OperandType::NULL) { continue; }
            for (c, l) in lanes.iter().enumerate() {
                if let Some(e) = l {
                    let mut e = self.conv(e.clone(), T::F);
                    if sat { e = ex(if self.r() { format!("sat({})", e.s) } else { format!("saturate({})", e.s) }, T::F); }
                    stmts.push((self.dst_target(dst, c)?, e));
                }
            }
            if let Some(ri) = dst.reg_index() { if self.reads_reg(ins, dst.ty.0, ri) { hazard = true; } }
            if dst.is(OperandType::INDEXABLE_TEMP) { hazard = true; }
        }
        if stmts.is_empty() { return Ok(()); }
        let multi = stmts.len() > 1;
        let mut binds: Vec<(String, String)> = vec![];
        if multi {
            for i in 0..stmts.len() {
                let s = stmts[i].1.s.clone();
                let nontrivial = s.contains(' ') || s.contains('(');
                let dup = stmts.iter().enumerate().any(|(j, o)| j != i && o.1.s == s);
                if (hazard || dup) && nontrivial {
                    if let Some((name, _)) = binds.iter().find(|(_, e)| *e == s) { stmts[i].1.s = name.clone(); continue; }
                    let name = format!("q{}", self.tmp);
                    self.tmp += 1;
                    binds.push((name.clone(), s));
                    stmts[i].1.s = name;
                }
            }
            if hazard {
                for st in stmts.iter_mut() {
                    if !binds.iter().any(|(n, _)| *n == st.1.s) && !is_literal(&st.1.s) {
                        let name = format!("q{}", self.tmp);
                        self.tmp += 1;
                        binds.push((name.clone(), st.1.s.clone()));
                        st.1.s = name;
                    }
                }
            }
        }
        let block = !binds.is_empty();
        if block { self.line("{"); self.ind += 1; }
        for (n, e) in &binds { let l = format!("let {n} = {};", strip_parens(e)); self.line(&l); }
        for (t, e) in &stmts { let l = format!("{t} = {};", strip_parens(&e.s)); self.line(&l); }
        if block { self.ind -= 1; self.line("}"); }
        Ok(())
    }

    // ---------------------------------------------------------------- texture ops
    fn tex_dim(&self, slot: u32) -> Res<u8> {
        match self.sh.resource(slot) { Some(r) => Ok(r.dim), None => unsup(format!("texture t{slot} without declaration")) }
    }

    fn coord_vec(&self, op: &Operand, n: usize, ty: Ty) -> Res<Vec<Ex>> { (0..n).map(|c| self.lane(op, c, ty)).collect() }

    fn swz_of(op: &Operand) -> [usize; 4] {
        match op.sel { Sel::Swizzle(s) => [s[0] as usize, s[1] as usize, s[2] as usize, s[3] as usize], Sel::Select1(s) => [s as usize; 4], _ => [0, 1, 2, 3] }
    }

    fn tex_result(&mut self, ins: &Instruction, expr: String, tex_op: &Operand) -> Res<()> {
        let sw = Self::swz_of(tex_op);
        let name = format!("q{}", self.tmp);
        self.tmp += 1;
        let dst = &ins.operands[0];
        let mask = dst.mask();
        let mut lanes: [Option<Ex>; 4] = [None, None, None, None];
        for c in 0..4 {
            if mask & (1 << c) != 0 { lanes[c] = Some(ex(if self.r() { format!("{name}[{}]", sw[c]) } else { format!("{name}.{}", comp_name(sw[c])) }, T::F)); }
        }
        self.line(&format!("let {name} = {expr};"));
        self.write_dst(ins, &[(0, lanes)])
    }

    #[allow(clippy::too_many_arguments)]
    fn rust_fetch(&self, op: &str, tex: u32, samp: u32, coord: &[Ex], icoord: &[Ex], offset: [i32; 3], arg: &str, ddx: Option<&[Ex]>, ddy: Option<&[Ex]>, kind: u32) -> String {
        let arr = |v: &[Ex]| { let mut p: Vec<String> = v.iter().map(|e| e.s.clone()).collect(); while p.len() < 4 { p.push("0.0".into()); } format!("[{}]", p.join(", ")) };
        let iarr = |v: &[Ex]| { let mut p: Vec<String> = v.iter().map(|e| e.s.clone()).collect(); while p.len() < 4 { p.push("0".into()); } format!("[{}]", p.join(", ")) };
        format!("tex(&Fetch {{ op: Op::{op}, texture: {tex}, sampler: {samp}, coord: {}, icoord: {}, offset: [{}, {}, {}], arg: {arg}, ddx: {}, ddy: {}, kind: {kind} }})",
            arr(coord), iarr(icoord), offset[0], offset[1], offset[2], ddx.map(arr).unwrap_or_else(|| "[0.0; 4]".into()), ddy.map(arr).unwrap_or_else(|| "[0.0; 4]".into()))
    }

    fn texture_instr(&mut self, ins: &Instruction) -> Res<()> {
        use Opcode::*;
        let ops = &ins.operands;
        let off = ins.ext.offset.map(|o| [o[0] as i32, o[1] as i32, o[2] as i32]).unwrap_or([0; 3]);
        match ins.op {
            Sample | SampleB | SampleL | SampleD | SampleC | SampleCLz | Gather4 | Lod => {
                let (tex_op, samp_op) = (&ops[2], &ops[3]);
                let slot = tex_op.reg_index().unwrap_or(0);
                let samp = samp_op.reg_index().unwrap_or(0);
                let dim = self.tex_dim(slot)?;
                if self.r() {
                    let coord = self.coord_vec(&ops[1], 4, Ty::Float)?;
                    let (opn, arg, ddx, ddy): (&str, String, Option<Vec<Ex>>, Option<Vec<Ex>>) = match ins.op {
                        Sample => ("Sample", "0.0".into(), None, None),
                        SampleB => ("SampleB", self.lane(&ops[4], 0, Ty::Float)?.s, None, None),
                        SampleL => ("SampleL", self.lane(&ops[4], 0, Ty::Float)?.s, None, None),
                        SampleC => ("SampleC", self.lane(&ops[4], 0, Ty::Float)?.s, None, None),
                        SampleCLz => ("SampleCLz", self.lane(&ops[4], 0, Ty::Float)?.s, None, None),
                        Gather4 => ("Gather4", format!("{}.0", match samp_op.sel { Sel::Select1(c) => c, _ => 0 }), None, None),
                        Lod => ("Lod", "0.0".into(), None, None),
                        _ => ("SampleD", "0.0".into(), Some(self.coord_vec(&ops[4], 4, Ty::Float)?), Some(self.coord_vec(&ops[5], 4, Ty::Float)?)),
                    };
                    let e = self.rust_fetch(opn, slot, samp, &coord, &[], off, &arg, ddx.as_deref(), ddy.as_deref(), 0);
                    return self.tex_result(ins, e, tex_op);
                }
                if ins.op == Lod { return unsup("lod"); }
                // WGSL
                let (ctype, ncoord, arr) = match dim {
                    2 => ("f32", 1, false),
                    3 => ("vec2<f32>", 2, false),
                    5 => ("vec3<f32>", 3, false),
                    6 => ("vec3<f32>", 3, false),
                    8 => ("vec2<f32>", 2, true),
                    10 => ("vec3<f32>", 3, true),
                    _ => return unsup(format!("sample from {}", crate::disasm::dim_name(dim))),
                };
                let cs = self.coord_vec(&ops[1], ncoord + arr as usize, Ty::Float)?;
                let coord = if ncoord == 1 { cs[0].s.clone() } else { format!("{ctype}({})", cs[..ncoord].iter().map(|e| e.s.clone()).collect::<Vec<_>>().join(", ")) };
                let aidx = if arr { format!(", i32(round({}))", cs[ncoord].s) } else { String::new() };
                let offs = if off != [0; 3] && matches!(dim, 3 | 5 | 8) {
                    if dim == 5 { format!(", vec3<i32>({}, {}, {})", off[0], off[1], off[2]) } else { format!(", vec2<i32>({}, {})", off[0], off[1]) }
                } else { String::new() };
                let (t, s) = (format!("t{slot}"), format!("s{samp}"));
                let depth = self.depth_slots.contains(&slot);
                if depth && matches!(ins.op, SampleB | SampleD | Gather4) { return unsup(format!("{} on a depth texture", ins.op.name())); }
                if depth && dim == 5 { return unsup("comparison sampling of a 3D texture"); }
                let e = match ins.op {
                    Sample if depth => format!("vec4<f32>(textureSample({t}, {s}, {coord}{aidx}{offs}), 0.0, 0.0, 1.0)"),
                    SampleL if depth => format!("vec4<f32>(textureSampleLevel({t}, {s}, {coord}{aidx}, i32({}){offs}), 0.0, 0.0, 1.0)", self.lane(&ops[4], 0, Ty::Float)?.s),
                    Sample => format!("textureSample({t}, {s}, {coord}{aidx}{offs})"),
                    SampleB => format!("textureSampleBias({t}, {s}, {coord}{aidx}, {}{offs})", self.lane(&ops[4], 0, Ty::Float)?.s),
                    SampleL => format!("textureSampleLevel({t}, {s}, {coord}{aidx}, {}{offs})", self.lane(&ops[4], 0, Ty::Float)?.s),
                    SampleD => {
                        let g = |o: &Operand| -> Res<String> { let v = self.coord_vec(o, ncoord, Ty::Float)?; Ok(if ncoord == 1 { v[0].s.clone() } else { format!("{ctype}({})", v.iter().map(|e| e.s.clone()).collect::<Vec<_>>().join(", ")) }) };
                        format!("textureSampleGrad({t}, {s}, {coord}{aidx}, {}, {}{offs})", g(&ops[4])?, g(&ops[5])?)
                    }
                    SampleC => format!("vec4<f32>(textureSampleCompare({t}, {s}, {coord}{aidx}, {}{offs}))", self.lane(&ops[4], 0, Ty::Float)?.s),
                    SampleCLz => format!("vec4<f32>(textureSampleCompareLevel({t}, {s}, {coord}{aidx}, {}{offs}))", self.lane(&ops[4], 0, Ty::Float)?.s),
                    _ => {
                        if !matches!(dim, 3 | 6 | 8) { return unsup(format!("gather4 from {}", crate::disasm::dim_name(dim))); }
                        let comp = match samp_op.sel { Sel::Select1(c) => c, _ => 0 };
                        format!("textureGather({comp}i, {t}, {s}, {coord}{aidx}{offs})")
                    }
                };
                self.tex_result(ins, e, tex_op)
            }
            Ld | LdMs => {
                let tex_op = &ops[2];
                let slot = tex_op.reg_index().unwrap_or(0);
                let dim = self.tex_dim(slot)?;
                let ic = self.coord_vec(&ops[1], 4, Ty::Int)?;
                if self.r() {
                    let arg = if ins.op == LdMs { format!("{} as f32", self.lane(&ops[3], 0, Ty::Int)?.s) } else { "0.0".into() };
                    let e = self.rust_fetch(if ins.op == Ld { "Ld" } else { "LdMs" }, slot, 0, &[], &ic, off, &arg, None, None, 0);
                    return self.tex_result(ins, e, tex_op);
                }
                let e = match (ins.op, dim) {
                    (Ld, 1) => {
                        let rd = self.sh.resource(slot).map(|r| r.ret[0]).unwrap_or(5);
                        let el = format!("t{slot}[{}]", self.conv(ic[0].clone(), T::U).s);
                        if self.r32_srv.contains(&slot) { format!("vec4<f32>(bitcast<f32>({el}), 0.0, 0.0, {})", if rd == 5 { "1.0" } else { "bitcast<f32>(1u)" }) } else { match rd { 4 => format!("bitcast<vec4<f32>>({el})"), 3 => format!("bitcast<vec4<f32>>({el})"), _ => el } }
                    }
                    (Ld, 2) => format!("textureLoad(t{slot}, {} + {}, {})", ic[0].s, off[0], ic[1].s),
                    (Ld, 3) => format!("textureLoad(t{slot}, vec2<i32>({}, {}) + vec2<i32>({}, {}), {})", ic[0].s, ic[1].s, off[0], off[1], ic[2].s),
                    (Ld, 5) => format!("textureLoad(t{slot}, vec3<i32>({}, {}, {}) + vec3<i32>({}, {}, {}), {})", ic[0].s, ic[1].s, ic[2].s, off[0], off[1], off[2], ic[3].s),
                    (LdMs, 4) => format!("textureLoad(t{slot}, vec2<i32>({}, {}) + vec2<i32>({}, {}), {})", ic[0].s, ic[1].s, off[0], off[1], self.lane(&ops[3], 0, Ty::Int)?.s),
                    (_, d) => return unsup(format!("{} from {}", ins.op.name(), crate::disasm::dim_name(d))),
                };
                self.tex_result(ins, e, tex_op)
            }
            Resinfo => {
                let tex_op = &ops[2];
                let slot = tex_op.reg_index().unwrap_or(0);
                let kind = (ins.token >> 11) & 3;
                if !self.r() && tex_op.is(OperandType::UAV) {
                    // storage textures have one mip level
                    let size = match (self.uav_decl(tex_op).map(|d| d.dim), self.uav_modes.get(&slot)) {
                        (Some(3), Some(UavMode::Texture { .. })) => format!("vec4<u32>(textureDimensions(u{slot}), 0u, 1u)"),
                        (Some(8), Some(UavMode::Texture { .. })) => format!("vec4<u32>(textureDimensions(u{slot}), textureNumLayers(u{slot}), 1u)"),
                        (Some(5), Some(UavMode::Texture { .. })) => format!("vec4<u32>(textureDimensions(u{slot}), 1u)"),
                        (Some(3 | 5 | 8), Some(UavMode::AtomicTexture)) => format!("vec4<u32>(u{slot}_size.xyz, 1u)"),
                        _ => return unsup("resinfo on this UAV"),
                    };
                    let e = match kind { 2 => format!("bitcast<vec4<f32>>({size})"), 1 => format!("(1.0 / vec4<f32>({size}))"), _ => format!("vec4<f32>({size})") };
                    return self.tex_result(ins, e, tex_op);
                }
                let dim = self.tex_dim(slot)?;
                if self.r() {
                    let mip = self.lane(&ops[1], 0, Ty::Uint)?;
                    let e = self.rust_fetch("Resinfo", slot, 0, &[], &[], [0; 3], &format!("{} as f32", mip.s), None, None, kind);
                    return self.tex_result(ins, e, tex_op);
                }
                let mip = self.lane(&ops[1], 0, Ty::Uint)?.s;
                let size = match dim { 3 => format!("vec4<u32>(textureDimensions(t{slot}, i32({mip})), 0u, textureNumLevels(t{slot}))"), 5 => format!("vec4<u32>(textureDimensions(t{slot}, i32({mip})), textureNumLevels(t{slot}))"), d => return unsup(format!("resinfo on {}", crate::disasm::dim_name(d))) };
                let e = match kind { 2 => format!("bitcast<vec4<f32>>({size})"), 1 => format!("(1.0 / vec4<f32>({size}))"), _ => format!("vec4<f32>({size})") };
                self.tex_result(ins, e, tex_op)
            }
            op => unsup(op.name()),
        }
    }

    // ---------------------------------------------------------------- UAV / raw / structured / atomics
    fn uav_decl(&self, op: &Operand) -> Option<&'a UavDecl> {
        let slot = op.reg_index()?;
        self.sh.decls.uavs.iter().find(|u| u.slot == slot)
    }

    fn uav_instr(&mut self, ins: &Instruction) -> Res<()> {
        use Opcode::*;
        let ops = &ins.operands;
        let code = ins.op.to_u16() as u32;
        // operand roles
        let (res_i, addr_i, val_i): (usize, Vec<usize>, Option<usize>) = match ins.op {
            LdUavTyped | LdRaw => (2, vec![1], None),
            LdStructured => (3, vec![1, 2], None),
            StoreUavTyped | StoreRaw => (0, vec![1], Some(2)),
            StoreStructured => (0, vec![1, 2], Some(3)),
            AtomicCmpStore => (0, vec![1], Some(3)),
            AtomicAnd | AtomicOr | AtomicXor | AtomicIadd | AtomicImax | AtomicImin | AtomicUmax | AtomicUmin => (0, vec![1], Some(2)),
            ImmAtomicAlloc | ImmAtomicConsume => (1, vec![], None),
            ImmAtomicCmpExch => (1, vec![2], Some(4)),
            _ => (1, vec![2], Some(3)),
        };
        let res_op = &ops[res_i];
        let slot = res_op.reg_index().unwrap_or(0);
        let is_tgsm = res_op.is(OperandType::THREAD_GROUP_SHARED_MEMORY);
        let is_uav = res_op.is(OperandType::UAV);
        let has_dst = matches!(ins.op, LdUavTyped | LdRaw | LdStructured | ImmAtomicAlloc | ImmAtomicConsume | ImmAtomicIadd | ImmAtomicAnd | ImmAtomicOr | ImmAtomicXor | ImmAtomicExch | ImmAtomicCmpExch | ImmAtomicImax | ImmAtomicImin | ImmAtomicUmax | ImmAtomicUmin);
        let is_store = matches!(ins.op, StoreUavTyped | StoreRaw | StoreStructured);
        if self.r() {
            let icoord: Vec<Ex> = addr_i.iter().map(|&i| self.lane(&ops[i], 0, Ty::Int)).collect::<Res<Vec<_>>>()?;
            // typed addresses may have several lanes (texture coordinates)
            let icoord = if matches!(ins.op, LdUavTyped | StoreUavTyped) { self.coord_vec(&ops[addr_i[0]], 4, Ty::Int)? } else { icoord };
            let val = match val_i { Some(i) if matches!(ins.op, AtomicCmpStore | ImmAtomicCmpExch) => self.coord_vec(&ops[i], 4, Ty::Untyped)?, Some(i) => self.coord_vec(&ops[i], 4, Ty::Untyped)?, None => vec![] };
            let cmp = match ins.op { AtomicCmpStore => Some(self.coord_vec(&ops[2], 4, Ty::Untyped)?), ImmAtomicCmpExch => Some(self.coord_vec(&ops[3], 4, Ty::Untyped)?), _ => None };
            let mask = if is_store { res_op.mask() as i32 } else { 0 };
            let e = self.rust_fetch("Uav", slot, if is_tgsm { 2 } else { is_uav as u32 }, &val, &icoord, [mask, 0, 0], "0.0", cmp.as_deref(), None, code);
            if has_dst {
                // loads: swizzle by the resource operand; atomics: previous value in lane 0 of the destination
                if matches!(ins.op, LdUavTyped | LdRaw | LdStructured) { return self.tex_result(ins, e, res_op); }
                let dst = &ops[0];
                let name = format!("q{}", self.tmp);
                self.tmp += 1;
                self.line(&format!("let {name} = {e};"));
                let mask = dst.mask();
                let mut lanes: [Option<Ex>; 4] = [None, None, None, None];
                for c in 0..4 { if mask & (1 << c) != 0 { lanes[c] = Some(ex(format!("{name}[0]"), T::F)); } }
                return self.write_dst(ins, &[(0, lanes)]);
            }
            self.line(&format!("{e};"));
            return Ok(());
        }
        // ---- WGSL
        let decl = if is_uav { self.uav_decl(res_op).cloned() } else { None };
        if let (Some(d), Some(m)) = (&decl, if is_uav { self.uav_modes.get(&slot).cloned() } else { None }) {
            if !matches!(m, UavMode::Buffer { .. }) { return self.uav_texture_instr(ins, d, &m, res_op, addr_i.first().copied(), val_i, has_dst); }
        }
        let u = format!("{}{}", if is_uav { "u" } else if is_tgsm { "g" } else { "t" }, slot);
        let atomic = (is_uav && self.atomic_uavs.contains(&slot)) || (is_tgsm && self.tg_atomic.contains(&slot));
        let tg_decl = if is_tgsm { self.sh.decls.tgsm.iter().find(|t| t.slot == slot) } else { None };
        let (stride_words, typed_buf) = if is_tgsm {
            match tg_decl { Some(t) => (if t.stride > 0 { Some(t.stride / 4) } else { None }, false), None => return unsup("groupshared memory without declaration") }
        } else { match &decl {
            Some(UavDecl { kind: ResKind::Structured(s), .. }) => (Some(*s / 4), false),
            Some(UavDecl { kind: ResKind::Raw, .. }) => (None, false),
            Some(UavDecl { kind: ResKind::Typed, dim: 1, .. }) => (None, true),
            Some(_) => return unsup("typed UAV texture"),
            None if !is_uav => (match self.sh.resource(slot).map(|r| r.kind) { Some(ResKind::Structured(s)) => Some(s / 4), _ => None }, self.sh.resource(slot).is_some_and(|r| r.kind == ResKind::Typed)),
            None => return unsup("UAV without declaration"),
        } };
        // word index expression of the first accessed word
        let word_base = |me: &Self| -> Res<String> {
            match ins.op {
                LdStructured | StoreStructured => {
                    let idx = me.lane(&ops[addr_i[0]], 0, Ty::Uint)?.s;
                    let off = me.lane(&ops[addr_i[1]], 0, Ty::Uint)?.s;
                    Ok(format!("({idx} * {}u + ({off} >> 2u))", stride_words.unwrap_or(1)))
                }
                _ => { let a = me.lane(&ops[addr_i[0]], 0, Ty::Uint)?.s; Ok(if typed_buf { a } else { format!("({a} >> 2u)") }) }
            }
        };
        match ins.op {
            LdRaw | LdStructured => {
                let base = word_base(self)?;
                let sw = Self::swz_of(res_op);
                let mask = ops[0].mask();
                let mut lanes: [Option<Ex>; 4] = [None, None, None, None];
                for c in 0..4 { if mask & (1 << c) != 0 {
                    let w = if atomic { format!("atomicLoad(&{u}[{base} + {}u])", sw[c]) } else { format!("{u}[{base} + {}u]", sw[c]) };
                    lanes[c] = Some(ex(format!("bitcast<f32>({w})"), T::F));
                } }
                self.write_dst(ins, &[(0, lanes)])
            }
            StoreRaw | StoreStructured => {
                let base = word_base(self)?;
                let mask = res_op.mask();
                let mut k = 0;
                for c in 0..4 { if mask & (1 << c) != 0 {
                    let v = self.conv(self.lane(&ops[val_i.unwrap()], c, Ty::Untyped)?, T::U).s;
                    if atomic { self.line(&format!("atomicStore(&{u}[{base} + {k}u], {v});")); } else { self.line(&format!("{u}[{base} + {k}u] = {v};")); }
                    k += 1;
                } }
                Ok(())
            }
            LdUavTyped => {
                if !typed_buf { return unsup("ld_uav_typed on a texture UAV"); }
                let rd = decl.as_ref().map(|d| d.ret[0]).unwrap_or(5);
                let a = word_base(self)?;
                // an R32 typed buffer holds one word per element: (value, 0, 0, 1) like a D3D load of an R32 format
                let r32 = atomic || matches!(self.uav_modes.get(&slot), Some(UavMode::Buffer { r32: true, .. }));
                let e = if r32 { format!("vec4<f32>(bitcast<f32>({}), 0.0, 0.0, {})", if atomic { format!("atomicLoad(&{u}[{a}])") } else { format!("{u}[{a}]") }, if rd == 5 { "1.0" } else { "bitcast<f32>(1u)" }) } else {
                    let el = format!("{u}[{a}]");
                    if matches!(rd, 3 | 4) { format!("bitcast<vec4<f32>>({el})") } else { el }
                };
                self.tex_result(ins, e, res_op)
            }
            StoreUavTyped => {
                if !typed_buf { return unsup("store_uav_typed on a texture UAV"); }
                let rd = decl.as_ref().map(|d| d.ret[0]).unwrap_or(5);
                let a = word_base(self)?;
                if atomic || matches!(self.uav_modes.get(&slot), Some(UavMode::Buffer { r32: true, .. })) {
                    let v = self.conv(self.lane(&ops[2], 0, Ty::Untyped)?, T::U).s;
                    self.line(&if atomic { format!("atomicStore(&{u}[{a}], {v});") } else { format!("{u}[{a}] = {v};") });
                    return Ok(());
                }
                let vals: Vec<String> = (0..4).map(|c| self.lane(&ops[2], c, Ty::Untyped).map(|e| e.s)).collect::<Res<Vec<_>>>()?;
                let v = format!("vec4<f32>({})", vals.join(", "));
                let v = match rd { 4 => format!("bitcast<vec4<u32>>({v})"), 3 => format!("bitcast<vec4<i32>>({v})"), _ => v };
                self.line(&format!("{u}[{a}] = {v};"));
                Ok(())
            }
            ImmAtomicAlloc | ImmAtomicConsume => {
                let name = format!("u{slot}_cnt");
                let e = if ins.op == ImmAtomicAlloc { format!("atomicAdd(&{name}, 1u)") } else { format!("(atomicSub(&{name}, 1u) - 1u)") };
                let mask = ops[0].mask();
                let mut lanes: [Option<Ex>; 4] = [None, None, None, None];
                for c in 0..4 { if mask & (1 << c) != 0 { lanes[c] = Some(self.conv(ex(e.clone(), T::U), T::F)); } }
                self.write_dst(ins, &[(0, lanes)])
            }
            AtomicAnd | AtomicOr | AtomicXor | AtomicIadd | AtomicUmax | AtomicUmin | AtomicCmpStore | ImmAtomicIadd | ImmAtomicAnd | ImmAtomicOr | ImmAtomicXor | ImmAtomicExch | ImmAtomicCmpExch | ImmAtomicUmax | ImmAtomicUmin => {
                if decl.is_none() && !is_tgsm { return unsup("atomic on a UAV without declaration"); }
                // typed buffers used with atomics are `array<atomic<u32>>` indexed by element (R32_UINT / R32_SINT; the signed ones of the
                // global shader cache only use iadd, which is the same bit pattern as the unsigned add)
                let a = word_base(self)?;
                let call = self.atomic_call(ins, &format!("&{u}[{a}]"), val_i.unwrap())?;
                self.atomic_result(ins, call, None, has_dst)
            }
            op => unsup(op.name()),
        }
    }

    /// WGSL call of an atomic instruction on `place` (a `&...` pointer to an `atomic<u32>`); the expression yields the previous value
    fn atomic_call(&self, ins: &Instruction, place: &str, val_i: usize) -> Res<String> {
        use Opcode::*;
        let ops = &ins.operands;
        let v = self.conv(self.lane(&ops[val_i], 0, Ty::Untyped)?, T::U).s;
        Ok(match ins.op {
            AtomicAnd | ImmAtomicAnd => format!("atomicAnd({place}, {v})"),
            AtomicOr | ImmAtomicOr => format!("atomicOr({place}, {v})"),
            AtomicXor | ImmAtomicXor => format!("atomicXor({place}, {v})"),
            AtomicIadd | ImmAtomicIadd => format!("atomicAdd({place}, {v})"),
            AtomicUmax | ImmAtomicUmax => format!("atomicMax({place}, {v})"),
            AtomicUmin | ImmAtomicUmin => format!("atomicMin({place}, {v})"),
            ImmAtomicExch => format!("atomicExchange({place}, {v})"),
            AtomicCmpStore | ImmAtomicCmpExch => {
                let cmpi = if ins.op == AtomicCmpStore { 2 } else { 3 };
                let cmp = self.conv(self.lane(&ops[cmpi], 0, Ty::Untyped)?, T::U).s;
                format!("atomicCompareExchangeWeak({place}, {cmp}, {v}).old_value")
            }
            op => return unsup(format!("{} on a UAV", op.name())),
        })
    }

    /// emits an atomic `call` (skipped when `guard` is false: out-of-bounds D3D11 UAV atomics do nothing and return 0) and writes the
    /// previous value to the destination of the `imm_atomic_*` forms
    fn atomic_result(&mut self, ins: &Instruction, call: String, guard: Option<String>, has_dst: bool) -> Res<()> {
        if !has_dst {
            match guard { Some(g) => self.line(&format!("if {g} {{ {call}; }}")), None => self.line(&format!("{call};")) }
            return Ok(());
        }
        let nm = format!("q{}", self.tmp);
        self.tmp += 1;
        match guard {
            Some(g) => { self.line(&format!("var {nm} = 0u;")); self.line(&format!("if {g} {{ {nm} = {call}; }}")); }
            None => self.line(&format!("let {nm} = {call};")),
        }
        let mask = ins.operands[0].mask();
        let mut lanes: [Option<Ex>; 4] = [None, None, None, None];
        for c in 0..4 { if mask & (1 << c) != 0 { lanes[c] = Some(self.conv(ex(nm.clone(), T::U), T::F)); } }
        self.write_dst(ins, &[(0, lanes)])
    }

    /// `ld_uav_typed` / `store_uav_typed` / atomics on a typed TEXTURE UAV (see [`UavMode`]).  D3D11 semantics: stores and atomics outside
    /// the texture are dropped, loads outside it return 0.  WGSL leaves out-of-bounds storage-texture writes implementation-defined, so every
    /// access is guarded with the texture size (D3D-SEMANTICS SHIM, not an instruction of the DXBC).
    #[allow(clippy::too_many_arguments)]
    fn uav_texture_instr(&mut self, ins: &Instruction, d: &UavDecl, mode: &UavMode, res_op: &Operand, addr_i: Option<usize>, val_i: Option<usize>, has_dst: bool) -> Res<()> {
        use Opcode::*;
        let ops = &ins.operands;
        let u = format!("u{}", d.slot);
        // number of address lanes; 2D arrays carry the layer in the last one
        let (n, layered) = match d.dim { 2 => (1, false), 3 => (2, false), 5 => (3, false), 8 => (3, true), dim => return unsup(format!("typed UAV {}", crate::disasm::dim_name(dim))) };
        let Some(ai) = addr_i else { return unsup(format!("{} on a typed UAV texture", ins.op.name())) };
        let c: Vec<String> = (0..n).map(|k| self.lane(&ops[ai], k, Ty::Uint).map(|e| self.conv(e, T::U).s)).collect::<Res<Vec<_>>>()?;
        // the coordinates are named once (the guard and the access both use them)
        let cn = format!("q{}", self.tmp);
        self.tmp += 1;
        self.line(&format!("let {cn} = {}({});", ["", "vec2<u32>", "vec3<u32>"][n - 1], c.join(", ")));
        match mode {
            UavMode::Texture { format, .. } => {
                let (coord, guard) = match (n, layered) {
                    (1, _) => (cn.clone(), format!("{cn} < textureDimensions({u})")),
                    (_, false) => (cn.clone(), format!("all({cn} < textureDimensions({u}))")),
                    _ => (format!("{cn}.xy, {cn}.z"), format!("(all({cn}.xy < textureDimensions({u})) && {cn}.z < textureNumLayers({u}))")),
                };
                let sc = format_scalar(format);
                match ins.op {
                    StoreUavTyped => {
                        let vals: Vec<String> = (0..4).map(|k| self.lane(&ops[val_i.unwrap_or(2)], k, Ty::Untyped).map(|e| e.s)).collect::<Res<Vec<_>>>()?;
                        let v = format!("vec4<f32>({})", vals.join(", "));
                        let v = if sc == "f32" { v } else { format!("bitcast<vec4<{sc}>>({v})") };
                        self.line(&format!("if {guard} {{ textureStore({u}, {coord}, {v}); }}"));
                        Ok(())
                    }
                    LdUavTyped => {
                        let l = format!("textureLoad({u}, {coord})");
                        let l = if sc == "f32" { l } else { format!("bitcast<vec4<f32>>({l})") };
                        self.tex_result(ins, format!("select(vec4<f32>(0.0), {l}, {guard})"), res_op)
                    }
                    op => unsup(format!("{} on a typed UAV texture", op.name())),
                }
            }
            _ => {
                // atomic texture: texels of the storage buffer in x-fastest order, size from `u<slot>_size`
                let sz = format!("{u}_size");
                let (idx, guard) = match n {
                    1 => (cn.clone(), format!("{cn} < {sz}.x")),
                    2 => (format!("({cn}.x + {cn}.y * {sz}.x)"), format!("all({cn} < {sz}.xy)")),
                    _ => (format!("({cn}.x + ({cn}.y + {cn}.z * {sz}.y) * {sz}.x)"), format!("all({cn} < {sz}.xyz)")),
                };
                let place = format!("&{u}[{idx}]");
                match ins.op {
                    StoreUavTyped => {
                        let v = self.conv(self.lane(&ops[val_i.unwrap_or(2)], 0, Ty::Untyped)?, T::U).s;
                        self.line(&format!("if {guard} {{ atomicStore({place}, {v}); }}"));
                        Ok(())
                    }
                    LdUavTyped => {
                        let one = if d.ret[0] == 5 { "1.0" } else { "bitcast<f32>(1u)" };
                        let nm = format!("q{}", self.tmp);
                        self.tmp += 1;
                        self.line(&format!("var {nm} = 0u;"));
                        self.line(&format!("if {guard} {{ {nm} = atomicLoad({place}); }}"));
                        self.tex_result(ins, format!("vec4<f32>(bitcast<f32>({nm}), 0.0, 0.0, {one})"), res_op)
                    }
                    _ => {
                        let call = self.atomic_call(ins, &place, val_i.unwrap())?;
                        self.atomic_result(ins, call, Some(guard), has_dst)
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- statements
    fn cond(&self, ins: &Instruction) -> Res<String> {
        let c = self.conv(self.lane(&ins.operands[0], 0, Ty::Untyped)?, T::U);
        Ok(if ins.test_nz() { format!("{} != 0{}", c.s, if self.r() { "" } else { "u" }) } else { format!("{} == 0{}", c.s, if self.r() { "" } else { "u" }) })
    }

    fn emit_call(&mut self, stream: u32, cut: bool) {
        if !self.r() { return; }
        let outs: Vec<String> = (0..self.nout).map(|i| format!("o{i}")).collect();
        self.line(&format!("emit(&Emit {{ stream: {stream}, cut: {cut}, o: vec![{}] }});", outs.join(", ")));
    }

    fn emit(&mut self) -> Res<()> {
        use Opcode::*;
        let body: Vec<&Instruction> = self.sh.body().collect();
        let mut i = 0;
        let mut open_case: Vec<bool> = vec![];
        // per open switch: (is default arm, start offset in `out`) of every arm; Rust needs the `_` arm last
        let mut sw_arms: Vec<Vec<(bool, usize)>> = vec![];
        while i < body.len() {
            let ins = body[i];
            match ins.op {
                If => { let c = self.cond(ins)?; self.line(&format!("if {c} {{")); self.ind += 1; }
                Else => { self.ind -= 1; self.line("} else {"); self.ind += 1; }
                EndIf => { self.ind -= 1; self.line("}"); }
                Loop => { self.line("loop {"); self.ind += 1; }
                EndLoop => { self.ind -= 1; self.line("}"); }
                Break => {
                    if let Some(&d) = self.sw_depth.last() {
                        if self.r() { self.line(&format!("break 'sw{d};")); } else { self.line("break;"); }
                    } else { self.line("break;"); }
                }
                Breakc => {
                    let c = self.cond(ins)?;
                    let b = if let Some(&d) = self.sw_depth.last() { if self.r() { format!("break 'sw{d};") } else { "break;".into() } } else { "break;".into() };
                    self.line(&format!("if {c} {{ {b} }}"));
                }
                Continue => self.line("continue;"),
                Continuec => { let c = self.cond(ins)?; self.line(&format!("if {c} {{ continue; }}")); }
                Ret => { let r = self.ret_stmt(); self.line(&r); }
                Retc => { let c = self.cond(ins)?; let r = self.ret_stmt(); self.line(&format!("if {c} {{ {r} }}")); }
                Discard => {
                    let c = self.cond(ins)?;
                    if self.r() { self.line(&format!("if {c} {{ return None; }}")); } else { self.line(&format!("if {c} {{ discard; }}")); }
                }
                Emit | EmitStream | EmitThenCut | EmitThenCutStream | Cut | CutStream => {
                    if self.pt() != ProgramType::Geometry { return unsup(ins.op.name()); }
                    let stream = ins.operands.first().and_then(|o| o.reg_index()).unwrap_or(0);
                    match ins.op {
                        Emit | EmitStream => self.emit_call(stream, false),
                        Cut | CutStream => self.emit_call(stream, true),
                        _ => { self.emit_call(stream, false); self.emit_call(stream, true); }
                    }
                }
                Switch => {
                    let v = self.conv(self.lane(&ins.operands[0], 0, Ty::Untyped)?, T::U);
                    let id = self.sw_count;
                    self.sw_count += 1;
                    if self.r() {
                        self.line(&format!("'sw{id}: {{"));
                        self.ind += 1;
                        self.line(&format!("match {} {{", v.s));
                    } else { self.line(&format!("switch {} {{", v.s)); }
                    self.ind += 1;
                    self.sw_depth.push(id);
                    open_case.push(false);
                    sw_arms.push(vec![]);
                }
                Case | Default => {
                    let mut labels: Vec<String> = vec![];
                    let mut j = i;
                    let mut is_default = false;
                    while j < body.len() && matches!(body[j].op, Case | Default) {
                        if body[j].op == Default { is_default = true; } else { labels.push(format!("{}{}", body[j].operands[0].imm.first().copied().unwrap_or(0), if self.r() { "u32" } else { "u" })); }
                        j += 1;
                    }
                    if let Some(o) = open_case.last_mut() {
                        if *o { self.ind -= 1; self.line("}"); }
                        *o = true;
                    }
                    if let Some(a) = sw_arms.last_mut() { a.push((is_default, self.out.len())); }
                    let head = if self.r() {
                        if is_default { "_".to_string() } else { labels.join(" | ") }
                    } else if is_default { if labels.is_empty() { "default".into() } else { format!("case {}, default", labels.join(", ")) } } else { format!("case {}", labels.join(", ")) };
                    self.line(&format!("{head}{}", if self.r() { " => {" } else { ": {" }));
                    self.ind += 1;
                    i = j;
                    continue;
                }
                EndSwitch => {
                    if open_case.pop() == Some(true) { self.ind -= 1; self.line("}"); }
                    let arms = sw_arms.pop().unwrap_or_default();
                    if self.r() {
                        match arms.iter().position(|a| a.0) {
                            Some(di) => {
                                // move the default arm behind the others
                                if di + 1 < arms.len() {
                                    let (s, e) = (arms[di].1, arms[di + 1].1);
                                    let text: String = self.out[s..e].to_string();
                                    self.out.replace_range(s..e, "");
                                    self.out.push_str(&text);
                                }
                            }
                            None => self.line("_ => {}"),
                        }
                        self.ind -= 1;
                        self.line("}");
                        self.ind -= 1;
                        self.line("}");
                    } else {
                        self.ind -= 1;
                        self.line("}");
                    }
                    self.sw_depth.pop();
                }
                Nop => {}
                Sync => {
                    if !self.r() { self.line("storageBarrier();"); self.line("workgroupBarrier();"); }
                }
                Sample | SampleB | SampleL | SampleD | SampleC | SampleCLz | Gather4 | Lod | Ld | LdMs | Resinfo => self.texture_instr(ins)?,
                LdUavTyped | StoreUavTyped | LdRaw | StoreRaw | LdStructured | StoreStructured | AtomicAnd | AtomicOr | AtomicXor | AtomicCmpStore | AtomicIadd | AtomicImax | AtomicImin | AtomicUmax | AtomicUmin
                | ImmAtomicAlloc | ImmAtomicConsume | ImmAtomicIadd | ImmAtomicAnd | ImmAtomicOr | ImmAtomicXor | ImmAtomicExch | ImmAtomicCmpExch | ImmAtomicImax | ImmAtomicImin | ImmAtomicUmax | ImmAtomicUmin => {
                    if !self.r() && matches!(ins.op, AtomicImax | AtomicImin | ImmAtomicImax | ImmAtomicImin) { return unsup(ins.op.name()); }
                    self.uav_instr(ins)?
                }
                DerivRtx | DerivRty | DerivRtxCoarse | DerivRtxFine | DerivRtyCoarse | DerivRtyFine => {
                    let dst = &ins.operands[0];
                    let mask = dst.mask();
                    let name = if matches!(ins.op, DerivRtx | DerivRtxCoarse | DerivRtxFine) { "x" } else { "y" };
                    if self.r() {
                        let v = self.coord_vec(&ins.operands[1], 4, Ty::Float)?;
                        let e = self.rust_fetch(if name == "x" { "Ddx" } else { "Ddy" }, 0, 0, &v, &[], [0; 3], "0.0", None, None, 0);
                        let nm = format!("q{}", self.tmp);
                        self.tmp += 1;
                        self.line(&format!("let {nm} = {e};"));
                        let mut lanes: [Option<Ex>; 4] = [None, None, None, None];
                        for c in 0..4 { if mask & (1 << c) != 0 { lanes[c] = Some(ex(format!("{nm}[{c}]"), T::F)); } }
                        self.write_dst(ins, &[(0, lanes)])?;
                    } else {
                        let fn_name = match ins.op { DerivRtxCoarse => "dpdxCoarse", DerivRtxFine => "dpdxFine", DerivRtyCoarse => "dpdyCoarse", DerivRtyFine => "dpdyFine", DerivRtx => "dpdx", _ => "dpdy" };
                        let mut lanes: [Option<Ex>; 4] = [None, None, None, None];
                        for c in 0..4 { if mask & (1 << c) != 0 { lanes[c] = Some(ex(format!("{fn_name}({})", self.lane(&ins.operands[1], c, Ty::Float)?.s), T::F)); } }
                        self.write_dst(ins, &[(0, lanes)])?;
                    }
                }
                op if op.is_decl() || op == CustomData => {}
                op => {
                    let (nd, _) = op.arity();
                    if nd == 0 { return unsup(op.name()); }
                    let mut dsts: Vec<(usize, [Option<Ex>; 4])> = (0..nd).map(|d| (d, [None, None, None, None])).collect();
                    for c in 0..4usize {
                        let needed = (0..nd).any(|d| ins.operands[d].mask() & (1 << c) != 0);
                        if !needed { continue; }
                        let r = self.lanes(ins, c)?;
                        for (d, e) in r.into_iter().enumerate() {
                            if d < nd && ins.operands[d].mask() & (1 << c) != 0 { dsts[d].1[c] = Some(e); }
                        }
                    }
                    self.write_dst(ins, &dsts)?;
                }
            }
            i += 1;
        }
        Ok(())
    }
}

fn is_literal(s: &str) -> bool { s.parse::<f64>().is_ok() || s.starts_with("f32::from_bits(0x") && !s.contains(' ') && s.ends_with(')') && s.matches('(').count() == 1 }
fn paren(s: &str) -> String {
    if s.starts_with('(') && s.ends_with(')') && balanced_outer(s) { return s.to_string(); }
    if s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '[' || c == ']') { return s.to_string(); }
    format!("({s})")
}
fn balanced_outer(s: &str) -> bool {
    let mut d = 0;
    for (i, c) in s.char_indices() {
        if c == '(' { d += 1; } else if c == ')' { d -= 1; if d == 0 && i != s.len() - 1 { return false; } }
    }
    d == 0
}
fn strip_parens(s: &str) -> String {
    let mut t = s;
    while t.starts_with('(') && t.ends_with(')') && balanced_outer(t) { t = &t[1..t.len() - 1]; }
    t.to_string()
}

fn check_supported(sh: &Shader, be: Backend) -> Res<()> {
    match sh.program_type {
        ProgramType::Pixel | ProgramType::Vertex | ProgramType::Compute => Ok(()),
        ProgramType::Geometry if be == Backend::Rust => Ok(()),
        ProgramType::Geometry => unsup("program type gs (no WGSL geometry stage)"),
        p => unsup(format!("program type {} (tessellation)", p.prefix())),
    }
}

fn new_gen<'a>(sh: &'a Shader, be: Backend, name: &str) -> Gen<'a> {
    let gs = sh.program_type == ProgramType::Geometry;
    let nin = sh.decls.inputs.iter().filter(|d| d.file == OperandType::INPUT).map(|d| (if gs { d.reg2.unwrap_or(d.reg) } else { d.reg }) as usize + 1).max().unwrap_or(0);
    let nvert = if gs {
        sh.decls.inputs.iter().filter_map(|d| if d.reg2.is_some() { Some(d.reg as usize) } else { None }).max().unwrap_or_else(|| match sh.decls.gs_input_primitive { Some(1) => 1, Some(2) => 2, Some(3) => 3, Some(6) => 4, Some(7) => 6, _ => 3 })
    } else { 0 };
    let nout = sh.decls.outputs.iter().filter(|d| d.file == OperandType::OUTPUT).map(|d| d.reg as usize + 1).max().unwrap_or(0);
    let has_depth = sh.decls.outputs.iter().any(|d| matches!(d.file, OperandType::OUTPUT_DEPTH | OperandType::OUTPUT_DEPTH_GE | OperandType::OUTPUT_DEPTH_LE));
    let mut names: Vec<String> = (0..nout).map(|i| format!("o{i}")).collect();
    if has_depth { names.push("od".into()); }
    let ret = match (sh.program_type, be) {
        (ProgramType::Compute | ProgramType::Geometry, _) => String::new(),
        (_, Backend::Rust) => format!("Some([{}])", names.join(", ")),
        (_, Backend::Wgsl) => format!("array<vec4<f32>, {}>({})", names.len().max(1), if names.is_empty() { "vec4<f32>(0.0)".to_string() } else { names.join(", ") }),
    };
    // UAV slots touched by atomics / counters
    let mut atomic_uavs = HashSet::new();
    let mut counter_uavs = HashSet::new();
    let mut depth_slots = HashSet::new();
    let mut tg_atomic = HashSet::new();
    for i in sh.body() {
        use Opcode::*;
        let uav_slot = |k: usize| i.operands.get(k).and_then(|o| o.reg_index());
        let is_tg = |k: usize| i.operands.get(k).is_some_and(|o| o.is(OperandType::THREAD_GROUP_SHARED_MEMORY));
        match i.op {
            AtomicAnd | AtomicOr | AtomicXor | AtomicCmpStore | AtomicIadd | AtomicImax | AtomicImin | AtomicUmax | AtomicUmin => { if let Some(s) = uav_slot(0) { if is_tg(0) { tg_atomic.insert(s); } else { atomic_uavs.insert(s); } } }
            ImmAtomicIadd | ImmAtomicAnd | ImmAtomicOr | ImmAtomicXor | ImmAtomicExch | ImmAtomicCmpExch | ImmAtomicImax | ImmAtomicImin | ImmAtomicUmax | ImmAtomicUmin => { if let Some(s) = uav_slot(1) { if is_tg(1) { tg_atomic.insert(s); } else { atomic_uavs.insert(s); } } }
            ImmAtomicAlloc | ImmAtomicConsume => { if let Some(s) = uav_slot(1) { counter_uavs.insert(s); } }
            SampleC | SampleCLz | Gather4C | Gather4PoC => { if let Some(s) = uav_slot(2) { depth_slots.insert(s); } }
            _ => {}
        }
    }
    Gen { be, sh, out: String::new(), ind: 1, sw_depth: vec![], sw_count: 0, nout, nin, nvert, has_depth, ret_expr: ret, tmp: 0, atomic_uavs, counter_uavs, depth_slots, tg_atomic, uav_modes: HashMap::new(), r32_srv: HashSet::new(), icb_name: format!("{}_ICB", name.to_uppercase()) }
}

fn rust_regs_decl(sh: &Shader, g: &Gen) -> String {
    let mut head = String::new();
    for i in 0..sh.decls.temps { let _ = writeln!(head, "    let mut r{i} = [0.0f32; 4];"); }
    for i in 0..g.nout { let _ = writeln!(head, "    let mut o{i} = [0.0f32; 4];"); }
    if g.has_depth { let _ = writeln!(head, "    let mut od = [0.0f32; 4];"); }
    for (r, n, _) in &sh.decls.indexable_temps { let _ = writeln!(head, "    let mut x{r} = [[0.0f32; 4]; {n}];"); }
    head
}

/// Readable Rust function for a vertex / pixel / compute / geometry shader.  Needs [`rust_prelude`] in scope.
///
/// * VS / PS: `fn NAME(v: &[[f32; 4]; NIN], cb: &[&[[f32; 4]]], tex: &mut dyn FnMut(&Fetch) -> [f32; 4]) -> Option<[[f32; 4]; NOUT]>`
///   (`None` = discard; a shader writing `oDepth` returns one extra register whose `.x` is the depth)
/// * CS: `fn NAME(tid: [u32; 3], gid: [u32; 3], ltid: [u32; 3], cb, tex)`
/// * GS: `fn NAME(v: &[[[f32; 4]; NIN]; NVERT], prim_id: u32, cb, tex, emit: &mut dyn FnMut(&Emit))`
pub fn translate_rust(sh: &Shader, name: &str) -> std::result::Result<String, CodegenError> {
    check_supported(sh, Backend::Rust)?;
    let mut g = new_gen(sh, Backend::Rust, name);
    for d in &sh.decls.outputs { if !matches!(d.file, OperandType::OUTPUT | OperandType::OUTPUT_DEPTH | OperandType::OUTPUT_DEPTH_GE | OperandType::OUTPUT_DEPTH_LE) { return unsup(format!("output {}", optype_name(d.file))); } }
    let mut head = String::new();
    let _ = writeln!(head, "/// {} shader, {} temps.", sh.version_string(), sh.decls.temps);
    let _ = writeln!(head, "#[allow(unused_mut, unused_variables, unused_assignments, unused_parens, clippy::all)]");
    match sh.program_type {
        ProgramType::Compute => { let _ = writeln!(head, "pub fn {name}(tid: [u32; 3], gid: [u32; 3], ltid: [u32; 3], cb: &[&[[f32; 4]]], tex: &mut dyn FnMut(&Fetch) -> [f32; 4]) {{"); }
        ProgramType::Geometry => { let _ = writeln!(head, "pub fn {name}(v: &[[[f32; 4]; {}]; {}], prim_id: u32, cb: &[&[[f32; 4]]], tex: &mut dyn FnMut(&Fetch) -> [f32; 4], emit: &mut dyn FnMut(&Emit)) {{", g.nin, g.nvert); }
        _ => { let _ = writeln!(head, "pub fn {name}(v: &[[f32; 4]; {}], cb: &[&[[f32; 4]]], tex: &mut dyn FnMut(&Fetch) -> [f32; 4]) -> Option<[[f32; 4]; {}]> {{", g.nin, g.nout + g.has_depth as usize); }
    }
    head.push_str(&rust_regs_decl(sh, &g));
    g.emit()?;
    let mut s = head;
    // a trailing `ret` becomes the tail expression of the function
    let ret_line = format!("    {}\n", g.ret_stmt());
    let ends_with_ret = g.out.ends_with(&ret_line);
    if ends_with_ret { g.out.truncate(g.out.len() - ret_line.len()); }
    s.push_str(&g.out);
    if !g.ret_expr.is_empty() { let _ = writeln!(s, "    {}", g.ret_expr); }
    s.push_str("}\n");
    if let Some(icb) = &sh.decls.icb {
        let rows: Vec<String> = icb.iter().map(|r| format!("[{}]", r.iter().map(|&b| f_lit(Backend::Rust, b)).collect::<Vec<_>>().join(", "))).collect();
        let _ = write!(s, "\n#[allow(dead_code)]\nconst {}: [[f32; 4]; {}] = [\n    {},\n];\n", g.icb_name, icb.len(), rows.join(",\n    "));
    }
    Ok(s)
}

fn wgsl_tex_type(dim: u8, depth: bool) -> Option<&'static str> {
    if depth {
        return Some(match dim { 3 => "texture_depth_2d", 6 => "texture_depth_cube", 8 => "texture_depth_2d_array", 10 => "texture_depth_cube_array", _ => return None });
    }
    Some(match dim { 3 => "texture_2d<f32>", 5 => "texture_3d<f32>", 6 => "texture_cube<f32>", 8 => "texture_2d_array<f32>", 10 => "texture_cube_array<f32>", 2 => "texture_1d<f32>", 4 => "texture_multisampled_2d<f32>", _ => return None })
}
fn wgsl_elem(ret: u8) -> &'static str { match ret { 4 => "vec4<u32>", 3 => "vec4<i32>", _ => "vec4<f32>" } }

/// WGSL module: uniform arrays `cb<slot>` (group 0), textures / SRV buffers `t<slot>` (group 1), samplers `s<slot>` (group 2),
/// UAVs `u<slot>` (group 3: storage buffers, storage textures, see [`UavMode`]) and their counters `u<slot>_cnt` (group 3, binding 64 + slot)
/// / atomic-texture sizes `u<slot>_size` (group 3, binding 32 + slot), the translated function and an entry point `NAME_main` that maps the
/// declared inputs / outputs to `@location`s / builtins.  Typed texture UAVs get [`default_uav_format`]; see [`translate_wgsl_with`].
pub fn translate_wgsl(sh: &Shader, name: &str) -> std::result::Result<String, CodegenError> { translate_wgsl_with(sh, name, &WgslOptions::default()) }

/// [`translate_wgsl`] with per-slot storage-texture formats.
pub fn translate_wgsl_with(sh: &Shader, name: &str, opts: &WgslOptions) -> std::result::Result<String, CodegenError> {
    check_supported(sh, Backend::Wgsl)?;
    let mut g = new_gen(sh, Backend::Wgsl, name);
    g.uav_modes = uav_modes(sh, opts);
    g.r32_srv = opts.r32_srv_buffers.iter().copied().collect();
    g.emit()?;
    let compute = sh.program_type == ProgramType::Compute;
    let mut s = String::new();
    let _ = writeln!(s, "// {} shader translated from DXBC (fn-dxbc). Registers are vec4<f32> holding raw bit patterns.", sh.version_string());
    // SV_PrimitiveID in a pixel shader needs the WebGPU `primitive-index` feature
    if sh.program_type == ProgramType::Pixel && sh.decls.inputs.iter().any(|d| d.file == OperandType::INPUT && d.sysvalue == Some(7)) { let _ = writeln!(s, "enable primitive_index;"); }
    let _ = writeln!(s, "diagnostic(off, derivative_uniformity);");
    let mut cbs: Vec<_> = sh.decls.cbuffers.iter().collect();
    cbs.sort_by_key(|c| c.slot);
    for c in cbs { let _ = writeln!(s, "@group(0) @binding({}) var<uniform> cb{}: array<vec4<f32>, {}>;", c.slot, c.slot, c.size.max(1)); }
    let used_res: HashSet<u32> = sh.body().flat_map(|i| i.operands.iter()).filter(|o| o.is(OperandType::RESOURCE)).filter_map(|o| o.reg_index()).collect();
    let mut texs: Vec<_> = sh.decls.resources.iter().collect();
    texs.sort_by_key(|r| r.slot);
    for r in texs {
        if !used_res.contains(&r.slot) { continue; }
        match (r.kind, r.dim) {
            (ResKind::Raw, _) | (ResKind::Structured(_), _) => { let _ = writeln!(s, "@group(1) @binding({}) var<storage, read> t{}: array<u32>;", r.slot, r.slot); }
            (_, 1) => { let _ = writeln!(s, "@group(1) @binding({}) var<storage, read> t{}: array<{}>;", r.slot, r.slot, if g.r32_srv.contains(&r.slot) { "u32" } else { wgsl_elem(r.ret[0]) }); }
            _ => match wgsl_tex_type(r.dim, g.depth_slots.contains(&r.slot)) { Some(t) => { let _ = writeln!(s, "@group(1) @binding({}) var t{}: {};", r.slot, r.slot, t); } None => return unsup(format!("resource dimension {}", crate::disasm::dim_name(r.dim))) },
        }
    }
    let mut smps: Vec<_> = sh.decls.samplers.iter().collect();
    smps.sort_by_key(|r| r.slot);
    for r in smps { let _ = writeln!(s, "@group(2) @binding({}) var s{}: {};", r.slot, r.slot, if r.mode == 1 { "sampler_comparison" } else { "sampler" }); }
    let used_uav: HashSet<u32> = sh.body().flat_map(|i| i.operands.iter()).filter(|o| o.is(OperandType::UAV)).filter_map(|o| o.reg_index()).collect();
    let mut uavs: Vec<_> = sh.decls.uavs.iter().collect();
    uavs.sort_by_key(|r| r.slot);
    for u in uavs {
        if !used_uav.contains(&u.slot) { continue; }
        let atomic = g.atomic_uavs.contains(&u.slot);
        match (u.kind, u.dim) {
            (ResKind::Raw | ResKind::Structured(_), _) => { let _ = writeln!(s, "@group(3) @binding({}) var<storage, read_write> u{}: array<{}>;", u.slot, u.slot, if atomic { "atomic<u32>" } else { "u32" }); }
            (ResKind::Typed, 1) => { let _ = writeln!(s, "@group(3) @binding({}) var<storage, read_write> u{}: array<{}>;", u.slot, u.slot, if atomic { "atomic<u32>" } else if matches!(g.uav_modes.get(&u.slot), Some(UavMode::Buffer { r32: true, .. })) { "u32" } else { wgsl_elem(u.ret[0]) }); }
            (ResKind::Typed, dim) => match g.uav_modes.get(&u.slot) {
                Some(UavMode::Texture { format, access }) => {
                    if *access == "read_write" && !format.starts_with("r32") { return unsup(format!("read_write storage texture u{} needs an r32 format, not {format} (WebGPU)", u.slot)); }
                    let ty = match dim { 2 => "texture_storage_1d", 3 => "texture_storage_2d", 5 => "texture_storage_3d", 8 => "texture_storage_2d_array", _ => return unsup(format!("typed UAV {}", crate::disasm::dim_name(dim))) };
                    let _ = writeln!(s, "@group(3) @binding({}) var u{}: {ty}<{format}, {access}>;", u.slot, u.slot);
                }
                Some(UavMode::AtomicTexture) => {
                    let _ = writeln!(s, "@group(3) @binding({}) var<storage, read_write> u{}: array<atomic<u32>>;", u.slot, u.slot);
                    let _ = writeln!(s, "@group(3) @binding({}) var<uniform> u{}_size: vec4<u32>;", 32 + u.slot, u.slot);
                }
                _ => return unsup("typed UAV texture"),
            },
        }
        if g.counter_uavs.contains(&u.slot) { let _ = writeln!(s, "@group(3) @binding({}) var<storage, read_write> u{}_cnt: atomic<u32>;", 64 + u.slot, u.slot); }
    }
    for t in &sh.decls.tgsm {
        if !sh.body().any(|i| i.operands.iter().any(|o| o.is(OperandType::THREAD_GROUP_SHARED_MEMORY) && o.reg_index() == Some(t.slot))) { continue; }
        let words = if t.stride == 0 { t.count.div_ceil(4) } else { t.stride / 4 * t.count }.max(1);
        let _ = writeln!(s, "var<workgroup> g{}: array<{}, {}>;", t.slot, if g.tg_atomic.contains(&t.slot) { "atomic<u32>" } else { "u32" }, words);
    }
    if let Some(icb) = &sh.decls.icb {
        if icb_needs_bits(sh) {
            // NaN / Inf / denormal words are not f32 const-expressions (`bitcast` is not one in naga / Tint): keep the raw bits
            let rows: Vec<String> = icb.iter().map(|r| format!("vec4<u32>({})", r.iter().map(|b| format!("{b:#010x}u")).collect::<Vec<_>>().join(", "))).collect();
            let _ = writeln!(s, "var<private> icb_bits: array<vec4<u32>, {}> = array<vec4<u32>, {}>(\n    {});", icb.len(), icb.len(), rows.join(",\n    "));
        } else {
            let rows: Vec<String> = icb.iter().map(|r| format!("vec4<f32>({})", r.iter().map(|&b| f_lit(Backend::Wgsl, b)).collect::<Vec<_>>().join(", "))).collect();
            let _ = writeln!(s, "var<private> icb: array<vec4<f32>, {}> = array<vec4<f32>, {}>(\n    {});", icb.len(), icb.len(), rows.join(",\n    "));
        }
    }
    let _ = writeln!(s);
    if compute {
        let _ = writeln!(s, "fn {name}(tid: vec3<u32>, gid: vec3<u32>, ltid: vec3<u32>) {{");
    } else {
        let params = if g.nin > 0 { format!("v: array<vec4<f32>, {}>", g.nin) } else { String::new() };
        let nret = g.nout + g.has_depth as usize;
        let _ = writeln!(s, "fn {name}({params}) -> array<vec4<f32>, {}> {{", nret.max(1));
    }
    for i in 0..sh.decls.temps { let _ = writeln!(s, "    var r{i} = vec4<f32>(0.0);"); }
    if !compute {
        for i in 0..g.nout { let _ = writeln!(s, "    var o{i} = vec4<f32>(0.0);"); }
        if g.has_depth { let _ = writeln!(s, "    var od = vec4<f32>(0.0);"); }
        if g.nout == 0 && !g.has_depth { let _ = writeln!(s, "    var o0 = vec4<f32>(0.0);"); }
    }
    for (r, n, _) in &sh.decls.indexable_temps { let _ = writeln!(s, "    var x{r}: array<vec4<f32>, {n}>;"); }
    s.push_str(&g.out);
    if !compute {
        let fin = if g.nout == 0 && !g.has_depth { "    return array<vec4<f32>, 1>(o0);\n".to_string() } else { format!("    return {};\n", g.ret_expr) };
        // the program's own trailing `ret` is already the return statement of the function
        let own = format!("    {}\n", g.ret_stmt());
        if !(g.out.ends_with(&own) && g.nout + g.has_depth as usize > 0) { s.push_str(&fin); }
    }
    s.push_str("}\n\n");
    s.push_str(&wgsl_entry(sh, name, &g)?);
    Ok(s)
}

fn wgsl_entry(sh: &Shader, name: &str, g: &Gen) -> Res<String> {
    if sh.program_type == ProgramType::Compute {
        let t = g.tg();
        return Ok(format!("@compute @workgroup_size({}, {}, {})\nfn {name}_main(@builtin(global_invocation_id) tid: vec3<u32>, @builtin(workgroup_id) gid: vec3<u32>, @builtin(local_invocation_id) ltid: vec3<u32>) {{\n    {name}(tid, gid, ltid);\n}}\n", t[0], t[1], t[2]));
    }
    let pixel = sh.program_type == ProgramType::Pixel;
    let mut args: Vec<String> = vec![];
    let mut init = String::new();
    let mut seen_in = HashSet::new();
    for d in &sh.decls.inputs {
        if d.file != OperandType::INPUT || !seen_in.insert(d.reg) { continue; }
        let vn = format!("in{}", d.reg);
        match d.sysvalue {
            Some(1) if pixel => { args.push(format!("@builtin(position) {vn}: vec4<f32>")); let _ = writeln!(init, "    v[{}] = {vn};", d.reg); }
            Some(9) if pixel => { args.push(format!("@builtin(front_facing) {vn}: bool")); let _ = writeln!(init, "    v[{}] = vec4<f32>(bitcast<f32>(select(0u, 0xffffffffu, {vn})), 0.0, 0.0, 0.0);", d.reg); }
            Some(10) if pixel => { args.push(format!("@builtin(sample_index) {vn}: u32")); let _ = writeln!(init, "    v[{}] = vec4<f32>(bitcast<f32>({vn}), 0.0, 0.0, 0.0);", d.reg); }
            Some(7) if pixel => { args.push(format!("@builtin(primitive_index) {vn}: u32")); let _ = writeln!(init, "    v[{}] = vec4<f32>(bitcast<f32>({vn}), 0.0, 0.0, 0.0);", d.reg); }
            Some(6) if !pixel => { args.push(format!("@builtin(vertex_index) {vn}: u32")); let _ = writeln!(init, "    v[{}] = vec4<f32>(bitcast<f32>({vn}), 0.0, 0.0, 0.0);", d.reg); }
            Some(8) if !pixel => { args.push(format!("@builtin(instance_index) {vn}: u32")); let _ = writeln!(init, "    v[{}] = vec4<f32>(bitcast<f32>({vn}), 0.0, 0.0, 0.0);", d.reg); }
            _ => {
                // plain varying / vertex attribute; system values without a WGSL builtin (render target array index, ...) are passed as flat locations
                let interp = match (pixel, d.sysvalue.is_some(), d.interp) { (true, true, _) | (true, _, 1) => "@interpolate(flat) ", (true, _, 4) => "@interpolate(linear) ", (true, _, 5) => "@interpolate(linear, centroid) ", (true, _, 3) => "@interpolate(perspective, centroid) ", _ => "" };
                args.push(format!("@location({}) {interp}{vn}: vec4<f32>", d.reg));
                let _ = writeln!(init, "    v[{}] = {vn};", d.reg);
            }
        }
    }
    let mut outs: Vec<String> = vec![];
    let mut build = String::new();
    let mut has_position = false;
    let mut seen_out = HashSet::new();
    let mut depth_done = false;
    for d in &sh.decls.outputs {
        if d.file == OperandType::OUTPUT && !seen_out.insert(d.reg) { continue; }
        if matches!(d.file, OperandType::OUTPUT_DEPTH | OperandType::OUTPUT_DEPTH_GE | OperandType::OUTPUT_DEPTH_LE) { if depth_done { continue; } depth_done = true; }
        match d.file {
            OperandType::OUTPUT => match d.sysvalue {
                Some(1) if !pixel => { has_position = true; outs.push(format!("@builtin(position) out{}: vec4<f32>", d.reg)); let _ = writeln!(build, "    r.out{} = o[{}];", d.reg, d.reg); }
                Some(s) => return unsup(format!("output system value {}", crate::disasm::sysvalue_long(s))),
                None => { outs.push(format!("@location({}) out{}: vec4<f32>", d.reg, d.reg)); let _ = writeln!(build, "    r.out{} = o[{}];", d.reg, d.reg); }
            },
            OperandType::OUTPUT_DEPTH | OperandType::OUTPUT_DEPTH_GE | OperandType::OUTPUT_DEPTH_LE => {
                outs.push("@builtin(frag_depth) depth: f32".into());
                let _ = writeln!(build, "    r.depth = o[{}].x;", g.nout);
            }
            t => return unsup(format!("output {}", optype_name(t))),
        }
    }
    if !pixel && !has_position {
        // a vertex shader feeding a geometry shader / stream-out has no SV_Position; WebGPU needs one
        outs.insert(0, "@builtin(position) out_position: vec4<f32>".into());
    }
    let mut s = String::new();
    let ret_ty = format!("{}Out", camel(name));
    let _ = writeln!(s, "struct {ret_ty} {{");
    for o in &outs { let _ = writeln!(s, "    {o},"); }
    if outs.is_empty() { let _ = writeln!(s, "    @location(0) dummy: vec4<f32>,"); }
    let _ = writeln!(s, "}}\n");
    let _ = writeln!(s, "@{} fn {name}_main({}) -> {ret_ty} {{", if pixel { "fragment" } else { "vertex" }, args.join(", "));
    if g.nin > 0 { let _ = writeln!(s, "    var v: array<vec4<f32>, {}>;", g.nin); }
    s.push_str(&init);
    let call_args = if g.nin > 0 { "v" } else { "" };
    let _ = writeln!(s, "    let o = {name}({call_args});");
    let _ = writeln!(s, "    var r: {ret_ty};");
    s.push_str(&build);
    if !pixel && !has_position { let _ = writeln!(s, "    r.out_position = vec4<f32>(0.0);"); }
    let _ = writeln!(s, "    return r;\n}}");
    Ok(s)
}

fn camel(n: &str) -> String {
    let mut out = String::new();
    let mut up = true;
    for c in n.chars() { if c == '_' || !c.is_alphanumeric() { up = true; } else if up { out.extend(c.to_uppercase()); up = false; } else { out.push(c); } }
    out
}

/// A Rust module text holding [`rust_prelude`], the translated shaders and a registry (`ENTRIES`) of uniform-signature wrappers for
/// the vertex / pixel shaders among them.  Returns the text and the shaders that could not be translated (name, reason).
pub fn translate_rust_module(shaders: &[(String, &Shader)]) -> (String, Vec<(String, CodegenError)>) {
    let mut text = String::new();
    let mut failed = vec![];
    let mut entries = vec![];
    text.push_str("// GENERATED by fn-dxbc (`translate_rust_module`). Do not edit.\n#![allow(dead_code, clippy::all, unused_mut, unused_variables, unused_assignments, unused_parens, unreachable_code)]\n\n");
    text.push_str(rust_prelude());
    text.push('\n');
    for (name, sh) in shaders {
        match translate_rust(sh, name) {
            Ok(code) => {
                text.push_str(&code);
                text.push('\n');
                if matches!(sh.program_type, ProgramType::Pixel | ProgramType::Vertex) {
                    let g = new_gen(sh, Backend::Rust, name);
                    entries.push((name.clone(), g.nin, g.nout + g.has_depth as usize));
                }
            }
            Err(e) => failed.push((name.clone(), e)),
        }
    }
    text.push_str("pub type Run = fn(&[[f32; 4]], &[&[[f32; 4]]], &mut dyn FnMut(&Fetch) -> [f32; 4]) -> Option<Vec<[f32; 4]>>;\n");
    text.push_str("/// (name, number of input registers, number of output registers, entry)\npub static ENTRIES: &[(&str, usize, usize, Run)] = &[\n");
    for (n, nin, nout) in entries {
        let _ = writeln!(text, "    (\"{n}\", {nin}, {nout}, |v, cb, tex| {n}(v.try_into().unwrap(), cb, tex).map(|o| o.to_vec())),");
    }
    text.push_str("];\n");
    (text, failed)
}










