//! Dumps the recovered lobby scene data (actors, lights, camera, placements, post volumes) as text.
//! `cargo run --release -p fn-app --example lobby_dump [-- <install>] [statics] [lights] [all]`
#![allow(dead_code)]
#[path = "../src/lobby_world.rs"]
mod lobby_world;

use lobby_world::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args.iter().find(|a| !["statics", "lights", "all", "post"].contains(&a.as_str())).map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("FN_ROOT").map(std::path::PathBuf::from))
        .unwrap_or_else(|| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let all = args.iter().any(|a| a == "all");
    let t0 = std::time::Instant::now();
    let d = match read_lobby_scene(&root) { Ok(d) => d, Err(e) => { eprintln!("ERROR {e}"); std::process::exit(1) } };
    println!("loaded in {:?}", t0.elapsed());
    let sel = d.selection.as_ref().unwrap();
    println!("selection: LobbySelect={} label={} -> {}", sel.lobby_select, sel.lobby_select_label, sel.map);
    for (n, a, t) in &sel.frontend_streaming { println!("  streaming {n}: {a} offset {t:?}"); }
    for (t, a) in &sel.camera_pairs { println!("  camera {t} -> {a}"); }
    let c = d.camera.as_ref().unwrap();
    println!("camera {} ({}): loc {:?} rot {:?} fov {} aspect {} ppw {} child {:?}", c.actor, c.actor_class, c.xf.t, c.xf.rotator_deg(), c.fov_deg, c.aspect_ratio, c.post_process_blend_weight, c.child_camera.map(|(x, f)| (x.t, x.rotator_deg(), f)));
    for p in &d.placements { println!("placement {} id {} hidden {} onpad {} root {:?} rot {:?} char {:?} rot {:?}", p.actor, p.player_id, p.hidden, p.player_on_pad, p.root.t, p.root.rotator_deg(), p.character.t, p.character.rotator_deg()); }
    println!("statics {} (visible {}), lights {}, fogs {}, particles {}, skeletals {}, post volumes {}", d.statics.len(), d.statics.iter().filter(|s| s.visible).count(), d.lights.len(), d.fogs.len(), d.particles.len(), d.skeletals.len(), d.post_volumes.len());
    if all || args.iter().any(|a| a == "statics") {
        for s in &d.statics { println!("S {:<34} {:<26} vis={} t=({:.1},{:.1},{:.1}) rot={:?} s={:?} mesh={} mats={:?} [{}]", s.actor, s.component, s.visible, s.xf.t.x, s.xf.t.y, s.xf.t.z, s.xf.rotator_deg().map(|v| (v * 100.).round() / 100.), s.xf.s, s.mesh.rsplit('/').next().unwrap_or(""), s.material_overrides.iter().map(|m| m.as_deref().map(|m| m.rsplit('/').next().unwrap_or("")).unwrap_or("-")).collect::<Vec<_>>(), s.source); }
    }
    if all || args.iter().any(|a| a == "lights") {
        for l in &d.lights { println!("L {:?} {:<30} {:<22} vis={} aw={} I={} col={:?} T={}({}) att={} cone={}/{} shadows={} inv={} fexp={} mob={:?} lf={:?} ies={:?} ch={:?} ind={} spec={} t=({:.0},{:.0},{:.0}) rot={:?} cube={:?} [{}]", l.kind, l.actor, l.component, l.visible, l.affects_world, l.intensity, l.color_srgb, l.temperature, l.use_temperature, l.attenuation_radius, l.inner_cone_deg, l.outer_cone_deg, l.cast_shadows, l.inverse_squared, l.falloff_exponent, l.mobility, l.light_function, l.ies_texture, l.lighting_channels, l.indirect_intensity, l.specular_scale, l.xf.t.x, l.xf.t.y, l.xf.t.z, l.xf.rotator_deg().map(|v| (v * 10.).round() / 10.), l.cubemap, l.source); }
        for f in &d.fogs { println!("F {} vis={} density={} falloff={} maxop={} col={:?} direxp={} start={} vol={} t={:?}", f.actor, f.visible, f.density, f.height_falloff, f.max_opacity, f.inscattering_color, f.directional_exponent, f.start_distance, f.volumetric, f.xf.t); }
        for p in &d.particles { println!("P {} {} vis={} auto={} {:?} t={:?}", p.actor, p.component, p.visible, p.auto_activate, p.template, p.xf.t); }
        for s in &d.skeletals { println!("K {} {:?} t={:?} mats={:?}", s.actor, s.mesh, s.xf.t, s.materials); }
    }
    for v in &d.post_volumes { println!("PPV {} [{}] prio {} bounds {:?} lut {:?} blendables {:?}", v.name, v.map, v.priority, v.bounds, v.lut, v.blendables); }
    println!("post: lut {:?} blendables {:?} overridden {}", d.post.color_grading_lut, d.post.blendables, d.post.overridden.len());
    for k in ["FilmSlope", "FilmToe", "FilmShoulder", "FilmBlackClip", "FilmWhiteClip", "FilmContrast", "FilmToeAmount", "AutoExposureMinBrightness", "AutoExposureMaxBrightness", "AutoExposureBias", "BloomIntensity", "BloomThreshold", "VignetteIntensity", "WhiteTemp", "AmbientOcclusionIntensity", "ColorGradingIntensity", "DepthOfFieldFocalDistance", "DepthOfFieldFarBlurSize"] { println!("  {k} = {:?}", d.post.get(k)); }
    for k in ["ColorSaturationShadows", "ColorSaturationMidtones", "ColorSaturationHighlights"] { println!("  {k} = {:?}", d.post.v4(k)); }
    for u in &d.unrendered { println!("UNRENDERED {u:?}"); }
    for m in &d.missing { println!("MISSING {m}"); }
}
