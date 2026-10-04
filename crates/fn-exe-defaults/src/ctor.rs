//! Native constructor analysis: every store of a known constant to `[base+disp]` inside a function.
//!
//! UE C++ constructors assign each UPROPERTY default with `mov dword [this+disp], imm32`, `movss [this+disp], xmm`
//! (value loaded from .rdata or zeroed) or 128-bit `movups [this+disp], xmm` of an .rdata constant, so the stored
//! constant IS the default. Member names are attached by value and by the UE 4.16 declaration order (see `postprocess`).
use crate::emu::*;
use fn_exe_slate::Pe;
use iced_x86::{Mnemonic, OpKind, Register};

#[derive(Clone, Debug)]
pub struct Store {
    pub rva: u32,
    pub base: Register,
    pub disp: i64,
    /// little-endian bytes stored (1, 2, 4, 8 or 16)
    pub bytes: Vec<u8>,
}
impl Store {
    pub fn u32(&self) -> Option<u32> { self.bytes.get(0..4).map(|b| u32::from_le_bytes(b.try_into().unwrap())) }
    pub fn f32(&self) -> Option<f32> { self.u32().map(f32::from_bits) }
}

/// All constant stores in `[start,end)` to non-RSP, non-RIP bases (straight-line decode, registers tracked by [`Emu`]).
pub fn collect_stores(pe: &Pe, start: u32, end: u32) -> Vec<Store> {
    let ins = decode_range(pe, start, end);
    let mut emu = Emu::new();
    emu.allow_data = true;
    let mut out = vec![];
    for it in &ins {
        use Mnemonic::*;
        if it.op0_kind() == OpKind::Memory
            && matches!(it.mnemonic(), Mov | Movss | Movsd | Movd | Movq | Movaps | Movups | Movdqa | Movdqu)
            && !it.is_ip_rel_memory_operand()
            && it.memory_base() != Register::RSP
            && it.memory_base() != Register::None
            && it.memory_index() == Register::None
        {
            let size = it.memory_size().size();
            let mut bytes: Option<Vec<u8>> = None;
            match it.op1_kind() {
                OpKind::Register => {
                    let r = it.op1_register();
                    if let Some(i) = xmm_idx(r) {
                        if size == 16 { bytes = emu.wide[i].map(|w| w.to_le_bytes().to_vec()); }
                        else if let Some(V::Imm(x)) = emu.xmm[i] { bytes = Some(x.to_le_bytes()[..size.min(8)].to_vec()); }
                    } else if let Some(i) = gpr_idx(r) {
                        if let Some(V::Imm(x)) = emu.gpr[i] { bytes = Some(x.to_le_bytes()[..size.min(8)].to_vec()); }
                    }
                }
                OpKind::Immediate8 | OpKind::Immediate16 | OpKind::Immediate32 | OpKind::Immediate64 | OpKind::Immediate8to16 | OpKind::Immediate8to32 | OpKind::Immediate8to64 | OpKind::Immediate32to64 => {
                    bytes = Some(it.immediate(1).to_le_bytes()[..size.clamp(1, 8)].to_vec());
                }
                _ => {}
            }
            if let Some(b) = bytes {
                out.push(Store { rva: rva_of(pe, it), base: it.memory_base(), disp: it.memory_displacement64() as i64, bytes: b });
            }
        }
        emu.step(it, pe);
    }
    out
}

/// Expand a store into per-dword (disp, u32) pairs.
pub fn dwords(s: &Store) -> Vec<(i64, u32)> {
    s.bytes.chunks_exact(4).enumerate().map(|(i, c)| (s.disp + 4 * i as i64, u32::from_le_bytes(c.try_into().unwrap()))).collect()
}

/// Functions (by `.pdata`) whose body contains at least `min` of the given 32-bit immediates/constants as stores.
pub fn rank_functions(pe: &Pe, xi: &fn_exe_slate::XrefIndex, funcs: &[(u32, u32)], values: &[u32], min: usize) -> Vec<((u32, u32), usize)> {
    let _ = xi;
    let mut out = vec![];
    for &(b, e) in funcs {
        if e - b < 0x200 { continue; }
        let st = collect_stores(pe, b, e);
        let mut hit = 0;
        for v in values {
            if st.iter().any(|s| dwords(s).iter().any(|(_, x)| x == v)) { hit += 1; }
        }
        if hit >= min { out.push(((b, e), hit)); }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.1));
    out
}

#[derive(Clone, Debug)]
pub struct BitOp { pub rva: u32, pub base: Register, pub disp: i64, pub size: usize, pub is_or: bool, pub imm: u64 }

/// `or/and [base+disp], imm` (bit-field bool defaults such as `bAffectsWorld`), non-RSP bases only.
pub fn collect_bit_ops(pe: &Pe, start: u32, end: u32) -> Vec<BitOp> {
    decode_range(pe, start, end).iter().filter(|i| {
        matches!(i.mnemonic(), Mnemonic::Or | Mnemonic::And) && i.op0_kind() == OpKind::Memory && !i.is_ip_rel_memory_operand()
            && i.memory_base() != Register::RSP && i.memory_base() != Register::None
            && matches!(i.op1_kind(), OpKind::Immediate8 | OpKind::Immediate16 | OpKind::Immediate32 | OpKind::Immediate8to32 | OpKind::Immediate8to16 | OpKind::Immediate32to64 | OpKind::Immediate8to64)
    }).map(|i| BitOp { rva: rva_of(pe, i), base: i.memory_base(), disp: i.memory_displacement64() as i64, size: i.memory_size().size(), is_or: i.mnemonic() == Mnemonic::Or, imm: i.immediate(1) }).collect()
}
