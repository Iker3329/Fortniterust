use fn_exe_defaults::{ana::*, classes::*, ctor::*};
use fn_exe_slate::*;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let pe = Pe::open(exe_path()).unwrap();
    let xi = build_xrefs(&pe);
    for arg in &a {
        let (cn, hint) = arg.split_once(":").unwrap_or((arg.as_str(), ""));
        let Some(mut ci) = find_class(&pe, &xi, cn) else { println!("{cn}: NOT FOUND"); continue };
        if !hint.is_empty() { if let Some((r, p)) = props_by_hint_max(&pe, &xi, hint, ci.sizeof_class) { ci.z_construct = Some(r); ci.props = p; } }
        println!("== {cn}: sizeof {:#x} GPSC {:x} thunk {:x} ctor {:x} zc {:x?} props {}", ci.sizeof_class, ci.get_private_static_class, ci.default_ctor_thunk, ci.ctor, ci.z_construct, ci.props.len());
        let (b, e) = pe.func_containing(ci.ctor).unwrap();
        println!("ctor func {b:x}-{e:x}");
        for st in collect_stores(&pe, b, e) {
            for (o, x) in dwords(&st) {
                let names: Vec<&str> = ci.props.iter().filter(|p| p.offset as i64 == o).map(|p| p.name.as_str()).collect();
                println!("  {:x} {:?}+{o:#x} = {x:#x} / {} {:?}", st.rva, st.base, f32::from_bits(x), names);
            }
            if st.bytes.len() < 4 { println!("  {:x} {:?}+{:#x} = byte {:x?}", st.rva, st.base, st.disp, st.bytes); }
        }
        let mut ps = ci.props.clone(); ps.sort_by_key(|p| p.offset);
        for p in ps { println!("    prop {:<40} off {:#x} size {:x?}", p.name, p.offset, p.obj_size); }
        let _ = fmt_dis;
    }
}


