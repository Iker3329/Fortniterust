//! Verification modes: `--stages` (per-stage diff against the old runtime's post-smoke captures) and `--verify-post` (GPU post chain vs the CPU
//! reference of the old maths, fed with the HDR target read back from the GPU).
use crate::{compare, rgb_image, Cli, Session};
use fn_render::tonemap;

/// (capture, debug mode, material debug bits, exposure the old runtime had when it took the capture: reports verification\post-smoke.txt)
const STAGES: [(&str, i32, u32, f32); 10] = [
    ("post-final", 0, 0, 1.2063),
    ("post-ao-only", 2, 0, 1.2055),
    ("post-bloom-only", 3, 0, 1.2048),
    ("post-no-film-curve", 1, 0, 1.2042),
    ("post-exposure-linear", 4, 0, 1.2037),
    ("mat-ground-plain", 0, 1, 1.2032),
    ("mat-character-plain", 0, 2, 1.2028),
    ("fog-off", 0, 4, 1.2023),
    ("fog-amount", 0, 8, 1.2129),
    ("post-vignette-only", 5, 0, 1.2639),
];

pub fn run_stages(cli: &Cli) -> Result<(), String> {
    let mut cli = cli.clone();
    cli.size = (1264, 761);
    let mut s = Session::open(&cli)?;
    let out_dir = cli.compare_out.clone().map(|p| p.parent().map(|d| d.to_path_buf()).unwrap_or_default()).unwrap_or_default();
    println!("stage-by-stage diff vs {} at 1264x761 (new vs old; the old frame also contains the HUD, which is drawn here only with --demo-ui)", cli.old_dir.display());
    for (name, dbg, mat, exposure) in STAGES {
        let mut o = cli.options.clone();
        o.debug_mode = dbg;
        o.material_debug = mat;
        o.fixed_exposure = Some(exposure);
        let r = s.render(&cli, &o, 1);
        let old_path = cli.old_dir.join(format!("{name}.bmp"));
        let old = image::open(&old_path).map_err(|e| format!("{}: {e}", old_path.display()))?.to_rgb8();
        let new = rgb_image(&r.rgba, 1264, 761);
        let (text, sbs) = compare::report(&new, &old, "new", name);
        println!("== {name} (debug {dbg}, material bits {mat}, exposure {exposure})");
        for l in text.lines().filter(|l| l.contains("full frame mean RGB") || l.trim_start().starts_with('[')) { println!("{}", l.replace("  px", " px")); }
        let _ = sbs.save(out_dir.join(format!("stage-{name}.png")));
    }
    Ok(())
}

/// GPU output vs `tonemap::display_transform` applied to the read-back HDR values (bloom + AO off, vignette on).
pub fn verify_post(cli: &Cli) -> Result<(), String> {
    let mut s = Session::open(cli)?;
    let (w, h) = cli.size;
    let mut o = cli.options.clone();
    o.ao = false;
    o.bloom = false;
    o.fixed_exposure = Some(1.2);
    let r = s.render(cli, &o, 1);
    let hdr = s.scene.renderer.read_hdr().ok_or("no HDR target")?;
    let a = h as f32 / w as f32;
    let mut diffs = vec![];
    let (mut sum, mut worst) = (0f64, (0f32, 0u32, 0u32));
    for y in (0..h).step_by(7) {
        for x in (0..w).step_by(7) {
            let p = hdr.get(x, y);
            let (u, v) = ((x as f32 + 0.5) / w as f32, 1. - (y as f32 + 0.5) / h as f32);
            let m = tonemap::vignette_mask((u, v), a, tonemap::VIGNETTE_INTENSITY) * 1.2;
            let cpu = tonemap::display_transform([p[0] * m, p[1] * m, p[2] * m], o.display_exponent);
            let g = &r.rgba[((y * w + x) * 4) as usize..];
            for c in 0..3 {
                let d = ((cpu[c] * 255.).round() - g[c] as f32).abs();
                diffs.push(d);
                sum += d as f64;
                if d > worst.0 { worst = (d, x, y); }
            }
        }
    }
    diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = diffs.len();
    println!("verify-post {w}x{h}: {n} channel samples, mean |GPU - CPU reference| {:.3} levels, p99 {:.1}, max {:.1} at ({}, {})", sum / n as f64, diffs[n * 99 / 100], worst.0, worst.1, worst.2);
    let _ = &r;
    if worst.0 > 3.0 { return Err(format!("post chain deviates from the CPU reference by {} levels", worst.0)); }
    Ok(())
}
