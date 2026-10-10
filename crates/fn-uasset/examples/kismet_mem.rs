//! Dev tool (MATERIAL PARAMETERS, 2026-10-06): `kismet_mem <long package path> <function export name>` - the function's statements with BOTH offsets: the
//! in-memory offset the bytecode's jumps / `pushflow` / Ubergraph entry points use (file offset + 4 per pointer operand before it: object / property / function
//! pointers are 8 bytes loaded and 4 in the cooked file, an FName operand 12 vs 8; the rule fn-ui `athena_settings::kismet` runs the Settings graphs with) and
//! the file offset `fn-inspect disasm` prints. Install root from `$FN_ROOT`.
use fn_pak::PakSet;
use fn_uasset::kismet::Expr;
use fn_uasset::PackageData;

/// Pointer operands of an expression tree (copy of fn-ui `athena_settings::kismet::ptr_count`).
fn ptr_count(e: &Expr) -> usize {
    use fn_uasset::kismet::TextLit;
    use Expr::*;
    let own = match e {
        LocalVariable(_) | InstanceVariable(_) | DefaultVariable(_) | LocalOutVariable(_) | ObjectConst(_) | PropertyConst(_) => 1,
        Let { .. } | LetValueOnPersistentFrame { .. } | ClassContext { .. } | Context { .. } | ContextFailSilent { .. } | StructMemberContext { .. } => 1,
        MetaCast { .. } | DynamicCast { .. } | ObjToInterfaceCast { .. } | CrossInterfaceCast { .. } | InterfaceToObjCast { .. } => 1,
        FinalFunction { .. } | LocalFinalFunction { .. } | CallMath { .. } | CallMulticastDelegate { .. } => 1,
        StructConst { .. } | ArrayConst { .. } => 1,
        TextConst(TextLit::StringTable { .. }) => 1,
        VirtualFunction { .. } | LocalVirtualFunction { .. } | NameConst(_) | InstanceDelegate(_) | BindDelegate { .. } => 1,
        _ => 0,
    };
    own + e.children().into_iter().map(ptr_count).sum::<usize>()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let set = PakSet::open_install(&root).unwrap();
    let d = PackageData::load(&set, &a[1]).unwrap();
    let p = d.package().unwrap();
    for (i, e) in p.exports.iter().enumerate() {
        if a.get(2).map_or(false, |n| &e.name != n) || p.export_class_name(i) != "Function" { continue; }
        let f = match fn_uasset::classes::read_function(&p, i) { Ok(f) => f, Err(err) => { println!("{}: {err}", e.name); continue } };
        let st = match fn_uasset::kismet::parse(&p, f.script) { Ok(s) => s, Err(err) => { println!("{}: {err}", e.name); continue } };
        println!("== {} (file {} bytes, memory {} bytes)", e.name, f.script.len(), f.script_bytecode_size);
        let mut extra = 0usize;
        for s in &st {
            println!("{:6} {:5x}: {}", s.offset + extra, s.offset, s.expr.render());
            extra += 4 * ptr_count(&s.expr);
        }
    }
}
