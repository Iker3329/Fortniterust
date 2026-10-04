//! Finds functions of the 1.8 exe whose bytes contain all the given little-endian disp32 values and prints the lines of their disassembly that mention a float immediate.
//! `cargo run --release -p fn-exe-defaults --example scanfn -- <disp hex>... [-- <f32 literal>]`
use fn_exe_defaults::ana::*;
use fn_exe_slate::*;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (disps, rest): (Vec<_>, Vec<_>) = { let p = a.iter().position(|s| s == "--"); match p { Some(p) => (a[..p].to_vec(), a[p + 1..].to_vec()), None => (a.clone(), vec![]) } };
    let want: Option<String> = rest.first().cloned();
    let pe = Pe::open(exe_path()).unwrap();
    let pats: Vec<[u8; 4]> = disps.iter().map(|d| u32::from_str_radix(d, 16).unwrap().to_le_bytes()).collect();
    for &(b, e) in &pe.funcs {
        let Some(bytes) = pe.bytes_at_rva(b, (e - b) as usize) else { continue };
        if !pats.iter().all(|p| bytes.windows(4).any(|w| w == p)) { continue };
        let text = fmt_dis(&pe, b, 500);
        let lines: Vec<&str> = text.lines().filter(|l| want.as_ref().map(|w| l.contains(w.as_str())).unwrap_or(true)).collect();
        if want.is_none() || !lines.is_empty() {
            println!("== fn {b:x}-{e:x}");
            for l in lines.iter().take(6) { println!("  {l}"); }
        }
    }
}
