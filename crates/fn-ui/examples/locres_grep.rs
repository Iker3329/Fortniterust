//! Research tool: every Game.locres entry whose text contains the given substring. `cargo run --release -p fn-ui --example locres_grep -- "Days" ...`
use fn_ui::screens::athena_lobby::store::Store;
fn main() {
    let install = fn_ui::pak_assets::default_install();
    let store = Store::open(&install).unwrap();
    let l = store.locres().expect("locres");
    for t in std::env::args().skip(1) {
        let mut v = l.find_containing(&t); v.sort();
        println!("{t:?}: {} hits", v.len());
        for (ns, k, text) in v.iter().take(60) { println!("  [{ns}] {k} = {text:?}"); }
    }
}
