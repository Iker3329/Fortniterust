//! `fn-export`: pure-Rust replacement for `tools/export-assets.cjs` + UEViewer.
//!
//! Reads the (read-only) Fortnite 1.8 pak set with `fn-pak`, parses packages with `fn-ueio`, decodes with
//! `fn-texture` / `fn-mesh` / `fn-anim` and writes the new runtime asset tree:
//!
//! | kind       | output (relative to `--out`)                                   |
//! |------------|-----------------------------------------------------------------|
//! | textures   | the manifest `out` path (`exported/Game/.../X.png`), RGBA8 PNG  |
//! | aliases    | the manifest `out` and `alias` paths (identical PNG bytes)       |
//! | meshes     | `exported/<package>.gltf` + `.bin` (glTF 2.0, old convention)    |
//! | materials  | `exported/<material package>.mat.json` for every mesh material   |
//! | animations | the manifest `out` path with `.fnanim` instead of `.md5anim` (`--md5` also writes the old text format) |
//!
//! Usage: `fn-export --paks <Paks dir> --manifest <asset-manifest.json> --out <assets dir>
//!         [textures|meshes|animations|materials|all] [--jobs N] [--only substring] [--md5] [--verify <old assets dir>]`
//!
//! A `reports/export-assets.json` is written next to `--out` (parent dir `reports/`) with the per-asset status; the
//! exit code is non-zero if any non-optional asset failed.
mod anim_stage;
mod br_character;
mod mat;
mod mesh_stage;
mod props;
mod source;
mod tex_stage;

use serde_json::{json, Value};
use source::Source;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

pub struct Args {
    pub paks: PathBuf,
    pub manifest: PathBuf,
    pub out: PathBuf,
    pub what: String,
    pub jobs: usize,
    pub only: Option<String>,
    pub md5: bool,
    pub verify: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        paks: PathBuf::from("C:/Users/Brady/Downloads/1.8/Fortnite/Fortnite/FortniteGame/Content/Paks"),
        manifest: PathBuf::new(),
        out: PathBuf::from("assets"),
        what: "all".into(),
        jobs: std::thread::available_parallelism().map_or(4, |n| n.get()).min(8),
        only: None,
        md5: false,
        verify: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match x.as_str() {
            "--paks" => a.paks = PathBuf::from(val("--paks")?),
            "--manifest" => a.manifest = PathBuf::from(val("--manifest")?),
            "--out" => a.out = PathBuf::from(val("--out")?),
            "--jobs" => a.jobs = val("--jobs")?.parse().map_err(|_| "bad --jobs")?,
            "--only" => a.only = Some(val("--only")?.to_lowercase()),
            "--verify" => a.verify = Some(PathBuf::from(val("--verify")?)),
            "--md5" => a.md5 = true,
            "textures" | "meshes" | "animations" | "materials" | "all" => a.what = x,
            "-h" | "--help" => return Err("usage: fn-export --paks <Paks> --manifest <asset-manifest.json> --out <assets> [textures|meshes|animations|materials|all] [--jobs N] [--only s] [--md5] [--verify <old assets>]".into()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if a.manifest.as_os_str().is_empty() {
        return Err("--manifest is required".into());
    }
    Ok(a)
}

/// Run `f` over `items` on `jobs` threads, results in item order.
pub fn parallel<T: Sync, R: Send>(items: &[T], jobs: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= items.len() {
                    break;
                }
                let r = f(&items[i]);
                results.lock().unwrap()[i] = Some(r);
            });
        }
    });
    results.into_inner().unwrap().into_iter().map(|r| r.unwrap()).collect()
}

pub fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("props") {
        std::process::exit(props::main(&raw[1..]));
    }
    if raw.first().map(String::as_str) == Some("br-character") {
        std::process::exit(br_character::main(&raw[1..]));
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let t0 = Instant::now();
    let manifest: Value = match std::fs::read_to_string(&args.manifest).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string())) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("cannot read manifest {}: {e}", args.manifest.display());
            std::process::exit(2);
        }
    };
    let src = match Source::open(&args.paks) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    println!("opened {} paks, {} entries ({:.1}s)", src.set.paks().len(), src.set.len(), t0.elapsed().as_secs_f32());
    let mut report = serde_json::Map::new();
    let mut failures = 0usize;
    let want = |k: &str| args.what == "all" || args.what == k;
    if want("textures") {
        let r = tex_stage::run(&src, &manifest, &args);
        failures += r.failures;
        report.insert("textures".into(), Value::Array(r.entries));
    }
    if want("meshes") || want("materials") {
        let r = mesh_stage::run(&src, &manifest, &args);
        failures += r.failures;
        report.insert("meshes".into(), Value::Array(r.entries));
    }
    if want("animations") {
        let r = anim_stage::run(&src, &manifest, &args);
        failures += r.failures;
        report.insert("animations".into(), Value::Array(r.entries));
    }
    report.insert("failures".into(), json!(failures));
    report.insert("seconds".into(), json!(t0.elapsed().as_secs_f32()));
    let rep_path = args.out.parent().map(|p| p.join("reports")).unwrap_or_else(|| PathBuf::from("reports")).join("export-assets.json");
    if let Err(e) = write_file(&rep_path, serde_json::to_string_pretty(&Value::Object(report)).unwrap().as_bytes()) {
        eprintln!("report: {e}");
    }
    println!("done in {:.1}s, {failures} failures; report {}", t0.elapsed().as_secs_f32(), rep_path.display());
    if failures > 0 {
        std::process::exit(1);
    }
}

/// Outcome of one stage.
pub struct Stage {
    pub entries: Vec<Value>,
    pub failures: usize,
}

pub fn entry(kind: &str, pkg: &str, out: &str, res: &Result<Value, String>, ms: u128) -> Value {
    match res {
        Ok(d) => json!({"kind": kind, "pkg": pkg, "out": out, "ok": true, "ms": ms, "detail": d}),
        Err(e) => json!({"kind": kind, "pkg": pkg, "out": out, "ok": false, "ms": ms, "error": e}),
    }
}
