//! Ad-hoc exploration CLI (read-only). Usage:
//!   explore sections | strings | xrefs | dis <rva-hex> [count] | func <rva-hex> | callers <rva-hex> | find <text> | ptrs <rva-hex> | bytes <rva-hex> <n>
use fn_exe_slate::*;
use iced_x86::{Decoder, DecoderOptions, Formatter, IntelFormatter, Instruction};

fn hex(s: &str) -> u64 { u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap() }

pub fn dis(pe: &Pe, rva: u32, count: usize, until_end: Option<u32>) {
    let bytes = pe.bytes_at_rva(rva, 0x4000).unwrap();
    let mut dec = Decoder::with_ip(64, bytes, pe.image_base + rva as u64, DecoderOptions::NONE);
    let mut f = IntelFormatter::new();
    f.options_mut().set_hex_prefix("0x"); f.options_mut().set_hex_suffix("");
    f.options_mut().set_branch_leading_zeros(false);
    f.options_mut().set_uppercase_hex(false);
    let mut ins = Instruction::default();
    let mut n = 0;
    while dec.can_decode() && n < count {
        dec.decode_out(&mut ins);
        let r = (ins.ip() - pe.image_base) as u32;
        if let Some(e) = until_end { if r >= e { break; } }
        let mut s = String::new();
        f.format(&ins, &mut s);
        let mut note = String::new();
        if ins.is_ip_rel_memory_operand() {
            let t = (ins.ip_rel_memory_address() - pe.image_base) as u32;
            note = describe(pe, t);
        }
        let b = pe.bytes_at_rva(r, ins.len()).unwrap();
        let bs: String = b.iter().map(|x| format!("{x:02x}")).collect();
        println!("{r:08x}  {bs:<24} {s:<48} {note}");
        n += 1;
    }
}

pub fn describe(pe: &Pe, t: u32) -> String {
    let Some(off) = pe.rva_to_off(t) else { return format!("; [{t:x}] (bss)") };
    let sec = pe.section_by_rva(t).map(|s| s.name.clone()).unwrap_or_default();
    let (a, w) = pe.probe_string(off);
    let f32v = pe.read_f32(t).unwrap_or(0.0);
    let u64v = pe.read_u64(t).unwrap_or(0);
    let mut s = format!("; {sec}:{t:x}");
    if a.len() >= 3 { s += &format!(" a\"{}\"", a); }
    if w.len() >= 3 { s += &format!(" w\"{}\"", w); }
    if a.len() < 3 && w.len() < 3 { s += &format!(" f32={f32v} u64={u64v:#x}"); }
    s
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pe = Pe::open(exe_path()).unwrap();
    match args.first().map(|s| s.as_str()).unwrap_or("") {
        "sections" => {
            println!("image_base {:#x}, funcs {}", pe.image_base, pe.funcs.len());
            for s in &pe.sections { println!("{:8} rva {:08x} vsize {:08x} raw {:08x} rawsize {:08x} chr {:08x}", s.name, s.rva, s.vsize, s.raw_off, s.raw_size, s.characteristics); }
        }
        "strings" => {
            for (n, off) in USER_STRING_OFFSETS {
                let (a, w) = pe.probe_string(*off as usize);
                println!("{n:32} off {off:#x} rva {:?} ascii={a:?} utf16={w:?}", pe.off_to_rva(*off));
            }
        }
        "find" => { for (o, e) in pe.find_string(&args[1]) { println!("off {o:#x} {e} rva {:?}", pe.off_to_rva(o as u64).map(|r| format!("{r:#x}"))); } }
        "dis" => { let rva = hex(&args[1]) as u32; dis(&pe, rva, args.get(2).map(|s| s.parse().unwrap()).unwrap_or(60), None); }
        "func" => {
            let rva = hex(&args[1]) as u32;
            let (b, e) = pe.func_containing(rva).expect("no pdata func");
            println!("; func {b:x}..{e:x} ({} bytes)", e - b);
            dis(&pe, b, 100000, Some(e));
        }
        "emu" => {
            let start = hex(&args[1]) as u32;
            let (b, e) = match args.get(2) { Some(x) => (start, hex(x) as u32), None => pe.func_containing(start).expect("no pdata func") };
            let mut emu = fn_exe_slate::emu::Emu::new(&pe);
            fn_exe_slate::slate::configure(&mut emu);
            emu.run(b, e);
            print!("{}", emu.render());
        }
        "xi" => {
            // xrefs with the referencing instruction text: xi lo [hi]
            let idx = XrefIndex::build(&pe);
            let lo = hex(&args[1]) as u32; let hi = args.get(2).map(|s| hex(s) as u32).unwrap_or(lo + 1);
            for x in idx.refs_in(lo, hi) {
                let b = pe.bytes_at_rva(x.from, 16).unwrap();
                let mut d = Decoder::with_ip(64, b, pe.image_base + x.from as u64, DecoderOptions::NONE);
                let mut ins = Instruction::default(); d.decode_out(&mut ins);
                let mut f = IntelFormatter::new(); let mut s = String::new(); f.format(&ins, &mut s);
                println!("{:08x} (func {:x?}) {:?} {s}", x.from, pe.func_containing(x.from).map(|f| f.0), x.kind);
            }
        }
        "funcs" => {
            let lo = hex(&args[1]) as u32; let hi = hex(&args[2]) as u32;
            for f in pe.funcs.iter().filter(|f| f.0 >= lo && f.0 < hi) { println!("{:08x}..{:08x} ({})", f.0, f.1, f.1 - f.0); }
        }
        "vt" => {
            // vtable dump: vt <rva> <n>
            let rva = hex(&args[1]) as u32; let n: usize = args[2].parse().unwrap();
            for i in 0..n { let r = rva + 8 * i as u32; let v = pe.read_u64(r).unwrap_or(0); let t = pe.rva_of_va(v); let info = t.and_then(|t| pe.func_containing(t)).map(|f| format!("func {:x}..{:x}", f.0, f.1)).unwrap_or_default(); println!("[{:#x}] +{:#x}: {:#x} {}", r, 8 * i, v, info); }
        }
        "strs" => {
            // list strings in an RVA range: strs lo hi
            let lo = hex(&args[1]) as u32; let hi = hex(&args[2]) as u32;
            let mut r = lo;
            while r < hi {
                let Some(o) = pe.rva_to_off(r) else { break };
                let (a, w) = pe.probe_string(o);
                if w.len() >= 3 && w.len() * 2 >= a.len() { println!("{r:08x} w\"{w}\""); r += (w.len() as u32 + 1) * 2; r = (r + 1) & !1; }
                else if a.len() >= 3 { println!("{r:08x} a\"{a}\""); r += a.len() as u32 + 1; }
                else { r += 1; }
            }
        }
        "ctor" => {
            // emulate a constructor and dump what it writes into `this`: ctor <rva> [span-hex]
            let start = hex(&args[1]) as u32; let span = args.get(2).map(|s| hex(s) as i64).unwrap_or(0x900);
            let (b, e) = pe.func_containing(start).expect("no pdata func");
            let mut emu = fn_exe_slate::emu::Emu::new(&pe);
            fn_exe_slate::slate::configure(&mut emu);
            emu.skip_indirect = false;
            emu.run(b, e);
            print!("{}", emu.render());
            println!("THIS:{}", emu.dump_param(1, span).replace(" +", "\n  +"));
        }
        "sweep" => {
            let idx = XrefIndex::build(&pe);
            let loc = fn_exe_slate::slate::locate_construct(&pe, &idx);
            println!("construct: {:x?}", loc);
            let sites = fn_exe_slate::slate::sweep_constructs(&pe, &idx);
            println!("SNew sites: {}", sites.len());
            for (class, v) in fn_exe_slate::slate::construct_table(&sites) {
                let total: usize = v.iter().map(|x| x.1).sum();
                let top: Vec<String> = v.iter().take(3).map(|(r, n)| format!("{:#x}x{n}", pe.va(*r))).collect();
                println!("{class:28} sites {total:5}  {}", top.join(" "));
            }
            for s in sites.iter().filter(|s| s.file_tail.contains("FortniteLoadingScreen.cpp")) { println!("  {:08x} {:16} line {:4} -> {:x?}", s.at, s.class, s.line, s.construct); }
        }
        "findings" => {
            let idx = XrefIndex::build(&pe);
            let f = fn_exe_slate::findings::recover(&pe, &idx);
            for e in &f.errors { println!("ERROR {e}"); }
            for it in &f.items { println!("{:34} {:9} {:>10} {}", it.id, it.conf.as_str(), it.rva.map(|r| format!("{r:#x}")).unwrap_or_default(), serde_json::to_string(&it.value).unwrap()); }
        }
        "bytes" => {
            let rva = hex(&args[1]) as u32; let n: usize = args[2].parse().unwrap();
            let b = pe.bytes_at_rva(rva, n).unwrap();
            for (i, c) in b.chunks(16).enumerate() { println!("{:08x}  {}", rva as usize + i * 16, c.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")); }
        }
        "ptrs" => {
            let t = pe.va(hex(&args[1]) as u32).to_le_bytes();
            for s in pe.sections.iter().filter(|s| !s.is_exec()) {
                let d = &pe.data[s.raw_off as usize..(s.raw_off + s.raw_size) as usize];
                let mut i = 0; while i + 8 <= d.len() { if d[i..i + 8] == t { println!("{}:{:08x}", s.name, s.rva as usize + i); } i += 8; }
            }
        }
        "xrefs" | "callers" => {
            let idx = XrefIndex::build(&pe);
            if args[0] == "callers" { for c in idx.callers_of(hex(&args[1]) as u32) { println!("{c:x} in {:x?}", pe.func_containing(*c)); } return; }
            let lo = hex(&args[1]) as u32; let hi = args.get(2).map(|s| hex(s) as u32).unwrap_or(lo + 1);
            for x in idx.refs_in(lo, hi) { println!("{:?} from {:08x} (func {:x?}) -> {:08x}", x.kind, x.from, pe.func_containing(x.from).map(|f| f.0), x.to); }
        }
        _ => eprintln!("see source"),
    }
}












