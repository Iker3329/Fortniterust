//! Scan a directory tree of `.uasset` files for textures and print histograms.
//!
//! usage: `cargo run --release -p fn-texture --example scan -- <dir> [--list-rare]`
//!
//! * With a sibling `.uexp` present the texture is fully parsed ([`fn_texture::Texture2D`]): pixel format,
//!   compression settings, sRGB, slices, mips and streaming are histogrammed.
//! * Header-only `.uasset` files (no `.uexp`) are classified from the export table and the name table only: the
//!   `PF_*` FName entries of a texture package are its pixel formats, so `PF_BC7` / `PF_BC6H` / `TextureCube` can be
//!   found without the payload.

use fn_texture::Texture2D;
use fn_ueio::Package;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map(|x| x == "uasset").unwrap_or(false) {
                out.push(p);
            }
        }
    }
}

fn bump(m: &mut BTreeMap<String, usize>, k: impl Into<String>) {
    *m.entry(k.into()).or_default() += 1;
}

fn print(title: &str, m: &BTreeMap<String, usize>) {
    println!("--- {title}");
    for (k, v) in m {
        println!("  {k:<48} {v}");
    }
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("directory"));
    let list_rare = std::env::args().any(|a| a == "--list-rare");
    // `--format=PF_BC4` additionally lists header-only packages that contain that pixel format name
    let list_formats: Vec<String> = std::env::args().filter_map(|a| a.strip_prefix("--format=").map(str::to_string)).collect();
    let mut files = Vec::new();
    walk(&dir, &mut files);
    files.sort();
    let (mut classes, mut formats, mut compression, mut srgb, mut slices, mut stream, mut rare) = (
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        Vec::new(),
    );
    let mut parse_fail = 0;
    let mut textures = 0;
    for f in &files {
        let uasset = std::fs::read(f).unwrap();
        let uexp = std::fs::read(f.with_extension("uexp")).unwrap_or_default();
        let ubulk = std::fs::read(f.with_extension("ubulk")).unwrap_or_default();
        let has_exp = f.with_extension("uexp").exists();
        let pkg = match Package::parse(&uasset, &uexp, &ubulk) {
            Ok(p) => p,
            Err(_) => {
                parse_fail += 1;
                continue;
            }
        };
        for i in 0..pkg.exports.len() {
            let class = pkg.export_class(i);
            if !class.contains("Texture") || class.contains("Material") || class.contains("Sample") {
                continue;
            }
            bump(&mut classes, class.clone());
            if class != "Texture2D" && class != "TextureCube" {
                rare.push(format!("{} [{}]", f.display(), class));
                continue;
            }
            textures += 1;
            if has_exp {
                match Texture2D::parse_export(&pkg, i) {
                    Ok(t) => {
                        bump(&mut formats, format!("{} ({})", t.format.name(), t.class));
                        bump(&mut compression, format!("{} / {}", t.compression_settings, t.format.name()));
                        bump(&mut srgb, format!("SRGB={} / {}", t.srgb, t.compression_settings));
                        bump(&mut slices, format!("slices={}", t.num_slices));
                        let ub = t.mips.iter().filter(|m| m.in_ubulk()).count();
                        let avail = t.mips.iter().filter(|m| m.is_available()).count();
                        bump(&mut stream, format!("mips={} available={} ubulk_mips={}", t.mips.len(), avail, ub));
                        if matches!(t.format.name(), "PF_BC7" | "PF_BC6H") || t.is_cube() || !t.format.bytes_per_unit().is_some() {
                            rare.push(format!("{} [{} {}]", f.display(), t.class, t.format.name()));
                        }
                    }
                    Err(e) => {
                        bump(&mut formats, format!("PARSE ERROR: {e}"));
                    }
                }
            } else {
                let pf: Vec<&String> = pkg.names.iter().filter(|n| n.starts_with("PF_")).collect();
                let key = if pf.is_empty() { "(no PF_ name)".to_string() } else { pf.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("+") };
                bump(&mut formats, format!("{key} ({class}) [name table]"));
                if class == "TextureCube" || pf.iter().any(|n| *n == "PF_BC7" || *n == "PF_BC6H" || list_formats.contains(n)) {
                    rare.push(format!("{} [{} {}]", f.display(), class, key));
                }
            }
        }
    }
    println!("{} .uasset files, {} parse failures, {} Texture2D/TextureCube exports", files.len(), parse_fail, textures);
    print("texture-like export classes", &classes);
    print("pixel formats", &formats);
    print("compression settings / format", &compression);
    print("sRGB / compression", &srgb);
    print("slices", &slices);
    print("mip availability", &stream);
    println!("--- rare / unsupported ({})", rare.len());
    if list_rare {
        for r in &rare {
            println!("  {r}");
        }
    }
}
