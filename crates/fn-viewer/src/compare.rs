//! `--compare <old.bmp>`: numeric comparison against a capture of the OpenGL prototype, plus a side-by-side PNG.
use image::{Rgb, RgbImage};
use std::fmt::Write;

/// Mean absolute per-channel difference (0..255) over the whole image.
pub fn mean_abs_diff(a: &RgbImage, b: &RgbImage) -> [f64; 3] {
    assert_eq!(a.dimensions(), b.dimensions());
    let mut s = [0f64; 3];
    for (p, q) in a.pixels().zip(b.pixels()) { for c in 0..3 { s[c] += (p[c] as f64 - q[c] as f64).abs(); } }
    let n = (a.width() * a.height()) as f64;
    s.map(|v| v / n)
}
/// Mean RGB of a pixel rectangle `[x0,x1) x [y0,y1)`.
pub fn region_mean(img: &RgbImage, r: (u32, u32, u32, u32)) -> [f64; 3] {
    let (x0, y0, x1, y1) = (r.0.min(img.width()), r.1.min(img.height()), r.2.min(img.width()), r.3.min(img.height()));
    let mut s = [0f64; 3];
    let mut n = 0f64;
    for y in y0..y1 { for x in x0..x1 { let p = img.get_pixel(x, y); for c in 0..3 { s[c] += p[c] as f64; } n += 1.; } }
    s.map(|v| v / n.max(1.))
}
pub fn luma(c: [f64; 3]) -> f64 { 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2] }
/// Mean abs difference inside a rectangle.
pub fn region_diff(a: &RgbImage, b: &RgbImage, r: (u32, u32, u32, u32)) -> [f64; 3] {
    let mut s = [0f64; 3];
    let mut n = 0f64;
    for y in r.1.min(a.height())..r.3.min(a.height()) { for x in r.0.min(a.width())..r.2.min(a.width()) {
        let (p, q) = (a.get_pixel(x, y), b.get_pixel(x, y));
        for c in 0..3 { s[c] += (p[c] as f64 - q[c] as f64).abs(); }
        n += 1.;
    } }
    s.map(|v| v / n.max(1.))
}

/// Regions as fractions of the frame (x0, y0, x1, y1), measured on the prototype's 1264x761 idle capture so that they avoid the HUD
/// (compass, minimap, quickbar, health bar, resource panel) and the placeholder tree/rock props the viewer does not draw.
pub const REGIONS: [(&str, (f32, f32, f32, f32)); 5] = [
    ("sky (between the trees, below the compass)", (0.396, 0.105, 0.672, 0.269)),
    ("far ground (horizon band)", (0.380, 0.289, 0.601, 0.434)),
    ("near ground left (bottom-left)", (0.016, 0.631, 0.316, 0.972)),
    ("near ground right of character", (0.601, 0.736, 0.696, 0.854)),
    ("character box (incl. ground behind)", (0.340, 0.447, 0.589, 0.972)),
];
/// Regions that contain the props (trees, rock + their shadows) and the HUD; only meaningful when the viewer draws them (props on, --demo-ui).
pub const EXTRA_REGIONS: [(&str, (f32, f32, f32, f32)); 7] = [
    ("tree canopy (left)", (0.07, 0.03, 0.33, 0.20)),
    ("tree trunk (left)", (0.215, 0.26, 0.255, 0.44)),
    ("rock", (0.64, 0.37, 0.74, 0.50)),
    ("tree shadow on ground", (0.01, 0.50, 0.17, 0.56)),
    ("quickbar (HUD)", (0.71, 0.86, 0.98, 0.935)),
    ("vitals bars (HUD)", (0.37, 0.86, 0.63, 0.92)),
    ("minimap (HUD)", (0.83, 0.02, 0.985, 0.28)),
];
pub fn region_px(img: &RgbImage, f: (f32, f32, f32, f32)) -> (u32, u32, u32, u32) {
    let (w, h) = (img.width() as f32, img.height() as f32);
    ((f.0 * w) as u32, (f.1 * h) as u32, (f.2 * w) as u32, (f.3 * h) as u32)
}

/// Builds the text report and the side-by-side image (new | old | |diff| x4).
pub fn report(new: &RgbImage, old: &RgbImage, label_new: &str, label_old: &str) -> (String, RgbImage) {
    let mut out = String::new();
    let mad = mean_abs_diff(new, old);
    let _ = writeln!(out, "compare {label_new} vs {label_old} ({}x{})", new.width(), new.height());
    let _ = writeln!(out, "  full frame mean |diff| per channel (0..255): R {:.2} G {:.2} B {:.2}  (mean {:.2}); includes HUD + props the viewer does not draw", mad[0], mad[1], mad[2], (mad[0] + mad[1] + mad[2]) / 3.);
    let (mn, mo) = (region_mean(new, (0, 0, new.width(), new.height())), region_mean(old, (0, 0, old.width(), old.height())));
    let _ = writeln!(out, "  full frame mean RGB new ({:.1},{:.1},{:.1}) luma {:.1} | old ({:.1},{:.1},{:.1}) luma {:.1}", mn[0], mn[1], mn[2], luma(mn), mo[0], mo[1], mo[2], luma(mo));
    for (name, f) in REGIONS.iter().chain(EXTRA_REGIONS.iter()).copied() {
        let r = region_px(new, f);
        let (a, b, d) = (region_mean(new, r), region_mean(old, r), region_diff(new, old, r));
        let _ = writeln!(out, "  [{name}] px {:?}: new ({:.1},{:.1},{:.1}) luma {:.1} | old ({:.1},{:.1},{:.1}) luma {:.1} | luma delta {:+.1} ({:+.1}%) | mean |diff| ({:.1},{:.1},{:.1})",
            r, a[0], a[1], a[2], luma(a), b[0], b[1], b[2], luma(b), luma(a) - luma(b), 100. * (luma(a) - luma(b)) / luma(b).max(1.), d[0], d[1], d[2]);
    }
    let (w, h) = new.dimensions();
    let mut sbs = RgbImage::new(w * 3, h);
    for y in 0..h { for x in 0..w {
        let (p, q) = (new.get_pixel(x, y), old.get_pixel(x, y));
        sbs.put_pixel(x, y, *p);
        sbs.put_pixel(x + w, y, *q);
        sbs.put_pixel(x + 2 * w, y, Rgb([0, 1, 2].map(|c| ((p[c] as i32 - q[c] as i32).unsigned_abs() * 4).min(255) as u8)));
    } }
    (out, sbs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_images_have_zero_difference_and_shifts_are_measured() {
        let a = RgbImage::from_pixel(8, 4, Rgb([10, 20, 30]));
        assert_eq!(mean_abs_diff(&a, &a), [0.; 3]);
        let b = RgbImage::from_pixel(8, 4, Rgb([20, 20, 25]));
        assert_eq!(mean_abs_diff(&a, &b), [10., 0., 5.]);
    }
    #[test]
    fn region_mean_and_diff_only_look_inside_the_rectangle() {
        let mut a = RgbImage::from_pixel(10, 10, Rgb([0, 0, 0]));
        for y in 0..5 { for x in 0..5 { a.put_pixel(x, y, Rgb([100, 100, 100])); } }
        let b = RgbImage::from_pixel(10, 10, Rgb([0, 0, 0]));
        assert_eq!(region_mean(&a, (0, 0, 5, 5)), [100.; 3]);
        assert_eq!(region_mean(&a, (5, 5, 10, 10)), [0.; 3]);
        assert_eq!(region_diff(&a, &b, (0, 0, 10, 10)), [25.; 3]);
    }
    #[test]
    fn regions_stay_inside_the_frame_and_report_builds() {
        for (_, f) in REGIONS { assert!(f.0 >= 0. && f.1 >= 0. && f.2 <= 1. && f.3 <= 1. && f.0 < f.2 && f.1 < f.3); }
        let a = RgbImage::from_pixel(64, 48, Rgb([50, 60, 70]));
        let (text, sbs) = report(&a, &a, "new", "old");
        assert!(text.contains("full frame") && sbs.width() == 192 && sbs.height() == 48);
    }
}
