//! Renders the 3D lobby scene alone (no UI) to PNG files.
//! `cargo run --release -p fn-app --example lobby_shot -- <out dir> [time_s] [WxH ...]`  (default sizes 1920x1080 and 1280x720)
#![allow(dead_code)]
#[path = "../src/lobby_world.rs"]
mod lobby_world;
#[path = "../src/lobby_scene.rs"]
mod lobby_scene;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = std::path::PathBuf::from(args.first().cloned().unwrap_or_else(|| ".".into()));
    let time: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(6.0);
    let mut sizes: Vec<(u32, u32)> = args.iter().skip(2).filter_map(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).collect();
    if sizes.is_empty() { sizes = vec![(1920, 1080), (1280, 720)]; }
    let root = std::env::var_os("FN_ROOT").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let gpu = fn_render::Gpu::headless().expect("gpu");
    eprintln!("gpu: {}", gpu.describe());
    let t0 = std::time::Instant::now();
    let mut scene = match lobby_scene::LobbyScene::load(&root, &gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8Unorm) { Ok(s) => s, Err(e) => { eprintln!("ERROR {e}"); std::process::exit(1) } };
    eprintln!("loaded in {:?}", t0.elapsed());
    for l in &scene.report.lines { eprintln!("  {l}"); }
    for (m, why) in &scene.report.substituted { eprintln!("  NOT ORIGINAL {m}: {why}"); }
    for (m, why) in &scene.report.failed_meshes { eprintln!("  MESH FAILED {m}: {why}"); }
    let c = scene.camera_xf();
    eprintln!("camera {} {:?} fov {}", c.t, c.rotator_deg(), scene.fov_deg());
    for s in scene.party_slots() {
        eprintln!("slot {} hidden={} occupied={} pawn={:?} add@1920x1080={:?} top@1920x1080={:?}", s.player_id, s.hidden, s.occupied, s.pawn_world_cm, scene.project(s.add_widget_cm, 1920, 1080), scene.project(s.top_widget_cm, 1920, 1080));
    }
    for (w, h) in sizes {
        let target = fn_render::OffscreenTarget::new(&gpu.device, w, h, wgpu::TextureFormat::Rgba8Unorm);
        scene.render(&target.view, w, h, time);
        let px = target.read_rgba8(&gpu);
        std::fs::create_dir_all(&out).ok();
        let name = if (w, h) == (1920, 1080) { "scene-1920x1080.png".to_owned() } else { format!("scene-{w}x{h}.png") };
        image::save_buffer(out.join(&name), &px, w, h, image::ColorType::Rgba8).expect("png");
        eprintln!("wrote {}", out.join(&name).display());
        // sanity check only (never tuned): mean sRGB colour of three screen regions, as fractions of the frame (x0, y0, x1, y1)
        for (label, r) in [("sky (top centre)", [0.30, 0.02, 0.70, 0.22]), ("hero torso", [0.45, 0.32, 0.55, 0.45]), ("foreground (bottom third)", [0.0, 0.67, 1.0, 1.0]), ("whole frame", [0.0, 0.0, 1.0, 1.0])] {
            let (x0, y0, x1, y1) = ((r[0] * w as f32) as usize, (r[1] * h as f32) as usize, (r[2] * w as f32) as usize, (r[3] * h as f32) as usize);
            let (mut s, mut n) = ([0f64; 3], 0f64);
            for y in y0..y1 { for x in x0..x1 { let i = (y * w as usize + x) * 4; for c in 0..3 { s[c] += px[i + c] as f64; } n += 1.0; } }
            eprintln!("  mean {label}: rgb({:.0}, {:.0}, {:.0})", s[0] / n, s[1] / n, s[2] / n);
        }
    }
}
