//! Compare `fn-texture` output with the old UEViewer PNG exports listed in `asset-manifest.json`.
//!
//! usage: `cargo run --release -p fn-texture --example verify -- [testdata_dir] [runtime_dir]`
//! defaults: `%TEMP%\fn-testdata`, and the prototype tree next to this workspace (`..\FortniteRuntime`).
//!
//! Per texture: format, dims, max abs channel error, mean abs error, differing pixel count; then a per-format table.
//! Old PNGs are loaded with `image::to_rgba8`; our output is converted to RGBA8 and, like UEViewer's PNG writer,
//! alpha is forced to 255 when the whole image has alpha 0 (the writer drops the channel in that case).

use fn_texture::{DecodeOptions, Texture2D};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Summary {
    count: usize,
    exact: usize,
    dim_mismatch: usize,
    failed: usize,
    worst: u32,
    mean_sum: f64,
}

fn pkg_path(testdata: &Path, pkg: &str, ext: &str) -> PathBuf {
    if let Some(rest) = pkg.strip_prefix("Engine/") {
        testdata.join("Engine").join("Content").join(rest).with_extension(ext)
    } else if let Some(rest) = pkg.strip_prefix("Game/") {
        testdata.join("FortniteGame").join("Content").join(rest).with_extension(ext)
    } else {
        testdata.join(pkg).with_extension(ext)
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let testdata = args.next().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("TEMP").unwrap_or_default()).join("fn-testdata"));
    let runtime = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Brady\Documents\Codex\2026-10-02\im-seeing-titkoks-where-people-are-2\FortniteRuntime"));
    if !testdata.is_dir() || !runtime.is_dir() {
        println!("SKIP: testdata ({}) or runtime ({}) directory missing", testdata.display(), runtime.display());
        return;
    }
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(runtime.join("tools").join("asset-manifest.json")).unwrap()).unwrap();
    let assets = runtime.join("assets");

    // (pkg, old png relative path)
    let mut jobs: Vec<(String, String)> = Vec::new();
    for t in manifest["textures"].as_array().unwrap() {
        jobs.push((t["pkg"].as_str().unwrap().to_string(), t["out"].as_str().unwrap().to_string()));
    }
    for t in manifest["aliases"].as_array().unwrap() {
        jobs.push((t["pkg"].as_str().unwrap().to_string(), t["alias"].as_str().unwrap().to_string()));
    }

    let mut sums: BTreeMap<String, Summary> = BTreeMap::new();
    let opts = DecodeOptions::default();
    let (mut skipped_pkg, mut skipped_png) = (0, 0);
    println!("{:<70} {:<12} {:>11} {:>11} {:>5} {:>8} {:>9}", "texture", "format", "ours", "old", "max", "mean", "diffpx");
    for (pkg, out) in &jobs {
        let (ua, ue, ub) = (pkg_path(&testdata, pkg, "uasset"), pkg_path(&testdata, pkg, "uexp"), pkg_path(&testdata, pkg, "ubulk"));
        if !ua.exists() {
            skipped_pkg += 1;
            println!("{pkg}: raw package not in testdata (skipped)");
            continue;
        }
        let png_path = assets.join(out);
        if !png_path.exists() {
            skipped_png += 1;
            println!("{pkg}: old png {} missing (skipped)", png_path.display());
            continue;
        }
        let rd = |p: &Path| std::fs::read(p).unwrap_or_default();
        let tex = match Texture2D::from_files(&rd(&ua), &rd(&ue), &rd(&ub)) {
            Ok(t) => t,
            Err(e) => {
                println!("{pkg}: PARSE ERROR {e}");
                sums.entry("<parse error>".into()).or_default().failed += 1;
                continue;
            }
        };
        let fmt = tex.format.name().to_string();
        let s = sums.entry(fmt.clone()).or_default();
        s.count += 1;
        let Some(mip) = tex.best_mip() else {
            println!("{pkg}: no mip with data");
            s.failed += 1;
            continue;
        };
        let img = match tex.decode(mip, opts) {
            Ok(i) => i,
            Err(e) => {
                println!("{pkg}: DECODE ERROR {e}");
                s.failed += 1;
                continue;
            }
        };
        let mut mine = img.to_rgba8();
        if mine.chunks_exact(4).all(|p| p[3] == 0) {
            mine.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        }
        let old = image::open(&png_path).unwrap().to_rgba8();
        let (ow, oh) = old.dimensions();
        let ours = format!("{}x{}", img.width, img.height);
        let oldd = format!("{ow}x{oh}");
        if (ow as usize, oh as usize) != (img.width, img.height) {
            s.dim_mismatch += 1;
            println!("{:<70} {:<12} {:>11} {:>11} DIMENSION MISMATCH (mip {mip}, top {}x{})", short(pkg), fmt, ours, oldd, tex.size_x, tex.size_y);
            continue;
        }
        let o = old.as_raw();
        let (mut max, mut sum, mut diffpx) = (0u32, 0u64, 0usize);
        for (a, b) in mine.chunks_exact(4).zip(o.chunks_exact(4)) {
            let mut d = false;
            for k in 0..4 {
                let e = (a[k] as i32 - b[k] as i32).unsigned_abs();
                if e != 0 {
                    d = true;
                    max = max.max(e);
                    sum += e as u64;
                }
            }
            diffpx += d as usize;
        }
        let mean = sum as f64 / mine.len() as f64;
        if max == 0 {
            s.exact += 1;
        }
        s.worst = s.worst.max(max);
        s.mean_sum += mean;
        println!("{:<70} {:<12} {:>11} {:>11} {:>5} {:>8.4} {:>9}  mip{mip} srgb={} {}", short(pkg), fmt, ours, oldd, max, mean, diffpx, tex.srgb, tex.compression_settings);
    }
    println!();
    println!("{:<14} {:>6} {:>7} {:>9} {:>7} {:>9} {:>10}", "format", "count", "exact", "dimMismat", "failed", "worstMax", "meanOfMean");
    for (f, s) in &sums {
        let cmp = s.count.saturating_sub(s.dim_mismatch + s.failed);
        println!("{:<14} {:>6} {:>7} {:>9} {:>7} {:>9} {:>10.5}", f, s.count, s.exact, s.dim_mismatch, s.failed, s.worst, if cmp > 0 { s.mean_sum / cmp as f64 } else { 0.0 });
    }
    println!("skipped (no raw package): {skipped_pkg}, skipped (no old png): {skipped_png}");
}

fn short(p: &str) -> String {
    if p.len() <= 70 {
        p.to_string()
    } else {
        format!("...{}", &p[p.len() - 67..])
    }
}
