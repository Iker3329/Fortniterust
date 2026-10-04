//! UClass registration -> native constructor + reflected property offsets.
//!
//! `IMPLEMENT_CLASS(UFoo, hash)` expands to `UFoo::GetPrivateStaticClass()` which calls
//! `GetPrivateStaticClassBody(Package, TEXT("UFoo")+1, ..., sizeof(UFoo), flags, castflags, config, ClassConstructor, ...)`.
//! The literal `u"UFoo"` is referenced with a `+2` bytes pointer bump (skipping the U prefix) and the ClassConstructor
//! pointer (`[rsp+0x40]` stack argument) is `UFoo::__DefaultConstructor`, which tail-calls `new(obj) UFoo(FObjectInitializer&)`.
use crate::emu::*;
use crate::reflect::{props_in, Prop};
use fn_exe_slate::{Pe, XrefIndex, XrefKind};
use iced_x86::{Mnemonic, OpKind};

#[derive(Clone, Debug, Default)]
pub struct ClassInfo {
    pub name: String,
    pub get_private_static_class: u32,
    pub sizeof_class: u32,
    pub default_ctor_thunk: u32,
    pub ctor: u32,
    pub z_construct: Option<(u32, u32)>,
    pub props: Vec<Prop>,
}

fn first_call_target(pe: &Pe, f: u32, skip: usize) -> Option<(u32, Mnemonic)> {
    let (fb, fe) = pe.func_containing(f).unwrap_or((f, f + 0x20));
    let ins = decode_range(pe, fb.max(f), fe);
    let mut n = 0;
    for i in &ins {
        if matches!(i.mnemonic(), Mnemonic::Call | Mnemonic::Jmp) && i.op0_kind() == OpKind::NearBranch64 {
            if n == skip { return Some(((i.near_branch_target() - pe.image_base) as u32, i.mnemonic())); }
            n += 1;
        }
    }
    None
}

/// `cname` includes the U/A prefix (e.g. "UDirectionalLightComponent").
pub fn find_class(pe: &Pe, xi: &XrefIndex, cname: &str) -> Option<ClassInfo> {
    let mut info = ClassInfo { name: cname.to_string(), ..Default::default() };
    for s in crate::ana::utf16_exact(pe, cname) {
        for x in xi.refs_to(s + 0) {
            if x.kind != XrefKind::Lea { continue; }
            let Some((fb, fe)) = pe.func_containing(x.from) else { continue };
            if fe - fb > 0x200 { continue; }
            // is it GetPrivateStaticClass? look for `mov dword [rsp+20h], imm` (sizeof) and `add rcx,2` after the lea
            let ins = decode_range(pe, fb, fe);
            let mut emu = Emu::new();
            let mut has_add2 = false;
            for i in &ins {
                if i.mnemonic() == Mnemonic::Add && i.op1_kind() == OpKind::Immediate8to64 && i.immediate(1) == 2 { has_add2 = true; }
                emu.step(i, pe);
            }
            if !has_add2 { continue; }
            // stack args as recorded at function end (nothing clears them)
            let size = match emu.stack.get(&0x20) { Some(V::Imm(s)) => *s as u32, _ => continue };
            let thunk = match emu.stack.get(&0x40) { Some(V::Rva(r)) => *r, _ => continue };
            info.get_private_static_class = fb;
            info.sizeof_class = size;
            info.default_ctor_thunk = thunk;
            // thunk -> call InternalConstructor-like helper -> jmp/call real ctor
            let (b, _) = first_call_target(pe, thunk, 0)?;
            let (ctor, _) = first_call_target(pe, b, 0)?;
            info.ctor = ctor;
            // Z_Construct: the caller of GetPrivateStaticClass with the most reflected properties
            let mut best: Option<((u32, u32), Vec<Prop>)> = None;
            for &c in xi.callers_of(fb) {
                if let Some((cb, ce)) = pe.func_containing(c) {
                    if ce - cb < 0x200 { continue; }
                    let p = props_in(pe, cb, ce);
                    if best.as_ref().map(|b| p.len() > b.1.len()).unwrap_or(true) { best = Some(((cb, ce), p)); }
                }
            }
            if let Some((r, p)) = best { info.z_construct = Some(r); info.props = p; }
            return Some(info);
        }
    }
    None
}



/// Reflected properties of the `Z_Construct_*` function that references the property name `hint`
/// (largest function wins when several reference it). Handles names whose literal is suffix-merged by looking at every xref
/// target rather than only whole-string matches when none is found.
pub fn props_by_hint(pe: &Pe, xi: &XrefIndex, hint: &str) -> Option<((u32, u32), Vec<Prop>)> { props_by_hint_max(pe, xi, hint, u32::MAX) }

/// As [`props_by_hint`] but every property offset must be `< max_off` (pass `sizeof(class)`): rejects Z_Construct functions of
/// other classes/structs that merely reuse a property name.
pub fn props_by_hint_max(pe: &Pe, xi: &XrefIndex, hint: &str, max_off: u32) -> Option<((u32, u32), Vec<Prop>)> {
    let mut best: Option<((u32, u32), Vec<Prop>)> = None;
    for s in crate::ana::utf16_exact(pe, hint) {
        for x in xi.refs_to(s) {
            if x.kind != XrefKind::Lea { continue; }
            let Some((fb, fe)) = pe.func_containing(x.from) else { continue };
            if fe - fb < 0x300 { continue; }
            let p = props_in(pe, fb, fe);
            if p.iter().any(|q| q.name == hint) && p.iter().all(|q| q.offset < max_off) && best.as_ref().map(|b| p.len() > b.1.len()).unwrap_or(true) { best = Some(((fb, fe), p)); }
        }
    }
    best
}

