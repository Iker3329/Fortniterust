//! Renders the 3D scene behind a front-end screen alone: `cargo run --release -p fn-app --example lobby_view_shot -- <Store|StoreItemInspect|Cosmetics> <out.png> [radius cm] [w h]`
#![allow(dead_code)]
#[path = "../src/lobby_world.rs"]
mod lobby_world;
#[path = "../src/lobby_scene.rs"]
mod lobby_scene;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cam = args.first().cloned().unwrap_or_else(|| "Store".into());
    let out = args.get(1).cloned().unwrap_or_else(|| "view.png".into());
    let radius: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(6000.);
    let (w, h): (u32, u32) = (args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1920), args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1080));
    let install = fn_ui::pak_assets::default_install();
    let assets = lobby_scene::LobbyAssets::load_view(&install, &cam, radius).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("failed meshes {:?}\nsubstituted {:?}\nlines {:?}", assets.report.failed_meshes, assets.report.substituted, assets.report.lines);
    eprintln!("mpc {:?} darken visible {:?}", assets.data.mpc, assets.data.statics.iter().filter(|s| s.component == "Mesh-DarkenBG").map(|s| s.visible).collect::<Vec<_>>());
    eprintln!("statics {} lights {:?}", assets.data.statics.len(), assets.data.lights.iter().map(|l| (l.component.clone(), l.visible)).collect::<Vec<_>>());
    if std::env::var_os("FN_VIEW_DEBUG").is_some() {
        let cam = assets.data.camera.as_ref().unwrap().xf.t;
        for s in assets.data.statics.iter().filter(|s| s.visible) {
            let Some(m) = assets.meshes.get(&s.mesh) else { continue };
            let c = s.xf.matrix().transform_point3(m.center);
            let mats: Vec<String> = (0..m.default_materials.len().max(s.material_overrides.len())).map(|i| {
                let p = s.material_overrides.get(i).cloned().flatten().or_else(|| m.default_materials.get(i).cloned().flatten());
                match p.and_then(|p| assets.materials.get(&p).map(|mm| (p.rsplit('/').next().unwrap_or("").to_owned(), mm.flags.two_sided))) { Some((n, t)) => format!("{n}(2s={t})"), None => "-".into() }
            }).collect();
            println!("{:<28} {:<26} center_off=({:.0},{:.0},{:.0}) radius {:.0} mats {:?}", s.actor, s.mesh.rsplit('/').next().unwrap_or(""), c.x - cam.x, c.y - cam.y, c.z - cam.z, m.radius * s.xf.s.max_element(), mats);
        }
    }
    let gpu = fn_render::Gpu::headless().expect("gpu");
    let mut scene = lobby_scene::LobbyScene::from_assets(assets, &gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8Unorm).unwrap_or_else(|e| panic!("{e}"));
    let target = fn_render::OffscreenTarget::new(&gpu.device, w, h, wgpu::TextureFormat::Rgba8Unorm);
    scene.render(&target.view, w, h, 6.0);
    let px = target.read_rgba8(&gpu);
    image::save_buffer(&out, &px, w, h, image::ColorType::Rgba8).expect("png");
    eprintln!("wrote {out}");
}
