//! Dev tool: compose a widget class of the 1.8 paks through the generic UMG interpreter and dump the tree with arranged rectangles.
//! `cargo run --release -p fn-ui --example locker_dump -- <class path> [--w 1920 --h 1080]`
use fn_ui::scene::UiContext;
use fn_ui::screens::athena_lobby::{layout, store::Store, umg::Ui};
use std::rc::Rc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let class = args.first().cloned().unwrap_or_else(|| "/Game/Athena/UI/Customization/AthenaCustomizationTab.AthenaCustomizationTab_C".into());
    let get = |k: &str, d: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| d.to_owned());
    let (w, h): (f32, f32) = (get("--w", "1920").parse().unwrap(), get("--h", "1080").parse().unwrap());
    let install = fn_ui::pak_assets::default_install();
    let mut uic = UiContext::from_install(&install, None).expect("ui context");
    let store = Rc::new(Store::open(&install).expect("store"));
    let mut ui = Ui::new(store);
    let root = ui.add_class_instance(&class, None, "", serde_json::json!({}), None).expect("compose");
    let scale = fn_ui::layout::dpi_scale(w, h);
    let lay = layout::run(&mut ui, &mut uic, root, [w / scale, h / scale], scale, &[]);
    let mut s = String::new();
    ui.dump(root, &mut s, 0, Some(&lay.rect));
    println!("{s}");
    eprintln!("log: {:?}\nunresolved: {:?}", ui.log, ui.unresolved.borrow());
}
