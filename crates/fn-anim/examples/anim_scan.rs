//! Parse every AnimSequence in the testdata dir and print the codec histogram.
//! usage: cargo run --release -p fn-anim --example scan -- <testdata root> [-v]
use fn_anim::AnimSequence;
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

fn main() {
    let root = std::env::args().nth(1).expect("root");
    let verbose = std::env::args().any(|a| a == "-v");
    let mut files = Vec::new();
    walk(Path::new(&root), &mut files);
    files.sort();
    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    let (mut ok, mut fail, mut skipped) = (0, 0, 0);
    let mut notify_names: BTreeMap<String, usize> = BTreeMap::new();
    let (mut curves, mut with_scale, mut raw) = (0, 0, 0);
    let mut scale_stats: BTreeMap<String, (f32, f32, usize)> = BTreeMap::new();
    let mut track_hist: BTreeMap<String, usize> = BTreeMap::new();
    let mut key_hist: BTreeMap<String, usize> = BTreeMap::new();
    for f in files {
        let rd = |e: &str| std::fs::read(f.with_extension(e)).unwrap_or_default();
        let pkg = match Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if pkg.find_export("AnimSequence").is_none() {
            skipped += 1;
            continue;
        }
        match AnimSequence::parse(&pkg) {
            Ok(s) => {
                ok += 1;
                let k = format!("{} trans={} rot={} scale={}", s.key_format.name(), s.translation_format.name(), s.rotation_format.name(), s.scale_format.name());
                *hist.entry(k).or_default() += 1;
                for n in &s.notifies {
                    *notify_names.entry(n.notify_class.clone().unwrap_or_else(|| n.notify_name.clone())).or_default() += 1;
                }
                if !s.curves.is_empty() {
                    curves += 1;
                }
                if s.tracks.iter().any(|t| !t.scale.is_empty()) {
                    with_scale += 1;
                }
                for t in &s.tracks {
                    let f = |c: &str, v: Option<(fn_anim::CompressionFormat, u8)>| match v {
                        Some((f, m)) => format!("{c} {} mask{:x}", f.name(), m & 7),
                        None => format!("{c} -"),
                    };
                    *track_hist.entry(f("pos", t.pos_format)).or_default() += 1;
                    *track_hist.entry(f("rot", t.rot_format)).or_default() += 1;
                    if let Some((f, m)) = t.scale_format {
                        let e = scale_stats.entry(format!("{} additive={} {}", f.name(), s.is_additive(), m & 7)).or_insert((f32::MAX, f32::MIN, 0));
                        for k in &t.scale {
                            for c in k {
                                e.0 = e.0.min(*c);
                                e.1 = e.1.max(*c);
                            }
                        }
                        e.2 += t.scale.len();
                    }
                    if t.scale_format.is_some() {
                        *track_hist.entry(f("scale", t.scale_format)).or_default() += 1;
                    }
                    let kk = |c: &str, n: usize, times: &[u32]| format!("{c} keys={} {}", if n <= 1 { n.to_string() } else if n == s.num_frames { "NumFrames".to_string() } else { "other".to_string() }, if times.is_empty() { "no-times" } else { "times" });
                    *key_hist.entry(kk("pos", t.pos.len(), &t.pos_times)).or_default() += 1;
                    *key_hist.entry(kk("rot", t.rot.len(), &t.rot_times)).or_default() += 1;
                    if !t.scale.is_empty() {
                        *key_hist.entry(kk("scale", t.scale.len(), &t.scale_times)).or_default() += 1;
                    }
                }
                if s.from_raw {
                    raw += 1;
                }
                if verbose {
                    println!("{} frames={} len={} tracks={} bytes={} add={:?}/{:?} retarget={} notifies={} curves={}", s.name, s.num_frames, s.length, s.tracks.len(), s.compressed_bytes, s.additive.kind, s.additive.base, s.retarget_source, s.notifies.len(), s.curves.len());
                }
            }
            Err(e) => {
                fail += 1;
                println!("FAIL {}: {e}", f.display());
            }
        }
    }
    println!("parsed {ok}, failed {fail}, non-anim {skipped}; with curves {curves}, with scale keys {with_scale}, raw {raw}");
    for (k, v) in hist {
        println!("{v:5}  {k}");
    }
    println!("per-track formats:");
    for (k, v) in track_hist {
        println!("{v:8}  {k}");
    }
    println!("scale value ranges:");
    for (k, v) in scale_stats {
        println!("  {k}: min {} max {} keys {}", v.0, v.1, v.2);
    }
    println!("key layouts:");
    for (k, v) in key_hist {
        println!("{v:8}  {k}");
    }
    println!("notify classes: {notify_names:?}");
}
