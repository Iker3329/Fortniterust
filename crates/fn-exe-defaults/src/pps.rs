//! `FPostProcessSettings::FPostProcessSettings()` (UE 4.16) recovered from the exe.
//!
//! * The struct's reflected property names + byte offsets come from its `Z_Construct_UScriptStruct_FPostProcessSettings`
//!   (see [`crate::reflect`]).
//! * The constructor is a ~1.7 KB function that (1) default-constructs the FVector4 members, (2) `memset(this, 0, 0x4C0)`,
//!   (3) stores every non-zero default as an immediate / .rdata constant. It is located by a value fingerprint (twelve
//!   distinctive defaults) and confirmed by `UPostProcessComponent::UPostProcessComponent` calling it for its `Settings` member.
use crate::classes::props_by_hint;
use crate::ctor::{collect_stores, dwords, Store};
use crate::emu::*;
use crate::reflect::Prop;
use fn_exe_slate::{Pe, XrefIndex};
use iced_x86::Mnemonic;
use std::collections::BTreeMap;

pub const FINGERPRINT: [f32; 12] = [0.03, 2.0, 0.675, 0.4, 40.0, 0.04, 6500.0, 0.0001, 0.5, 8.0, 3.0, 0.98];

#[derive(Clone, Debug)]
pub struct PpsCtor {
    pub func: (u32, u32),
    /// RVA of the `memset(this,0,0x4C0)` call: only stores after it are effective.
    pub memset_call: u32,
    pub stores: Vec<Store>,
    pub props: Vec<Prop>,
    pub z_construct: (u32, u32),
}

/// Locate the constructor (fingerprint search over every large `.text` function) and the struct's reflection data.
pub fn find(pe: &Pe, xi: &XrefIndex) -> Option<PpsCtor> {
    let text = pe.section_named(".text")?.clone();
    let funcs: Vec<(u32, u32)> = pe.funcs.iter().copied().filter(|x| x.0 >= text.rva && x.0 < text.rva + text.vsize && x.1 - x.0 > 0x400 && x.1 - x.0 < 0x1000).collect();
    let vals: Vec<u32> = FINGERPRINT.iter().map(|f| f.to_bits()).collect();
    let ranked = crate::ctor::rank_functions(pe, xi, &funcs, &vals, 9);
    let (func, _) = *ranked.first()?;
    let ins = decode_range(pe, func.0, func.1);
    let memset_call = ins.iter().find(|i| i.mnemonic() == Mnemonic::Call).map(|i| rva_of(pe, i))?;
    let stores: Vec<Store> = collect_stores(pe, func.0, func.1).into_iter().filter(|s| s.rva > memset_call).collect();
    let (z_construct, props) = props_by_hint(pe, xi, "AutoExposureMinBrightness")?;
    Some(PpsCtor { func, memset_call, stores, props, z_construct })
}

/// Final value of every dword written by the constructor (last store wins), keyed by byte offset: `(u32 bits, store rva)`.
pub fn final_dwords(stores: &[Store]) -> BTreeMap<i64, (u32, u32)> {
    let mut m = BTreeMap::new();
    for s in stores {
        for (o, v) in dwords(s) { m.insert(o, (v, s.rva)); }
        // sub-dword stores (bool/byte enums): record as byte values at the exact offset, bits = value
        if s.bytes.len() < 4 { m.insert(s.disp, (s.bytes[0] as u32, s.rva)); }
    }
    m
}

/// Decoded value of one reflected property.
#[derive(Clone, Debug)]
pub struct FieldValue {
    pub name: String,
    pub offset: u32,
    /// 1 for float / enum, 4 for FLinearColor / FVector4 (3 for FVector, 2 for FVector2D are not used by the PPS)
    pub components: Vec<f32>,
    /// raw dwords (bits)
    pub raw: Vec<u32>,
    pub store_rva: Option<u32>,
    /// true when a store targets the field; false = left at the memset zero
    pub explicit: bool,
    pub kind: &'static str,
}

pub fn field_values(props: &[Prop], dw: &BTreeMap<i64, (u32, u32)>, stores: &[Store]) -> Vec<FieldValue> {
    let mut ps: Vec<&Prop> = props.iter().collect();
    ps.sort_by_key(|p| p.offset);
    let mut out = vec![];
    for (i, p) in ps.iter().enumerate() {
        let gap = ps.get(i + 1).map(|n| n.offset - p.offset).unwrap_or(4);
        let kind = match p.ctor_target { Some(0x13f53c0) if gap >= 16 => "struct16", Some(0x9cf5e0) => "byte_enum", Some(0x9d02e0) => "object_ptr", Some(0x13f53c0) => "array_or_struct", _ => "float" };
        let n = match kind { "struct16" => 4, "float" => 1, "byte_enum" => 1, _ => 0 };
        if n == 0 { continue; }
        let mut raw = vec![];
        let mut rva = None;
        let mut explicit = false;
        for k in 0..n {
            let off = p.offset as i64 + 4 * k as i64;
            if let Some((v, r)) = dw.get(&off) { raw.push(*v); rva = Some(*r); explicit = true; }
            else if kind == "byte_enum" {
                // byte store at the exact offset
                if let Some(s) = stores.iter().rev().find(|s| s.disp == p.offset as i64 && s.bytes.len() < 4) { raw.push(s.bytes[0] as u32); rva = Some(s.rva); explicit = true; } else { raw.push(0); }
            } else { raw.push(0); }
        }
        let components = if kind == "byte_enum" { vec![raw[0] as f32] } else { raw.iter().map(|b| f32::from_bits(*b)).collect() };
        out.push(FieldValue { name: p.name.clone(), offset: p.offset, components, raw, store_rva: rva, explicit, kind });
    }
    out
}
