//! Read-only: list every RIP-relative reference in a code range whose target lies in [tlo, thi]. Usage: scan_lea <code-lo> <code-hi> <target-lo> <target-hi> (hex RVAs)
use fn_exe_slate::*;
use iced_x86::{Decoder, DecoderOptions, Formatter, IntelFormatter, Instruction};

fn hex(s: &str) -> u64 { u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap() }

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (lo, hi, tlo, thi) = (hex(&a[0]) as u32, hex(&a[1]) as u32, hex(&a[2]), hex(&a[3]));
    let pe = Pe::open(exe_path()).unwrap();
    let mut f = IntelFormatter::new();
    f.options_mut().set_hex_prefix("0x"); f.options_mut().set_hex_suffix(""); f.options_mut().set_uppercase_hex(false);
    let bytes = pe.bytes_at_rva(lo, (hi - lo) as usize).unwrap();
    let mut dec = Decoder::with_ip(64, bytes, pe.image_base + lo as u64, DecoderOptions::NONE);
    let mut ins = Instruction::default();
    while dec.can_decode() {
        dec.decode_out(&mut ins);
        if ins.is_ip_rel_memory_operand() {
            let t = ins.ip_rel_memory_address() - pe.image_base;
            if t >= tlo && t <= thi {
                let r = (ins.ip() - pe.image_base) as u32;
                let fb = pe.func_group(r).map(|x| x.0).unwrap_or(0);
                let mut s = String::new(); f.format(&ins, &mut s);
                println!("{r:08x} (func {fb:x})  {s}");
            }
        }
    }
}
