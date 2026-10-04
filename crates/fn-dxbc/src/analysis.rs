//! Static analysis helpers: which constant-buffer registers / components a program reads, resource usage, instruction statistics.

use crate::ir::*;
use crate::opcode::Opcode;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CbUsage {
    pub slot: u32,
    /// declared size in float4 registers
    pub declared_size: u32,
    /// element -> component mask (bit0 = x) read with a constant index
    pub elements: BTreeMap<u32, u8>,
    /// read through a relative (dynamic) index
    pub dynamic: bool,
}

/// Opcodes whose result component `c` depends only on component `c` of every source.
pub fn is_componentwise(op: Opcode) -> bool {
    use Opcode::*;
    matches!(op, Add | And | Div | Eq | Exp | Frc | Ftoi | Ftou | Ge | Iadd | Ieq | Ige | Ilt | Imad | Imax | Imin | Imul | Ine | Ineg | Ishl | Ishr | Itof | Log | Lt | Mad | Min | Max | Mov | Movc | Mul | Ne | Not | Or
        | RoundNe | RoundNi | RoundPi | RoundZ | Rsq | Sqrt | Udiv | Ult | Uge | Umul | Umad | Umax | Umin | Ushr | Utof | Xor | Rcp | DerivRtx | DerivRty | DerivRtxCoarse | DerivRtxFine | DerivRtyCoarse | DerivRtyFine
        | F32toF16 | F16toF32 | Countbits | FirstbitHi | FirstbitLo | FirstbitShi | Ubfe | Ibfe | Bfi | Bfrev | Uaddc | Usubb | Msad)
}

fn demand(inst: &Instruction, src: &Operand) -> u8 {
    use Opcode::*;
    let dst_mask = inst.dst().first().map(|d| d.mask()).unwrap_or(0xf);
    let from_dst = |mask: u8| -> u8 {
        let mut m = 0u8;
        for c in 0..4 { if mask & (1 << c) != 0 { m |= 1 << src.src_comp(c); } }
        m
    };
    match inst.op {
        op if is_componentwise(op) => from_dst(dst_mask),
        Dp2 => from_dst(0b0011),
        Dp3 => from_dst(0b0111),
        Dp4 => from_dst(0b1111),
        Sincos => from_dst(0b0001),
        _ => match src.sel { Sel::Select1(c) => 1 << c, Sel::Swizzle(s) => s.iter().fold(0, |m, &c| m | (1 << c)), _ => 0xf },
    }
}

fn visit_operand(inst: &Instruction, op: &Operand, is_src: bool, out: &mut BTreeMap<u32, CbUsage>, shader: &Shader) {
    if op.is(OperandType::CONSTANT_BUFFER) && is_src {
        let slot = op.index.first().map(|i| i.imm as u32).unwrap_or(0);
        let e = out.entry(slot).or_insert_with(|| CbUsage { slot, declared_size: shader.cbuffer(slot).map_or(0, |c| c.size), ..Default::default() });
        match op.index.get(1) {
            Some(i) if i.rel.is_none() => { *e.elements.entry(i.imm as u32).or_insert(0) |= demand(inst, op); }
            Some(_) => e.dynamic = true,
            None => {}
        }
    }
    for i in &op.index { if let Some(r) = &i.rel { visit_operand(inst, r, true, out, shader); } }
}

pub fn cbuffer_usage(shader: &Shader) -> Vec<CbUsage> {
    let mut m: BTreeMap<u32, CbUsage> = BTreeMap::new();
    for inst in shader.body() {
        let (nd, _) = inst.op.arity();
        for (k, op) in inst.operands.iter().enumerate() { visit_operand(inst, op, k >= nd, &mut m, shader); }
    }
    m.into_values().collect()
}

/// (texture slots, sampler slots) referenced by sampling / load instructions
pub fn resource_usage(shader: &Shader) -> (Vec<u32>, Vec<u32>) {
    let (mut t, mut s) = (vec![], vec![]);
    for inst in shader.body() {
        for op in &inst.operands {
            match op.ty.0 {
                OperandType::RESOURCE => if let Some(r) = op.reg_index() { if !t.contains(&r) { t.push(r) } },
                OperandType::SAMPLER => if let Some(r) = op.reg_index() { if !s.contains(&r) { s.push(r) } },
                _ => {}
            }
        }
    }
    t.sort();
    s.sort();
    (t, s)
}

/// Histogram of opcodes in the executable body.
pub fn opcode_histogram(shader: &Shader) -> BTreeMap<&'static str, usize> {
    let mut m = BTreeMap::new();
    for i in shader.body() { *m.entry(i.op.name()).or_insert(0) += 1; }
    m
}

/// Opcodes of the program the decoder could not name.
pub fn unknown_opcodes(shader: &Shader) -> Vec<u16> {
    let mut v: Vec<u16> = shader.instructions.iter().filter_map(|i| if let Opcode::Unknown(n) = i.op { Some(n) } else { None }).collect();
    v.sort();
    v.dedup();
    v
}
