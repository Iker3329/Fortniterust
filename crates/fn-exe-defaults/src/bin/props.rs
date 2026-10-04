use fn_exe_defaults::reflect::*;
use fn_exe_slate::*;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let pe = Pe::open(exe_path()).unwrap();
    let s = u32::from_str_radix(&a[0], 16).unwrap();
    let (b, e) = pe.func_containing(s).unwrap();
    println!("func {b:x}-{e:x}");
    for p in props_in(&pe, b, e) {
        println!("{:<40} off {:#x} size {:x?} flags {:x?} ctor {:x?}", p.name, p.offset, p.obj_size, p.flags, p.ctor_target);
    }
}
