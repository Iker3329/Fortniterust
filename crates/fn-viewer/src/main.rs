//! fn-viewer: winit viewer + headless screenshot tool for the fn-render wgpu renderer.
//!
//! ```text
//! fn-viewer                                     interactive window (V: camera mode, 1-5 toggles, F1 help)
//! fn-viewer --screenshot out.png --size 1280x720    render one frame offscreen (no window) and write a PNG
//! fn-viewer --screenshot out.png --size 1264x761 --compare ..\FortniteRuntime\verification\idle.bmp
//! ```
//! fn-viewer --screenshot out.png --size 1264x761 --demo-ui --compare ..\FortniteRuntime\verification\idle.bmp   (HUD over the 3D scene)
//! fn-viewer --stages --size 1264x761             stage-by-stage diff against the old runtime's post-smoke captures (exposure, no-film, plain materials, fog)
//! fn-viewer --verify-post --size 1280x720        GPU post chain vs the CPU reference (HDR read-back)
//! fn-viewer --gpu-timings --size 1920x1080 [--frames n] [--prop-count n] [--demo-ui]   per-pass GPU timing table (mean / p99 over n frames, default 300)
//! Textured static props: --props <dir> (exported real 1.8 trees / rocks), --prop-grid n (scatter n of them; synthetic foliage tree + rock without --props), --prop-seed n.
//! New options: --prop-count n (scatter n extra trees/rocks), --demo-ui, --no-props, --fxaa-level 0..3, --shadow-level 0..3, --texture-level 0..3, --view-distance 0..3, --brightness 0..1,
//! --shadow-size n, --aniso n, --skip-mips n, --far metres, --backdrop dim,blur, --hdr-dump, --old-dir <verification dir>.
//! Options: --assets <dir> (env FN_ASSETS, default ..\FortniteRuntime\assets), --camera athena|orbit, --yaw, --pitch, --eye x,y,z,
//! --look x,y,z, --pose rest|clip, --no-noise, --anim-time s, --sky-time s, --exposure x, --frames n (benchmark frames),
//! --no-shadows/--no-ao/--no-bloom/--no-fog/--no-vignette/--no-sky, --sky-layers mask, --debug n, --material-debug n,
//! --compare-out file.png, --no-vsync.
mod app;
mod assets;
mod athena_cam;
mod character;
mod compare;
mod prop_shot;
mod props;
mod scene;
mod static_props;
mod static_shot;
mod stages;
mod timings;
mod ui_overlay;

use fn_render::{CameraParams, FrameParams, Gpu, OffscreenTarget, RenderOptions};
use glam::Vec3;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Cli {
    pub assets: PathBuf,
    pub screenshot: Option<PathBuf>,
    pub size: (u32, u32),
    pub compare: Option<PathBuf>,
    pub compare_out: Option<PathBuf>,
    pub camera: CameraMode,
    pub yaw: f32,
    pub pitch: f32,
    pub eye: Option<Vec3>,
    pub look: Option<Vec3>,
    pub use_clip: bool,
    pub use_noise: bool,
    pub anim_time: f32,
    pub sky_time: f32,
    pub frames: u32,
    pub vsync: bool,
    pub options: RenderOptions,
    pub props: bool,
    pub prop_count: usize,
    /// `--props <dir>`: exported real props folder; `--prop-grid n`: scatter n textured static props (real ones with --props, synthetic otherwise).
    pub static_dir: Option<PathBuf>,
    pub prop_grid: usize,
    pub prop_seed: u32,
    /// `--prop-kind a,b`: only kinds whose id contains one of the substrings (case-insensitive).
    pub prop_kind: Vec<String>,
    pub gpu_timings: bool,
    pub demo_ui: bool,
    pub hdr_dump: bool,
    pub stages: bool,
    pub verify_post: bool,
    pub far: Option<f32>,
    pub old_dir: PathBuf,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode { Athena, Orbit, Fly }

fn parse_vec3(s: &str) -> Result<Vec3, String> {
    let v: Vec<f32> = s.split(',').map(|x| x.trim().parse::<f32>().map_err(|e| format!("bad number {x:?}: {e}"))).collect::<Result<_, _>>()?;
    if v.len() != 3 { return Err(format!("expected x,y,z but got {s:?}")); }
    Ok(Vec3::new(v[0], v[1], v[2]))
}
fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once(['x', 'X']).ok_or_else(|| format!("--size expects WxH, got {s:?}"))?;
    Ok((w.parse().map_err(|e| format!("bad width: {e}"))?, h.parse().map_err(|e| format!("bad height: {e}"))?))
}

pub fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut cli = Cli {
        assets: assets::default_assets_root(), screenshot: None, size: (1280, 720), compare: None, compare_out: None, camera: CameraMode::Athena,
        yaw: 0., pitch: -12., eye: None, look: None, use_clip: true, use_noise: true, anim_time: 0.17, sky_time: 5.0, frames: 3, vsync: true,
        options: RenderOptions::default(), props: true, prop_count: 0, static_dir: None, prop_grid: 0, prop_seed: 1, prop_kind: vec![], gpu_timings: false, demo_ui: false, hdr_dump: false, stages: false, verify_post: false, far: None,
        old_dir: PathBuf::from(r"..\FortniteRuntime\verification"),
    };
    let parse_f = |s: String, n: &str| s.parse::<f32>().map_err(|e| format!("{n}: {e}"));
    let parse_u = |s: String, n: &str| s.parse::<u32>().map_err(|e| format!("{n}: {e}"));
    let mut i = 0;
    let next = |i: &mut usize, name: &str| -> Result<String, String> { *i += 1; args.get(*i).cloned().ok_or_else(|| format!("{name} needs a value")) };
    while i < args.len() {
        let a = args[i].clone();
        match a.as_str() {
            "--assets" => cli.assets = PathBuf::from(next(&mut i, &a)?),
            "--screenshot" => cli.screenshot = Some(PathBuf::from(next(&mut i, &a)?)),
            "--size" => cli.size = parse_size(&next(&mut i, &a)?)?,
            "--compare" => cli.compare = Some(PathBuf::from(next(&mut i, &a)?)),
            "--compare-out" => cli.compare_out = Some(PathBuf::from(next(&mut i, &a)?)),
            "--camera" => cli.camera = match next(&mut i, &a)?.as_str() { "athena" => CameraMode::Athena, "orbit" => CameraMode::Orbit, "fly" => CameraMode::Fly, o => return Err(format!("unknown camera mode {o}")) },
            "--yaw" => cli.yaw = next(&mut i, &a)?.parse().map_err(|e| format!("--yaw: {e}"))?,
            "--pitch" => cli.pitch = next(&mut i, &a)?.parse().map_err(|e| format!("--pitch: {e}"))?,
            "--eye" => cli.eye = Some(parse_vec3(&next(&mut i, &a)?)?),
            "--look" => cli.look = Some(parse_vec3(&next(&mut i, &a)?)?),
            "--pose" => cli.use_clip = match next(&mut i, &a)?.as_str() { "clip" | "idle" => true, "rest" => false, o => return Err(format!("unknown pose {o}")) },
            "--no-noise" => cli.use_noise = false,
            "--anim-time" => cli.anim_time = next(&mut i, &a)?.parse().map_err(|e| format!("--anim-time: {e}"))?,
            "--sky-time" => cli.sky_time = next(&mut i, &a)?.parse().map_err(|e| format!("--sky-time: {e}"))?,
            "--exposure" => cli.options.fixed_exposure = Some(next(&mut i, &a)?.parse().map_err(|e| format!("--exposure: {e}"))?),
            "--frames" => cli.frames = next(&mut i, &a)?.parse().map_err(|e| format!("--frames: {e}"))?,
            "--no-shadows" => cli.options.shadows = false,
            "--no-ao" => cli.options.ao = false,
            "--no-bloom" => cli.options.bloom = false,
            "--no-fog" => cli.options.fog = false,
            "--no-vignette" => cli.options.vignette = false,
            "--no-sky" => cli.options.sky_layers = 0,
            "--sky-layers" => cli.options.sky_layers = next(&mut i, &a)?.parse().map_err(|e| format!("--sky-layers: {e}"))?,
            "--debug" => cli.options.debug_mode = next(&mut i, &a)?.parse().map_err(|e| format!("--debug: {e}"))?,
            "--material-debug" => cli.options.material_debug = next(&mut i, &a)?.parse().map_err(|e| format!("--material-debug: {e}"))?,
            "--no-vsync" => cli.vsync = false,
            "--no-props" => cli.props = false,
            "--prop-count" => cli.prop_count = next(&mut i, &a)?.parse().map_err(|e| format!("--prop-count: {e}"))?,
            "--props" => cli.static_dir = Some(PathBuf::from(next(&mut i, &a)?)),
            "--prop-grid" => cli.prop_grid = next(&mut i, &a)?.parse().map_err(|e| format!("--prop-grid: {e}"))?,
            "--prop-kind" => cli.prop_kind = next(&mut i, &a)?.split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect(),
            "--prop-seed" => cli.prop_seed = next(&mut i, &a)?.parse().map_err(|e| format!("--prop-seed: {e}"))?,
            "--gpu-timings" => cli.gpu_timings = true,
            "--demo-ui" => cli.demo_ui = true,
            "--hdr-dump" => cli.hdr_dump = true,
            "--stages" => cli.stages = true,
            "--verify-post" => cli.verify_post = true,
            "--old-dir" => cli.old_dir = PathBuf::from(next(&mut i, &a)?),
            "--far" => cli.far = Some(parse_f(next(&mut i, &a)?, "--far")?),
            "--fxaa-level" => cli.options.fxaa = fn_render::quality::aa_params(parse_u(next(&mut i, &a)?, "--fxaa-level")?),
            "--shadow-level" => match fn_render::quality::shadow_map_size(parse_u(next(&mut i, &a)?, "--shadow-level")?) { Some(s) => cli.options.shadow_map_size = s, None => cli.options.shadows = false },
            "--shadow-size" => cli.options.shadow_map_size = parse_u(next(&mut i, &a)?, "--shadow-size")?,
            "--texture-level" => { let (skip, aniso) = fn_render::quality::texture_params(parse_u(next(&mut i, &a)?, "--texture-level")?); cli.options.skip_mips = skip; cli.options.anisotropy = aniso; }
            "--aniso" => cli.options.anisotropy = parse_u(next(&mut i, &a)?, "--aniso")? as u16,
            "--skip-mips" => cli.options.skip_mips = parse_u(next(&mut i, &a)?, "--skip-mips")?,
            "--view-distance" => cli.far = Some(fn_render::quality::far_plane_m(parse_u(next(&mut i, &a)?, "--view-distance")?)),
            "--brightness" => cli.options.display_exponent = fn_render::quality::display_exponent_for_brightness(parse_f(next(&mut i, &a)?, "--brightness")?),
            "--backdrop" => { let s = next(&mut i, &a)?; let (d, b) = s.split_once(',').ok_or("--backdrop expects dim,blur")?; cli.options.backdrop = fn_render::Backdrop { dim: parse_f(d.into(), "--backdrop")?, blur: parse_f(b.into(), "--backdrop")? }; }
            "--help" | "-h" => return Err("help".into()),
            o => return Err(format!("unknown option {o}")),
        }
        i += 1;
    }
    Ok(cli)
}

/// Camera for a screenshot / initial window view.
pub fn initial_camera(cli: &Cli, scene: &scene::Scene, w: u32, h: u32) -> CameraParams {
    let mut cam = match (cli.eye, cli.camera) {
        (Some(eye), _) => {
            let target = cli.look.unwrap_or_else(|| scene.focus());
            let f = (target - eye).normalize_or_zero();
            let right = f.cross(Vec3::Y).normalize_or_zero();
            let mut c = CameraParams::from_yaw_pitch(eye, 0., 0., w, h);
            c.forward = f;
            c.up = right.cross(f);
            c
        }
        (None, _) => {
            let eye = athena_cam::settled_eye(scene.feet, false, cli.yaw, cli.pitch);
            CameraParams::from_yaw_pitch(eye, cli.yaw, cli.pitch, w, h)
        }
    };
    if let Some(far) = cli.far { cam.far = far; }
    cam
}

/// Offscreen session shared by the screenshot, stage-report and post-verification modes.
pub struct Session {
    pub gpu: Gpu,
    pub target: OffscreenTarget,
    pub scene: scene::Scene,
    pub hud: Option<ui_overlay::HudOverlay>,
    pub cam: CameraParams,
    pub size: (u32, u32),
}
pub struct Rendered { pub rgba: Vec<u8>, pub gpu_ms: Vec<f32>, pub cpu_ms: Vec<f32> }

impl Session {
    pub fn open(cli: &Cli) -> Result<Session, String> {
        let index = assets::AssetIndex::scan(&cli.assets)?;
        let gpu = Gpu::headless()?;
        eprintln!("[gpu] {}", gpu.describe());
        let (w, h) = cli.size;
        let target = OffscreenTarget::new(&gpu.device, w, h, fn_render::wgpu::TextureFormat::Rgba8Unorm);
        let mut scene = scene::Scene::load(&gpu, &index, target.format, &scene::SceneConfig { use_clip: cli.use_clip, use_noise: cli.use_noise, props: cli.props, extra_props: cli.prop_count, static_dir: cli.static_dir.clone(), prop_grid: cli.prop_grid, prop_seed: cli.prop_seed, prop_kind: cli.prop_kind.clone() })?;
        let cam = initial_camera(cli, &scene, w, h);
        scene.animate(cli.anim_time);
        let hud = if cli.demo_ui { Some(ui_overlay::HudOverlay::new(&gpu.device, &gpu.queue, target.format)?) } else { None };
        Ok(Session { gpu, target, scene, hud, cam, size: (w, h) })
    }
    /// Renders `frames` timed frames after 3 warm-up frames (the first frame snaps the exposure) and reads the last one back.
    pub fn render(&mut self, cli: &Cli, options: &RenderOptions, frames: u32) -> Rendered {
        let ids = self.scene.model_ids();
        let (mut gpu_ms, mut cpu_ms) = (vec![], vec![]);
        for k in 0..(frames.max(1) + 3) {
            let t = std::time::Instant::now();
            let fp = FrameParams { camera: &self.cam, time: cli.sky_time, dt: 1. / 60., models: &ids, options };
            let stats = match self.hud.as_mut() {
                Some(h) => { h.update(self.size.0, self.size.1, cli.yaw, 1. / 60.); self.scene.renderer.render_with_overlay(&fp, &self.target.view, Some(&mut h.pass())) }
                None => self.scene.renderer.render(&fp, &self.target.view),
            };
            self.scene.renderer.wait_idle();
            if k >= 3 { gpu_ms.push(t.elapsed().as_secs_f32() * 1000.); cpu_ms.push(stats.cpu_ms); }
        }
        Rendered { rgba: self.target.read_rgba8(&self.gpu), gpu_ms, cpu_ms }
    }
}
pub fn rgb_image(rgba: &[u8], w: u32, h: u32) -> image::RgbImage {
    image::RgbImage::from_fn(w, h, |x, y| { let p = &rgba[((y * w + x) * 4) as usize..]; image::Rgb([p[0], p[1], p[2]]) })
}

fn run_screenshot(cli: &Cli, out: &PathBuf) -> Result<(), String> {
    let mut s = Session::open(cli)?;
    let (w, h) = cli.size;
    let r = s.render(cli, &cli.options, cli.frames);
    let exposure = s.scene.renderer.read_exposure();
    image::save_buffer(out, &r.rgba, w, h, image::ColorType::Rgba8).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut times = r.gpu_ms.clone();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = times.iter().sum::<f32>() / times.len() as f32;
    let enc_mean = r.cpu_ms.iter().sum::<f32>() / r.cpu_ms.len() as f32;
    println!("wrote {} ({w}x{h}) on {}", out.display(), s.gpu.describe());
    println!("frame time (submit + GPU complete) over {} frames: mean {:.2} ms, median {:.2} ms, min {:.2} ms, max {:.2} ms; CPU encode+submit mean {:.2} ms", times.len(), mean, times[times.len() / 2], times[0], times[times.len() - 1], enc_mean);
    println!("exposure {:.4} (avg luminance {:.4})", exposure.exposure, exposure.avg_luminance);
    if cli.hdr_dump {
        if let Some(hdr) = s.scene.renderer.read_hdr() {
            for (name, f) in compare::REGIONS {
                let (x0, y0, x1, y1) = (f.0, f.1, f.2, f.3);
                let m = hdr.region_mean((x0 * w as f32) as u32, (y0 * h as f32) as u32, (x1 * w as f32) as u32, (y1 * h as f32) as u32);
                println!("  hdr [{name}] mean linear ({:.4},{:.4},{:.4}) x exposure = ({:.4},{:.4},{:.4})", m[0], m[1], m[2], m[0] * exposure.exposure as f64, m[1] * exposure.exposure as f64, m[2] * exposure.exposure as f64);
            }
        }
    }
    if let Some(old) = &cli.compare {
        let old_img = image::open(old).map_err(|e| format!("{}: {e}", old.display()))?.to_rgb8();
        let new_img = rgb_image(&r.rgba, w, h);
        if old_img.dimensions() != new_img.dimensions() {
            return Err(format!("--compare: capture is {}x{} but --size is {w}x{h}; use --size {}x{}", old_img.width(), old_img.height(), old_img.width(), old_img.height()));
        }
        let (text, sbs) = compare::report(&new_img, &old_img, &out.display().to_string(), &old.display().to_string());
        print!("{text}");
        let sbs_path = cli.compare_out.clone().unwrap_or_else(|| out.with_file_name(format!("{}-compare.png", out.file_stem().unwrap().to_string_lossy())));
        sbs.save(&sbs_path).map_err(|e| format!("{}: {e}", sbs_path.display()))?;
        println!("side-by-side (new | old | |diff| x4): {}", sbs_path.display());
    }
    Ok(())
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse_args(&args) {
        Ok(c) => c,
        Err(e) => {
            if e != "help" { eprintln!("error: {e}\n"); }
            eprintln!("{}", include_str!("main.rs").lines().skip(1).take_while(|l| l.starts_with("//!")).map(|l| l.trim_start_matches("//!").trim_start_matches(' ')).collect::<Vec<_>>().join("\n"));
            std::process::exit(if e == "help" { 0 } else { 2 });
        }
    };
    let result = if cli.gpu_timings { timings::run(&cli) } else if cli.stages { stages::run_stages(&cli) } else if cli.verify_post { stages::verify_post(&cli) } else { match &cli.screenshot {
        Some(out) => run_screenshot(&cli, out),
        None => app::run(cli.clone()),
    } };
    if let Err(e) = result { eprintln!("error: {e}"); std::process::exit(1); }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(s: &str) -> Vec<String> { s.split_whitespace().map(String::from).collect() }
    #[test]
    fn parses_screenshot_size_and_flags() {
        let c = parse_args(&args("--screenshot a.png --size 1920x1080 --no-ao --exposure 1.2 --yaw 30 --eye 1,2,3")).unwrap();
        assert_eq!(c.size, (1920, 1080));
        assert!(!c.options.ao && c.options.shadows);
        assert_eq!(c.options.fixed_exposure, Some(1.2));
        assert_eq!(c.yaw, 30.);
        assert_eq!(c.eye, Some(Vec3::new(1., 2., 3.)));
    }
    #[test]
    fn rejects_bad_input() {
        assert!(parse_args(&args("--size 12")).is_err());
        assert!(parse_args(&args("--bogus")).is_err());
        assert!(parse_args(&args("--yaw")).is_err());
        assert!(parse_args(&args("--eye 1,2")).is_err());
    }
}
