use fn_exe_defaults::{ana::*, cvar::*};
use fn_exe_slate::*;
use std::collections::BTreeMap;
fn main() {
    let t = std::time::Instant::now();
    let pe = Pe::open(exe_path()).unwrap();
    let xi = build_xrefs(&pe);
    eprintln!("xrefs {:?}", t.elapsed());
    let v = scan(&pe, &xi);
    eprintln!("regs {} {:?}", v.len(), t.elapsed());
    let mut h: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for r in &v { let e = h.entry(r.vtable_slot).or_default(); e.0 += 1; if r.value != CvarValue::Unknown { e.1 += 1; } }
    println!("slots (count, resolved): {h:x?}");
    let f = std::env::args().nth(1).unwrap_or_default();
    for r in &v { if r.name.contains(&f) { println!("{:<50} slot {:x} {:?} flags {:x?} f {:x}", r.name, r.vtable_slot, r.value, r.flags, r.func_rva); } }
}
