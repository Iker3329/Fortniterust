//! Dev tool for the Item Shop screen (`screens::athena_shop`): dump the composed tree / render it with the software rasteriser.
//! `cargo run --release -p fn-ui --example shop_ui -- [--w 1920 --h 1080] [--out shop.png] [--dump]`
use fn_ui::scene::UiContext;
use fn_ui::screens::athena_shop::{AthenaShopUi, OfferView, ShopState, ShopView};

fn offer(id: &str, name: &str, rarity: usize, price: i64, owned: bool) -> OfferView {
    OfferView { offer_id: format!("offer:{id}"), name: name.into(), short_description: "Outfit".into(), description: "A test offer.".into(), rarity, price, owned, tile_image: None, icon: None, attributes: vec![("Outfit".into(), "1".into())], is_glider: false, is_outfit: true }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str, d: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| d.to_owned());
    let (w, h): (u32, u32) = (get("--w", "1920").parse().unwrap(), get("--h", "1080").parse().unwrap());
    let install = fn_ui::pak_assets::default_install();
    let mut uic = UiContext::from_install(&install, None).expect("ui context");
    let mut shop = AthenaShopUi::load(&install).unwrap_or_else(|e| panic!("load: {e}"));
    let view = ShopView { weekly: vec![offer("a", "Skull Trooper", 3, 1500, false), offer("b", "Voyager", 2, 1200, false)], daily: (0..6).map(|i| offer(&format!("d{i}"), &format!("Daily {i}"), i % 5, 800 + 100 * i as i64, i == 2)).collect(), week_ends_in: 3 * 86400 + 7200, day_ends_in: 5 * 3600 + 120 };
    if let Some(i) = args.iter().position(|a| a == "--class") { println!("{}", shop.dump_class(&mut uic, w as f32, h as f32, &args[i + 1])); return; }
    let st = ShopState { selected_tab: shop.item_shop_tab, mtx_balance: 1_000_000, view, ..Default::default() };
    if args.iter().any(|a| a == "--dump") { println!("{}", shop.dump(&mut uic, w as f32, h as f32, &st)); }
    let out = get("--out", "");
    if out.is_empty() { return; }
    let frame = shop.draw(&mut uic, w as f32, h as f32, &st);
    eprintln!("[shop] {} commands; missing: {:?}", frame.list.cmds.len(), frame.missing);
    let px = fn_ui::raster::render(&frame.list, &mut uic.assets, w, h, None);
    let mut rgba = px.clone();
    for p in rgba.chunks_exact_mut(4) { let a = p[3] as u32; if a > 0 && a < 255 { for c in 0..3 { p[c] = ((p[c] as u32 * 255 + a / 2) / a).min(255) as u8; } } }
    image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).expect("save png");
    eprintln!("[shop] wrote {out}");
}
