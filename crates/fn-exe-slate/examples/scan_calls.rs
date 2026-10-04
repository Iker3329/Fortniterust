//! Read-only: list every direct call/jmp to a target RVA within an RVA range. Usage: scan_calls <target-rva-hex> <lo-hex> <hi-hex>
use fn_exe_slate::*;
use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction};

fn hex(s: &str) -> u64 { u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap() }

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let target = hex(&a[0]) as u32;
    let (lo, hi) = (hex(&a[1]) as u32, hex(&a[2]) as u32);
    let pe = Pe::open(exe_path()).unwrap();
    let bytes = pe.bytes_at_rva(lo, (hi - lo) as usize).unwrap();
    let mut dec = Decoder::with_ip(64, bytes, pe.image_base + lo as u64, DecoderOptions::NONE);
    let mut ins = Instruction::default();
    while dec.can_decode() {
        dec.decode_out(&mut ins);
        if matches!(ins.flow_control(), FlowControl::Call | FlowControl::UnconditionalBranch) && ins.near_branch_target() == pe.image_base + target as u64 {
            let r = (ins.ip() - pe.image_base) as u32;
            let fb = pe.func_group(r).map(|x| x.0).unwrap_or(0);
            println!("{r:08x}  in func {fb:x}  {:?}", ins.mnemonic());
        }
    }
}
