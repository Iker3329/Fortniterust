//! Research tool (read-only): whole-function disassembly (all chained .pdata chunks), displacement scans, callers, vtable slots.
use fn_exe_defaults::ana::*;
use fn_exe_slate::*;
use iced_x86::{Decoder, DecoderOptions, Formatter, IntelFormatter, Instruction, OpKind, Register};
fn hex(s: &str) -> u32 { u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap() }
fn group_chunks(pe: &Pe, root: u32) -> Vec<(u32, u32)> {
    pe.funcs.iter().copied().filter(|f| pe.func_group(f.0).map(|g| g.0) == Some(root)).collect()
}
fn dis_range(pe: &Pe, b: u32, e: u32) -> String {
    let bytes = pe.bytes_at_rva(b, (e - b) as usize).unwrap();
    let mut dec = Decoder::with_ip(64, bytes, pe.image_base + b as u64, DecoderOptions::NONE);
    let mut f = IntelFormatter::new();
    f.options_mut().set_hex_prefix("0x"); f.options_mut().set_hex_suffix("");
    f.options_mut().set_branch_leading_zeros(false);
    let mut ins = Instruction::default(); let mut out = String::new();
    while dec.can_decode() {
        dec.decode_out(&mut ins);
        let r = (ins.ip() - pe.image_base) as u32;
        let mut s = String::new(); f.format(&ins, &mut s);
        let mut note = String::new();
        if ins.is_ip_rel_memory_operand() { note = describe(pe, (ins.ip_rel_memory_address() - pe.image_base) as u32); }
        else if ins.is_call_near() || ins.is_jmp_near() { note = format!("; -> {:x}", ins.near_branch_target().wrapping_sub(pe.image_base)); }
        out += &format!("{r:08x}  {s:<56} {note}\n");
    }
    out
}
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let pe = Pe::open(exe_path()).unwrap();
    match a[0].as_str() {
        "grp" => {
            let root = pe.func_group(hex(&a[1])).unwrap();
            println!("root {:x}-{:x}", root.0, root.1);
            let mut ch = group_chunks(&pe, root.0); ch.sort();
            for (b, e) in ch { println!("; ---- chunk {b:x}-{e:x}"); print!("{}", dis_range(&pe, b, e)); }
        }
        "rng" => { let b = hex(&a[1]); let e = hex(&a[2]); print!("{}", dis_range(&pe, b, e)); }
        "disp" => {
            // scan executable sections for non-rip memory operands with displacement in [lo,hi], optionally restricted to RVA range
            let lo = hex(&a[1]) as i64; let hi = hex(&a[2]) as i64;
            let (rlo, rhi) = if a.len() > 4 { (hex(&a[3]), hex(&a[4])) } else { (0, u32::MAX) };
            for s in pe.sections.iter().filter(|s| s.is_exec()) {
                let b = s.rva.max(rlo); let e = (s.rva + s.raw_size).min(rhi); if b >= e { continue; }
                let bytes = pe.bytes_at_rva(b, (e - b) as usize).unwrap();
                let mut dec = Decoder::with_ip(64, bytes, pe.image_base + b as u64, DecoderOptions::NONE);
                let mut ins = Instruction::default();
                let mut f = IntelFormatter::new(); f.options_mut().set_hex_prefix("0x"); f.options_mut().set_hex_suffix("");
                while dec.can_decode() {
                    dec.decode_out(&mut ins);
                    for i in 0..ins.op_count() {
                        if ins.op_kind(i) == OpKind::Memory && ins.memory_base() != Register::RIP && ins.memory_base() != Register::None {
                            let d = ins.memory_displacement64() as i64;
                            if d >= lo && d <= hi {
                                let r = (ins.ip() - pe.image_base) as u32; let mut t = String::new(); f.format(&ins, &mut t);
                                println!("{r:08x} grp {:x?}  {t}", pe.func_group(r).map(|g| g.0));
                            }
                        }
                    }
                }
            }
        }
        "callers" => { let xi = build_xrefs(&pe); for c in xi.callers_of(hex(&a[1])) { println!("{c:x} grp {:x?}", pe.func_group(*c).map(|g| g.0)); } }
        "vt" => {
            let t = hex(&a[1]); let va = pe.image_base + t as u64;
            for sec in pe.sections.iter().filter(|s| s.name == ".rdata" || s.name == ".data") {
                let mut o = 0u32;
                while o + 8 <= sec.raw_size { let rv = sec.rva + o; if pe.read_u64(rv) == Some(va) { println!("ptr at {rv:x}"); } o += 8; }
            }
        }
        "strs" => { for s in &a[1..] { for (off, lbl) in pe.find_string(s) { let rv = pe.off_to_rva(off as u64).unwrap_or(0); let t = if lbl == "utf16" { pe.utf16_at_off(off) } else { pe.ascii_at_off(off) }; println!("{rv:x} {lbl} {t}"); } } }
        "refs" => { let xi = build_xrefs(&pe); for x in xi.refs_to(hex(&a[1])) { println!("{} from {:x} grp {:x?}", kind_s(x.kind), x.from, pe.func_group(x.from).map(|g| g.0)); } }
        "bytes" => { let b = pe.bytes_at_rva(hex(&a[1]), a[2].parse().unwrap()).unwrap(); println!("{}", b.iter().map(|c| format!("{c:02x}")).collect::<Vec<_>>().join(" ")); println!("{}", b.iter().map(|c| if (32..127).contains(c) { *c as char } else { '.' }).collect::<String>()); }
        "wr" => {
            let t = hex(&a[1]);
            for s in pe.sections.iter().filter(|s| s.is_exec()) {
                let bytes = pe.bytes_at_rva(s.rva, s.raw_size as usize).unwrap();
                let mut dec = Decoder::with_ip(64, bytes, pe.image_base + s.rva as u64, DecoderOptions::NONE);
                let mut ins = Instruction::default();
                let mut f = IntelFormatter::new(); f.options_mut().set_hex_prefix("0x"); f.options_mut().set_hex_suffix("");
                while dec.can_decode() { dec.decode_out(&mut ins);
                    if ins.is_ip_rel_memory_operand() && (ins.ip_rel_memory_address() - pe.image_base) as u32 == t && ins.op0_kind() == OpKind::Memory { let r = (ins.ip() - pe.image_base) as u32; let mut x = String::new(); f.format(&ins, &mut x); println!("{r:08x} {x}"); } }
            }
        }
        "json" => {
            // json <out.json> name=rva ... : whole-function (all chained chunks) instruction dumps
            let mut funcs = vec![];
            for kv in &a[2..] {
                let (name, r) = kv.split_once('=').unwrap();
                let (root, mut ch) = match pe.func_group(hex(r)) { Some(g) => (g, group_chunks(&pe, g.0)), None => ((hex(r), hex(r) + 0x20), vec![(hex(r), hex(r) + 0x20)]) };
                ch.sort();
                let chunks: Vec<_> = ch.iter().map(|&(b, e)| {
                    let txt = dis_range(&pe, b, e);
                    let insns: Vec<_> = txt.lines().map(|l| { let (r, t) = l.split_at(8); serde_json::json!({"rva": format!("0x{}", r), "text": t.trim()}) }).collect();
                    serde_json::json!({"begin": format!("0x{b:x}"), "end": format!("0x{e:x}"), "instructions": insns})
                }).collect();
                funcs.push(serde_json::json!({"name": name, "entry_rva": format!("0x{:x}", root.0), "image_base": "0x140000000", "chunks": chunks}));
            }
            std::fs::write(&a[1], serde_json::to_string_pretty(&serde_json::json!({"exe": "FortniteClient-Win64-Shipping.exe (1.8)", "functions": funcs})).unwrap()).unwrap();
        }
        "q" => { let t = hex(&a[1]); println!("{}", describe(&pe, t)); println!("u64 {:x?}", pe.read_u64(t)); }
        _ => {}
    }
}





