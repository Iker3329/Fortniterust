//! GPU tests of the textured static-mesh path (need a GPU and the `..\FortniteRuntime` asset folder; skipped with a message otherwise):
//! alpha test, two-sided lighting, alpha-tested shadows, per-instance batching, LOD choice / frustum + distance culling, the real exported
//! props (when `assets\props` exists) and validation-clean rendering.
#![cfg(test)]
use crate::{parse_args, Session};
use fn_render::synthetic::*;
use fn_render::{CameraParams, StaticMeshHandle, StaticMeshInstance};
use glam::{Mat4, Vec3};

const W: u32 = 640;
const H: u32 = 360;

fn session() -> Option<Session> {
    let cli = parse_args(&["--size".into(), format!("{W}x{H}"), "--no-props".into(), "--fxaa-level".into(), "0".into()]).ok()?;
    match Session::open(&cli) { Ok(s) => Some(s), Err(e) => { eprintln!("skipped: {e}"); None } }
}
fn shoot(s: &mut Session, cam: CameraParams, instances: &[StaticMeshInstance]) -> Vec<u8> { shoot_with(s, cam, instances, &[]) }
fn shoot_with(s: &mut Session, cam: CameraParams, instances: &[StaticMeshInstance], extra: &[&str]) -> Vec<u8> {
    s.scene.statics = instances.to_vec();
    s.scene.animate(0.0);
    s.cam = cam;
    let mut args: Vec<String> = vec!["--exposure".into(), "1.5".into()];
    args.extend(extra.iter().map(|a| a.to_string()));
    let cli = parse_args(&args).unwrap();
    s.render(&cli, &cli.options, 1).rgba
}
fn px(img: &[u8], x: i32, y: i32) -> [f32; 3] {
    let (x, y) = (x.clamp(0, W as i32 - 1) as usize, y.clamp(0, H as i32 - 1) as usize);
    let p = &img[(y * W as usize + x) * 4..];
    [p[0] as f32, p[1] as f32, p[2] as f32]
}
fn luma(c: [f32; 3]) -> f32 { 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2] }
fn is_yellow(c: [f32; 3]) -> bool { let m = c[0].max(c[1]); m > 70. && (c[0] - c[1]).abs() < 0.18 * m && c[2] < 0.5 * c[1].min(c[0]) }
fn is_blue(c: [f32; 3]) -> bool { c[2] > c[0] * 1.8 && c[2] > c[1] * 1.8 && c[2] > 40. }
fn count(img: &[u8], f: impl Fn([f32; 3]) -> bool) -> usize { (0..(W * H) as usize).filter(|&i| f([img[i * 4] as f32, img[i * 4 + 1] as f32, img[i * 4 + 2] as f32])).count() }
fn project(cam: &CameraParams, p: Vec3) -> (i32, i32) {
    let q = (cam.proj_matrix() * cam.view_matrix()).project_point3(p);
    (((q.x * 0.5 + 0.5) * W as f32) as i32, ((1. - (q.y * 0.5 + 0.5)) * H as f32) as i32)
}
fn quad(s: &mut Session, two_sided: bool, masked: bool) -> StaticMeshHandle {
    let mut m = checker_quad_material();
    m.two_sided = two_sided;
    if !masked { m.alpha_mode = fn_render::AlphaMode::Opaque; }
    let h = s.scene.renderer.create_static_material(&m);
    s.scene.renderer.create_static_mesh(&checker_quad_mesh("quad"), &[h])
}
fn save(img: &[u8], name: &str) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reports/statics");
    let _ = std::fs::create_dir_all(&dir);
    let _ = image::save_buffer(dir.join(name), img, W, H, image::ColorType::Rgba8);
}

#[test]
fn alpha_test_discards_transparent_texels_and_opaque_keeps_them() {
    let Some(mut s) = session() else { return };
    let cam = CameraParams::from_yaw_pitch(Vec3::new(0., 3., -1.), 0., -30., W, H);
    let masked = quad(&mut s, true, true);
    let opaque = quad(&mut s, true, false);
    let a = shoot(&mut s, cam, &[StaticMeshInstance::new(masked, Vec3::new(0., 0., -6.))]);
    save(&a, "test-alpha-masked.png");
    let b = shoot(&mut s, cam, &[StaticMeshInstance::new(opaque, Vec3::new(0., 0., -6.))]);
    save(&b, "test-alpha-opaque.png");
    let (ya, ba, yb, bb) = (count(&a, is_yellow), count(&a, is_blue), count(&b, is_yellow), count(&b, is_blue));
    eprintln!("masked: yellow {ya} blue {ba}; opaque: yellow {yb} blue {bb}");
    assert!(ya > 500 && yb > 500, "the opaque texels must render");
    assert!(bb > 500, "without the mask the transparent (blue) texels show: detector works");
    assert!(ba * 50 < ya, "alpha test left {ba} blue pixels (yellow {ya})");
}

/// Pixels that differ from the baseline render of the same camera (the quad); returns (count, mean luma of those pixels).
fn changed(img: &[u8], base: &[u8]) -> (usize, f32) {
    let v: Vec<f32> = (0..(W * H) as usize).filter(|&i| (0..3).any(|c| (img[i * 4 + c] as i32 - base[i * 4 + c] as i32).abs() > 24)).map(|i| luma([img[i * 4] as f32, img[i * 4 + 1] as f32, img[i * 4 + 2] as f32])).collect();
    (v.len(), v.iter().sum::<f32>() / v.len().max(1) as f32)
}

#[test]
fn two_sided_quads_are_lit_from_both_sides_and_one_sided_ones_cull_their_back() {
    let Some(mut s) = session() else { return };
    let two = quad(&mut s, true, true);
    let one = quad(&mut s, false, true);
    // quad plane x = 0, normal +X (yaw 90 deg), the sun is at +X: the +X side is the lit front.
    let place = |m| [StaticMeshInstance::new(m, Vec3::new(0., 0., -6.)).with_yaw(std::f32::consts::FRAC_PI_2)];
    let front_cam = CameraParams::from_yaw_pitch(Vec3::new(4., 1.2, -6.), -90., -5., W, H);
    let back_cam = CameraParams::from_yaw_pitch(Vec3::new(-4., 1.2, -6.), 90., -5., W, H);
    let (base_f, base_b) = (shoot(&mut s, front_cam, &[]), shoot(&mut s, back_cam, &[]));
    let (base_fn, base_bn) = (shoot_with(&mut s, front_cam, &[], &["--no-shadows"]), shoot_with(&mut s, back_cam, &[], &["--no-shadows"]));
    let f = shoot(&mut s, front_cam, &place(two));
    let b = shoot(&mut s, back_cam, &place(two));
    save(&f, "test-twosided-front.png");
    save(&b, "test-twosided-back.png");
    let ((nf, lf), (nb, lb)) = (changed(&f, &base_f), changed(&b, &base_b));
    eprintln!("two-sided front: {nf} px luma {lf:.1}; back: {nb} px luma {lb:.1}");
    assert!(nf > 3000 && nb > 3000);
    assert!(lb > 35. && lb > 0.3 * lf, "back face too dark: {lb:.1} vs front {lf:.1}");
    // (shadows off: the culled back face still casts its shadow, which would show up in the diff)
    let (nof, _) = changed(&shoot_with(&mut s, front_cam, &place(one), &["--no-shadows"]), &base_fn);
    let (nob, _) = changed(&shoot_with(&mut s, back_cam, &place(one), &["--no-shadows"]), &base_bn);
    eprintln!("one-sided: front {nof} px, back {nob} px");
    assert!(nof > 3000, "front of the one-sided quad must draw");
    assert!(nob < 30, "one-sided back face must be culled, saw {nob} px");
}
#[test]
fn leaf_shadows_are_alpha_tested_in_the_cascades() {
    let Some(mut s) = session() else { return };
    let masked = quad(&mut s, true, true);
    let opaque = quad(&mut s, true, false);
    let cam = CameraParams::from_yaw_pitch(Vec3::new(-0.6, 7., -8.), 0., -89., W, H);
    let place = |m| [StaticMeshInstance::new(m, Vec3::new(0., 0., -8.)).with_yaw(std::f32::consts::FRAC_PI_2)];
    // shadow of the 2 m quad on the ground: x in [-1.1, 0], z in [-9, -7]
    let dark_fraction = |img: &[u8]| -> f32 {
        let lit = luma(px(img, project(&cam, Vec3::new(-4., 0., -8.)).0, project(&cam, Vec3::new(-4., 0., -8.)).1));
        let (mut dark, mut n) = (0, 0);
        for i in 0..20 { for j in 0..20 {
            let p = Vec3::new(-1.0 + 0.9 * i as f32 / 19. - 0.05, 0., -8.9 + 1.8 * j as f32 / 19.);
            let (x, y) = project(&cam, p);
            if luma(px(img, x, y)) < 0.8 * lit { dark += 1; }
            n += 1;
        } }
        dark as f32 / n as f32
    };
    let a = shoot(&mut s, cam, &place(masked));
    let b = shoot(&mut s, cam, &place(opaque));
    save(&a, "test-shadow-masked.png");
    save(&b, "test-shadow-opaque.png");
    let (fa, fb) = (dark_fraction(&a), dark_fraction(&b));
    eprintln!("shadow dark fraction: masked {fa:.2}, opaque {fb:.2}");
    assert!(fb > 0.85, "the opaque quad shadows the whole rectangle ({fb:.2})");
    assert!(fa > 0.25 && fa < 0.75, "the masked quad shadow is checker-shaped ({fa:.2})");
}

#[test]
fn many_instances_batch_into_a_handful_of_draw_calls_and_lod_and_culling_work() {
    let Some(mut s) = session() else { return };
    let (bark, leaf) = (s.scene.renderer.create_static_material(&bark_material("b")), s.scene.renderer.create_static_material(&foliage_material("l", 1)));
    let tree = s.scene.renderer.create_static_mesh(&tree_mesh("t", 4), &[bark, leaf]);
    let cam = CameraParams::from_yaw_pitch(Vec3::new(0., 1.7, 0.), 0., 0., W, H);
    // 500 trees in a 25 x 20 grid in front of the camera, 12..80 m
    let grid: Vec<_> = (0..500).map(|i| StaticMeshInstance::new(tree, Vec3::new((i % 25) as f32 * 3. - 36., 0., -12. - (i / 25) as f32 * 3.4)).with_yaw(i as f32)).collect();
    shoot(&mut s, cam, &grid);
    let st = s.scene.renderer.static_stats().clone();
    eprintln!("{st:?}");
    assert_eq!(st.instances, 500);
    assert!(st.visible > 100 && st.visible < 500, "frustum culling removes the trees beside the view: {}", st.visible);
    assert!(st.draw_calls <= 6, "batched by (mesh, lod, section): {} draw calls for {} trees", st.draw_calls, st.visible);
    assert!(st.lod_counts[0] > 0 && st.lod_counts[1] > 0, "{:?}", st.lod_counts);
    assert_eq!(st.lod_counts.iter().sum::<u32>(), st.visible);
    assert!(st.shadow_instances > 0 && st.shadow_draw_calls > 0);
    // distance cull + behind the camera
    let far = [StaticMeshInstance::new(tree, Vec3::new(0., 0., -400.)), StaticMeshInstance::new(tree, Vec3::new(0., 0., 20.)), StaticMeshInstance::new(tree, Vec3::new(0., 0., -20.))];
    shoot(&mut s, cam, &far);
    let st = s.scene.renderer.static_stats().clone();
    assert_eq!(st.culled_distance, 1, "{st:?}");
    assert_eq!(st.culled_frustum, 1, "{st:?}");
    assert_eq!(st.visible, 1);
}

#[test]
fn lod_hysteresis_holds_the_lod_while_the_camera_hovers_at_the_switch_distance() {
    let Some(mut s) = session() else { return };
    let (bark, leaf) = (s.scene.renderer.create_static_material(&bark_material("b")), s.scene.renderer.create_static_material(&foliage_material("l", 1)));
    let tree = s.scene.renderer.create_static_mesh(&tree_mesh("t", 4), &[bark, leaf]);
    let inst = [StaticMeshInstance::new(tree, Vec3::new(0., 0., -40.))];
    // LOD 0 -> 1 at 35 m, back at 35 * 0.9 = 31.5 m (and out at 35 * 1.1 = 38.5 m). Start far (LOD 1), then hover across 35 m: no flip.
    let at = |s: &mut Session, d: f32| -> usize {
        let cam = CameraParams::from_yaw_pitch(Vec3::new(0., 3.5, -40. + d), 0., 0., W, H); 
        shoot(s, cam, &inst);
        let c = s.scene.renderer.static_stats().lod_counts;
        if c[0] == 1 { 0 } else if c[1] == 1 { 1 } else { 2 }
    };
    let mut seq = vec![at(&mut s, 45.)];
    for k in 0..8 { seq.push(at(&mut s, if k % 2 == 0 { 36.5 } else { 33. })); }
    let hover_start = 1;
    assert!(seq[hover_start..].iter().all(|&l| l == 1), "LOD flickers while hovering at the switch distance: {seq:?}");
    assert_eq!(seq[0], 1);
    assert_eq!(at(&mut s, 28.), 0, "well inside the band the detailed LOD returns");
    eprintln!("lod sequence {seq:?}");
}

#[test]
fn shadow_matrices_do_not_cull_casters_between_sun_and_view() {
    // a tall tree behind the camera casts into the view: its cascade lists must be non-empty even though it is outside the frustum
    let Some(mut s) = session() else { return };
    let (bark, leaf) = (s.scene.renderer.create_static_material(&bark_material("b")), s.scene.renderer.create_static_material(&foliage_material("l", 1)));
    let tree = s.scene.renderer.create_static_mesh(&tree_mesh("t", 4), &[bark, leaf]);
    let cam = CameraParams::from_yaw_pitch(Vec3::new(0., 1.7, 0.), 0., 0., W, H);
    // sun is at +X: a tree at x = 14 (beside the view, outside the frustum) shadows ground in front of the camera
    shoot(&mut s, cam, &[StaticMeshInstance::new(tree, Vec3::new(14., 0., -3.))]);
    let st = s.scene.renderer.static_stats().clone();
    eprintln!("{st:?}");
    assert_eq!(st.visible, 0, "the tree is outside the view");
    assert!(st.shadow_instances > 0, "...but it still casts a shadow into it");
}

#[test]
fn real_exported_props_render_textured_and_validation_clean() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../FortniteRuntime/assets/props");
    if !dir.join("catalog.json").exists() { eprintln!("skipped: no exported props at {}", dir.display()); return; }
    let Some(mut s) = session() else { return };
    let set = match fn_render::static_loader::load_dir(&mut s.scene.renderer, &dir) { Ok(v) => v, Err(e) => panic!("load_dir: {e}") };
    assert!(!set.props.is_empty());
    eprintln!("{} real props, {} materials, skipped {:?}", set.props.len(), set.material_count, set.skipped);
    let mut inst = vec![];
    for (i, p) in set.props.iter().enumerate() { inst.push(StaticMeshInstance::new(p.mesh, Vec3::new((i % 5) as f32 * 9. - 18., 0., -14. - (i / 5) as f32 * 9.)).with_yaw(i as f32)); }
    let cam = CameraParams::from_yaw_pitch(Vec3::new(0., 3.0, 6.), 0., -6., W, H);
    let img = shoot(&mut s, cam, &inst);
    save(&img, "test-real-props.png");
    let st = s.scene.renderer.static_stats();
    assert!(st.visible > 0 && st.draw_calls > 0);
    let _ = Mat4::IDENTITY;
}
