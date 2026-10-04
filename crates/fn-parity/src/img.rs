//! Image comparison: mean absolute error per channel, an SSIM-like structural score, per-region statistics, heatmap and side-by-side sheets.
//! Everything works on RGB8 (the old exe writes bottom-up BMPs, the `image` crate flips them to top-down on load).
use std::path::Path;

#[derive(Clone)]
pub struct Img { pub w: u32, pub h: u32, pub px: Vec<u8> }

impl Img {
    pub fn new(w: u32, h: u32, fill: [u8; 3]) -> Img { let mut px = Vec::with_capacity((w * h * 3) as usize); for _ in 0..w * h { px.extend_from_slice(&fill); } Img { w, h, px } }
    pub fn at(&self, x: u32, y: u32) -> [u8; 3] { let i = ((y * self.w + x) * 3) as usize; [self.px[i], self.px[i + 1], self.px[i + 2]] }
    pub fn set(&mut self, x: u32, y: u32, c: [u8; 3]) { let i = ((y * self.w + x) * 3) as usize; self.px[i..i + 3].copy_from_slice(&c); }
    pub fn luma(&self) -> Vec<f32> { self.px.chunks_exact(3).map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).collect() }
    pub fn mean_luma(&self) -> f64 { let l = self.luma(); l.iter().map(|&v| v as f64).sum::<f64>() / l.len().max(1) as f64 }
    pub fn crop(&self, w: u32, h: u32) -> Img {
        let (w, h) = (w.min(self.w), h.min(self.h));
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h { let s = (y * self.w * 3) as usize; px.extend_from_slice(&self.px[s..s + (w * 3) as usize]); }
        Img { w, h, px }
    }
}

pub fn load(path: &Path) -> Result<Img, String> {
    let im = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgb8();
    Ok(Img { w: im.width(), h: im.height(), px: im.into_raw() })
}
pub fn save_png(path: &Path, im: &Img) -> Result<(), String> {
    if let Some(d) = path.parent() { std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?; }
    image::save_buffer(path, &im.px, im.w, im.h, image::ColorType::Rgb8).map_err(|e| format!("{}: {e}", path.display()))
}
pub fn png_bytes(im: &Img) -> Vec<u8> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    let enc = image::codecs::png::PngEncoder::new_with_quality(&mut out, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Adaptive);
    let _ = enc.write_image(&im.px, im.w, im.h, image::ExtendedColorType::Rgb8);
    out
}

/// Box-filter downscale so the width is at most `max_w` (never upscales).
pub fn downscale(im: &Img, max_w: u32) -> Img {
    if im.w <= max_w { return im.clone(); }
    let nw = max_w.max(1); let nh = ((im.h as u64 * nw as u64 + im.w as u64 / 2) / im.w as u64).max(1) as u32;
    let mut out = Img::new(nw, nh, [0; 3]);
    for y in 0..nh {
        let (y0, y1) = ((y as u64 * im.h as u64 / nh as u64) as u32, (((y + 1) as u64 * im.h as u64 / nh as u64) as u32).max((y as u64 * im.h as u64 / nh as u64) as u32 + 1).min(im.h));
        for x in 0..nw {
            let (x0, x1) = ((x as u64 * im.w as u64 / nw as u64) as u32, (((x + 1) as u64 * im.w as u64 / nw as u64) as u32).max((x as u64 * im.w as u64 / nw as u64) as u32 + 1).min(im.w));
            let (mut s, mut n) = ([0u32; 3], 0u32);
            for yy in y0..y1 { for xx in x0..x1 { let p = im.at(xx, yy); s[0] += p[0] as u32; s[1] += p[1] as u32; s[2] += p[2] as u32; n += 1; } }
            let n = n.max(1); out.set(x, y, [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]);
        }
    }
    out
}

/// A rectangle in fractions of the image (0..1).
#[derive(Clone, Copy, Debug)]
pub struct Rect { pub x0: f32, pub y0: f32, pub x1: f32, pub y1: f32 }
impl Rect {
    pub const FULL: Rect = Rect { x0: 0., y0: 0., x1: 1., y1: 1. };
    pub fn px(&self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        let f = |v: f32, n: u32| ((v.clamp(0., 1.) * n as f32).round() as u32).min(n);
        let (x0, y0, x1, y1) = (f(self.x0, w), f(self.y0, h), f(self.x1, w), f(self.y1, h));
        (x0, y0, x1.max(x0), y1.max(y0))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Metrics {
    pub w: u32, pub h: u32, pub n: usize,
    /// Mean absolute error per channel (0..255).
    pub mae: [f64; 3], pub mae_all: f64, pub rmse: f64, pub psnr: f64,
    pub ssim: f64,
    /// Contrast/structure term of SSIM only (insensitive to brightness and colour shifts): used for framing / camera checks.
    pub structure: f64,
    /// Fraction of pixels whose largest channel difference exceeds 16/255.
    pub bad_pct: f64,
    pub mean_luma_a: f64, pub mean_luma_b: f64,
}

/// Compare `a` (reference) and `b` over `rect`. The images must have the same size (callers crop first).
pub fn metrics(a: &Img, b: &Img, rect: Rect) -> Metrics {
    let (w, h) = (a.w.min(b.w), a.h.min(b.h));
    let (x0, y0, x1, y1) = rect.px(w, h);
    let mut m = Metrics { w: x1 - x0, h: y1 - y0, ..Default::default() };
    if x1 <= x0 || y1 <= y0 { return m; }
    let (mut sum, mut sq, mut bad, mut la, mut lb) = ([0u64; 3], 0u64, 0usize, 0f64, 0f64);
    for y in y0..y1 { for x in x0..x1 {
        let (p, q) = (a.at(x, y), b.at(x, y));
        let d = [p[0].abs_diff(q[0]), p[1].abs_diff(q[1]), p[2].abs_diff(q[2])];
        for k in 0..3 { sum[k] += d[k] as u64; sq += (d[k] as u64) * (d[k] as u64); }
        if d[0].max(d[1]).max(d[2]) > 16 { bad += 1; }
        la += 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
        lb += 0.2126 * q[0] as f64 + 0.7152 * q[1] as f64 + 0.0722 * q[2] as f64;
    } }
    let n = ((x1 - x0) * (y1 - y0)) as usize; m.n = n;
    for k in 0..3 { m.mae[k] = sum[k] as f64 / n as f64; }
    m.mae_all = (m.mae[0] + m.mae[1] + m.mae[2]) / 3.;
    let mse = sq as f64 / (n as f64 * 3.); m.rmse = mse.sqrt();
    m.psnr = if mse <= 0. { 99. } else { (10. * (255f64 * 255. / mse).log10()).min(99.) };
    m.bad_pct = bad as f64 / n as f64 * 100.;
    m.mean_luma_a = la / n as f64; m.mean_luma_b = lb / n as f64;
    let (s, cs) = ssim(&a.luma(), &b.luma(), w as usize, (x0 as usize, y0 as usize, x1 as usize, y1 as usize), a.w as usize);
    m.ssim = s; m.structure = cs;
    m
}

/// SSIM-like structural score on luma: 8x8 windows, stride 4, uniform weights, mean over the windows inside the rectangle (1.0 = identical).
fn ssim(la: &[f32], lb: &[f32], _w: usize, r: (usize, usize, usize, usize), stride_a: usize) -> (f64, f64) {
    const WIN: usize = 8; const STEP: usize = 4;
    let (x0, y0, x1, y1) = r;
    let (c1, c2) = ((0.01f64 * 255.).powi(2), (0.03f64 * 255.).powi(2));
    let (mut total, mut total_cs, mut count) = (0f64, 0f64, 0usize);
    // images are cropped to a common width before this point, so both share `stride_a`
    let mut y = y0;
    while y + WIN <= y1 {
        let mut x = x0;
        while x + WIN <= x1 {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0f64, 0f64, 0f64, 0f64, 0f64);
            for yy in y..y + WIN { for xx in x..x + WIN {
                let (p, q) = (la[yy * stride_a + xx] as f64, lb[yy * stride_a + xx] as f64);
                sa += p; sb += q; saa += p * p; sbb += q * q; sab += p * q;
            } }
            let n = (WIN * WIN) as f64;
            let (ma, mb) = (sa / n, sb / n);
            let (va, vb, cov) = (saa / n - ma * ma, sbb / n - mb * mb, sab / n - ma * mb);
            let cs = (2. * cov + c2) / (va + vb + c2);
            total += ((2. * ma * mb + c1) / (ma * ma + mb * mb + c1)) * cs; total_cs += cs;
            count += 1;
            x += STEP;
        }
        y += STEP;
    }
    if count == 0 { (1., 1.) } else { (total / count as f64, total_cs / count as f64) }
}

/// |a-b| as a heatmap over a dimmed copy of `b` (the new capture) so the location of the error is readable.
pub fn heatmap(a: &Img, b: &Img) -> Img {
    let (w, h) = (a.w.min(b.w), a.h.min(b.h));
    let mut out = Img::new(w, h, [0; 3]);
    for y in 0..h { for x in 0..w {
        let (p, q) = (a.at(x, y), b.at(x, y));
        let d = p[0].abs_diff(q[0]).max(p[1].abs_diff(q[1])).max(p[2].abs_diff(q[2])) as f32;
        let t = (d / 64.).min(1.);
        let g = (0.2126 * q[0] as f32 + 0.7152 * q[1] as f32 + 0.0722 * q[2] as f32) * 0.35;
        let hot = [255. * t.powf(0.6), 255. * ((t - 0.4).max(0.) / 0.6).powf(0.9), 255. * ((t - 0.85).max(0.) / 0.15)];
        let k = t.powf(0.5);
        let c = |i: usize| (g * (1. - k) + hot[i] * k).clamp(0., 255.) as u8;
        out.set(x, y, [c(0), c(1), c(2)]);
    } }
    out
}

/// Three panels (old | new | heatmap), each scaled to `panel_w`, with 4 px separators and a coloured top bar per panel
/// (blue = old, green = new, red = heatmap) since there is no font renderer here; the report legend explains the order.
pub fn side_by_side(a: &Img, b: &Img, heat: &Img, panel_w: u32) -> Img {
    let panels = [downscale(a, panel_w), downscale(b, panel_w), downscale(heat, panel_w)];
    let (h, gap, bar) = (panels.iter().map(|p| p.h).max().unwrap_or(1), 4u32, 4u32);
    let total_w: u32 = panels.iter().map(|p| p.w).sum::<u32>() + gap * 2;
    let mut out = Img::new(total_w, h + bar, [24, 24, 24]);
    let colors = [[70, 130, 255], [70, 200, 110], [240, 80, 70]];
    let mut ox = 0;
    for (i, p) in panels.iter().enumerate() {
        for x in 0..p.w { for y in 0..bar { out.set(ox + x, y, colors[i]); } }
        for y in 0..p.h { for x in 0..p.w { out.set(ox + x, y + bar, p.at(x, y)); } }
        ox += p.w + gap;
    }
    out
}

/// Mean absolute error of a crop-aligned pair plus the crop applied.
pub fn align(a: &Img, b: &Img) -> (Img, Img, bool) {
    if a.w == b.w && a.h == b.h { return (a.clone(), b.clone(), false); }
    let (w, h) = (a.w.min(b.w), a.h.min(b.h));
    (a.crop(w, h), b.crop(w, h), true)
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len() * 4 / 3 + 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char); s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}
pub fn data_uri(im: &Img) -> String { format!("data:image/png;base64,{}", base64(&png_bytes(im))) }

#[cfg(test)]
mod tests {
    use super::*;
    fn gradient(w: u32, h: u32, off: u8) -> Img { let mut i = Img::new(w, h, [0; 3]); for y in 0..h { for x in 0..w { i.set(x, y, [((x * 3) as u8).wrapping_add(off), (y * 2) as u8, ((x ^ y) * 5) as u8]); } } i }
    #[test] fn identical_images_score_perfectly() { let a = gradient(64, 48, 0); let m = metrics(&a, &a, Rect::FULL); assert_eq!(m.mae_all, 0.); assert!((m.ssim - 1.).abs() < 1e-9); assert_eq!(m.bad_pct, 0.); }
    #[test] fn offset_images_have_the_expected_mae() { let a = Img::new(32, 32, [100, 100, 100]); let b = Img::new(32, 32, [110, 90, 100]); let m = metrics(&a, &b, Rect::FULL); assert!((m.mae[0] - 10.).abs() < 1e-9 && (m.mae[1] - 10.).abs() < 1e-9 && m.mae[2] == 0.); }
    #[test] fn structure_loss_lowers_ssim() { let mut a = Img::new(64, 64, [0; 3]); for y in 0..64 { for x in 0..64 { if (x / 2 + y / 2) % 2 == 0 { a.set(x, y, [255; 3]); } } } let b = Img::new(64, 64, [128, 128, 128]); assert!(metrics(&a, &b, Rect::FULL).ssim < 0.5); }
    #[test] fn region_only_sees_its_pixels() { let a = Img::new(100, 100, [0; 3]); let mut b = a.clone(); for y in 0..50 { for x in 0..100 { b.set(x, y, [255; 3]); } } let top = metrics(&a, &b, Rect { x0: 0., y0: 0., x1: 1., y1: 0.5 }); let bot = metrics(&a, &b, Rect { x0: 0., y0: 0.5, x1: 1., y1: 1. }); assert_eq!(top.mae_all, 255.); assert_eq!(bot.mae_all, 0.); }
    #[test] fn base64_matches_known_vectors() { assert_eq!(base64(b"Man"), "TWFu"); assert_eq!(base64(b"Ma"), "TWE="); assert_eq!(base64(b"M"), "TQ=="); }
    #[test] fn downscale_keeps_aspect_and_sheet_has_three_panels() { let a = gradient(200, 100, 0); let d = downscale(&a, 50); assert_eq!((d.w, d.h), (50, 25)); let s = side_by_side(&a, &a, &heatmap(&a, &a), 50); assert_eq!(s.w, 50 * 3 + 8); }
}

