use fn_exe_defaults::ctor::*;
use fn_exe_slate::*;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let pe = Pe::open(exe_path()).unwrap();
    let s = u32::from_str_radix(&a[0], 16).unwrap();
    let (b, e) = pe.func_containing(s).unwrap();
    println!("func {b:x}-{e:x}");
    for st in collect_stores(&pe, b, e) {
        let d = dwords(&st);
        let vs: Vec<String> = d.iter().map(|(o, x)| format!("+{o:#x}={x:#x}/{}", f32::from_bits(*x))).collect();
        println!("{:x} {:?} disp {:#x} len {} {}", st.rva, st.base, st.disp, st.bytes.len(), vs.join(" "));
    }
}


