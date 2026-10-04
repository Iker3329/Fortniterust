// TEMP (brskin task): list uassets whose name table contains any of the given names.
use rayon::prelude::*;
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let set = fn_pak::PakSet::open_install(fn_dxbc_cli::spec::default_install()).unwrap();
    let pats: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let ents: Vec<_> = set.effective().filter(|(_, e)| e.rel_path().ends_with(".uasset")).collect();
    let hits: Vec<String> = ents.par_iter().filter_map(|(pf, e)| {
        let b = pf.read(e).ok()?;
        for p in &pats { if b.windows(p.len()).any(|w| w == p.as_bytes()) { return Some(format!("{} [{}]", e.rel_path(), p)); } }
        None
    }).collect();
    for h in hits { println!("{h}"); }
}
