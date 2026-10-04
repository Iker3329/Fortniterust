//! UObject reflection recovery for UE 4.16 `Z_Construct_UScriptStruct_*` / `Z_Construct_UClass_*` functions.
//!
//! Each UPROPERTY is registered as
//! ```text
//!   mov r8d,1 ; lea rdx,[u"PropName"] ; mov rcx,[..] ; call FName::FName       ; name
//!   ... mov ecx,<sizeof(UXxxProperty)> ; call StaticAllocateObject / operator new
//!   mov rcx,<EPropertyFlags imm64> ; mov [rsp+20h],rcx ; mov r9d,<offset> ; xor r8d,r8d ; mov rdx,rax ; call UXxxProperty::UXxxProperty
//! ```
//! (`CPP_PROPERTY_BASE(Name, Class)` expands to the member offset), so every property name is paired with its
//! byte offset inside the struct/class, which is what lets us name the stores of the native constructor.
use crate::emu::*;
use fn_exe_slate::Pe;
use iced_x86::{Mnemonic, OpKind, Register};

#[derive(Clone, Debug)]
pub struct Prop {
    pub name: String,
    pub offset: u32,
    /// `mov ecx, imm` right before the allocation (sizeof the UProperty subclass: identifies the property type)
    pub obj_size: Option<u32>,
    pub flags: Option<u64>,
    /// target of the property constructor call
    pub ctor_target: Option<u32>,
    pub lea_rva: u32,
}

pub fn props_in(pe: &Pe, fb: u32, fe: u32) -> Vec<Prop> {
    let ins = decode_range(pe, fb, fe);
    let mut out: Vec<Prop> = vec![];
    let mut i = 0;
    while i < ins.len() {
        let it = &ins[i];
        if it.mnemonic() == Mnemonic::Lea && it.op0_register() == Register::RDX && it.is_ip_rel_memory_operand() {
            let t = (it.ip_rel_memory_address() - pe.image_base) as u32;
            let name = pe.rva_to_off(t).map(|o| pe.utf16_at_off(o)).unwrap_or_default();
            if name.len() >= 2 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                // scan forward for the property-ctor sequence, stopping at the next name-looking lea rdx
                let mut obj_size = None;
                let mut flags = None;
                let mut j = i + 1;
                let mut found = None;
                while j < ins.len() && j < i + 60 {
                    let c = &ins[j];
                    if c.mnemonic() == Mnemonic::Lea && c.op0_register() == Register::RDX && c.is_ip_rel_memory_operand() {
                        let tt = (c.ip_rel_memory_address() - pe.image_base) as u32;
                        let nn = pe.rva_to_off(tt).map(|o| pe.utf16_at_off(o)).unwrap_or_default();
                        if nn.len() >= 2 { break; }
                    }
                    if c.mnemonic() == Mnemonic::Mov && c.op0_kind() == OpKind::Register && c.op0_register() == Register::ECX && matches!(c.op1_kind(), OpKind::Immediate32) { obj_size = Some(c.immediate(1) as u32); }
                    if c.mnemonic() == Mnemonic::Mov && c.op0_kind() == OpKind::Register && c.op0_register() == Register::RCX && matches!(c.op1_kind(), OpKind::Immediate64 | OpKind::Immediate32to64) { flags = Some(c.immediate(1)); }
                    if c.mnemonic() == Mnemonic::Mov && c.op0_kind() == OpKind::Register && c.op0_register() == Register::R9D && matches!(c.op1_kind(), OpKind::Immediate32) {
                        found = Some(c.immediate(1) as u32);
                        // ctor call target: next direct call
                        let mut tgt = None;
                        for k in j + 1..(j + 8).min(ins.len()) { if ins[k].mnemonic() == Mnemonic::Call && ins[k].op0_kind() == OpKind::NearBranch64 { tgt = Some((ins[k].near_branch_target() - pe.image_base) as u32); break; } }
                        out.push(Prop { name: name.clone(), offset: found.unwrap(), obj_size, flags, ctor_target: tgt, lea_rva: rva_of(pe, it) });
                        break;
                    }
                    j += 1;
                }
                let _ = found;
            }
        }
        i += 1;
    }
    out
}
