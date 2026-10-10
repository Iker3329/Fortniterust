//! GPU passes vs the CPU reference on the synthetic scene (skipped without a Vulkan/DX12 adapter).
//! Tolerance: 1/255 of the display-referred value (`harness::compare_rgb` divides by max(1, |reference|)).
use fn_postfx::cpu::{self, Image};
use fn_postfx::gpu::*;
use fn_postfx::harness::*;
use fn_postfx::params::*;
use fn_postfx::synth;

const TOL: f32 = 1.0 / 255.0;

struct Run {
    color: Option<Image>,
    bloom: Option<Image>,
    result: PostFxResult,
    flat: Option<Image>,
    tiles: Option<Image>,
    gathered: Option<Image>,
}

fn run(h: &Headless, scene: &synth::Scene, params: &PostFxParams, velocity: bool, want_bloom: bool) -> Run {
    let (w, ht) = (scene.color.w as u32, scene.color.h as u32);
    let color = upload_rgba16f(h, &scene.color, "scene color");
    let depth = upload_depth(h, &scene.depth, "scene depth");
    let vel = upload_rgba16f(h, &scene.velocity, "velocity");
    let out = render_target(h, w, ht, "out color");
    let bloom_out = render_target(h, cpu::half(w as usize) as u32, cpu::half(ht as usize) as u32, "out bloom");
    let mut fx = PostFx::new(&h.device, PostFxFormats::default());
    let mut enc = h.device.create_command_encoder(&Default::default());
    let result = fx.encode(
        &h.queue,
        &mut enc,
        &PostFxInputs { scene_color: &color.view, depth: &depth.view, velocity: velocity.then_some(&vel.view), params },
        &PostFxOutputs { color: &out.view, bloom: want_bloom.then_some(&bloom_out.view) },
    );
    h.queue.submit([enc.finish()]);
    let mut r = Run { color: None, bloom: None, result, flat: None, tiles: None, gathered: None };
    if result.color_written { r.color = Some(read_image(h, &out.tex, w, ht)); }
    if result.bloom_written { r.bloom = Some(read_image(h, &bloom_out.tex, bloom_out.w, bloom_out.h)); }
    if let Some((f, a, b)) = fx.debug_motion_textures() {
        if params.motion_blur.enabled {
            let (tw, th) = (w.div_ceil(16), ht.div_ceil(16));
            r.flat = Some(read_image(h, f, w, ht));
            r.tiles = Some(read_image(h, a, tw, th));
            r.gathered = Some(read_image(h, b, tw, th));
        }
    }
    r
}

fn only(p: &PostFxParams, dof: bool, mb: bool, bloom: bool) -> PostFxParams {
    let mut q = *p;
    q.dof.enabled = dof;
    q.motion_blur.enabled = mb;
    q.bloom.enabled = bloom;
    q
}

fn quantized(scene: &synth::Scene) -> (Image, Image) {
    let mut c = scene.color.clone();
    c.quantize_f16();
    (c, scene.depth.clone())
}

#[test]
fn dof_far_matches_cpu() {
    let Some(h) = Headless::new() else { eprintln!("no adapter, skipping"); return };
    let scene = synth::build(&synth::SceneOptions { width: 640, height: 360, ..Default::default() });
    // strong far blur so the pass is exercised (the Athena value 0.02 % is sub-pixel, tested separately below)
    let mut p = only(&scene.params, true, false, false);
    p.dof.far_blur_size = 3.0;
    p.dof.focal_distance = 3000.0; // 30 m
    p.dof.far_transition = 20000.0;
    let r = run(&h, &scene, &p, false, false);
    assert!(r.result.color_written);
    let (c, d) = quantized(&scene);
    let reference = cpu::dof_far_full(&d, &c, &p, true);
    let gpu = r.color.unwrap();
    let (mx, mean) = compare_rgb(&gpu, &reference);
    eprintln!("dof strong: max err {mx:.5} mean {mean:.6} ({:.2}/255 max)", mx * 255.0);
    // the pass must really change the picture
    let (change, _) = compare_rgb(&gpu, &c);
    assert!(change > 0.05, "DOF had no visible effect (max change {change})");
    // the 4.16 kernel exp(-16.7 (x/R)^2) is narrower than the old 3-sigma guess: steeper tap weights magnify the GPU's 8-bit bilinear
    // weight quantisation on the HDR edges (1.45/255 max, mean 0.03/255)
    assert!(mx <= 2.0 * TOL, "GPU vs CPU max {mx}");
}

#[test]
fn dof_athena_default_matches_cpu_and_is_subtle() {
    let Some(h) = Headless::new() else { return };
    let scene = synth::build(&synth::SceneOptions { width: 1280, height: 720, ..Default::default() });
    let p = only(&scene.params, true, false, false);
    let r = run(&h, &scene, &p, false, false);
    let (c, d) = quantized(&scene);
    let reference = cpu::dof_far_full(&d, &c, &p, true);
    let gpu = r.color.unwrap();
    let (mx, _) = compare_rgb(&gpu, &reference);
    let (change, mean_change) = compare_rgb(&gpu, &c);
    eprintln!("dof athena day: gpu vs cpu max {:.2}/255; effect vs no-dof: max {:.2}/255 mean {:.4}/255", mx * 255.0, change * 255.0, mean_change * 255.0);
    assert!(mx <= TOL);
}

#[test]
fn motion_blur_camera_and_object_matches_cpu() { motion_case(0.05, 0.2, "camera + object"); }

/// Static camera, fast object only: exercises the depth/velocity weighted branch of the shader.
#[test]
fn motion_blur_object_only_matches_cpu() { motion_case(0.0, 0.0, "object only"); }

fn motion_case(yaw: f32, strafe: f32, label: &str) {
    let Some(h) = Headless::new() else { return };
    let scene = synth::build(&synth::SceneOptions { width: 640, height: 360, yaw_delta: yaw, strafe, object_velocity: [0.05, 0.0], ..Default::default() });
    let mut p = only(&scene.params, false, true, false);
    p.motion_blur.amount = 1.0;
    p.motion_blur.max_percent = 8.0;
    let r = run(&h, &scene, &p, true, false);
    assert!(r.result.color_written);
    let (c, d) = quantized(&scene);
    let u = cpu::MbUniforms::new(&p);
    let fl = cpu::velocity_flatten(Some(&scene.velocity), &d, &u);
    let gathered = cpu::velocity_gather(&fl.tiles, &u);
    // intermediate textures (lengths/angles/depths are f32 on both sides; trig differs slightly)
    let gpu_flat = r.flat.unwrap();
    let mut worst_flat = 0.0f32;
    for (a, b) in gpu_flat.px.iter().zip(&fl.flat.px) {
        worst_flat = worst_flat.max((a[0] - b[0]).abs()).max((a[1] - b[1]).abs()).max((a[2] - b[2]).abs() / b[2].abs().max(1.0));
    }
    let tiles_err = r.tiles.as_ref().unwrap().px.iter().zip(&fl.tiles.px).map(|(a, b)| (0..4).map(|i| (a[i] - b[i]).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
    let gath_err = r.gathered.as_ref().unwrap().px.iter().zip(&gathered.px).map(|(a, b)| (0..4).map(|i| (a[i] - b[i]).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
    eprintln!("[{label}] flatten max err {worst_flat:.2e}, tiles {tiles_err:.2e}, gather {gath_err:.2e}");
    assert!(worst_flat < 2e-3 && tiles_err < 2e-3 && gath_err < 2e-3);
    let reference = cpu::motion_blur_full(&c, Some(&scene.velocity), &d, &u);
    let gpu = r.color.unwrap();
    let (mx, mean) = compare_rgb(&gpu, &reference);
    let (change, _) = compare_rgb(&gpu, &c);
    eprintln!("[{label}] motion blur: max err {:.2}/255 mean {:.4}/255, effect max {:.2}/255", mx * 255.0, mean * 255.0, change * 255.0);
    let mut over = 0;
    let mut worst = (0usize, 0usize, 0.0f32);
    for y in 0..gpu.h { for x in 0..gpu.w {
        let (p, q) = (gpu.px[y * gpu.w + x], reference.px[y * gpu.w + x]);
        let e = (0..3).map(|i| (p[i] - q[i]).abs() / q[i].abs().max(1.0)).fold(0.0f32, f32::max);
        if e > TOL { over += 1; }
        if e > worst.2 { worst = (x, y, e); }
    } }
    eprintln!("  pixels over 1/255: {over} of {}, worst at ({}, {}): gpu {:?} cpu {:?} src {:?}", gpu.w * gpu.h, worst.0, worst.1, gpu.px[worst.1 * gpu.w + worst.0], reference.px[worst.1 * gpu.w + worst.0], c.px[worst.1 * gpu.w + worst.0]);
    assert!(change > 0.05, "motion blur had no visible effect");
    assert!(mx <= 3.0 * TOL && (over as f32) < 0.001 * (gpu.w * gpu.h) as f32, "GPU vs CPU max {mx}, {over} pixels over 1/255");
}

#[test]
fn bloom_matches_cpu() {
    let Some(h) = Headless::new() else { return };
    let scene = synth::build(&synth::SceneOptions { width: 640, height: 360, ..Default::default() });
    let p = only(&scene.params, false, false, true);
    let r = run(&h, &scene, &p, false, true);
    assert!(r.result.bloom_written && !r.result.color_written);
    let (c, _) = quantized(&scene);
    let reference = cpu::bloom_chain(&c, &p, true);
    let gpu = r.bloom.unwrap();
    let (mx, mean) = compare_rgb(&gpu, &reference);
    let peak = gpu.px.iter().map(|p| p[0]).fold(0.0f32, f32::max);
    eprintln!("bloom: max err {:.2}/255 mean {:.5}/255 (peak value {peak:.3})", mx * 255.0, mean * 255.0);
    assert!(peak > 0.01, "no bloom produced");
    // values reach 19x (HDR sun) through six f16 render targets and bilinear taps: 2/255 relative
    assert!(mx <= 2.0 * TOL, "GPU vs CPU max {mx}");
}

#[test]
fn nothing_active_writes_nothing() {
    let Some(h) = Headless::new() else { return };
    let scene = synth::build(&synth::SceneOptions { width: 128, height: 72, ..Default::default() });
    let p = only(&scene.params, false, false, false);
    let r = run(&h, &scene, &p, false, true);
    assert_eq!(r.result, PostFxResult::default());
}
