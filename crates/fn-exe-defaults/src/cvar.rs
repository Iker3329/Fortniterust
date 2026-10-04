//! Console variable registration recovery.
//!
//! UE 4.16 `TAutoConsoleVariable<T>(Name, Default, Help, Flags)` is inlined into a tiny static-init thunk:
//! ```text
//!   mov rcx,[IConsoleManager::Singleton] ; mov rax,[rcx]
//!   lea r9,[Help] ; mov r8d,<default> | movss xmm2,[<default>] ; mov dword [rsp+20h],<flags>
//!   lea rdx,[Name] ; call qword [rax+<slot>]      ; IConsoleManager::RegisterConsoleVariable(Name, Default, Help, Flags)
//! ```
//! The call slot selects the overload (see [`slot_kind`]); the default is the matching argument register.
use crate::emu::*;
use fn_exe_slate::{Pe, XrefIndex, XrefKind};
use iced_x86::{Mnemonic, OpKind, Register};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum CvarValue { Int(i64), Float(f32), Str(String), RefInit { target_rva: u32, init: Option<u32>, float: bool, in_raw_data: bool }, Unknown }

#[derive(Clone, Debug)]
pub struct CvarReg {
    pub name: String,
    pub value: CvarValue,
    pub help: String,
    pub flags: Option<u32>,
    pub vtable_slot: u32,
    pub func_rva: u32,
    pub call_rva: u32,
    pub name_lea_rva: u32,
}

pub fn slot_kind(slot: u32) -> &'static str {
    match slot { 0x00 => "string", 0x08 => "float", 0x10 => "int", 0x18 => "ref_bool", 0x20 => "ref_float", 0x28 => "ref_int", _ => "?" }
}

pub fn utf16_name(pe: &Pe, rva: u32) -> Option<String> {
    let off = pe.rva_to_off(rva)?;
    let s = pe.utf16_at_off(off);
    if s.len() >= 3 { Some(s) } else { None }
}

/// Find all cvar registrations: every `lea rdx,[utf16 string]` followed within 12 instructions by an indirect `call [reg+disp]`.
pub fn scan(pe: &Pe, xi: &XrefIndex) -> Vec<CvarReg> {
    let mut by_func: HashMap<u32, Vec<u32>> = HashMap::new();
    let rd = match pe.section_named(".rdata") { Some(s) => s.clone(), None => return vec![] };
    for x in &xi.by_target {
        if x.kind != XrefKind::Lea || x.to < rd.rva || x.to >= rd.rva + rd.vsize { continue; }
        let Some((fb, fe)) = pe.func_containing(x.from) else { continue };
        if fe - fb > 0x200 { continue; }
        by_func.entry(fb).or_default().push(x.from);
    }
    let mut out = vec![];
    let mut keys: Vec<_> = by_func.keys().copied().collect();
    keys.sort();
    for fb in keys {
        let fe = pe.func_containing(fb).unwrap().1;
        let ins = decode_range(pe, fb, fe);
        let mut emu = Emu::new();
        let mut last_str: Option<String> = None;
        let mut i = 0;
        while i < ins.len() {
            let it = &ins[i];
            emu.step(it, pe);
            if it.mnemonic() == Mnemonic::Lea && it.op0_register() == Register::RDX && it.is_ip_rel_memory_operand() {
                let lea_rva = rva_of(pe, it);
                let name_rva = (it.ip_rel_memory_address() - pe.image_base) as u32;
                let nm = utf16_name(pe, name_rva);
                if let Some(name) = nm.clone() {
                    let mut e2 = emu.clone();
                    for j in i + 1..(i + 13).min(ins.len()) {
                        let c = &ins[j];
                        if c.mnemonic() == Mnemonic::Call {
                            if c.op0_kind() == OpKind::Memory && c.memory_base() != Register::RIP && c.memory_index() == Register::None {
                                let slot = c.memory_displacement64() as u32;
                                out.push(make(pe, &e2, last_str.clone(), name.clone(), slot, fb, rva_of(pe, c), lea_rva));
                            }
                            break;
                        }
                        e2.step(c, pe);
                    }
                }
                last_str = nm;
            }
            i += 1;
        }
    }
    out
}

fn make(pe: &Pe, e: &Emu, last_str: Option<String>, name: String, slot: u32, func: u32, call: u32, lea: u32) -> CvarReg {
    let help = match e.gpr[9] { Some(V::Rva(r)) => utf16_name(pe, r).unwrap_or_default(), _ => String::new() };
    let flags = e.stack.get(&0x20).and_then(|v| if let V::Imm(x) = v { Some(*x as u32) } else { None });
    let r8 = e.gpr[8];
    let x2 = e.xmm[2];
    let value = match slot_kind(slot) {
        "int" => match r8 { Some(V::Imm(x)) => CvarValue::Int(x as u32 as i32 as i64), _ => CvarValue::Unknown },
        "float" => match x2 { Some(V::Imm(x)) => CvarValue::Float(f32::from_bits(x as u32)), _ => CvarValue::Unknown },
        "string" => match last_str { Some(s) => CvarValue::Str(s), None => CvarValue::Unknown },
        "ref_float" => match r8 { Some(V::Rva(r)) => CvarValue::RefInit { target_rva: r, init: Some(pe.read_u32(r).unwrap_or(0)), float: true, in_raw_data: pe.read_u32(r).is_some() }, _ => CvarValue::Unknown },
        "ref_int" | "ref_bool" => match r8 { Some(V::Rva(r)) => CvarValue::RefInit { target_rva: r, init: Some(pe.read_u32(r).unwrap_or(0)), float: false, in_raw_data: pe.read_u32(r).is_some() }, _ => CvarValue::Unknown },
        _ => CvarValue::Unknown,
    };
    CvarReg { name, value, help, flags, vtable_slot: slot, func_rva: func, call_rva: call, name_lea_rva: lea }
}

