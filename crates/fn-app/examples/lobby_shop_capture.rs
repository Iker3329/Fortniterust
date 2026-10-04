//! Headless captures of the offline Item Shop screen (`parity/lobby/shop-*.png`) and of the F2 console overlay (`console-*.png`), software rasteriser.
//! `cargo run --release -p fn-app --example lobby_shop_capture [-- <out dir>]`
#![allow(dead_code)]
#[path = "../src/lobby_world.rs"]
mod lobby_world;
#[path = "../src/lobby_scene.rs"]
mod lobby_scene;
#[path = "../src/lobby_profile.rs"]
mod lobby_profile;
#[path = "../src/lobby_shop.rs"]
mod lobby_shop;
#[path = "../src/lobby_console.rs"]
mod lobby_console;

use fn_ui::draw::{RenderList, TextureId};
use fn_ui::layout::Rect;
use fn_ui::screens::athena_lobby::store::Store;
use fn_ui::screens::athena_shop::{AthenaShopUi, Dialog, ShopState};
use std::rc::Rc;

fn save(px: &[u8], w: u32, h: u32, path: &std::path::Path) {
    let mut rgba = px.to_vec();
    for p in rgba.chunks_exact_mut(4) { let a = p[3] as u32; if a > 0 && a < 255 { for c in 0..3 { p[c] = ((p[c] as u32 * 255 + a / 2) / a).min(255) as u8; } } }
    image::save_buffer(path, &rgba, w, h, image::ColorType::Rgba8).expect("png");
    eprintln!("wrote {}", path.display());
}

/// The Item Shop backdrop: the 3D scene of `SetFrontEndCamera(3)` (`StoreCamera_Blueprint`, `LobbyAssets::load_backdrop`) darkened by `LobbyScreen::ItemShop.darken()`, rendered once per size.
fn backdrop_px(scene: &mut lobby_scene::LobbyScene, gpu: &fn_render::Gpu, w: u32, h: u32) -> Vec<u8> {
    let target = fn_render::OffscreenTarget::new(&gpu.device, w, h, wgpu::TextureFormat::Rgba8Unorm);
    scene.render(&target.view, w, h, 6.0);
    let mut px = target.read_rgba8(gpu);
    if let Some(d) = lobby_scene::LobbyScreen::ItemShop.darken() {
        for p in px.chunks_exact_mut(4) { for c in 0..3 { p[c] = (p[c] as f32 * (1.0 - d[3]) + d[c] * 255.0 * d[3]).round().clamp(0.0, 255.0) as u8; } }
    }
    px
}

/// UI layer (premultiplied raster of `list`) over the opaque backdrop pixels.
fn compose(bg: &[u8], list: &RenderList, uic: &mut fn_ui::scene::UiContext, w: u32, h: u32) -> Vec<u8> {
    let layer = fn_ui::raster::render(list, &mut uic.assets, w, h, None);
    let mut px = bg.to_vec();
    for (d, s) in px.chunks_exact_mut(4).zip(layer.chunks_exact(4)) {
        let ia = 255 - s[3] as u32;
        for c in 0..3 { d[c] = (s[c] as u32 + (d[c] as u32 * ia + 127) / 255).min(255) as u8; }
        d[3] = 255;
    }
    px
}
fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "parity/lobby".into()));
    std::fs::create_dir_all(&out).unwrap();
    let install = fn_ui::pak_assets::default_install();
    let store = Rc::new(Store::open(&install).expect("install"));
    let shop = lobby_shop::Shop::load(&store);
    let mut profile = lobby_profile::Profile::new(shop.catalog.default_grants());
    let mut ui = AthenaShopUi::from_store(store.clone()).unwrap_or_else(|e| panic!("{e}"));
    let mut uic = fn_ui::scene::UiContext::from_install(&install, None).expect("ui context");
    // a fixed instant (2026-10-04 15:00 UTC) so the captures are reproducible
    let now = 1_791_126_000;
    let gpu = fn_render::Gpu::headless().expect("gpu");
    let assets = lobby_scene::LobbyAssets::load_backdrop(&install, "Store", None).unwrap_or_else(|e| panic!("store scene: {e}"));
    let mut scene = lobby_scene::LobbyScene::from_assets(assets, &gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8Unorm).unwrap_or_else(|e| panic!("store scene: {e}"));
    let sizes = [(1920u32, 1080u32), (1280, 720)];
    for (w, h) in sizes {
        let bg = backdrop_px(&mut scene, &gpu, w, h);
        let mut st = ShopState { selected_tab: ui.item_shop_tab, mtx_balance: profile.balance(), view: shop.view(&profile, now), ..Default::default() };
        let mut shot = |name: &str, st: &ShopState, uic: &mut fn_ui::scene::UiContext, ui: &mut AthenaShopUi| {
            let frame = ui.draw(uic, w as f32, h as f32, st);
            if !frame.missing.is_empty() { eprintln!("{name}: unresolved: {:?}", frame.missing); }
            let px = compose(&bg, &frame.list, uic, w, h);
            image::save_buffer(out.join(format!("{name}-{w}x{h}.png")), &px, w, h, image::ColorType::Rgba8).expect("png");
            eprintln!("wrote {name}-{w}x{h}.png");
            frame.offers.clone()
        };
        if let Some(p) = std::env::var_os("FN_SHOP_DUMP") { if w == 1920 { std::fs::write(p, ui.dump(&mut uic, w as f32, h as f32, &st)).unwrap(); } }
        let offers = shot("shop", &st, &mut uic, &mut ui);
        if let Some((id, _)) = offers.get(3) {
            st.hovered_offer = Some(id.clone());
            st.hovered_tab = Some(0);
        }
        shot("shop-hover", &st, &mut uic, &mut ui);
        st.hovered_tab = None;
        let first = st.view.weekly[0].clone();
        st.dialog = Dialog::Details { offer: first.clone(), purchase_hovered: false };
        shot("shop-details", &st, &mut uic, &mut ui);
        // buy it and show the purchased widget
        let r = shop.buy(&mut profile, &first.offer_id, now).expect("buy");
        eprintln!("bought {} for {} -> balance {}", first.name, r.price, r.balance_after);
        st.mtx_balance = profile.balance();
        st.view = shop.view(&profile, now);
        st.dialog = Dialog::Purchased { offer: st.view.find(&first.offer_id).unwrap().clone() };
        shot("shop-purchased", &st, &mut uic, &mut ui);
        st.dialog = Dialog::None;
        st.hovered_offer = None;
        shot("shop-after-purchase", &st, &mut uic, &mut ui);
        profile.reset();
        // the F2 command bar over the storefront: a few executed commands in the log, then a half-typed command with its suggestion list
        {
            let mut console = lobby_console::Console::new();
            console.open();
            let st = ShopState { selected_tab: ui.item_shop_tab, mtx_balance: profile.balance(), view: shop.view(&profile, now), ..Default::default() };
            let mut run = |console: &mut lobby_console::Console, profile: &mut lobby_profile::Profile, line: &str| {
                console.set_input(line);
                let mut env = lobby_console::Env { profile, shop: &shop, now };
                console.key(lobby_console::Key::Enter, &mut env);
            };
            run(&mut console, &mut profile, "addvbucks 13500");
            run(&mut console, &mut profile, "setskin skull trooper");
            run(&mut console, &mut profile, "giveitem trooper");
            for (name, typed, tabs) in [("console-commands", "g", 0), ("console-items", "giveitem sk", 0), ("console-amounts", "addvbucks ", 0), ("console-tab", "setpickaxe fl", 1)] {
                console.set_input(typed);
                for _ in 0..tabs { let mut env = lobby_console::Env { profile: &mut profile, shop: &shop, now }; console.key(lobby_console::Key::Tab, &mut env); }
                let shot_state = ShopState { mtx_balance: profile.balance(), ..st.clone() };
                let frame = ui.draw(&mut uic, w as f32, h as f32, &shot_state);
                let mut list = RenderList::new();
                list.extend(frame.list);
                let env = lobby_console::Env { profile: &mut profile, shop: &shop, now };
                list.extend(console.draw(&mut uic, w as f32, h as f32, &env, 0.0));
                let px = compose(&bg, &list, &mut uic, w, h);
                image::save_buffer(out.join(format!("{name}-{w}x{h}.png")), &px, w, h, image::ColorType::Rgba8).expect("png");
            }
            profile.reset();
        }
    }
}
