//! Disassembles the function of the 1.8 exe that contains an RVA: `cargo run --release -p fn-exe-defaults --example disfn -- <rva hex> [max lines]`
use fn_exe_defaults::ana::*;
use fn_exe_slate::*;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let pe = Pe::open(exe_path()).unwrap();
    let rva = u32::from_str_radix(&a[0], 16).unwrap();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(400);
    let (b, e) = pe.func_containing(rva).expect("no function");
    println!("fn {b:x}-{e:x}");
    println!("{}", fmt_dis(&pe, b, n));
}
