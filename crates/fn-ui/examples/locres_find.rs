//! Research tool: list the (namespace, key) of every Game.locres entry with exactly the given text. `cargo run --release -p fn-ui --example locres_find -- "Global" ...`
use fn_ui::screens::athena_lobby::store::Store;
fn main() {
    let install = fn_ui::pak_assets::default_install();
    let store = Store::open(&install).unwrap();
    let l = store.locres().expect("locres");
    for t in std::env::args().skip(1) {
        let mut v = l.find_text(&t); v.sort();
        println!("{t:?}: {} hits", v.len());
        for (ns, k) in v { println!("  [{ns}] {k}"); }
    }
}
