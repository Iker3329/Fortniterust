//! Research tool: `fxflow <package> [function]` Kismet disassembly with the IN-MEMORY offsets (every object pointer is 8 bytes in memory, 4 on disk) so that the jump,
//! push-flow and `ExecuteUbergraph(entry)` targets of the cooked script line up with the statements.
use fn_uasset::kismet::{disassemble, Expr};
use fn_uasset::PackageData;

fn refs(e: &Expr) -> usize {
    use Expr::*;
    let own = match e {
        LocalVariable(_) | InstanceVariable(_) | DefaultVariable(_) | LocalOutVariable(_) | ObjectConst(_) | PropertyConst(_) => 1,
        Let { .. } | LetValueOnPersistentFrame { .. } | ClassContext { .. } | Context { .. } | ContextFailSilent { .. } | MetaCast { .. } | DynamicCast { .. } | ObjToInterfaceCast { .. } | CrossInterfaceCast { .. } | InterfaceToObjCast { .. } | FinalFunction { .. } | LocalFinalFunction { .. } | CallMath { .. } | CallMulticastDelegate { .. } | StructConst { .. } | ArrayConst { .. } | StructMemberContext { .. } => 1,
        TextConst(fn_uasset::kismet::TextLit::StringTable { .. }) => 1,
        // FName operands: 8 bytes on disk, 12 in memory (FScriptName keeps the display index)
        VirtualFunction { .. } | LocalVirtualFunction { .. } | NameConst(_) | InstanceDelegate(_) | BindDelegate { .. } => 1,
        _ => 0,
    };
    own + e.children().iter().map(|c| refs(c)).sum::<usize>()
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pkg_path = args.first().ok_or("package")?;
    let want = args.get(1).cloned();
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let data = PackageData::load(&set, pkg_path).map_err(|e| e.to_string())?;
    let pkg = data.package().map_err(|e| e.to_string())?;
    for i in 0..pkg.exports.len() {
        if pkg.export_class_name(i) != "Function" { continue; }
        if let Some(w) = &want { if &pkg.exports[i].name != w { continue; } }
        let Ok(f) = fn_uasset::classes::read_function(&pkg, i) else { continue };
        let dis = disassemble(&pkg, f.script);
        println!("== [{}] {} (script {} bytes on disk)", i, pkg.exports[i].name, f.script.len());
        let mut mem = 0usize;
        for (k, s) in dis.statements.iter().enumerate() {
            let next = dis.statements.get(k + 1).map(|n| n.offset).unwrap_or(f.script.len());
            let size = next - s.offset;
            println!("  m{mem:5}: {}", s.expr.render());
            mem += size + 4 * refs(&s.expr);
        }
    }
    Ok(())
}

