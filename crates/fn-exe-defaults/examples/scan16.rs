//! Finds functions of the 1.8 exe where a float 16.0 load follows (within 12 instructions) a `call qword ptr [reg+0x30]` + `test al,al`: the light-brightness x16 sites. `cargo run --release -p fn-exe-defaults --example scan16`
use fn_exe_defaults::ana::*;
use fn_exe_slate::*;
fn main() {
    let pe = Pe::open(exe_path()).unwrap();
    for &(b, e) in &pe.funcs {
        let Some(bytes) = pe.bytes_at_rva(b, (e - b) as usize) else { continue };
        if !bytes.windows(3).any(|w| w[0] == 0xFF && (0x50..=0x57).contains(&w[1]) && w[2] == 0x30) { continue; }
        let text = fmt_dis(&pe, b, 8000);
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            if l.contains("call qword ptr [") && l.contains("+0x30]") && lines[i + 1..(i + 14).min(lines.len())].iter().any(|m| m.contains("f32=16 ")) {
                println!("fn {b:x}-{e:x} size {} call at {}", e - b, l.split_whitespace().next().unwrap_or(""));
            }
        }
    }
}
