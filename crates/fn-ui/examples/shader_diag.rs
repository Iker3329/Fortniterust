//! Diagnostic: which shader blobs a material package carries and how `SlateMat` classifies them.
//! `cargo run --release -p fn-ui --example shader_diag -- /Game/Path/Pkg`
use fn_dxbc::extract::scan_blobs;
use fn_dxbc::{Opcode, ProgramType, Shader};
fn main() {
    let long = std::env::args().nth(1).expect("package");
    let install = fn_ui::pak_assets::default_install();
    let set = fn_pak::PakSet::open_install(&install).unwrap();
    let x = set.read_game(&long, "uexp").unwrap().into_owned();
    let blobs = scan_blobs(&x);
    println!("{long}: {} blobs", blobs.len());
    for b in &blobs {
        match Shader::parse(&b.bytes) {
            Ok(sh) => {
                let discards = sh.body().filter(|i| i.op == Opcode::Discard).count();
                let ins: Vec<u32> = sh.decls.inputs.iter().map(|d| d.reg).collect();
                let cbs: Vec<u32> = sh.decls.cbuffers.iter().map(|c| c.slot).collect();
                let reads_cb0_3 = sh.body().any(|i| i.src().iter().any(|o| o.is(fn_dxbc::OperandType::CONSTANT_BUFFER) && o.index.len() >= 2 && o.index[0].imm == 0 && o.index[1].imm == 3));
                let dp3_dis = sh.body().any(|i| i.op == Opcode::Dp3 && i.src().iter().any(|o| o.is_imm() && o.imm.iter().any(|&b| b == 0x3e99999a)));
                println!("  blob {} {:?} v{} discards {discards} inputs {ins:?} cbuffers {cbs:?} body {} instrs reads_cb0[3]={reads_cb0_3} disabled_effect={dp3_dis}", b.id, sh.program_type, sh.major, sh.body().count());
                let _ = ProgramType::Pixel;
            }
            Err(e) => println!("  blob {}: parse error {e:?}", b.id),
        }
    }
}
