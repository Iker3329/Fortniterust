//! Prints what the front-end camera of a screen sees: `cargo run --release -p fn-app --example lobby_view_probe -- <Store|StoreItemInspect|Cosmetics|...> [radius cm]`
#![allow(dead_code)]
#[path = "../src/lobby_world.rs"]
mod lobby_world;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cam = args.first().cloned().unwrap_or_else(|| "Store".into());
    let radius: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(6000.);
    let set = fn_pak::PakSet::open_install(&fn_ui::pak_assets::default_install()).expect("paks");
    let d = lobby_world::read_view_scene_from(&set, &cam, radius).unwrap_or_else(|e| panic!("{e}"));
    let c = d.camera.as_ref().unwrap();
    println!("camera {} ({}): loc {:?} rot {:?} fov {} aspect {} ppw {}", c.actor, c.actor_class, c.xf.t, c.xf.rotator_deg(), c.fov_deg, c.aspect_ratio, c.post_process_blend_weight);
    for u in &d.unrendered { println!("{u:?}"); }
    for s in &d.statics { println!("S {:<34} {:<24} vis={} t=({:.0},{:.0},{:.0}) rot={:?} s={:?} mesh={} mats={:?}", s.actor, s.component, s.visible, s.xf.t.x, s.xf.t.y, s.xf.t.z, s.xf.rotator_deg(), s.xf.s, s.mesh.rsplit('/').next().unwrap_or(""), s.material_overrides.iter().map(|m| m.as_deref().map(|m| m.rsplit('/').next().unwrap_or("")).unwrap_or("-")).collect::<Vec<_>>()); }
    for l in &d.lights { println!("L {:?} {} {} vis={} I={} t=({:.0},{:.0},{:.0})", l.kind, l.actor, l.component, l.visible, l.intensity, l.xf.t.x, l.xf.t.y, l.xf.t.z); }
    for f in &d.fogs { println!("F {} vis={} density={}", f.actor, f.visible, f.density); }
    for p in &d.particles { println!("P {} {} {:?}", p.actor, p.component, p.template); }
    for v in &d.post_volumes { println!("PPV {} [{}] prio {} lut {:?} blendables {:?}", v.name, v.map, v.priority, v.lut, v.blendables); }
    println!("post lut {:?} blendables {:?} overridden {:?}", d.post.color_grading_lut, d.post.blendables, d.post.overridden);
    for k in ["FilmSlope", "FilmToe", "FilmShoulder", "FilmBlackClip", "FilmWhiteClip", "AutoExposureMinBrightness", "AutoExposureMaxBrightness", "AutoExposureBias", "BloomIntensity", "VignetteIntensity", "DepthOfFieldFocalDistance", "DepthOfFieldFarBlurSize", "DepthOfFieldNearBlurSize", "DepthOfFieldMethod", "SceneColorTint", "GrainIntensity"] { println!("  {k} = {:?}", d.post.get(k)); }
    for v in &d.post_volumes { println!("PPV {} bounds {:?} unbound {:?}", v.name, v.bounds, v.priority); }
}

