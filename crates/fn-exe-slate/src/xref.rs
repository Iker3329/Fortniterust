//! Cross-reference index: every RIP-relative memory operand / direct call / jump in every `.pdata` function.
use crate::pe::Pe;
use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction, OpKind, Register};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XrefKind {
    /// `lea reg, [rip+x]` (address of the target taken)
    Lea,
    /// any other RIP-relative memory operand (mov/movss/cmp/...)
    Mem,
    Call,
    Jmp,
}

#[derive(Clone, Copy, Debug)]
pub struct Xref {
    /// RVA of the referencing instruction.
    pub from: u32,
    /// RVA of the target.
    pub to: u32,
    pub kind: XrefKind,
}

pub struct XrefIndex {
    /// all references sorted by target
    pub by_target: Vec<Xref>,
    /// callers lookup: callee RVA -> instruction RVAs
    pub calls_to: HashMap<u32, Vec<u32>>,
}

impl XrefIndex {
    /// Decode every `.pdata` function in executable sections (multithreaded) and index references.
    pub fn build(pe: &Pe) -> XrefIndex {
        let funcs: Vec<(u32, u32)> = pe.funcs.iter().copied().filter(|f| pe.section_by_rva(f.0).is_some_and(|s| s.is_exec())).collect();
        let nthreads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
        let chunk = funcs.len().div_ceil(nthreads).max(1);
        let mut all: Vec<Xref> = Vec::new();
        std::thread::scope(|sc| {
            let hs: Vec<_> = funcs.chunks(chunk).map(|c| sc.spawn(move || scan_funcs(pe, c))).collect();
            for h in hs { all.extend(h.join().unwrap()); }
        });
        all.sort_unstable_by_key(|x| (x.to, x.from));
        let mut calls_to: HashMap<u32, Vec<u32>> = HashMap::new();
        for x in &all { if matches!(x.kind, XrefKind::Call | XrefKind::Jmp) { calls_to.entry(x.to).or_default().push(x.from); } }
        XrefIndex { by_target: all, calls_to }
    }

    /// References whose target lies in `[lo, hi)`.
    pub fn refs_in(&self, lo: u32, hi: u32) -> &[Xref] {
        let a = self.by_target.partition_point(|x| x.to < lo);
        let b = self.by_target.partition_point(|x| x.to < hi);
        &self.by_target[a..b]
    }
    pub fn refs_to(&self, t: u32) -> &[Xref] { self.refs_in(t, t + 1) }
    pub fn callers_of(&self, f: u32) -> &[u32] { self.calls_to.get(&f).map(|v| v.as_slice()).unwrap_or(&[]) }
}

fn scan_funcs(pe: &Pe, funcs: &[(u32, u32)]) -> Vec<Xref> {
    let mut out = Vec::new();
    for &(b, e) in funcs {
        let Some(bytes) = pe.bytes_at_rva(b, (e - b) as usize) else { continue };
        scan_range(bytes, b, pe.image_base, &mut |x| out.push(x));
    }
    out
}

/// Decode `bytes` located at `rva`, reporting references.
pub fn scan_range(bytes: &[u8], rva: u32, image_base: u64, f: &mut dyn FnMut(Xref)) {
    let mut dec = Decoder::with_ip(64, bytes, image_base + rva as u64, DecoderOptions::NONE);
    let mut ins = Instruction::default();
    while dec.can_decode() {
        dec.decode_out(&mut ins);
        if ins.is_invalid() { continue; }
        let from = (ins.ip() - image_base) as u32;
        if ins.is_ip_rel_memory_operand() {
            let t = ins.ip_rel_memory_address();
            if t >= image_base && t - image_base < u32::MAX as u64 {
                let kind = if ins.mnemonic() == iced_x86::Mnemonic::Lea { XrefKind::Lea } else { XrefKind::Mem };
                f(Xref { from, to: (t - image_base) as u32, kind });
            }
        }
        match ins.flow_control() {
            FlowControl::Call | FlowControl::UnconditionalBranch | FlowControl::ConditionalBranch => {
                if ins.op0_kind() == OpKind::NearBranch64 {
                    let t = ins.near_branch_target();
                    if t >= image_base && t - image_base < u32::MAX as u64 {
                        let kind = if ins.flow_control() == FlowControl::Call { XrefKind::Call } else { XrefKind::Jmp };
                        // Only unconditional jumps are tail-call candidates; skip conditional ones to keep the index small.
                        if ins.flow_control() != FlowControl::ConditionalBranch { f(Xref { from, to: (t - image_base) as u32, kind }); }
                    }
                }
            }
            _ => {}
        }
    }
    let _ = Register::None;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_range_finds_rip_relative_lea_mem_and_calls() {
        // 0x1000: lea rcx,[rip+0x10] ; mov eax,[rip+0x20] ; call rel32(+0x100) ; jmp rel32(-0x20) ; ret
        let code: Vec<u8> = [&[0x48u8, 0x8D, 0x0D, 0x10, 0, 0, 0][..], &[0x8B, 0x05, 0x20, 0, 0, 0], &[0xE8, 0x00, 0x01, 0, 0], &[0xE9, 0xE0, 0xFF, 0xFF, 0xFF], &[0xC3]].concat();
        let mut got = vec![];
        scan_range(&code, 0x1000, 0x1_4000_0000, &mut |x| got.push(x));
        let k: Vec<(u32, u32, XrefKind)> = got.iter().map(|x| (x.from, x.to, x.kind)).collect();
        assert_eq!(k[0], (0x1000, 0x1000 + 7 + 0x10, XrefKind::Lea));
        assert_eq!(k[1], (0x1007, 0x1007 + 6 + 0x20, XrefKind::Mem));
        assert_eq!(k[2], (0x100d, 0x100d + 5 + 0x100, XrefKind::Call));
        assert_eq!(k[3], (0x1012, (0x1012i64 + 5 - 0x20) as u32, XrefKind::Jmp));
        assert_eq!(k.len(), 4);
    }
}

