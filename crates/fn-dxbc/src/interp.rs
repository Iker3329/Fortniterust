//! CPU interpreter for decoded SM4/SM5 programs (pixel and vertex shaders; flow control, resource fetches through [`Env`]).
//!
//! Registers hold raw 32-bit patterns; every instruction reinterprets them as float / int / uint exactly like the GPU does,
//! so integer and bit instructions (`and`, `ftou`, `ishl`, `bfi`, ...) are bit exact.

use crate::disasm::{operand_ty, Ty};
use crate::ir::*;
use crate::opcode::Opcode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexOp { Sample, SampleB, SampleL, SampleD, SampleC, SampleCLz, Ld, LdMs, Lod, Gather4, Gather4C, Gather4Po, Gather4PoC, Resinfo, Bufinfo, SampleInfo }

/// One resource fetch.  `coord` holds the float coordinate (`Ld`: the integer coordinate bits, use `icoord`).
#[derive(Clone, Debug)]
pub struct TexReq {
    pub op: TexOp,
    pub texture: u32,
    /// sampler slot (0 when the instruction has none)
    pub sampler: u32,
    pub coord: [f32; 4],
    pub icoord: [i32; 4],
    pub offset: [i32; 3],
    /// `sample_l` lod / `sample_b` bias / `sample_c` reference / `ld_ms` sample index (as float) / `resinfo` mip level
    pub arg: f32,
    pub ddx: [f32; 4],
    pub ddy: [f32; 4],
    /// resource return type of the instruction (`resinfo`: 0 float, 1 rcpFloat, 2 uint)
    pub resinfo_kind: u32,
}

/// Environment of a shader invocation: textures and screen-space derivatives.
pub trait Env {
    /// Returns the raw (un-swizzled) result of the fetch.
    fn texture(&mut self, _req: &TexReq) -> [f32; 4] { [0.0; 4] }
    fn ddx(&mut self, v: [f32; 4]) -> [f32; 4] { let _ = v; [0.0; 4] }
    fn ddy(&mut self, v: [f32; 4]) -> [f32; 4] { let _ = v; [0.0; 4] }
}
/// An [`Env`] that returns zeros for everything.
pub struct NullEnv;
impl Env for NullEnv {}

#[derive(Debug, Clone, PartialEq)]
pub enum InterpError { Unsupported(String), Limit }
impl std::fmt::Display for InterpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { InterpError::Unsupported(s) => write!(f, "unsupported: {s}"), InterpError::Limit => write!(f, "step limit exceeded") }
    }
}
impl std::error::Error for InterpError {}

#[derive(Clone, Debug, Default)]
pub struct Outputs {
    pub discarded: bool,
    /// output registers by index (`oN`), as floats
    pub o: Vec<[f32; 4]>,
    pub depth: Option<f32>,
    pub coverage: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// treat `x / 0` as `0` (what the old generated Rust did) instead of IEEE infinity / NaN
    pub div_zero_is_zero: bool,
    /// skip `discard` instructions (the old generated Rust dropped the Slate clip-quad test)
    pub ignore_discard: bool,
    /// round float immediates to the six decimals a `%f` listing prints (what the old text-based translators saw)
    pub imm_decimal6: bool,
    /// `_sat` of a NaN stays NaN (Rust `f32::clamp`, what the old generated code did) instead of becoming 0 (D3D)
    pub saturate_nan_propagates: bool,
    pub max_steps: u64,
}
impl Default for Options { fn default() -> Self { Options { div_zero_is_zero: false, ignore_discard: false, imm_decimal6: false, saturate_nan_propagates: false, max_steps: 20_000_000 } } }

pub struct Machine<'s> {
    sh: &'s Shader,
    body: Vec<&'s Instruction>,
    pub opts: Options,
    pub v: Vec<[u32; 4]>,
    pub cb: Vec<Vec<[u32; 4]>>,
    temps: Vec<[u32; 4]>,
    xtemps: Vec<Vec<[u32; 4]>>,
    out: Vec<[u32; 4]>,
    depth: Option<u32>,
    coverage: Option<u32>,
    specials: Vec<(u8, [u32; 4])>,
    /// when `Some`, a snapshot of the temp registers is appended after every executed instruction (debugging aid)
    pub trace: Option<Vec<Vec<[u32; 4]>>>,
    // flow tables (indices into `body`)
    jump: Vec<usize>,
}

fn to_f(b: [u32; 4]) -> [f32; 4] { [f32::from_bits(b[0]), f32::from_bits(b[1]), f32::from_bits(b[2]), f32::from_bits(b[3])] }
fn from_f(f: [f32; 4]) -> [u32; 4] { [f[0].to_bits(), f[1].to_bits(), f[2].to_bits(), f[3].to_bits()] }
const TRUE: u32 = 0xffff_ffff;
fn bl(b: bool) -> u32 { if b { TRUE } else { 0 } }

pub fn f32_to_f16(f: f32) -> u16 {
    let x = f.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let man = x & 0x7f_ffff;
    if exp == 0xff { return sign | 0x7c00 | if man != 0 { 0x200 | (man >> 13) as u16 } else { 0 }; }
    let e = exp - 127 + 15;
    if e >= 0x1f { return sign | 0x7c00; }
    if e <= 0 {
        if e < -10 { return sign; }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let mut h = (m >> shift) as u16;
        let rem = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && h & 1 == 1) { h += 1; }
        return sign | h;
    }
    let mut h = sign | ((e as u16) << 10) | (man >> 13) as u16;
    let rem = man & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && h & 1 == 1) { h += 1; }
    h
}
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h as u32) & 0x8000) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if man == 0 { sign } else {
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x400 == 0 { m <<= 1; e -= 1; }
            sign | ((e as u32) << 23) | ((m & 0x3ff) << 13)
        }
    } else if exp == 0x1f { sign | 0x7f80_0000 | (man << 13) } else { sign | ((exp + 127 - 15) << 23) | (man << 13) };
    f32::from_bits(bits)
}

impl<'s> Machine<'s> {
    pub fn new(sh: &'s Shader) -> Machine<'s> {
        let body: Vec<&Instruction> = sh.body().collect();
        let nin = sh.decls.inputs.iter().map(|d| d.reg + 1).max().unwrap_or(0) as usize;
        let nout = sh.decls.outputs.iter().map(|d| d.reg + 1).max().unwrap_or(0) as usize;
        let mut cb = vec![];
        for c in &sh.decls.cbuffers {
            if cb.len() <= c.slot as usize { cb.resize(c.slot as usize + 1, vec![]); }
            cb[c.slot as usize] = vec![[0; 4]; c.size as usize];
        }
        let xtemps = {
            let mut v: Vec<Vec<[u32; 4]>> = vec![];
            for (r, n, _) in &sh.decls.indexable_temps { if v.len() <= *r as usize { v.resize(*r as usize + 1, vec![]); } v[*r as usize] = vec![[0; 4]; *n as usize]; }
            v
        };
        let mut m = Machine { sh, body, opts: Options::default(), v: vec![[0; 4]; nin], cb, temps: vec![[0; 4]; sh.decls.temps as usize], xtemps, out: vec![[0; 4]; nout], depth: None, coverage: None, specials: vec![], trace: None, jump: vec![] };
        m.jump = m.build_jumps();
        m
    }

    pub fn shader(&self) -> &Shader { self.sh }
    pub fn set_input(&mut self, reg: usize, v: [f32; 4]) {
        if self.v.len() <= reg { self.v.resize(reg + 1, [0; 4]); }
        self.v[reg] = from_f(v);
    }
    pub fn set_input_bits(&mut self, reg: usize, v: [u32; 4]) {
        if self.v.len() <= reg { self.v.resize(reg + 1, [0; 4]); }
        self.v[reg] = v;
    }
    pub fn set_cb(&mut self, slot: usize, data: &[[f32; 4]]) {
        if self.cb.len() <= slot { self.cb.resize(slot + 1, vec![]); }
        self.cb[slot] = data.iter().map(|&x| from_f(x)).collect();
    }
    /// value of a special input register (`vThreadID`, `vPrim`, ...) by operand type
    pub fn set_special(&mut self, ty: u8, v: [u32; 4]) { self.specials.retain(|(t, _)| *t != ty); self.specials.push((ty, v)); }

    fn build_jumps(&self) -> Vec<usize> {
        // for if: index of matching else/endif; else: matching endif; loop: endloop; endloop: loop; switch: endswitch;
        // break/continue are resolved at run time through `enclosing`
        use Opcode::*;
        let n = self.body.len();
        let mut jump = vec![usize::MAX; n];
        let mut stack: Vec<usize> = vec![];
        for (i, ins) in self.body.iter().enumerate() {
            match ins.op {
                If | Loop | Switch => stack.push(i),
                Else => { if let Some(&t) = stack.last() { jump[t] = i; stack.push(i); } }
                EndIf => {
                    if let Some(t) = stack.pop() {
                        if self.body[t].op == Else { jump[t] = i; if let Some(f) = stack.pop() { if jump[f] == usize::MAX || self.body[jump[f]].op != Else { jump[f] = i; } } } else { jump[t] = i; }
                    }
                }
                EndLoop => { if let Some(t) = stack.pop() { jump[t] = i; jump[i] = t; } }
                EndSwitch => { if let Some(t) = stack.pop() { jump[t] = i; jump[i] = t; } }
                _ => {}
            }
        }
        jump
    }

    // ---- operand access

    fn reg_raw(&self, op: &Operand) -> [u32; 4] {
        let idx = |k: usize| -> i64 {
            match op.index.get(k) {
                None => 0,
                Some(i) => {
                    let mut v = i.imm as i64;
                    if let Some(r) = &i.rel { v += self.fetch(r, Ty::Int)[0] as i32 as i64; }
                    v
                }
            }
        };
        let get = |v: &Vec<[u32; 4]>, i: i64| -> [u32; 4] { if i >= 0 { v.get(i as usize).copied().unwrap_or([0; 4]) } else { [0; 4] } };
        match op.ty.0 {
            OperandType::TEMP => get(&self.temps, idx(0)),
            OperandType::INPUT => { let k = if op.index.len() > 1 { 1 } else { 0 }; get(&self.v, idx(k)) }
            OperandType::OUTPUT => get(&self.out, idx(0)),
            OperandType::INDEXABLE_TEMP => self.xtemps.get(idx(0) as usize).map(|x| get(x, idx(1))).unwrap_or([0; 4]),
            OperandType::CONSTANT_BUFFER => self.cb.get(idx(0) as usize).map(|x| get(x, idx(1))).unwrap_or([0; 4]),
            OperandType::IMMEDIATE_CONSTANT_BUFFER => match &self.sh.decls.icb { Some(v) if idx(0) >= 0 => v.get(idx(0) as usize).copied().unwrap_or([0; 4]), _ => [0; 4] },
            OperandType::NULL => [0; 4],
            t => self.specials.iter().find(|(k, _)| *k == t).map(|(_, v)| *v).unwrap_or([0; 4]),
        }
    }

    fn fetch(&self, op: &Operand, ty: Ty) -> [u32; 4] {
        let mut out = if op.ty.0 == OperandType::IMMEDIATE32 {
            let mut im = if op.imm.len() == 1 { [op.imm[0]; 4] } else { [op.imm[0], op.imm[1], op.imm[2], op.imm[3]] };
            if self.opts.imm_decimal6 && matches!(ty, Ty::Float | Ty::Untyped) {
                for c in im.iter_mut() {
                    let normal = (1..=254).contains(&((*c >> 23) & 0xff));
                    if ty == Ty::Float || normal { *c = format!("{:.6}", f32::from_bits(*c) as f64).parse::<f32>().unwrap_or(0.0).to_bits(); }
                }
            }
            im
        } else {
            let raw = self.reg_raw(op);
            match op.sel {
                Sel::Swizzle(s) => [raw[s[0] as usize], raw[s[1] as usize], raw[s[2] as usize], raw[s[3] as usize]],
                Sel::Select1(s) => [raw[s as usize]; 4],
                _ => raw,
            }
        };
        if op.modifier != Modifier::None {
            let is_int = matches!(ty, Ty::Int | Ty::Uint);
            for c in out.iter_mut() {
                *c = if is_int {
                    let v = *c as i32;
                    match op.modifier { Modifier::Neg => v.wrapping_neg() as u32, Modifier::Abs => v.wrapping_abs() as u32, Modifier::AbsNeg => v.wrapping_abs().wrapping_neg() as u32, Modifier::None => *c }
                } else {
                    match op.modifier { Modifier::Neg => *c ^ 0x8000_0000, Modifier::Abs => *c & 0x7fff_ffff, Modifier::AbsNeg => *c | 0x8000_0000, Modifier::None => *c }
                };
            }
        }
        out
    }

    fn write(&mut self, dst: &Operand, vals: [u32; 4], sat: bool) {
        let mask = match dst.sel { Sel::Mask(m) => m, _ => 0xf };
        let mut v = vals;
        if sat {
            let keep_nan = self.opts.saturate_nan_propagates;
            for c in v.iter_mut() {
                let f = f32::from_bits(*c);
                *c = (if f.is_nan() { if keep_nan { f } else { 0.0 } } else { f.clamp(0.0, 1.0) }).to_bits();
            }
        }
        let idx = |me: &Self, k: usize| -> i64 {
            match dst.index.get(k) { None => 0, Some(i) => { let mut x = i.imm as i64; if let Some(r) = &i.rel { x += me.fetch(r, Ty::Int)[0] as i32 as i64; } x } }
        };
        let (slot, arr): (i64, Option<&mut Vec<[u32; 4]>>) = match dst.ty.0 {
            OperandType::TEMP => { let i = idx(self, 0); (i, Some(&mut self.temps)) }
            OperandType::OUTPUT => { let i = idx(self, 0); (i, Some(&mut self.out)) }
            OperandType::INDEXABLE_TEMP => { let r = idx(self, 0); let e = idx(self, 1); (e, self.xtemps.get_mut(r as usize)) }
            OperandType::OUTPUT_DEPTH | OperandType::OUTPUT_DEPTH_GE | OperandType::OUTPUT_DEPTH_LE => { self.depth = Some(v[0]); return; }
            OperandType::OUTPUT_COVERAGE_MASK => { self.coverage = Some(v[0]); return; }
            _ => return,
        };
        if let Some(a) = arr {
            if slot < 0 { return; }
            let s = slot as usize;
            if s >= a.len() { if dst.ty.0 == OperandType::OUTPUT { a.resize(s + 1, [0; 4]); } else { return; } }
            for c in 0..4 { if mask & (1 << c) != 0 { a[s][c] = v[c]; } }
        }
    }

    fn texture_slot(op: &Operand) -> (u32, [usize; 4]) {
        let sw = match op.sel { Sel::Swizzle(s) => [s[0] as usize, s[1] as usize, s[2] as usize, s[3] as usize], Sel::Select1(s) => [s as usize; 4], _ => [0, 1, 2, 3] };
        (op.reg_index().unwrap_or(0), sw)
    }

    /// Runs the program once.
    pub fn run(&mut self, env: &mut dyn Env) -> Result<Outputs, InterpError> {
        use Opcode::*;
        self.temps.iter_mut().for_each(|t| *t = [0; 4]);
        for x in self.xtemps.iter_mut() { x.iter_mut().for_each(|t| *t = [0; 4]); }
        self.out.iter_mut().for_each(|t| *t = [0; 4]);
        let mut discarded = false;
        let n = self.body.len();
        let mut pc = 0usize;
        let mut steps = 0u64;
        // loop / switch context stack: (kind, begin index)
        let mut ctx: Vec<(Opcode, usize)> = vec![];
        let mut switch_val: Vec<(u32, bool)> = vec![]; // value, matched-already
        while pc < n {
            steps += 1;
            if steps > self.opts.max_steps { return Err(InterpError::Limit); }
            let ins = self.body[pc];
            let sat = ins.saturate();
            let ops = &ins.operands;
            let ty = |i: usize| operand_ty(ins, i);
            macro_rules! src { ($i:expr) => { self.fetch(&ops[$i], ty($i)) }; }
            macro_rules! srcf { ($i:expr) => { to_f(self.fetch(&ops[$i], Ty::Float)) }; }
            macro_rules! f1 { ($f:expr) => {{ let a = srcf!(1); let f: &dyn Fn(f32) -> f32 = &$f; self.write(&ops[0], from_f([f(a[0]), f(a[1]), f(a[2]), f(a[3])]), sat); }}; }
            macro_rules! f2 { ($f:expr) => {{ let a = srcf!(1); let b = srcf!(2); let f: &dyn Fn(f32, f32) -> f32 = &$f; self.write(&ops[0], from_f([f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2]), f(a[3], b[3])]), sat); }}; }
            macro_rules! cmpf { ($f:expr) => {{ let a = srcf!(1); let b = srcf!(2); let f: &dyn Fn(f32, f32) -> bool = &$f; self.write(&ops[0], [bl(f(a[0], b[0])), bl(f(a[1], b[1])), bl(f(a[2], b[2])), bl(f(a[3], b[3]))], false); }}; }
            macro_rules! u2 { ($f:expr) => {{ let a = src!(1); let b = src!(2); let f: &dyn Fn(u32, u32) -> u32 = &$f; self.write(&ops[0], [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2]), f(a[3], b[3])], false); }}; }
            macro_rules! u1 { ($f:expr) => {{ let a = src!(1); let f: &dyn Fn(u32) -> u32 = &$f; self.write(&ops[0], [f(a[0]), f(a[1]), f(a[2]), f(a[3])], false); }}; }
            macro_rules! test { () => {{ let c = src!(0)[0]; if ins.test_nz() { c != 0 } else { c == 0 } }}; }
            let mut next = pc + 1;
            match ins.op {
                Mov => { let a = src!(1); self.write(&ops[0], a, sat); }
                Movc => { let c = src!(1); let a = src!(2); let b = src!(3); self.write(&ops[0], [if c[0] != 0 { a[0] } else { b[0] }, if c[1] != 0 { a[1] } else { b[1] }, if c[2] != 0 { a[2] } else { b[2] }, if c[3] != 0 { a[3] } else { b[3] }], sat); }
                Swapc => {
                    let c = src!(2); let a = src!(3); let b = src!(4);
                    let (mut d0, mut d1) = ([0u32; 4], [0u32; 4]);
                    for k in 0..4 { if c[k] != 0 { d0[k] = b[k]; d1[k] = a[k]; } else { d0[k] = a[k]; d1[k] = b[k]; } }
                    if !ops[0].is(OperandType::NULL) { self.write(&ops[0], d0, false); }
                    if !ops[1].is(OperandType::NULL) { self.write(&ops[1], d1, false); }
                }
                Add => f2!(|x, y| x + y),
                Mul => f2!(|x, y| x * y),
                Div => { let z = self.opts.div_zero_is_zero; f2!(|x, y| if z && y == 0.0 { 0.0 } else { x / y }) }
                Min => f2!(|x: f32, y: f32| x.min(y)),
                Max => f2!(|x: f32, y: f32| x.max(y)),
                Mad => { let a = srcf!(1); let b = srcf!(2); let c = srcf!(3); self.write(&ops[0], from_f([a[0] * b[0] + c[0], a[1] * b[1] + c[1], a[2] * b[2] + c[2], a[3] * b[3] + c[3]]), sat); }
                Dp2 => { let a = srcf!(1); let b = srcf!(2); let s = a[0] * b[0] + a[1] * b[1]; self.write(&ops[0], from_f([s; 4]), sat); }
                Dp3 => { let a = srcf!(1); let b = srcf!(2); let s = a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; self.write(&ops[0], from_f([s; 4]), sat); }
                Dp4 => { let a = srcf!(1); let b = srcf!(2); let s = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]; self.write(&ops[0], from_f([s; 4]), sat); }
                Rsq => f1!(|x: f32| 1.0 / x.sqrt()),
                Sqrt => f1!(|x: f32| x.sqrt()),
                Rcp => f1!(|x: f32| 1.0 / x),
                Exp => f1!(|x: f32| x.exp2()),
                Log => f1!(|x: f32| x.log2()),
                Frc => f1!(|x: f32| x - x.floor()),
                RoundNe => f1!(|x: f32| x.round_ties_even()),
                RoundNi => f1!(|x: f32| x.floor()),
                RoundPi => f1!(|x: f32| x.ceil()),
                RoundZ => f1!(|x: f32| x.trunc()),
                Sincos => {
                    let a = srcf!(2);
                    let s = [a[0].sin(), a[1].sin(), a[2].sin(), a[3].sin()];
                    let c = [a[0].cos(), a[1].cos(), a[2].cos(), a[3].cos()];
                    if !ops[0].is(OperandType::NULL) { self.write(&ops[0], from_f(s), sat); }
                    if !ops[1].is(OperandType::NULL) { self.write(&ops[1], from_f(c), sat); }
                }
                Lt => cmpf!(|x, y| x < y),
                Ge => cmpf!(|x, y| x >= y),
                Eq => cmpf!(|x, y| x == y),
                Ne => cmpf!(|x, y| x != y),
                Ftoi => f1!(|x: f32| f32::from_bits((x as i32) as u32)),
                Ftou => f1!(|x: f32| f32::from_bits(x as u32)),
                Itof => { let a = src!(1); self.write(&ops[0], from_f([(a[0] as i32) as f32, (a[1] as i32) as f32, (a[2] as i32) as f32, (a[3] as i32) as f32]), sat); }
                Utof => { let a = src!(1); self.write(&ops[0], from_f([a[0] as f32, a[1] as f32, a[2] as f32, a[3] as f32]), sat); }
                And => u2!(|x, y| x & y),
                Or => u2!(|x, y| x | y),
                Xor => u2!(|x, y| x ^ y),
                Not => u1!(|x| !x),
                Iadd => u2!(|x, y| x.wrapping_add(y)),
                Ineg => u1!(|x| (x as i32).wrapping_neg() as u32),
                Imax => u2!(|x, y| (x as i32).max(y as i32) as u32),
                Imin => u2!(|x, y| (x as i32).min(y as i32) as u32),
                Umax => u2!(|x, y| x.max(y)),
                Umin => u2!(|x, y| x.min(y)),
                Ishl => u2!(|x, y| x << (y & 31)),
                Ishr => u2!(|x, y| ((x as i32) >> (y & 31)) as u32),
                Ushr => u2!(|x, y| x >> (y & 31)),
                Ieq => u2!(|x, y| bl(x == y)),
                Ine => u2!(|x, y| bl(x != y)),
                Ilt => u2!(|x, y| bl((x as i32) < (y as i32))),
                Ige => u2!(|x, y| bl((x as i32) >= (y as i32))),
                Ult => u2!(|x, y| bl(x < y)),
                Uge => u2!(|x, y| bl(x >= y)),
                Imad => { let a = src!(1); let b = src!(2); let c = src!(3); let mut r = [0u32; 4]; for k in 0..4 { r[k] = (a[k] as i32).wrapping_mul(b[k] as i32).wrapping_add(c[k] as i32) as u32; } self.write(&ops[0], r, false); }
                Umad => { let a = src!(1); let b = src!(2); let c = src!(3); let mut r = [0u32; 4]; for k in 0..4 { r[k] = a[k].wrapping_mul(b[k]).wrapping_add(c[k]); } self.write(&ops[0], r, false); }
                Imul | Umul => {
                    let a = src!(2); let b = src!(3);
                    let (mut hi, mut lo) = ([0u32; 4], [0u32; 4]);
                    for k in 0..4 {
                        if ins.op == Imul { let p = (a[k] as i32 as i64) * (b[k] as i32 as i64); hi[k] = (p >> 32) as u32; lo[k] = p as u32; }
                        else { let p = (a[k] as u64) * (b[k] as u64); hi[k] = (p >> 32) as u32; lo[k] = p as u32; }
                    }
                    if !ops[0].is(OperandType::NULL) { self.write(&ops[0], hi, false); }
                    if !ops[1].is(OperandType::NULL) { self.write(&ops[1], lo, false); }
                }
                Udiv => {
                    let a = src!(2); let b = src!(3);
                    let (mut q, mut r) = ([0u32; 4], [0u32; 4]);
                    for k in 0..4 { if b[k] == 0 { q[k] = u32::MAX; r[k] = u32::MAX; } else { q[k] = a[k] / b[k]; r[k] = a[k] % b[k]; } }
                    if !ops[0].is(OperandType::NULL) { self.write(&ops[0], q, false); }
                    if !ops[1].is(OperandType::NULL) { self.write(&ops[1], r, false); }
                }
                Uaddc | Usubb => {
                    let a = src!(2); let b = src!(3);
                    let (mut s, mut c) = ([0u32; 4], [0u32; 4]);
                    for k in 0..4 { let (v, o) = if ins.op == Uaddc { a[k].overflowing_add(b[k]) } else { a[k].overflowing_sub(b[k]) }; s[k] = v; c[k] = if o { 1 } else { 0 }; }
                    if !ops[0].is(OperandType::NULL) { self.write(&ops[0], s, false); }
                    if !ops[1].is(OperandType::NULL) { self.write(&ops[1], c, false); }
                }
                Bfi => {
                    let w = src!(1); let o = src!(2); let ins_v = src!(3); let base = src!(4);
                    let mut r = [0u32; 4];
                    for k in 0..4 { let width = w[k] & 31; let off = o[k] & 31; let mask = (((1u64 << width) - 1) as u32) << off; r[k] = ((ins_v[k] << off) & mask) | (base[k] & !mask); }
                    self.write(&ops[0], r, false);
                }
                Ubfe => {
                    let w = src!(1); let o = src!(2); let s = src!(3);
                    let mut r = [0u32; 4];
                    for k in 0..4 { let width = w[k] & 31; let off = o[k] & 31; r[k] = if width == 0 { 0 } else if width + off < 32 { (s[k] << (32 - width - off)) >> (32 - width) } else { s[k] >> off }; }
                    self.write(&ops[0], r, false);
                }
                Ibfe => {
                    let w = src!(1); let o = src!(2); let s = src!(3);
                    let mut r = [0u32; 4];
                    for k in 0..4 { let width = w[k] & 31; let off = o[k] & 31; r[k] = if width == 0 { 0 } else if width + off < 32 { (((s[k] as i32) << (32 - width - off)) >> (32 - width)) as u32 } else { ((s[k] as i32) >> off) as u32 }; }
                    self.write(&ops[0], r, false);
                }
                Countbits => u1!(|x| x.count_ones()),
                FirstbitHi => u1!(|x| if x == 0 { u32::MAX } else { x.leading_zeros() }),
                FirstbitLo => u1!(|x| if x == 0 { u32::MAX } else { x.trailing_zeros() }),
                FirstbitShi => u1!(|x| { let y = if (x as i32) < 0 { !x } else { x }; if y == 0 { u32::MAX } else { y.leading_zeros() } }),
                Bfrev => u1!(|x| x.reverse_bits()),
                F32toF16 => { let a = srcf!(1); self.write(&ops[0], [f32_to_f16(a[0]) as u32, f32_to_f16(a[1]) as u32, f32_to_f16(a[2]) as u32, f32_to_f16(a[3]) as u32], false); }
                F16toF32 => { let a = src!(1); self.write(&ops[0], from_f([f16_to_f32(a[0] as u16), f16_to_f32(a[1] as u16), f16_to_f32(a[2] as u16), f16_to_f32(a[3] as u16)]), false); }
                DerivRtx | DerivRtxCoarse | DerivRtxFine => { let a = srcf!(1); let r = env.ddx(a); self.write(&ops[0], from_f(r), sat); }
                DerivRty | DerivRtyCoarse | DerivRtyFine => { let a = srcf!(1); let r = env.ddy(a); self.write(&ops[0], from_f(r), sat); }
                Discard => { if !self.opts.ignore_discard && test!() { discarded = true; break; } }
                If => { if !test!() { next = self.jump[pc] + 1; } }
                Else => { next = self.jump[pc] + 1; }
                EndIf => {}
                Loop => { ctx.push((Loop, pc)); }
                EndLoop => { next = self.jump[pc] + 1; }
                Break | Breakc => {
                    if ins.op == Break || test!() {
                        loop {
                            match ctx.pop() {
                                Some((Loop, b)) => { next = self.jump[b] + 1; break; }
                                Some((Switch, b)) => { switch_val.pop(); next = self.jump[b] + 1; break; }
                                Some(_) => {}
                                None => { return Err(InterpError::Unsupported("break outside loop".into())); }
                            }
                        }
                    }
                }
                Continue | Continuec => {
                    if ins.op == Continue || test!() {
                        // jump to the endloop of the innermost loop (which re-enters at loop+1)
                        let mut k = ctx.len();
                        let mut found = None;
                        while k > 0 { k -= 1; if ctx[k].0 == Loop { found = Some(k); break; } }
                        let Some(k) = found else { return Err(InterpError::Unsupported("continue outside loop".into())) };
                        let b = ctx[k].1;
                        ctx.truncate(k + 1);
                        next = b + 1;
                    }
                }
                Switch => {
                    let v = src!(0)[0];
                    ctx.push((Switch, pc));
                    switch_val.push((v, false));
                    // find the first matching case / default
                    let end = self.jump[pc];
                    let mut depth = 0i32;
                    let mut default_at = None;
                    let mut target = None;
                    let mut j = pc + 1;
                    while j < end {
                        match self.body[j].op {
                            Switch => depth += 1,
                            EndSwitch => depth -= 1,
                            Case if depth == 0 => { if self.body[j].operands[0].imm.first().copied() == Some(v) { target = Some(j); break; } }
                            Default if depth == 0 => default_at = Some(j),
                            _ => {}
                        }
                        j += 1;
                    }
                    next = match target.or(default_at) { Some(t) => t + 1, None => end + 1 };
                    if next == end + 1 { ctx.pop(); switch_val.pop(); }
                }
                Case | Default => {
                    // reached by fall-through from the previous case body: skip the label
                }
                EndSwitch => { if matches!(ctx.last(), Some((Switch, _))) { ctx.pop(); switch_val.pop(); } }
                Ret => break,
                Retc => { if test!() { break; } }
                Nop | Sync | HsDecls => {}
                Sample | SampleB | SampleL | SampleD | SampleC | SampleCLz | Gather4 | Gather4C | Gather4Po | Gather4PoC | Lod => {
                    // dst, coord, [offset for po], resource, sampler, extra...
                    let po = matches!(ins.op, Gather4Po | Gather4PoC);
                    let base = if po { 3 } else { 2 };
                    let (tex, sw) = Self::texture_slot(&ops[base]);
                    let samp = ops[base + 1].reg_index().unwrap_or(0);
                    let coord = srcf!(1);
                    let off = ins.ext.offset.map(|o| [o[0] as i32, o[1] as i32, o[2] as i32]).unwrap_or([0; 3]);
                    let (mut arg, mut ddx, mut ddy) = (0.0, [0.0; 4], [0.0; 4]);
                    let ex = base + 2;
                    let top = match ins.op {
                        Sample => TexOp::Sample, SampleB => TexOp::SampleB, SampleL => TexOp::SampleL, SampleD => TexOp::SampleD, SampleC => TexOp::SampleC, SampleCLz => TexOp::SampleCLz,
                        Gather4 => TexOp::Gather4, Gather4C => TexOp::Gather4C, Gather4Po => TexOp::Gather4Po, Gather4PoC => TexOp::Gather4PoC, _ => TexOp::Lod,
                    };
                    match ins.op {
                        SampleB | SampleL | SampleC | SampleCLz | Gather4C | Gather4PoC => arg = srcf!(ex)[0],
                        SampleD => { ddx = srcf!(ex); ddy = srcf!(ex + 1); }
                        _ => {}
                    }
                    let mut off = off;
                    if po { let o = src!(2); off = [o[0] as i32, o[1] as i32, 0]; }
                    let req = TexReq { op: top, texture: tex, sampler: samp, coord, icoord: [0; 4], offset: off, arg, ddx, ddy, resinfo_kind: 0 };
                    let r = env.texture(&req);
                    let res = [r[sw[0]], r[sw[1]], r[sw[2]], r[sw[3]]];
                    self.write(&ops[0], from_f(res), sat);
                }
                Ld | LdMs => {
                    let (tex, sw) = Self::texture_slot(&ops[2]);
                    let c = src!(1);
                    let icoord = [c[0] as i32, c[1] as i32, c[2] as i32, c[3] as i32];
                    let arg = if ins.op == LdMs { src!(3)[0] as f32 } else { 0.0 };
                    let off = ins.ext.offset.map(|o| [o[0] as i32, o[1] as i32, o[2] as i32]).unwrap_or([0; 3]);
                    let req = TexReq { op: if ins.op == Ld { TexOp::Ld } else { TexOp::LdMs }, texture: tex, sampler: 0, coord: to_f(c), icoord, offset: off, arg, ddx: [0.0; 4], ddy: [0.0; 4], resinfo_kind: 0 };
                    let r = env.texture(&req);
                    let res = [r[sw[0]], r[sw[1]], r[sw[2]], r[sw[3]]];
                    self.write(&ops[0], from_f(res), false);
                }
                Resinfo | Bufinfo | SampleInfo => {
                    let (res_op, mip) = if ins.op == Resinfo { (&ops[2], src!(1)[0]) } else { (&ops[1], 0) };
                    let (tex, sw) = Self::texture_slot(res_op);
                    let req = TexReq { op: match ins.op { Resinfo => TexOp::Resinfo, Bufinfo => TexOp::Bufinfo, _ => TexOp::SampleInfo }, texture: tex, sampler: 0, coord: [0.0; 4], icoord: [0; 4], offset: [0; 3], arg: mip as f32, ddx: [0.0; 4], ddy: [0.0; 4], resinfo_kind: (ins.token >> 11) & 3 };
                    let r = env.texture(&req);
                    let res = [r[sw[0]], r[sw[1]], r[sw[2]], r[sw[3]]];
                    self.write(&ops[0], from_f(res), false);
                }
                op => return Err(InterpError::Unsupported(op.name().to_string())),
            }
            if let Some(tr) = self.trace.as_mut() { tr.push(self.temps.clone()); }
            pc = next;
        }
        let o: Vec<[f32; 4]> = self.out.iter().map(|&b| to_f(b)).collect();
        Ok(Outputs { discarded, o, depth: self.depth.map(f32::from_bits), coverage: self.coverage })
    }
}

/// Whether [`Machine::run`] can execute `op` (declarations and custom data are never executed).
pub fn supports(op: Opcode) -> bool {
    use Opcode::*;
    op.is_decl() || matches!(op, CustomData | Mov | Movc | Swapc | Add | Mul | Div | Min | Max | Mad | Dp2 | Dp3 | Dp4 | Rsq | Sqrt | Rcp | Exp | Log | Frc | RoundNe | RoundNi | RoundPi | RoundZ | Sincos | Lt | Ge | Eq | Ne
        | Ftoi | Ftou | Itof | Utof | And | Or | Xor | Not | Iadd | Ineg | Imax | Imin | Umax | Umin | Ishl | Ishr | Ushr | Ieq | Ine | Ilt | Ige | Ult | Uge | Imad | Umad | Imul | Umul | Udiv | Uaddc | Usubb | Bfi | Ubfe | Ibfe
        | Countbits | FirstbitHi | FirstbitLo | FirstbitShi | Bfrev | F32toF16 | F16toF32 | DerivRtx | DerivRtxCoarse | DerivRtxFine | DerivRty | DerivRtyCoarse | DerivRtyFine | Discard | If | Else | EndIf | Loop | EndLoop
        | Break | Breakc | Continue | Continuec | Switch | Case | Default | EndSwitch | Ret | Retc | Nop | Sync | HsDecls | Sample | SampleB | SampleL | SampleD | SampleC | SampleCLz | Gather4 | Gather4C | Gather4Po | Gather4PoC | Lod
        | Ld | LdMs | Resinfo | Bufinfo | SampleInfo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_conversion_roundtrips() {
        for h in [0u16, 0x3c00, 0xbc00, 0x7bff, 0x0001, 0x03ff, 0x5640] {
            assert_eq!(f32_to_f16(f16_to_f32(h)), h, "{h:#x}");
        }
        assert_eq!(f32_to_f16(1.0), 0x3c00);
        assert_eq!(f32_to_f16(65504.0), 0x7bff);
    }
}







