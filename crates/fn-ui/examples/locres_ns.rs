//! Research tool: every Game.locres entry of a namespace. `cargo run --release -p fn-ui --example locres_ns -- OnlineAccount`
use fn_ui::screens::athena_lobby::store::Store;
fn main() {
    let install = fn_ui::pak_assets::default_install();
    let store = Store::open(&install).unwrap();
    let l = store.locres().expect("locres");
    let ns = std::env::args().nth(1).unwrap_or_default();
    let mut v: Vec<_> = l.find_containing("").into_iter().filter(|(n, _, _)| *n == ns).collect(); v.sort();
    for (n, k, text) in v { println!("[{n}] {k} = {text:?}"); }
}
