//! Texture stage: manifest `textures[]` + `aliases[]` -> PNG files; optional pixel comparison against an old export tree.
use crate::source::Source;
use crate::{entry, parallel, write_file, Args, Stage};
use fn_texture::{DecodeOptions, Texture2D};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

struct Job {
    pkg: String,
    outs: Vec<String>,
    optional: bool,
}

fn jobs_from(manifest: &Value, only: &Option<String>) -> Vec<Job> {
    let mut v = Vec::new();
    for t in manifest["textures"].as_array().into_iter().flatten() {
        v.push(Job { pkg: t["pkg"].as_str().unwrap_or("").into(), outs: vec![t["out"].as_str().unwrap_or("").into()], optional: false });
    }
    for a in manifest["aliases"].as_array().into_iter().flatten() {
        v.push(Job { pkg: a["pkg"].as_str().unwrap_or("").into(), outs: vec![a["out"].as_str().unwrap_or("").into(), a["alias"].as_str().unwrap_or("").into()], optional: false });
    }
    // packages the manifest lists as missing in 1.8 are not exported (they do not exist in the install)
    if let Some(f) = only {
        v.retain(|j| j.pkg.to_lowercase().contains(f.as_str()));
    }
    v
}

/// Compare two RGBA8 images: (max abs channel diff, mean abs diff, differing pixels). `None` when sizes differ.
pub fn compare_rgba(a: &image_like::Rgba, b: &image_like::Rgba) -> Option<(u8, f64, usize)> {
    if a.w != b.w || a.h != b.h {
        return None;
    }
    let (mut max, mut sum, mut cnt) = (0u8, 0u64, 0usize);
    for (pa, pb) in a.px.chunks_exact(4).zip(b.px.chunks_exact(4)) {
        let mut d = false;
        for c in 0..4 {
            let x = pa[c].abs_diff(pb[c]);
            max = max.max(x);
            sum += x as u64;
            d |= x != 0;
        }
        cnt += d as usize;
    }
    Some((max, sum as f64 / (a.px.len().max(1)) as f64, cnt))
}

pub mod image_like {
    pub struct Rgba {
        pub w: u32,
        pub h: u32,
        pub px: Vec<u8>,
    }
    pub fn load_png(path: &std::path::Path) -> Result<Rgba, String> {
        let im = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8();
        Ok(Rgba { w: im.width(), h: im.height(), px: im.into_raw() })
    }
}

fn export_one(src: &Source, job: &Job, args: &Args) -> Result<Value, String> {
    let raw = src.load_raw(&job.pkg)?;
    let tex = Texture2D::from_files(&raw.uasset, &raw.uexp, &raw.ubulk).map_err(|e| e.to_string())?;
    let mip = tex.best_mip().ok_or("no mip with data")?;
    let img = tex.decode(mip, DecodeOptions::default()).map_err(|e| e.to_string())?;
    let mut d = json!({"format": format!("{:?}", tex.format), "width": img.width, "height": img.height, "mip": mip});
    for o in &job.outs {
        let dest = args.out.join(o);
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        img.write_png(&dest).map_err(|e| e.to_string())?;
    }
    if let Some(old_root) = &args.verify {
        let new = image_like::load_png(&args.out.join(&job.outs[0]))?;
        let old_path = old_root.join(&job.outs[0]);
        match image_like::load_png(&old_path) {
            Ok(old) => match compare_rgba(&new, &old) {
                Some((max, mean, diff)) => {
                    d["verify"] = json!({"max_abs": max, "mean_abs": mean, "diff_pixels": diff, "old_size": [old.w, old.h]});
                }
                None => d["verify"] = json!({"error": "size mismatch", "new": [new.w, new.h], "old": [old.w, old.h]}),
            },
            Err(e) => d["verify"] = json!({"error": e}),
        }
    }
    let _ = write_file; // (kept for symmetry with other stages)
    Ok(d)
}

pub fn run(src: &Source, manifest: &Value, args: &Args) -> Stage {
    let jobs = jobs_from(manifest, &args.only);
    println!("textures: {} jobs", jobs.len());
    let results = parallel(&jobs, args.jobs, |j| {
        let t = Instant::now();
        let r = export_one(src, j, args);
        (r, t.elapsed().as_millis())
    });
    let mut entries = Vec::new();
    let mut failures = 0;
    let (mut exact, mut near, mut bad, mut unv) = (0, 0, 0, 0);
    for (j, (r, ms)) in jobs.iter().zip(results) {
        if r.is_err() && !j.optional {
            failures += 1;
        }
        if let Ok(d) = &r {
            match d.get("verify") {
                Some(v) if v.get("max_abs").is_some() => {
                    let m = v["max_abs"].as_u64().unwrap_or(255);
                    if m == 0 {
                        exact += 1
                    } else if m <= 2 {
                        near += 1
                    } else {
                        bad += 1
                    }
                }
                Some(_) => unv += 1,
                None => {}
            }
        }
        entries.push(entry("texture", &j.pkg, &j.outs[0], &r, ms));
    }
    if args.verify.is_some() {
        println!("texture verify vs old PNGs: exact {exact}, max<=2 {near}, worse {bad}, size mismatch/unreadable {unv}");
    }
    println!("textures: {} ok, {failures} failed", jobs.len() - failures);
    let _ = Path::new("");
    Stage { entries, failures }
}
