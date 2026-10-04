//! Headless captures of the lobby UI layer: `parity/lobby/ui-<w>x<h>.png` (transparent) and `ui-backdrop-<w>x<h>.png` (over `T_LobbyCastle_Matte`).
//! `cargo run --release -p fn-app --example lobby_ui_capture [-- <out dir>]`
#[path = "../src/lobby_ui.rs"]
mod lobby_ui;

use fn_ui::draw::{RenderList, TextureId};
use fn_ui::layout::Rect;

fn save(px: &[u8], w: u32, h: u32, path: &std::path::Path) {
    let mut rgba = px.to_vec();
    for p in rgba.chunks_exact_mut(4) { let a = p[3] as u32; if a > 0 && a < 255 { for c in 0..3 { p[c] = ((p[c] as u32 * 255 + a / 2) / a).min(255) as u8; } } }
    image::save_buffer(path, &rgba, w, h, image::ColorType::Rgba8).expect("png");
    eprintln!("wrote {}", path.display());
}

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "parity/lobby".into()));
    std::fs::create_dir_all(&out).unwrap();
    let install = fn_ui::pak_assets::default_install();
    let mut ui = lobby_ui::LobbyUi::load(&install).unwrap_or_else(|e| panic!("{e}"));
    let state = lobby_ui::LobbyState::default();
    for (w, h) in [(1920u32, 1080u32), (1280, 720)] {
        // demo of the pad API (anchors are arbitrary demo points, not from any reference): ui-pads-<w>x<h>.png over the backdrop
        {
            let (fw, fh) = (w as f32, h as f32);
            let st = lobby_ui::LobbyState { pads: vec![lobby_ui::Pad { x: fw * 0.5, y: fh * 0.25, pivot: [0.5, 1.0], kind: lobby_ui::PadKind::Player { name: None, leader: true, ready: false } }, lobby_ui::Pad { x: fw * 0.35, y: fh * 0.55, pivot: [0.5, 0.5], kind: lobby_ui::PadKind::Empty }, lobby_ui::Pad { x: fw * 0.65, y: fh * 0.55, pivot: [0.5, 0.5], kind: lobby_ui::PadKind::Empty }], ..Default::default() };
            let frame = ui.draw(fw, fh, &st);
            let a = ui.ctx_mut();
            let mut list = RenderList::new();
            let t = a.assets.texture("T_LobbyCastle_Matte.png");
            if t.id != TextureId::NONE { let s = (fw / t.w as f32).max(fh / t.h as f32); let (dw, dh) = (t.w as f32 * s, t.h as f32 * s); list.sprite(t.id, Rect::new((fw - dw) * 0.5, (fh - dh) * 0.5, dw, dh), [1.0; 4]); }
            list.extend(frame.list);
            save(&fn_ui::raster::render(&list, &mut a.assets, w, h, None), w, h, &out.join(format!("ui-pads-{w}x{h}.png")));
        }
        let frame = ui.draw(w as f32, h as f32, &state);
        if !frame.missing.is_empty() { eprintln!("unresolved: {:?}", frame.missing); }
        let a = ui.ctx_mut();
        save(&fn_ui::raster::render(&frame.list, &mut a.assets, w, h, None), w, h, &out.join(format!("ui-{w}x{h}.png")));
        let mut list = RenderList::new();
        let t = a.assets.texture("T_LobbyCastle_Matte.png");
        if t.id != TextureId::NONE {
            let s = (w as f32 / t.w as f32).max(h as f32 / t.h as f32);
            let (dw, dh) = (t.w as f32 * s, t.h as f32 * s);
            list.sprite(t.id, Rect::new((w as f32 - dw) * 0.5, (h as f32 - dh) * 0.5, dw, dh), [1.0; 4]);
        }
        list.extend(frame.list);
        save(&fn_ui::raster::render(&list, &mut a.assets, w, h, None), w, h, &out.join(format!("ui-backdrop-{w}x{h}.png")));
    }
}

