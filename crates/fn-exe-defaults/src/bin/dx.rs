use fn_exe_defaults::ana::*;
use fn_exe_slate::*;
fn hex(s: &str) -> u32 { u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap() }
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let pe = Pe::open(exe_path()).unwrap();
    match a[0].as_str() {
        "dis" => print!("{}", fmt_dis(&pe, hex(&a[1]), a.get(2).map(|x| x.parse().unwrap()).unwrap_or(60))),
        "ptrs" => {
            for s in &a[1..] {
                for r in ascii_exact(&pe, s) {
                    let va = pe.image_base + r as u64;
                    for sec in pe.sections.iter().filter(|s| s.name == ".rdata" || s.name == ".data") {
                        let mut o = 0u32;
                        while o + 16 <= sec.raw_size {
                            let rv = sec.rva + o;
                            if pe.read_u64(rv) == Some(va) { let f = pe.read_u64(rv + 8).unwrap_or(0); println!("{s} table entry at {rv:x} -> fn {:x}", f.wrapping_sub(pe.image_base)); }
                            o += 8;
                        }
                    }
                }
            }
        }
        "xr2" => {
            let xi = build_xrefs(&pe);
            for s in &a[1..] {
                for r in utf16_exact(&pe, s) {
                    println!("string {s:?} at rva {r:x}");
                    for d in [0u32, 2] { for x in xi.refs_to(r + d) { println!("   +{d} {} from {:x} func {:?}", kind_s(x.kind), x.from, pe.func_containing(x.from).map(|f| format!("{:x}-{:x}", f.0, f.1))); } }
                }
            }
        }
        "xr" => {
            let xi = build_xrefs(&pe);
            for s in &a[1..] {
                for r in utf16_exact(&pe, s).into_iter().chain(ascii_exact(&pe, s)) {
                    println!("string {s:?} at rva {r:x}");
                    for x in xi.refs_to(r) { println!("   {} from {:x} func {:?}", kind_s(x.kind), x.from, pe.func_containing(x.from).map(|f| format!("{:x}-{:x}", f.0, f.1))); }
                }
            }
        }
        _ => {}
    }
}


