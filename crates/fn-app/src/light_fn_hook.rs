//! LIGHT FUNCTION HOOK (2026-10-07, runtime half): loads the two textures of the sun's High-quality light function (`T_spiralLF`, `T_FORT_LF`, see fn-render
//! `light_function.rs`) from the 1.8 paks once a scene exists, and hands the renderer the renderer -> absolute UE world translation every frame (the layout mapping
//! of the props / backdrop / storm, `storm_hook::offset_cm`). Without paks the renderer keeps its 1x1 placeholder (the program then yields ~1: no cloud pattern).
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static DONE: AtomicBool = AtomicBool::new(false);

fn load_grey(set: &fn_pak::PakSet, pkg: &str) -> Result<fn_render::light_function::GreyImage, String> {
    let rd = |ext: &str| set.read_game(pkg, ext).map(|c| c.into_owned());
    let a = rd("uasset").map_err(|e| format!("{pkg}: {e}"))?;
    let tex = fn_texture::Texture2D::from_files(&a, &rd("uexp").unwrap_or_default(), &rd("ubulk").unwrap_or_default()).map_err(|e| format!("{pkg}: {e}"))?;
    // the top mip the cook kept (LODBias 3 / 4 of the 2048^2 imports drop the top mips: the client never streams them in)
    let mip = tex.best_mip().ok_or_else(|| format!("{pkg}: no mip with data"))?;
    let img = tex.decode(mip, Default::default()).map_err(|e| format!("{pkg} mip {mip}: {e}"))?;
    if fn_live::var_os("FN_LF_DUMP").is_some() { eprintln!("[light-function] {pkg}: format {:?} srgb {} mip {mip}", tex.format, tex.srgb); }
    let rgba = img.to_rgba8();
    Ok(fn_render::light_function::GreyImage { width: img.width as u32, height: img.height as u32, texels: rgba.chunks_exact(4).map(|p| p[0]).collect(), srgb: tex.srgb })
}

/// true until the textures were loaded (or given up on).
pub fn pending() -> bool { !DONE.load(Ordering::Relaxed) }
/// Once per frame while a scene exists (`install` is only read while [`pending`]).
pub fn frame(renderer: &mut fn_render::Renderer, install: Option<PathBuf>) {
    fn_render::light_function::set_ue_offset_cm(crate::storm_hook::offset_cm());
    if DONE.swap(true, Ordering::Relaxed) { return; }
    let Some(dir) = install else { eprintln!("[light-function] no install / paks: the sun's light function stays untextured"); return };
    let r = fn_pak::PakSet::open_install(&dir).map_err(|e| e.to_string()).and_then(|set| {
        Ok((load_grey(&set, fn_render::light_function::SPIRAL_PACKAGE)?, load_grey(&set, fn_render::light_function::FORT_PACKAGE)?))
    });
    match r {
        Ok((spiral, fort)) => {
            eprintln!("[light-function] High program textures: T_spiralLF {}x{}, T_FORT_LF {}x{}", spiral.width, spiral.height, fort.width, fort.height);
            if let Some(d) = fn_live::var_os("FN_LF_DUMP") { for (n, g) in [("spiral", &spiral), ("fort", &fort)] { let p = std::path::Path::new(&d).join(format!("lf_{n}.png")); let _ = image::GrayImage::from_raw(g.width, g.height, g.texels.clone()).map(|i| i.save(&p)); let m = g.texels.iter().map(|&v| v as f32).sum::<f32>() / g.texels.len() as f32 / 255.; eprintln!("[light-function] {n} mean {m:.3} -> {}", p.display()); } }
            renderer.set_light_function_textures(spiral, fort);
        }
        Err(e) => eprintln!("[light-function] textures unavailable: {e}"),
    }
}
