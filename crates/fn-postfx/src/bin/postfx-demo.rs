//! `postfx-demo --screenshot [--out DIR] [--width 1280 --height 720] [--perf N]`
//!
//! Renders the synthetic scene (sky + HDR sun, checkered ground with a perspective depth ramp, eight colour-bar poles from 5 m to 1.2 km)
//! through every effect at the 1.8 Athena Day values and at exaggerated values (the Day DOF/motion blur are nearly invisible by design),
//! writes PNGs (linear HDR -> clamp -> sRGB) plus difference images, prints GPU-vs-CPU errors and per-pass GPU times.
use fn_postfx::cpu::{self, Image};
use fn_postfx::gpu::*;
use fn_postfx::harness::*;
use fn_postfx::params::*;
use fn_postfx::synth;
use std::path::PathBuf;

struct Args {
    out: PathBuf,
    width: usize,
    height: usize,
    perf_iters: u32,
    screenshot: bool,
}

fn parse() -> Args {
    let mut a = Args { out: PathBuf::from("reports/postfx-demo"), width: 1280, height: 720, perf_iters: 200, screenshot: false };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--screenshot" => a.screenshot = true,
            "--out" => a.out = it.next().expect("--out dir").into(),
            "--width" => a.width = it.next().expect("w").parse().expect("w"),
            "--height" => a.height = it.next().expect("h").parse().expect("h"),
            "--perf" => a.perf_iters = it.next().expect("n").parse().expect("n"),
            other => panic!("unknown argument {other} (usage: postfx-demo --screenshot [--out DIR] [--width W --height H] [--perf N])"),
        }
    }
    a
}

struct Gpu<'a> {
    h: &'a Headless,
    fx: PostFx,
    color: GpuTex,
    depth: GpuTex,
    vel: GpuTex,
    out: GpuTex,
    bloom: GpuTex,
}

impl<'a> Gpu<'a> {
    fn new(h: &'a Headless, s: &synth::Scene) -> Self {
        let (w, ht) = (s.color.w as u32, s.color.h as u32);
        let mut fx = PostFx::new(&h.device, PostFxFormats::default());
        fx.enable_timing(&h.queue);
        Gpu {
            h,
            fx,
            color: upload_rgba16f(h, &s.color, "color"),
            depth: upload_depth(h, &s.depth, "depth"),
            vel: upload_rgba16f(h, &s.velocity, "velocity"),
            out: render_target(h, w, ht, "out"),
            bloom: render_target(h, cpu::half(w as usize) as u32, cpu::half(ht as usize) as u32, "bloom"),
        }
    }
    fn run(&mut self, p: &PostFxParams, with_velocity: bool) -> (PostFxResult, Vec<PassTime>) {
        let mut enc = self.h.device.create_command_encoder(&Default::default());
        let r = self.fx.encode(
            &self.h.queue,
            &mut enc,
            &PostFxInputs { scene_color: &self.color.view, depth: &self.depth.view, velocity: with_velocity.then_some(&self.vel.view), params: p },
            &PostFxOutputs { color: &self.out.view, bloom: p.bloom.enabled.then_some(&self.bloom.view) },
        );
        self.h.queue.submit([enc.finish()]);
        let t = self.fx.read_timings();
        (r, t)
    }
}

fn only(p: &PostFxParams, dof: bool, mb: bool, bloom: bool) -> PostFxParams {
    let mut q = *p;
    q.dof.enabled = dof;
    q.motion_blur.enabled = mb;
    q.bloom.enabled = bloom;
    q
}

/// |a - b| * gain, as an image
fn diff(a: &Image, b: &Image, gain: f32) -> Image { Image::from_fn(a.w, a.h, |x, y| { let (p, q) = (a.px[y * a.w + x], b.px[y * a.w + x]); [(p[0] - q[0]).abs() * gain, (p[1] - q[1]).abs() * gain, (p[2] - q[2]).abs() * gain, 1.0] }) }

/// max / mean 8-bit difference of two HDR images after the demo tonemap + sRGB encode (what a viewer would see)
fn display_diff(a: &Image, b: &Image) -> (f32, f32) {
    let enc = |v: f32| { let c = (1.0 - (-v * 1.2).exp()).clamp(0.0, 1.0); let s = if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }; (s * 255.0 + 0.5).floor() };
    let (mut mx, mut sum) = (0.0f32, 0.0f64);
    for (p, q) in a.px.iter().zip(&b.px) { for c in 0..3 { let d = (enc(p[c]) - enc(q[c])).abs(); mx = mx.max(d); sum += d as f64; } }
    (mx, (sum / (a.px.len() * 3) as f64) as f32)
}

fn tonemap(img: &Image) -> Image { Image::from_fn(img.w, img.h, |x, y| { let p = img.px[y * img.w + x]; let f = |v: f32| 1.0 - (-v * 1.2).exp(); [f(p[0]), f(p[1]), f(p[2]), 1.0] }) }

fn main() {
    let a = parse();
    let Some(h) = Headless::new() else { eprintln!("no Vulkan/DX12 adapter"); std::process::exit(2) };
    println!("adapter: {}", h.describe());
    std::fs::create_dir_all(&a.out).ok();
    let scene = synth::build(&synth::SceneOptions { width: a.width, height: a.height, yaw_delta: 0.03, strafe: 0.1, object_velocity: [0.03, 0.0], exposure: 1.0 });
    let mut src = scene.color.clone();
    src.quantize_f16();
    let mut g = Gpu::new(&h, &scene);
    let save = |name: &str, img: &Image| { save_png(&tonemap(img), &a.out.join(name), 1.0); };
    let mut report = Vec::new();
    // ---- baseline
    save("00-scene.png", &src);
    // ---- DOF: Athena Day values, then a strong far blur to show the pass working
    let day = only(&scene.params, true, false, false);
    let (r, t) = g.run(&day, false);
    assert!(r.color_written);
    let dof_day = read_image(&h, &g.out.tex, g.out.w, g.out.h);
    save("01-dof-athena-day.png", &dof_day);
    save("02-dof-athena-day-diff-x32.png", &diff(&dof_day, &src, 32.0));
    let (mx, mean) = compare_rgb(&dof_day, &cpu::dof_far_full(&scene.depth, &src, &day, true));
    let (chg, chg_mean) = display_diff(&dof_day, &src);
    println!("DOF  Athena Day  : GPU vs CPU max {:.2}/255 mean {:.4}/255 | visible change vs untouched (tonemapped 8-bit): max {chg:.0} mean {chg_mean:.4}", mx * 255.0, mean * 255.0);
    report.push(("dof_day", mx, t));
    let mut strong = day;
    strong.dof.far_blur_size = 2.5;
    strong.dof.focal_distance = 2500.0;
    strong.dof.far_transition = 15000.0;
    let (_, t) = g.run(&strong, false);
    let dof_strong = read_image(&h, &g.out.tex, g.out.w, g.out.h);
    save("03-dof-strong.png", &dof_strong);
    let (mx, _) = compare_rgb(&dof_strong, &cpu::dof_far_full(&scene.depth, &src, &strong, true));
    println!("DOF  strong      : GPU vs CPU max {:.2}/255", mx * 255.0);
    report.push(("dof_strong", mx, t));
    // ---- motion blur
    let mbd = only(&scene.params, false, true, false);
    let (r, t) = g.run(&mbd, true);
    assert!(r.color_written);
    let mb_day = read_image(&h, &g.out.tex, g.out.w, g.out.h);
    save("04-motion-blur-athena-day.png", &mb_day);
    save("05-motion-blur-athena-day-diff-x8.png", &diff(&mb_day, &src, 8.0));
    let u = cpu::MbUniforms::new(&mbd);
    let (mx, mean) = compare_rgb(&mb_day, &cpu::motion_blur_full(&src, Some(&scene.velocity), &scene.depth, &u));
    let (chg, chg_mean) = display_diff(&mb_day, &src);
    println!("MB   Athena Day  : GPU vs CPU max {:.2}/255 mean {:.4}/255 | visible change vs untouched (tonemapped 8-bit): max {chg:.0} mean {chg_mean:.4}", mx * 255.0, mean * 255.0);
    report.push(("mb_day", mx, t));
    let mut mbs = mbd;
    mbs.motion_blur.amount = 1.0;
    mbs.motion_blur.max_percent = 8.0;
    let (_, t) = g.run(&mbs, true);
    let mb_strong = read_image(&h, &g.out.tex, g.out.w, g.out.h);
    save("06-motion-blur-strong.png", &mb_strong);
    let u = cpu::MbUniforms::new(&mbs);
    let (mx, _) = compare_rgb(&mb_strong, &cpu::motion_blur_full(&src, Some(&scene.velocity), &scene.depth, &u));
    println!("MB   strong      : GPU vs CPU max {:.2}/255", mx * 255.0);
    report.push(("mb_strong", mx, t));
    // ---- bloom
    let bl = only(&scene.params, false, false, true);
    let (r, t) = g.run(&bl, false);
    assert!(r.bloom_written);
    let bloom_img = read_image(&h, &g.bloom.tex, g.bloom.w, g.bloom.h);
    let (mx, _) = compare_rgb(&bloom_img, &cpu::bloom_chain(&src, &bl, true));
    // composite as the tonemapper does: scene + bloom * BloomIntensity (bilinear upsample)
    let comp = Image::from_fn(src.w, src.h, |x, y| {
        let b = bloom_img.sample((x as f32 + 0.5) / src.w as f32, (y as f32 + 0.5) / src.h as f32);
        let s = src.px[y * src.w + x];
        [s[0] + b[0] * bl.bloom.intensity, s[1] + b[1] * bl.bloom.intensity, s[2] + b[2] * bl.bloom.intensity, 1.0]
    });
    save("07-bloom-composite.png", &comp);
    save("08-bloom-only-x4.png", &Image::from_fn(bloom_img.w, bloom_img.h, |x, y| { let p = bloom_img.px[y * bloom_img.w + x]; [p[0] * 4.0, p[1] * 4.0, p[2] * 4.0, 1.0] }));
    println!("BLOOM Athena Day : GPU vs CPU max {:.2}/255 (relative above 1.0)", mx * 255.0);
    report.push(("bloom_day", mx, t));
    // ---- everything together (the real chain: DOF -> motion blur -> bloom)
    let all = scene.params;
    let (r, t) = g.run(&all, true);
    println!("ALL  Athena Day  : passes {} color_written {} bloom_written {}", r.passes, r.color_written, r.bloom_written);
    let all_img = read_image(&h, &g.out.tex, g.out.w, g.out.h);
    save("09-all-athena-day.png", &all_img);
    report.push(("all_day", 0.0, t));

    // ---- timings
    println!("\nGPU time per pass ({}x{}), mean of {} frames, {}:", a.width, a.height, a.perf_iters, h.describe());
    let configs: [(&str, PostFxParams, bool); 4] = [("DOF (Athena Day)", only(&scene.params, true, false, false), false), ("Motion blur (Athena Day)", only(&scene.params, false, true, false), true), ("Bloom (Athena Day, six levels minus Bloom6)", only(&scene.params, false, false, true), false), ("DOF + motion blur + bloom", scene.params, true)];
    let mut json = String::from("{\n  \"adapter\": ");
    json += &format!("{:?},\n  \"resolution\": [{}, {}],\n  \"frames\": {},\n  \"configs\": [\n", h.describe(), a.width, a.height, a.perf_iters);
    for (ci, (name, p, vel)) in configs.iter().enumerate() {
        let mut sums: Vec<(&'static str, f32)> = Vec::new();
        let mut total = 0.0f32;
        let warm = 10;
        for i in 0..(a.perf_iters + warm) {
            let (_, t) = g.run(p, *vel);
            if i < warm { continue; }
            if sums.is_empty() { sums = t.iter().map(|x| (x.name, 0.0)).collect(); }
            for (s, x) in sums.iter_mut().zip(&t) { s.1 += x.ms; }
            total += t.iter().map(|x| x.ms).sum::<f32>();
        }
        let n = a.perf_iters as f32;
        println!("  {name}: {:.3} ms total", total / n);
        for (pn, ms) in &sums { println!("      {pn:<18} {:.3} ms", ms / n); }
        json += &format!("    {{ \"name\": {:?}, \"total_ms\": {:.4}, \"passes\": [", name, total / n);
        json += &sums.iter().map(|(pn, ms)| format!("{{\"pass\": {:?}, \"ms\": {:.4}}}", pn, ms / n)).collect::<Vec<_>>().join(", ");
        json += if ci + 1 < configs.len() { "] },\n" } else { "] }\n" };
    }
    json += "  ]\n}\n";
    std::fs::write(a.out.join("perf.json"), json).ok();
    let _ = report;
    println!("\nwrote PNGs + perf.json to {}", a.out.display());
    if !a.screenshot { println!("(run with --screenshot for the PNG set; PNGs are always written)"); }
}
