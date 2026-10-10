//! The full lobby frame: the 3D scene (`lobby_scene`) with the UI layer (`lobby_ui`) on top; the party name plate and "+" markers are
//! placed at the scene's projected pad widget anchors (`LobbyScene::party_slots` / `project`).
//! `cargo run --release -p fn-app --example lobby_composite -- <out dir> [time_s] [WxH ...]`  (default 1920x1080 and 1280x720)
#![allow(dead_code)]
#[path = "../src/lobby_world.rs"]
mod lobby_world;
#[path = "../src/lobby_scene.rs"]
mod lobby_scene;
#[path = "../src/lobby_ui.rs"]
mod lobby_ui;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = std::path::PathBuf::from(args.first().cloned().unwrap_or_else(|| "parity/lobby".into()));
    let time: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(6.0);
    let mut sizes: Vec<(u32, u32)> = args.iter().skip(2).filter_map(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).collect();
    if sizes.is_empty() { sizes = vec![(1920, 1080), (1280, 720)]; }
    std::fs::create_dir_all(&out).ok();
    let install = fn_ui::pak_assets::default_install();
    let gpu = fn_render::Gpu::headless().expect("gpu");
    let mut scene = lobby_scene::LobbyScene::load(&install, &gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8Unorm).unwrap_or_else(|e| panic!("scene: {e}"));
    let mut ui = lobby_ui::LobbyUi::load(&install).unwrap_or_else(|e| panic!("ui: {e}"));
    for (w, h) in sizes {
        let target = fn_render::OffscreenTarget::new(&gpu.device, w, h, wgpu::TextureFormat::Rgba8Unorm);
        scene.render(&target.view, w, h, time);
        let mut px = target.read_rgba8(&gpu);
        // the local player stands on the first visible occupied slot; every other visible slot shows the "+" invite marker
        let mut pads = vec![];
        for s in scene.party_slots().iter().filter(|s| !s.hidden) {
            if s.occupied {
                if let Some([x, y]) = scene.project(s.top_widget_cm, w, h) { pads.push(lobby_ui::Pad { x, y, pivot: s.top_pivot, kind: lobby_ui::PadKind::Player { name: None, leader: true, ready: false } }); }
            } else if let Some([x, y]) = scene.project(s.add_widget_cm, w, h) {
                pads.push(lobby_ui::Pad { x, y, pivot: s.add_pivot, kind: lobby_ui::PadKind::Empty });
            }
        }
        let state = lobby_ui::LobbyState { pads, ..Default::default() };
        let frame = ui.draw(w as f32, h as f32, &state);
        if !frame.missing.is_empty() { eprintln!("ui unresolved: {:?}", frame.missing); }
        let a = ui.ctx_mut();
        // INVENTORY FIX4 (verify3 V1): the UI is rasterised over the scene pixels in one framebuffer (the game's `render_texture` on the frame), so every
        // `BackgroundBlur` blurs the scene behind it (`SBackgroundBlur::OnPaint`); the scene is opaque, so the result is too
        for p in px.chunks_exact_mut(4) { p[3] = 255; }
        let mut px = fn_ui::raster::render_over(&frame.list, &mut a.assets, w, h, &px);
        for p in px.chunks_exact_mut(4) { p[3] = 255; }
        let path = out.join(format!("lobby-{w}x{h}.png"));
        image::save_buffer(&path, &px, w, h, image::ColorType::Rgba8).expect("png");
        eprintln!("wrote {}", path.display());
    }
}
