use std::path::Path;

pub fn run(manifest: &str, assets: &Path, quality: f32, out: Option<&Path>) {
    let mut seen = std::collections::HashSet::new();
    let (mut n, mut before, mut after, mut fails) = (0usize, 0u64, 0u64, 0usize);
    let mut by: std::collections::BTreeMap<String, (u64, u64)> = Default::default();
    for (p, _, _, _) in crate::analyze::manifest_ranges(manifest) {
        let Some(rel) = p.strip_prefix("/fn/assets/") else { continue };
        if !rel.to_ascii_lowercase().ends_with(".png") || !seen.insert(rel.to_ascii_lowercase()) { continue; }
        let src = assets.join(rel);
        let Ok(bytes) = std::fs::read(&src) else { continue };
        let Ok(img) = image::load_from_memory(&bytes) else { fails += 1; continue };
        let rgba = img.to_rgba8();
        let ui = rel.to_ascii_lowercase().contains("/ui/");
        let enc = webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height());
        let mem = if ui { enc.encode_lossless() } else { enc.encode(quality) };
        let w = mem.to_vec();
        let back = image::load_from_memory(&w).map(|i| i.to_rgba8());
        if back.as_ref().map_or(true, |b| b.dimensions() != rgba.dimensions()) { fails += 1; continue; }
        let k = rel.split('/').next().unwrap_or("").to_owned() + if ui { " (ui, lossless)" } else { "" };
        let e = by.entry(k).or_default();
        e.0 += bytes.len() as u64; e.1 += w.len().min(bytes.len()) as u64;
        before += bytes.len() as u64; after += w.len().min(bytes.len()) as u64; n += 1;
        if let Some(o) = out { let d = o.join(rel); let _ = std::fs::create_dir_all(d.parent().unwrap()); let _ = std::fs::write(d, if w.len() < bytes.len() { &w } else { &bytes }); }
    }
    let m = |b: u64| b as f64 / 1048576.;
    println!("png -> webp q{quality}: {n} files {:.1} MB -> {:.1} MB ({fails} failed)", m(before), m(after));
    for (k, (a, b)) in by { println!("  {k:<28} {:.1} -> {:.1} MB", m(a), m(b)); }
}
