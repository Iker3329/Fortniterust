//! Screenshot regression of the static prop shading against the old runtime's capture (`verification\idle.bmp`): trees (canopy, trunk), the
//! rock (and its shaded underside) and the tree shadow must stay within 2 % luma of the old runtime and nothing may render black.
//! Skipped (with a message) when the GPU, the asset folder or the old capture is not available.
#![cfg(test)]
use crate::compare::{luma, region_mean, region_px, EXTRA_REGIONS};
use crate::{parse_args, rgb_image, Session};
use image::RgbImage;
use std::path::{Path, PathBuf};

fn old_capture() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("FN_OLD_DIR") { let p = PathBuf::from(d).join("idle.bmp"); return p.exists().then_some(p); }
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../FortniteRuntime/verification/idle.bmp");
    p.exists().then_some(p)
}
fn region(name: &str) -> (f32, f32, f32, f32) { EXTRA_REGIONS.iter().find(|r| r.0.starts_with(name)).unwrap_or_else(|| panic!("no region {name}")).1 }
/// Share of pixels in the rectangle darker than `thr` luma (0..255).
fn dark_fraction(img: &RgbImage, r: (u32, u32, u32, u32), thr: f64) -> f64 {
    let (mut n, mut d) = (0f64, 0f64);
    for y in r.1..r.3.min(img.height()) { for x in r.0..r.2.min(img.width()) {
        let p = img.get_pixel(x, y);
        if luma([p[0] as f64, p[1] as f64, p[2] as f64]) < thr { d += 1.; }
        n += 1.;
    } }
    d / n.max(1.)
}

#[test]
fn props_match_the_old_runtime_capture_within_two_percent_and_nothing_is_black() {
    let Some(old_path) = old_capture() else { eprintln!("skipped: no old idle.bmp"); return; };
    let cli = parse_args(&["--size".into(), "1264x761".into(), "--demo-ui".into()]).unwrap();
    let mut s = match Session::open(&cli) { Ok(s) => s, Err(e) => { eprintln!("skipped: {e}"); return; } };
    let (w, h) = cli.size;
    let r = s.render(&cli, &cli.options, 1);
    let new = rgb_image(&r.rgba, w, h);
    let old = image::open(&old_path).unwrap().to_rgb8();
    assert_eq!(old.dimensions(), new.dimensions());
    for name in ["tree canopy", "tree trunk", "rock", "tree shadow"] {
        let px = region_px(&new, region(name));
        let (a, b) = (luma(region_mean(&new, px)), luma(region_mean(&old, px)));
        let delta = 100. * (a - b) / b;
        eprintln!("[{name}] new luma {a:.1} old {b:.1} ({delta:+.2}%)");
        assert!(delta.abs() <= 2.0, "{name}: luma {a:.1} vs old {b:.1} = {delta:+.2}% (limit 2%)");
        // no blackened faces: the dark-pixel share may not grow versus the old capture
        let (dn, dop) = (dark_fraction(&new, px, 10.), dark_fraction(&old, px, 10.));
        assert!(dn <= dop + 0.01, "{name}: {:.1}% of pixels are near-black (old {:.1}%)", dn * 100., dop * 100.);
    }
    // shaded underside of the rock: lower third of the rock rectangle keeps the old luma and the blue-tinted sky ambient
    let rr = region_px(&new, region("rock"));
    let low = (rr.0, rr.1 + (rr.3 - rr.1) * 2 / 3, rr.2, rr.3);
    let (m, mo) = (region_mean(&new, low), region_mean(&old, low));
    eprintln!("[rock underside] new {m:.1?} old {mo:.1?}");
    assert!((luma(m) - luma(mo)).abs() / luma(mo) <= 0.03, "rock underside luma {:.1} vs {:.1}", luma(m), luma(mo));
    assert!(luma(m) > 20. && m[2] > m[0], "rock underside black or not sky-tinted: {m:?}");
}
