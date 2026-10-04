//! Read-only: list every instruction in an RVA range whose memory operand has a given displacement (and optional base register filter).
//! Usage: scan_disp <disp-hex> <lo-rva-hex> <hi-rva-hex>
use fn_exe_slate::*;
use iced_x86::{Decoder, DecoderOptions, Formatter, IntelFormatter, Instruction, OpKind};

fn hex(s: &str) -> u64 { u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap() }

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let disp = hex(&a[0]);
    let (lo, hi) = (hex(&a[1]) as u32, hex(&a[2]) as u32);
    let pe = Pe::open(exe_path()).unwrap();
    let mut f = IntelFormatter::new();
    f.options_mut().set_hex_prefix("0x"); f.options_mut().set_hex_suffix(""); f.options_mut().set_uppercase_hex(false);
    let bytes = pe.bytes_at_rva(lo, (hi - lo) as usize).unwrap();
    let mut dec = Decoder::with_ip(64, bytes, pe.image_base + lo as u64, DecoderOptions::NONE);
    let mut ins = Instruction::default();
    let mut last_func = 0u32;
    while dec.can_decode() {
        dec.decode_out(&mut ins);
        for i in 0..ins.op_count() {
            if ins.op_kind(i) == OpKind::Memory && ins.memory_displacement64() == disp && !ins.is_ip_rel_memory_operand() {
                let r = (ins.ip() - pe.image_base) as u32;
                let fb = pe.func_group(r).map(|x| x.0).unwrap_or(0);
                if fb != last_func { println!("-- func {fb:x}"); last_func = fb; }
                let mut s = String::new(); f.format(&ins, &mut s);
                println!("{r:08x}  {s}");
                break;
            }
        }
    }
}
