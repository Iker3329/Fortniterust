//! Dumps a vtable of the 1.8 exe and flags the virtual functions that contain a given float immediate / rdata float (static read only).
//! `cargo run --release -p fn-exe-defaults --example vtab -- <vtable rva hex> <count> [float]`
use fn_exe_defaults::ana::*;
use fn_exe_slate::*;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let pe = Pe::open(exe_path()).unwrap();
    let rva = u32::from_str_radix(&a[0], 16).unwrap();
    let n: u32 = a[1].parse().unwrap();
    let want: Option<f32> = a.get(2).and_then(|s| s.parse().ok());
    for i in 0..n {
        let Some(p) = pe.read_u64(rva + i * 8) else { break };
        let Some(f) = pe.rva_of_va(p) else { println!("{i}: {p:x}"); continue };
        let mut note = String::new();
        if let (Some(w), Some((b, e))) = (want, pe.func_containing(f)) {
            let text = fmt_dis(&pe, b, 400);
            let _ = e;
            let hit = text.lines().any(|l| (l.contains("mulss") || l.contains("movss") || l.contains("mov ")) && (l.contains(&format!("f32={w} ")) || l.to_lowercase().contains(&format!("{:#x}", w.to_bits())))); if hit { note = format!(" <-- uses {w}"); }
        }
        println!("{i}: fn {f:x}{note}");
    }
}
