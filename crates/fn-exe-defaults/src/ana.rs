//! Shared analysis helpers on top of `fn_exe_slate::Pe`.
use fn_exe_slate::{Pe, XrefIndex, XrefKind};
use iced_x86::{Decoder, DecoderOptions, Formatter, IntelFormatter, Instruction};

/// Every occurrence of `needle` as a complete UTF-16 string (preceded by NUL or start of a string run) -> RVAs.
pub fn utf16_exact(pe: &Pe, needle: &str) -> Vec<u32> {
    let mut w: Vec<u8> = needle.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    w.extend_from_slice(&[0, 0]);
    let mut out = vec![];
    for (_, lbl) in [(0, 0)] { let _ = lbl; }
    for (off, lbl) in pe.find_string(needle) {
        if lbl != "utf16" { continue; }
        if pe.data.get(off..off + w.len()) != Some(&w[..]) { continue; }
        if off >= 2 && (pe.data[off - 2] != 0 || pe.data[off - 1] != 0) { continue; }
        if let Some(r) = pe.off_to_rva(off as u64) { out.push(r); }
    }
    out
}

pub fn ascii_exact(pe: &Pe, needle: &str) -> Vec<u32> {
    let mut w = needle.as_bytes().to_vec(); w.push(0);
    let mut out = vec![];
    for (off, lbl) in pe.find_string(needle) {
        if lbl != "ascii" { continue; }
        if pe.data.get(off..off + w.len()) != Some(&w[..]) { continue; }
        if off >= 1 && pe.data[off - 1] != 0 { continue; }
        if let Some(r) = pe.off_to_rva(off as u64) { out.push(r); }
    }
    out
}

pub fn fmt_dis(pe: &Pe, rva: u32, count: usize) -> String {
    let Some(bytes) = pe.bytes_at_rva(rva, 0x4000) else { return String::new() };
    let mut dec = Decoder::with_ip(64, bytes, pe.image_base + rva as u64, DecoderOptions::NONE);
    let mut f = IntelFormatter::new();
    f.options_mut().set_hex_prefix("0x"); f.options_mut().set_hex_suffix("");
    f.options_mut().set_branch_leading_zeros(false);
    let mut ins = Instruction::default();
    let mut out = String::new();
    let mut n = 0;
    while dec.can_decode() && n < count {
        dec.decode_out(&mut ins);
        let r = (ins.ip() - pe.image_base) as u32;
        let mut s = String::new(); f.format(&ins, &mut s);
        let mut note = String::new();
        if ins.is_ip_rel_memory_operand() {
            let t = (ins.ip_rel_memory_address() - pe.image_base) as u32;
            note = describe(pe, t);
        } else if ins.near_branch_target() != 0 && ins.is_call_near() { }
        out += &format!("{r:08x}  {s:<52} {note}\n");
        n += 1;
    }
    out
}

pub fn describe(pe: &Pe, t: u32) -> String {
    let Some(off) = pe.rva_to_off(t) else { return format!("; [{t:x}] bss") };
    let (a, w) = pe.probe_string(off);
    let f = pe.read_f32(t).unwrap_or(0.0);
    let u = pe.read_u32(t).unwrap_or(0);
    let q = pe.read_u64(t).unwrap_or(0);
    if w.len() >= 3 { format!("; [{t:x}] u\"{w}\"") }
    else if a.len() >= 4 { format!("; [{t:x}] \"{a}\"") }
    else { format!("; [{t:x}] u32={u:#x} f32={f} u64={q:#x}") }
}

pub fn kind_s(k: XrefKind) -> &'static str { match k { XrefKind::Lea => "lea", XrefKind::Mem => "mem", XrefKind::Call => "call", XrefKind::Jmp => "jmp" } }
pub fn build_xrefs(pe: &Pe) -> XrefIndex { XrefIndex::build(pe) }
