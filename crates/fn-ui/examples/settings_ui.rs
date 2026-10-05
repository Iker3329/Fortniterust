//! Dev tool for the BR Settings screen (`screens::athena_settings`): render a tab with the software rasteriser (and optionally dump the composed tree).
//!
//! `cargo run --release -p fn-ui --example settings_ui -- [--w 1920 --h 1080] [--tab Video] [--sub HUD] --out settings.png [--dump] [--hover x,y] [--set key=value]`
//! The PNG is the UI over opaque black (the 3D scene is behind the screen in the game; its blurred copy is not drawn here).
use fn_settings::Settings;
use fn_ui::draw::RenderList;
use fn_ui::layout::Rect;
use fn_ui::scene::UiContext;
use fn_ui::screens::athena_lobby::store::Store;
use fn_ui::screens::athena_settings::host::SettingsBacking;
use fn_ui::screens::athena_settings::{Pointer, SettingsUi};
use std::rc::Rc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str, d: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| d.to_owned());
    let (w, h): (u32, u32) = (get("--w", "1920").parse().unwrap(), get("--h", "1080").parse().unwrap());
    let install = fn_ui::pak_assets::default_install();
    let store = Rc::new(Store::open(&install).expect("store"));
    let mut uic = UiContext::from_install(&install, None).expect("ui context");
    let backing = SettingsBacking { settings: Settings::default(), sizes: vec![(1280, 720), (1600, 900), (1920, 1080), (2560, 1440)], in_zone: true, locres: store.locres() };
    let t0 = std::time::Instant::now();
    let mut s = SettingsUi::load(store.clone(), &backing, true).unwrap_or_else(|e| panic!("load: {e}"));
    eprintln!("[settings] loaded in {:?}; pages: {:?}", t0.elapsed(), s.pages.iter().map(|p| (&p.name, p.parent)).collect::<Vec<_>>());
    for l in &s.log { eprintln!("[settings] log: {l}"); }
    let tab = get("--tab", "");
    if !tab.is_empty() {
        let i = s.top_pages().iter().position(|&p| s.pages[p].name.eq_ignore_ascii_case(&tab)).unwrap_or_else(|| panic!("no tab {tab}"));
        s.select_tab(&backing, i);
    }
    let sub = get("--sub", "");
    if !sub.is_empty() { let p = s.top_pages()[s.active]; s.sub_active = s.sub_pages(p).iter().position(|&q| s.pages[q].name.eq_ignore_ascii_case(&sub)).unwrap_or(0); }
    let hover = get("--hover", "");
    let mut ptr = Pointer::default();
    if let Some((x, y)) = hover.split_once(',') { ptr = Pointer { x: x.parse().unwrap(), y: y.parse().unwrap(), present: true, down: false, clicked: false }; }
    let t1 = std::time::Instant::now();
    let (frame, _ev) = s.frame(&mut uic, &backing, w as f32, h as f32, ptr);
    eprintln!("[settings] frame in {:?}: {} commands; missing: {:?}", t1.elapsed(), frame.list.cmds.len(), frame.missing);
    if args.iter().any(|a| a == "--dump") {
        let mut out = String::new();
        let scale = fn_ui::layout::dpi_scale(w as f32, h as f32);
        for (i, wd) in s.ui.w.iter().enumerate() {
            let r = frame.id_rects[i];
            if r.w <= 0.0 && r.h <= 0.0 { continue; }
            out.push_str(&format!("{:4} {} \"{}\" {:?} [{:.1},{:.1} {:.1}x{:.1}]{}\n", i, wd.class, wd.name, wd.vis, r.x / scale, r.y / scale, r.w / scale, r.h / scale, wd.text.as_ref().map(|t| format!(" text={t:?}")).unwrap_or_default()));
        }
        println!("{out}");
    }
    let out = get("--out", "");
    if out.is_empty() { return; }
    let mut list = RenderList::new();
    list.fill(Rect::new(0., 0., w as f32, h as f32), [0., 0., 0., 1.]);
    list.extend(frame.list);
    let px = fn_ui::raster::render(&list, &mut uic.assets, w, h, None);
    let mut rgba = px.clone();
    for p in rgba.chunks_exact_mut(4) { let a = p[3] as u32; if a > 0 && a < 255 { for c in 0..3 { p[c] = ((p[c] as u32 * 255 + a / 2) / a).min(255) as u8; } } }
    image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).expect("save png");
    eprintln!("[settings] wrote {out}");
}
