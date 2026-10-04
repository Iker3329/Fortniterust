//! fn-ui-demo: shows each fn-ui screen with test state.
//!
//!   fn-ui-demo --list
//!   fn-ui-demo --screenshot <screen> <WxH> <out.png> [--cpu | --gpu]
//!   fn-ui-demo --diff <screen|all> <WxH[,WxH...]> [--out <dir>] [--cpu | --gpu]
//!   fn-ui-demo [--screen <name>] [--size WxH]          (winit window; Left/Right or PageUp/PageDown cycle screens, Esc quits)
//!   fn-ui-demo --live [--size WxH] [--ini <file> | --no-save] [--frames N]   (the REAL menu + Settings + HUD driven by winit input, see live.rs)
//!
//! `--assets dir|pak|auto|layered` (all modes; `--install <FortniteGame dir>`, `--folder <exported assets dir>`): where UI textures and fonts come from.
//! `dir` (default of the screenshot / diff modes) = the old exported folder, `pak` = straight from the Fortnite install, `layered` = install first and the
//! folder as the fallback, `auto` (default of `--live`) = layered when the install exists, else the folder.
//!
//! `--cpu` uses the software reference renderer (`fn_ui::raster`), `--gpu` the wgpu backend (`fn-ui-wgpu`); the default is gpu when built with
//! the `gpu` feature, else cpu.
mod diff;
mod gpu;
mod live;
mod window;
mod winput;

use fn_ui::assets::UiAssets;
use fn_ui::draw::RenderList;
use fn_ui::scene::{all_scenes, find_scene, SceneDef, UiContext};
use std::path::PathBuf;

pub enum Backend { Cpu(fn_ui::raster::Rasterizer), Gpu(gpu::GpuBackend) }
impl Backend {
    pub fn name(&self) -> &'static str { match self { Backend::Cpu(_) => "cpu (fn_ui::raster)", Backend::Gpu(_) => "gpu (fn-ui-wgpu)" } }
    /// Premultiplied RGBA8, row 0 top.
    pub fn render(&mut self, list: &RenderList, assets: &mut UiAssets, w: u32, h: u32) -> Result<Vec<u8>, String> {
        match self { Backend::Cpu(r) => Ok(r.render(list, assets, w, h, None)), Backend::Gpu(g) => g.render(list, assets, w, h) }
    }
}

/// UI assets context chosen by --assets (default dir).
pub fn make_context(args: &[String]) -> Result<UiContext, String> { make_context_default(args, "dir") }
pub fn make_context_default(args: &[String], default: &str) -> Result<UiContext, String> {
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let mode = opt("--assets").unwrap_or_else(|| default.to_owned());
    let install = opt("--install").map(PathBuf::from).unwrap_or_else(fn_ui::pak_assets::default_install);
    let folder = opt("--folder").map(PathBuf::from).unwrap_or_else(fn_ui::assets::DirAssets::default_root);
    match mode.as_str() {
        "dir" => UiContext::new(Box::new(fn_ui::assets::DirAssets::new(folder))),
        "pak" => UiContext::from_install(&install, None),
        "layered" => UiContext::from_install(&install, Some(&folder)),
        "auto" => if install.exists() { UiContext::from_install(&install, folder.is_dir().then_some(folder.as_path())).or_else(|_| UiContext::new(Box::new(fn_ui::assets::DirAssets::new(folder)))) } else { UiContext::new(Box::new(fn_ui::assets::DirAssets::new(folder))) },
        other => Err(format!("unknown --assets '{other}' (dir|pak|auto|layered)")),
    }
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once(['x', 'X']).ok_or_else(|| format!("bad size '{s}', expected WxH"))?;
    Ok((w.parse().map_err(|_| format!("bad width in '{s}'"))?, h.parse().map_err(|_| format!("bad height in '{s}'"))?))
}

fn make_backend(args: &[String]) -> Result<Backend, String> {
    let want_cpu = args.iter().any(|a| a == "--cpu");
    let want_gpu = args.iter().any(|a| a == "--gpu");
    if want_cpu { return Ok(Backend::Cpu(fn_ui::raster::Rasterizer::new())); }
    match gpu::GpuBackend::new() {
        Ok(g) => Ok(Backend::Gpu(g)),
        Err(e) if want_gpu => Err(format!("gpu backend unavailable: {e}")),
        Err(e) => { eprintln!("note: gpu backend unavailable ({e}); using the cpu reference renderer"); Ok(Backend::Cpu(fn_ui::raster::Rasterizer::new())) }
    }
}

fn build(scene: &SceneDef, ctx: &mut UiContext, w: u32, h: u32) -> RenderList { (scene.build)(ctx, w as f32, h as f32) }

fn screenshot(args: &[String]) -> Result<(), String> {
    let i = args.iter().position(|a| a == "--screenshot").unwrap();
    let (name, size, out) = (args.get(i + 1), args.get(i + 2), args.get(i + 3));
    let (Some(name), Some(size), Some(out)) = (name, size, out) else { return Err("usage: --screenshot <screen> <WxH> <out.png>".into()) };
    let (w, h) = parse_size(size)?;
    let scene = find_scene(name).ok_or_else(|| format!("unknown screen '{name}' (try --list)"))?;
    let mut ctx = make_context(args)?;
    let mut be = make_backend(args)?;
    let list = build(&scene, &mut ctx, w, h);
    let px = be.render(&list, &mut ctx.assets, w, h)?;
    // Written over black like the old captures (premultiplied colour == colour over black).
    diff::save_png_rgb(&PathBuf::from(out), w, h, &diff::over_black(&px))?;
    println!("{name} {w}x{h} -> {out} ({} cmds, {} quads, backend {})", list.cmds.len(), list.quad_count(), be.name());
    if !ctx.assets.missing.is_empty() { println!("missing assets: {}", ctx.assets.missing.join("; ")); }
    Ok(())
}

fn run_diff(args: &[String]) -> Result<(), String> {
    let i = args.iter().position(|a| a == "--diff").unwrap();
    let (Some(which), Some(sizes)) = (args.get(i + 1), args.get(i + 2)) else { return Err("usage: --diff <screen|all> <WxH[,WxH]> [--out dir]".into()) };
    let out_dir = args.iter().position(|a| a == "--out").and_then(|j| args.get(j + 1)).map(PathBuf::from);
    if let Some(d) = &out_dir { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
    let sizes: Vec<(u32, u32)> = sizes.split(',').map(parse_size).collect::<Result<_, _>>()?;
    let scenes: Vec<SceneDef> = if which == "all" { all_scenes() } else { vec![find_scene(which).ok_or_else(|| format!("unknown screen '{which}'"))?] };
    let mut ctx = make_context(args)?;
    let mut be = make_backend(args)?;
    println!("backend: {}", be.name());
    println!("{:<34} {:>9} {:>8} {:>8} {:>7} {:>5} {:>8}  old capture", "screen", "size", "full", "masked", "bad>8", "max", "covered");
    for scene in &scenes {
        for &(w, h) in &sizes {
            let list = build(scene, &mut ctx, w, h);
            let px = be.render(&list, &mut ctx.assets, w, h)?;
            let old = scene.old_capture.and_then(|t| diff::find_old(t, w, h));
            let Some(oldp) = old else {
                println!("{:<34} {:>9} {:>8} {:>8} {:>7} {:>5} {:>8}  (no old capture at this size)", scene.name, format!("{w}x{h}"), "-", "-", "-", "-", "-");
                if let Some(d) = &out_dir { diff::save_png_rgb(&d.join(format!("{}-{w}x{h}-ours.png", scene.name)), w, h, &diff::over_black(&px))?; }
                continue;
            };
            let oldimg = diff::load_rgb(&oldp)?;
            if (oldimg.w, oldimg.h) != (w, h) { println!("{:<34} {:>9}  old capture has size {}x{}, skipped", scene.name, format!("{w}x{h}"), oldimg.w, oldimg.h); continue; }
            let (m, d) = diff::compare(&px, &oldimg);
            let f = |v: f64| if v.is_nan() { "-".to_owned() } else { format!("{v:.2}") };
            println!("{:<34} {:>9} {:>8.2} {:>8} {:>7} {:>5} {:>8}  {}", scene.name, format!("{w}x{h}"), m.full, f(m.masked), if m.bad8.is_nan() { "-".to_owned() } else { format!("{:.1}%", m.bad8 * 100.) }, m.max, m.covered, oldp.file_name().unwrap().to_string_lossy());
            if let Some(dir) = &out_dir {
                let ours = diff::over_black(&px);
                let (sw, sh, sheet) = diff::sheet(w, h, &ours, &oldimg.px, &d);
                diff::save_png_rgb(&dir.join(format!("{}-{w}x{h}-diff.png", scene.name)), sw, sh, &sheet)?;
                diff::save_png_rgb(&dir.join(format!("{}-{w}x{h}-ours.png", scene.name)), w, h, &ours)?;
            }
        }
    }
    if !ctx.assets.missing.is_empty() { println!("missing assets: {}", ctx.assets.missing.join("; ")); }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = if args.iter().any(|a| a == "--list") {
        for s in all_scenes() { println!("{:<34} old capture: {}", s.name, s.old_capture.unwrap_or("-")); }
        Ok(())
    } else if args.iter().any(|a| a == "--live") { live::run(&args) }
    else if args.iter().any(|a| a == "--screenshot") { screenshot(&args) }
    else if args.iter().any(|a| a == "--diff") { run_diff(&args) }
    else { window::run(&args) };
    if let Err(e) = r { eprintln!("error: {e}"); std::process::exit(1); }
}

