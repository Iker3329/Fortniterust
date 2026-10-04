//! Writes `reports/exe-loading-screen.json` (workspace root) from the analysed exe. Usage: `gen-report [out.json]`.
use fn_exe_slate::*;
fn main() {
    let pe = Pe::open(exe_path()).expect("open exe");
    let idx = XrefIndex::build(&pe);
    let f = findings::recover(&pe, &idx);
    for e in &f.errors { eprintln!("ERROR {e}"); }
    let v = report::build(&pe, &idx, &f);
    let out = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reports/exe-loading-screen.json"));
    if let Some(d) = out.parent() { std::fs::create_dir_all(d).unwrap(); }
    std::fs::write(&out, serde_json::to_string_pretty(&v).unwrap() + "\n").unwrap();
    println!("wrote {} ({} findings, {} errors)", out.display(), f.items.len(), f.errors.len());
}
