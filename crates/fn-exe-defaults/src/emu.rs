//! Straight-line register/stack constant tracker (x64 Windows ABI). Deliberately small: it only needs to answer
//! "what constant was in rdx/r8/r9/xmm2/[rsp+0x20] when this call happened" for tiny registration thunks and
//! "which immediates were stored at [reg+disp]" for constructors.
use fn_exe_slate::Pe;
use iced_x86::{Code, Decoder, DecoderOptions, Instruction, Mnemonic, OpKind, Register};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum V {
    /// integer / float bit pattern
    Imm(u64),
    /// absolute address inside the image (as RVA) that was formed by `lea reg,[rip+x]`
    Rva(u32),
}

#[derive(Default, Clone)]
pub struct Emu {
    pub gpr: [Option<V>; 16],
    pub xmm: [Option<V>; 16],
    pub stack: HashMap<i64, V>,
    /// full 128-bit constant held in an xmm register (from an aligned/unaligned 16-byte .rdata load or a zeroing idiom)
    pub wide: [Option<u128>; 16],
    /// also treat loads from initialised `.data` as constants (globals such as `FColor::White`); off for call-site tracking
    pub allow_data: bool,
}

pub fn gpr_idx(r: Register) -> Option<usize> {
    let f = r.full_register();
    use Register::*;
    Some(match f { RAX => 0, RCX => 1, RDX => 2, RBX => 3, RSP => 4, RBP => 5, RSI => 6, RDI => 7, R8 => 8, R9 => 9, R10 => 10, R11 => 11, R12 => 12, R13 => 13, R14 => 14, R15 => 15, _ => return Option::None })
}
pub fn xmm_idx(r: Register) -> Option<usize> { if r.is_xmm() { Some(r.number()) } else { None } }

fn mask(r: Register, v: u64) -> u64 {
    match r.size() { 1 => v & 0xff, 2 => v & 0xffff, 4 => v & 0xffff_ffff, _ => v }
}

impl Emu {
    pub fn new() -> Emu { Emu::default() }
    pub fn get_op(&self, ins: &Instruction, op: u32, pe: &Pe) -> Option<V> {
        match ins.op_kind(op) {
            OpKind::Register => {
                let r = ins.op_register(op);
                if let Some(i) = gpr_idx(r) { return self.gpr[i].map(|v| match v { V::Imm(x) => V::Imm(mask(r, x)), o => o }); }
                if let Some(i) = xmm_idx(r) { return self.xmm[i]; }
                None
            }
            OpKind::Immediate8 | OpKind::Immediate16 | OpKind::Immediate32 | OpKind::Immediate64 | OpKind::Immediate8to16 | OpKind::Immediate8to32 | OpKind::Immediate8to64 | OpKind::Immediate32to64 => Some(V::Imm(ins.immediate(op))),
            OpKind::Memory => {
                if ins.is_ip_rel_memory_operand() {
                    let t = ins.ip_rel_memory_address().checked_sub(pe.image_base)? as u32;
                    // read from the static image only for constants (float/int loads from .rdata)
                    let sz = ins.memory_size().size();
                    let sz = if sz == 0 { 4 } else { sz };
                    if let Some(sec) = pe.section_by_rva(t) {
                        if sec.name == ".rdata" || sec.name == "_RDATA" || (self.allow_data && sec.name == ".data") {
                            return match sz { 4 => pe.read_u32(t).map(|x| V::Imm(x as u64)), 8 => pe.read_u64(t).map(V::Imm), 16 => pe.read_u32(t).map(|x| V::Imm(x as u64)), _ => None };
                        }
                    }
                    None
                } else if ins.memory_base() == Register::RSP && ins.memory_index() == Register::None {
                    self.stack.get(&(ins.memory_displacement64() as i64)).copied()
                } else { None }
            }
            _ => None,
        }
    }
    /// Apply one instruction.
    pub fn step(&mut self, ins: &Instruction, pe: &Pe) {
        use Mnemonic::*;
        match ins.mnemonic() {
            Call => { for i in [0usize, 1, 2, 8, 9, 10, 11] { self.gpr[i] = None; } for x in self.xmm.iter_mut().take(6) { *x = None; } for x in self.wide.iter_mut().take(6) { *x = None; } }
            Lea => {
                let r = ins.op0_register();
                if let Some(i) = gpr_idx(r) {
                    self.gpr[i] = if ins.is_ip_rel_memory_operand() { ins.ip_rel_memory_address().checked_sub(pe.image_base).map(|t| V::Rva(t as u32)) } else { None };
                }
            }
            Mov | Movss | Movsd | Movd | Movq | Movaps | Movups | Movdqa | Movdqu | Movzx | Movsxd => {
                if ins.op0_kind() == OpKind::Register {
                    let r = ins.op0_register();
                    let v = self.get_op(ins, 1, pe);
                    let v = if ins.mnemonic() == Movzx { v } else { v };
                    if let Some(i) = gpr_idx(r) {
                        // writing a 32-bit reg zero-extends; 8/16-bit leave the rest -> treat as unknown unless 32/64
                        self.gpr[i] = if r.size() >= 4 { v } else { None };
                    } else if let Some(i) = xmm_idx(r) {
                        // movss xmm,[mem] zeroes the upper bits; movss xmm,xmm keeps them -> still the low lane is what matters
                        self.xmm[i] = v;
                        self.wide[i] = None;
                        if matches!(ins.mnemonic(), Movaps | Movups | Movdqa | Movdqu) {
                            if ins.op1_kind() == OpKind::Register { if let Some(j) = xmm_idx(ins.op1_register()) { self.wide[i] = self.wide[j]; self.xmm[i] = self.xmm[j]; } }
                            else if ins.is_ip_rel_memory_operand() {
                                if let Some(t) = ins.ip_rel_memory_address().checked_sub(pe.image_base) {
                                    if let Some(b) = pe.bytes_at_rva(t as u32, 16) { if b.len() == 16 {
                                        let w = u128::from_le_bytes(b.try_into().unwrap());
                                        self.wide[i] = Some(w); self.xmm[i] = Some(V::Imm((w & 0xffff_ffff) as u64));
                                    } }
                                }
                            }
                        }
                    }
                } else if ins.op0_kind() == OpKind::Memory && ins.memory_base() == Register::RSP && ins.memory_index() == Register::None {
                    let d = ins.memory_displacement64() as i64;
                    match self.get_op(ins, 1, pe) { Some(v) => { self.stack.insert(d, v); } None => { self.stack.remove(&d); } }
                }
            }
            Xor | Xorps | Pxor | Sub if ins.op0_kind() == OpKind::Register && ins.op1_kind() == OpKind::Register && ins.op0_register() == ins.op1_register() => {
                let r = ins.op0_register();
                if let Some(i) = gpr_idx(r) { self.gpr[i] = Some(V::Imm(0)); } else if let Some(i) = xmm_idx(r) { self.xmm[i] = Some(V::Imm(0)); self.wide[i] = Some(0); }
            }
            Or if ins.op0_kind() == OpKind::Register && matches!(ins.op1_kind(), OpKind::Immediate8to32 | OpKind::Immediate8to64 | OpKind::Immediate32 | OpKind::Immediate32to64) && (ins.immediate(1) as i64 == -1 || ins.immediate(1) as u32 == 0xFFFF_FFFF) => {
                if let Some(i) = gpr_idx(ins.op0_register()) { self.gpr[i] = Some(V::Imm(if ins.op0_register().size() == 8 { u64::MAX } else { 0xFFFF_FFFF })); }
            }
            Xorps | Pxor => { if let Some(i) = xmm_idx(ins.op0_register()) { self.xmm[i] = None; self.wide[i] = None; } }
            Add | Sub | And | Or | Xor | Imul | Shl | Shr | Sar | Inc | Dec | Neg | Not | Cvtsi2ss | Cvtss2sd | Cvtsd2ss | Mulss | Addss | Subss | Divss | Sqrtss | Maxss | Minss | Andps | Orps | Cmovne | Cmove | Setne | Sete | Movsx | Pop | Xchg => {
                if ins.op0_kind() == OpKind::Register {
                    let r = ins.op0_register();
                    if let Some(i) = gpr_idx(r) { self.gpr[i] = None; } else if let Some(i) = xmm_idx(r) { self.xmm[i] = None; self.wide[i] = None; }
                }
            }
            _ => {}
        }
        let _ = Code::INVALID;
    }
}

/// Decoded instruction list for `[start, end)` RVAs.
pub fn decode_range(pe: &Pe, start: u32, end: u32) -> Vec<Instruction> {
    let Some(bytes) = pe.bytes_at_rva(start, (end - start) as usize) else { return vec![] };
    let mut dec = Decoder::with_ip(64, bytes, pe.image_base + start as u64, DecoderOptions::NONE);
    let mut v = vec![];
    let mut ins = Instruction::default();
    while dec.can_decode() { dec.decode_out(&mut ins); v.push(ins); }
    v
}
pub fn rva_of(pe: &Pe, ins: &Instruction) -> u32 { (ins.ip() - pe.image_base) as u32 }



