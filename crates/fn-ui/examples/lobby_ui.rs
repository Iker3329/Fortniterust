//! Dev tool for the BR lobby UI (`screens::athena_lobby`): dump the composed tree and/or render it with the software rasteriser.
//!
//! `cargo run --release -p fn-ui --example lobby_ui -- [--w 1920 --h 1080] [--out ui.png] [--backdrop] [--dump] [--tab N]`
//! The PNG is straight-alpha RGBA (transparent over nothing, or over `T_LobbyCastle_Matte` with `--backdrop`).
use fn_ui::draw::RenderList;
use fn_ui::layout::Rect;
use fn_ui::scene::UiContext;
use fn_ui::screens::athena_lobby::{AthenaLobbyUi, LobbyState, Pad, PadKind};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str, d: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| d.to_owned());
    let (w, h): (u32, u32) = (get("--w", "1920").parse().unwrap(), get("--h", "1080").parse().unwrap());
    let install = fn_ui::pak_assets::default_install();
    let t0 = std::time::Instant::now();
    let mut uic = UiContext::from_install(&install, None).expect("ui context");
    let mut lobby = AthenaLobbyUi::load(&install).unwrap_or_else(|e| panic!("load: {e}"));
    eprintln!("[lobby] loaded in {:?}; log: {:?}", t0.elapsed(), lobby.composition_log);
    let mut st = LobbyState::default();
    if let Some(i) = args.iter().position(|a| a == "--tab") { st.selected_tab = args[i + 1].parse().unwrap(); }
    if args.iter().any(|a| a == "--pads") {
        // demo anchors (NOT from any reference): a leader plate in the middle and two empty party slots
        let (fw, fh) = (w as f32, h as f32);
        st.pads = vec![Pad { x: fw * 0.5, y: fh * 0.25, pivot: [0.5, 1.0], kind: PadKind::Player { name: None, leader: true, ready: false } }, Pad { x: fw * 0.35, y: fh * 0.55, pivot: [0.5, 0.5], kind: PadKind::Empty }, Pad { x: fw * 0.65, y: fh * 0.55, pivot: [0.5, 0.5], kind: PadKind::Empty }];
    }
    if args.iter().any(|a| a == "--dump") { println!("{}", lobby.dump(&mut uic, w as f32, h as f32, &st)); }
    let out = get("--out", "");
    if out.is_empty() { return; }
    let t1 = std::time::Instant::now();
    let frame = lobby.draw(&mut uic, w as f32, h as f32, &st);
    eprintln!("[lobby] drew {} commands in {:?}; missing: {:?}", frame.list.cmds.len(), t1.elapsed(), frame.missing);
    let mut list = RenderList::new();
    if args.iter().any(|a| a == "--backdrop") {
        let t = uic.assets.texture("T_LobbyCastle_Matte.png");
        if t.id != fn_ui::draw::TextureId::NONE {
            let (tw, th) = (t.w as f32, t.h as f32);
            let s = (w as f32 / tw).max(h as f32 / th);
            let (dw, dh) = (tw * s, th * s);
            list.sprite(t.id, Rect::new((w as f32 - dw) * 0.5, (h as f32 - dh) * 0.5, dw, dh), [1.0; 4]);
        }
    }
    list.extend(frame.list);
    let px = fn_ui::raster::render(&list, &mut uic.assets, w, h, None);
    // premultiplied -> straight alpha for the PNG
    let mut rgba = px.clone();
    for p in rgba.chunks_exact_mut(4) { let a = p[3] as u32; if a > 0 && a < 255 { for c in 0..3 { p[c] = ((p[c] as u32 * 255 + a / 2) / a).min(255) as u8; } } }
    image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).expect("save png");
    eprintln!("[lobby] wrote {out}");
}

