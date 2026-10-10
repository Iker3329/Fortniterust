//! SKY REFLECTION HOOK (runtime half): reads the Athena sky light's specified cubemap `T_hdrsky_test03_low` (TODM_A `SkyLightComponent.Cubemap`, evidence in
//! fn-render `sky_light.rs`) from the 1.8 paks, filters it like UE4.16 `FilterReflectionEnvironment` (fn-render `sky_reflection.rs`, on a worker thread) and hands
//! the processed cube to the renderer, whose lit paths then add the sky light's specular (`lit_common.wgsl` `sky_specular`). Until it arrives (or without paks)
//! the renderer keeps `SkyLightParameters.y = 0`: no sky specular, as UE with no processed sky texture.
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;

enum St { Idle, Running(Receiver<Result<Vec<fn_render::sky_light::Cube>, String>>), Done }
static STATE: Mutex<St> = Mutex::new(St::Idle);

fn load(dir: &std::path::Path) -> Result<Vec<fn_render::sky_light::Cube>, String> {
    let set = fn_pak::PakSet::open_install(dir).map_err(|e| e.to_string())?;
    let pkg = fn_render::sky_light::CUBEMAP_PACKAGE;
    let rd = |ext: &str| set.read_game(pkg, ext).map(|c| c.into_owned());
    let a = rd("uasset").map_err(|e| format!("{pkg}: {e}"))?;
    let tex = fn_texture::Texture2D::from_files(&a, &rd("uexp").unwrap_or_default(), &rd("ubulk").unwrap_or_default()).map_err(|e| format!("{pkg}: {e}"))?;
    if !tex.is_cube() { return Err(format!("{pkg}: not a cube texture")); }
    // the top mip the cook kept (`CopyCubemapToCubeFaceColorPS` samples level 0 of the source; the same level sky_light.rs projects the SH from)
    let mip = tex.best_mip().ok_or_else(|| format!("{pkg}: no mip with data"))?;
    let size = tex.mips[mip].width;
    let mut faces: [Vec<[f32; 3]>; 6] = Default::default();
    for (f, face) in faces.iter_mut().enumerate() {
        let img = tex.decode_slice(mip, f, Default::default()).map_err(|e| format!("{pkg} face {f}: {e}"))?;
        let fn_texture::Pixels::RgbaF16(px) = img.pixels else { return Err(format!("{pkg}: face {f} is not RGBA16F")) };
        *face = px.chunks(4).map(|p| [0, 1, 2].map(|c| { let v = fn_ueio::f16_to_f32(p[c]); if v.is_finite() { v } else { 0. } })).collect();
    }
    Ok(fn_render::sky_reflection::filter_reflection_environment(&fn_render::sky_light::Cube { size, faces }))
}

/// `false` once the cube was installed or given up on.
pub fn pending() -> bool { !matches!(&*STATE.lock().unwrap_or_else(|p| p.into_inner()), St::Done) }

/// Once per frame while a scene exists: starts the load on the first call, installs the processed cube when it is ready.
pub fn frame(renderer: &mut fn_render::Renderer, install: Option<PathBuf>) {
    let mut st = STATE.lock().unwrap_or_else(|p| p.into_inner());
    match &*st {
        St::Done => {}
        St::Idle => {
            let Some(dir) = install else { eprintln!("[sky-reflection] no install / paks: the sky light's specular stays off"); *st = St::Done; return };
            let (tx, rx) = channel();
            std::thread::spawn(move || { let t0 = std::time::Instant::now(); let r = load(&dir); if r.is_ok() { eprintln!("[sky-reflection] {} filtered in {:.0} ms", fn_render::sky_light::CUBEMAP_PACKAGE, t0.elapsed().as_secs_f32() * 1000.); } let _ = tx.send(r); });
            *st = St::Running(rx);
        }
        St::Running(rx) => match rx.try_recv() {
            Ok(Ok(mips)) => { renderer.set_sky_reflection(&mips); eprintln!("[sky-reflection] sky light specular ON: processed cube {}^2, {} mips (UE4.16 FilterReflectionEnvironment)", mips[0].size, mips.len()); *st = St::Done; }
            Ok(Err(e)) => { eprintln!("[sky-reflection] the sky light's specular stays off: {e}"); *st = St::Done; }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => { eprintln!("[sky-reflection] loader thread died: the sky light's specular stays off"); *st = St::Done; }
        },
    }
}
