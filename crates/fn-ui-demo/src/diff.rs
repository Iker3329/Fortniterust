//! Pixel-diff helper against the OLD runtime's captures (BMP, 24/32-bit, taken from the GL framebuffer).
//!
//! Our render is produced over transparent black, so every pixel is (premultiplied colour, coverage). Metrics:
//!  * `full`   mean absolute RGB difference (0..255) over ALL pixels, our colour composited over black (valid for opaque full-screen UI);
//!  * `masked` the same but only over pixels our UI covers with alpha >= 0.99 (valid for HUD captures that have a 3D world behind the UI);
//!  * `bad8`   fraction of compared pixels whose max channel difference exceeds 8/255;  `max` largest channel difference.
use std::path::{Path, PathBuf};

pub struct Rgb { pub w: u32, pub h: u32, pub px: Vec<u8> } // RGB8, row 0 top

pub fn old_roots() -> Vec<PathBuf> {
    let base = fn_ui::assets::DirAssets::old_root();
    vec![base.join("verification"), base.join("verification-1.10")]
}

/// Find the old capture for a file-stem template like `settings18-video-{w}x{h}` at `w`x`h` (first of `verification`, `verification-1.10`).
pub fn find_old(template: &str, w: u32, h: u32) -> Option<PathBuf> {
    let stem = template.replace("{w}", &w.to_string()).replace("{h}", &h.to_string());
    for r in old_roots() { for ext in ["bmp", "png"] { let p = r.join(format!("{stem}.{ext}")); if p.exists() { return Some(p); } } }
    None
}

pub fn load_rgb(p: &Path) -> Result<Rgb, String> {
    let im = image::open(p).map_err(|e| format!("{}: {e}", p.display()))?.to_rgb8();
    Ok(Rgb { w: im.width(), h: im.height(), px: im.into_raw() })
}

#[derive(Clone, Debug, Default)]
pub struct Metrics { pub full: f64, pub masked: f64, pub bad8: f64, pub max: u8, pub covered: usize, pub total: usize }

/// `ours` = premultiplied RGBA8 (w*h*4), `old` = RGB8 same size. Returns metrics and an RGB diff image (|ours-old| x4, clamped).
pub fn compare(ours: &[u8], old: &Rgb) -> (Metrics, Vec<u8>) {
    let n = (old.w * old.h) as usize;
    let (mut sum_full, mut sum_m, mut cnt_m, mut bad, mut mx) = (0u64, 0u64, 0usize, 0usize, 0u8);
    let mut diff = vec![0u8; n * 3];
    for i in 0..n {
        let (o, p) = (&ours[i * 4..i * 4 + 4], &old.px[i * 3..i * 3 + 3]);
        let d = [o[0].abs_diff(p[0]), o[1].abs_diff(p[1]), o[2].abs_diff(p[2])];
        let s = d[0] as u64 + d[1] as u64 + d[2] as u64;
        sum_full += s;
        for k in 0..3 { diff[i * 3 + k] = (d[k] as u32 * 4).min(255) as u8; }
        if o[3] >= 252 { sum_m += s; cnt_m += 1; let m = d[0].max(d[1]).max(d[2]); mx = mx.max(m); if m > 8 { bad += 1; } }
    }
    let m = Metrics { full: sum_full as f64 / (n as f64 * 3.), masked: if cnt_m > 0 { sum_m as f64 / (cnt_m as f64 * 3.) } else { f64::NAN }, bad8: if cnt_m > 0 { bad as f64 / cnt_m as f64 } else { f64::NAN }, max: mx, covered: cnt_m, total: n };
    (m, diff)
}

/// Our framebuffer composited over black as RGB8.
pub fn over_black(ours: &[u8]) -> Vec<u8> { ours.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect() }

pub fn save_png_rgb(path: &Path, w: u32, h: u32, rgb: &[u8]) -> Result<(), String> {
    image::save_buffer(path, rgb, w, h, image::ColorType::Rgb8).map_err(|e| e.to_string())
}
pub fn save_png_rgba(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    image::save_buffer(path, rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
}

/// Un-premultiply for viewing a render as a normal image over a checker/black: here composited over mid grey so transparency is visible.
pub fn over_grey(ours: &[u8], grey: u8) -> Vec<u8> {
    ours.chunks_exact(4).flat_map(|p| { let inv = 255 - p[3] as u32; [0, 1, 2].map(|k| (p[k] as u32 + (grey as u32 * inv + 127) / 255).min(255) as u8) }).collect()
}

/// Side by side [ours | old | diff x4] contact sheet (all RGB8, same size), each panel downscaled to at most `max_w` pixels wide.
pub fn sheet(w: u32, h: u32, a: &[u8], b: &[u8], d: &[u8]) -> (u32, u32, Vec<u8>) {
    let max_w = 800u32;
    let (pw, ph) = if w > max_w { (max_w, (h as f32 * max_w as f32 / w as f32).round().max(1.) as u32) } else { (w, h) };
    let scale = |img: &[u8]| -> Vec<u8> {
        if (pw, ph) == (w, h) { return img.to_vec(); }
        let im = image::RgbImage::from_raw(w, h, img.to_vec()).unwrap();
        image::imageops::resize(&im, pw, ph, image::imageops::FilterType::Triangle).into_raw()
    };
    let (a, b, d) = (scale(a), scale(b), scale(d));
    let mut out = Vec::with_capacity((pw * ph * 9) as usize);
    for y in 0..ph as usize { for img in [&a, &b, &d] { out.extend_from_slice(&img[y * pw as usize * 3..(y + 1) * pw as usize * 3]); } }
    (pw * 3, ph, out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn identical_images_have_zero_diff() {
        let old = Rgb { w: 2, h: 1, px: vec![10, 20, 30, 40, 50, 60] };
        let ours = vec![10, 20, 30, 255, 40, 50, 60, 255];
        let (m, _) = compare(&ours, &old); assert_eq!(m.full, 0.); assert_eq!(m.masked, 0.); assert_eq!(m.max, 0);
    }
    #[test] fn masked_ignores_uncovered_pixels() {
        let old = Rgb { w: 2, h: 1, px: vec![0, 0, 0, 255, 255, 255] };
        let ours = vec![0, 0, 0, 255, 0, 0, 0, 0];
        let (m, _) = compare(&ours, &old); assert_eq!(m.masked, 0.); assert_eq!(m.covered, 1); assert!((m.full - 127.5).abs() < 1e-9);
    }
    #[test] fn template_expansion() { assert!(find_old("definitely-missing-{w}x{h}", 1, 1).is_none()); }
}

