//! fn-parity CLI.
//!
//!   fn-parity run --all [--out parity] [--thresholds FILE] [--no-build] [--skip-old] [--skip-new] [--reuse] [--quick] [--baseline]
//!                       [--work DIR (default %TEMP%\fn-parity-work)] [--old-tree DIR] [--old-exe EXE | --old-build] [--scratch DIR] [--target-dir DIR] [--only smoke,pickaxe,dir,light,sky,sim,ui]
//!   fn-parity dashboard [--baseline] [--reuse] [--no-build] [--quick] [--clean] [run options]   (the former tools\parity.ps1: `run --all` + prints the report path;\r\n//!                       --reuse = report, --clean = clean).  From the workspace: `cargo run --release -p fn-parity -- dashboard`\r\n//!   fn-parity regen-old-logs [--snapshot DIR] [--build] [--exe PATH] [--only m1,m2]   regenerate testdata\old-logs* from the old exe (see src/regen.rs)
//!   fn-parity report            re-compare the outputs of the last run (same as run --reuse)
//!   fn-parity clean             release a leaked cursor clip (no processes are left behind by `run`; this is the manual safety net)
//!   fn-parity compare-images A B [sheet.png]      MAE / SSIM of two captures (+ old | new | heatmap sheet)
//!   fn-parity png IN OUT                          convert a capture (BMP) to PNG
use fn_parity::{analyze, config::Config, exec::{Ctx, Opts}, img, model::RunRecord, report, run};
use std::path::{Path, PathBuf};

fn arg(args: &[String], name: &str) -> Option<String> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()) }
fn flag(args: &[String], name: &str) -> bool { args.iter().any(|a| a == name) }

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("run") => cmd_run(&args[1..], false),
        Some("report") => cmd_run(&args[1..], true),
        Some("dashboard") => cmd_dashboard(&args[1..]),
        Some("regen-old-logs") => {
            let root = match run::find_root(&std::env::current_dir().unwrap_or_default()).or_else(|| std::env::current_exe().ok().and_then(|e| run::find_root(&e))) { Some(r) => r, None => { eprintln!("cannot find the workspace root (run from the repository)"); return std::process::exit(2); } };
            match fn_parity::regen::run(&args[1..], &root) { Ok(()) => 0, Err(e) => { eprintln!("error: {e}"); 1 } }
        }
        Some("clean") => cmd_clean(&args[1..]),
        Some("png") if args.len() >= 3 => { match img::load(Path::new(&args[1])).and_then(|i| img::save_png(Path::new(&args[2]), &i)) { Ok(()) => 0, Err(e) => { eprintln!("{e}"); 1 } } }
        Some("compare-images") if args.len() >= 3 => {
            match (img::load(Path::new(&args[1])), img::load(Path::new(&args[2]))) {
                (Ok(a), Ok(b)) => {
                    let (a, b, cropped) = img::align(&a, &b);
                    let m = img::metrics(&a, &b, img::Rect::FULL);
                    println!("{}x{} cropped={cropped} MAE rgb {:.2}/{:.2}/{:.2} (all {:.3}) SSIM {:.4} structure {:.4} bad>16 {:.2}%", a.w, a.h, m.mae[0], m.mae[1], m.mae[2], m.mae_all, m.ssim, m.structure, m.bad_pct);
                    if let Some(out) = args.get(3) { let h = img::heatmap(&a, &b); if let Err(e) = img::save_png(Path::new(out), &img::side_by_side(&a, &b, &h, 640)) { eprintln!("{e}"); } }
                    0
                }
                (a, b) => { eprintln!("{:?} {:?}", a.err(), b.err()); 1 }
            }
        }
        _ => { eprintln!("usage: fn-parity run --all | dashboard | report | clean | compare-images A B [sheet.png] | png IN OUT   (see crates/fn-parity/src/main.rs)"); 2 }
    };
    std::process::exit(code);
}

fn local_appdata_temp() -> PathBuf { std::env::temp_dir() }

fn opts_from(args: &[String]) -> Result<Opts, String> {
    let root = match arg(args, "--root").map(PathBuf::from) {
        Some(r) => r,
        None => run::find_root(&std::env::current_dir().map_err(|e| e.to_string())?).or_else(|| std::env::current_exe().ok().and_then(|e| run::find_root(&e))).ok_or("cannot find the workspace root (run from the repository, or pass --root)")?,
    };
    let parity = arg(args, "--out").map(PathBuf::from).unwrap_or_else(|| root.join("parity"));
    let old_tree = arg(args, "--old-tree").map(PathBuf::from).or_else(|| std::env::var_os("FN_OLD_ROOT").map(PathBuf::from)).unwrap_or_else(|| root.join("..").join("FortniteRuntime"));
    let old_tree = old_tree.canonicalize().map(|p| PathBuf::from(p.display().to_string().trim_start_matches(r"\\?\"))).unwrap_or(old_tree);
    Ok(Opts {
        thresholds: arg(args, "--thresholds").map(PathBuf::from).unwrap_or_else(|| parity.join("thresholds.toml")),
        scratch_old: arg(args, "--scratch").map(PathBuf::from).unwrap_or_else(|| local_appdata_temp().join("parity-old")),
        target_dir: arg(args, "--target-dir").map(PathBuf::from).or_else(|| std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from)).unwrap_or_else(|| local_appdata_temp().join("cargo-rs-parity")),
        old_exe: arg(args, "--old-exe").map(PathBuf::from), old_build: flag(args, "--old-build"), no_build: flag(args, "--no-build"), skip_old: flag(args, "--skip-old"), skip_new: flag(args, "--skip-new"), reuse: false, quick: flag(args, "--quick"),
        only: arg(args, "--only").map(|s| s.split(',').map(str::to_owned).collect()),
        work: arg(args, "--work").map(PathBuf::from).unwrap_or_else(|| local_appdata_temp().join("fn-parity-work")),
        root, parity, old_tree,
    })
}

/// `tools\parity.ps1` as a subcommand: full run (or `--reuse` re-compare, or `--clean`), then the path of the dashboard.
fn cmd_dashboard(args: &[String]) -> i32 {
    if flag(args, "--clean") { return cmd_clean(args); }
    let code = cmd_run(args, flag(args, "--reuse"));
    if let Ok(o) = opts_from(args) { let report = o.parity.join("report.html"); if report.exists() { println!("\nopen: {}", report.display()); } }
    code
}

fn cmd_clean(args: &[String]) -> i32 {
    let o = match opts_from(args) { Ok(o) => o, Err(e) => { eprintln!("{e}"); return 2 } };
    let mut ctx = Ctx::new(o, Config::defaults());
    let _ = std::fs::create_dir_all(&ctx.work);
    println!("cursor clip released: {}", ctx.release_cursor());
    0
}

struct Lock(PathBuf);
impl Drop for Lock { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }
fn take_lock(work: &Path) -> Result<Lock, String> {
    let p = work.join(".lock");
    if let Ok(t) = std::fs::read_to_string(&p) {
        if let Ok(pid) = t.trim().parse::<u32>() {
            let out = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output().ok();
            if out.map_or(false, |o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\""))) { return Err(format!("another fn-parity run (pid {pid}) is active; wait for it or delete {}", p.display())); }
        }
    }
    std::fs::write(&p, std::process::id().to_string()).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(Lock(p))
}

fn exe_info(p: &Path) -> String {
    match std::fs::metadata(p) { Ok(m) => format!("{} ({} KB, {})", p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), m.len() / 1024, m.modified().ok().map(fmt_time).unwrap_or_default()), Err(_) => format!("{} (missing)", p.display()) }
}
fn fmt_time(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86400, secs % 86400);
    // civil from days (UTC)
    let z = days as i64 + 719468; let era = z.div_euclid(146097); let doe = z.rem_euclid(146097); let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400; let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); let mp = (5 * doy + 2) / 153; let d = doy - (153 * mp + 2) / 5 + 1; let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02} {:02}:{:02}Z", y + (m <= 2) as i64, m, d, rem / 3600, rem % 3600 / 60)
}

fn save_runs(ctx: &Ctx) {
    let v: Vec<serde_json::Value> = ctx.runs.iter().map(|x| serde_json::json!({"id": x.id, "side": x.side, "cmd": x.cmd, "exit": x.exit, "timed_out": x.timed_out, "wall_s": x.wall_s, "cpu_s": x.cpu_s, "peak_ws_mb": x.peak_ws_mb, "stragglers_killed": x.stragglers_killed, "cursor_released": x.cursor_released, "ok": x.ok, "note": x.note, "out_dir": x.out_dir})).collect();
    let _ = std::fs::write(ctx.work.join("runs.json"), serde_json::json!({"runs": v, "warnings": ctx.warnings}).to_string());
}
fn load_runs(ctx: &mut Ctx) {
    let Some(t) = std::fs::read_to_string(ctx.work.join("runs.json")).ok() else { return };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { return };
    for x in v["runs"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        ctx.runs.push(RunRecord { id: x["id"].as_str().unwrap_or("").into(), side: x["side"].as_str().unwrap_or("").into(), cmd: x["cmd"].as_str().unwrap_or("").into(), exit: x["exit"].as_i64().map(|e| e as i32), timed_out: x["timed_out"].as_bool().unwrap_or(false), wall_s: x["wall_s"].as_f64().unwrap_or(0.), cpu_s: x["cpu_s"].as_f64().unwrap_or(0.), peak_ws_mb: x["peak_ws_mb"].as_f64().unwrap_or(0.), stragglers_killed: x["stragglers_killed"].as_u64().unwrap_or(0) as u32, cursor_released: x["cursor_released"].as_bool().unwrap_or(false), ok: x["ok"].as_bool().unwrap_or(false), note: x["note"].as_str().unwrap_or("").into(), out_dir: x["out_dir"].as_str().unwrap_or("").into() });
    }
    for w in v["warnings"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) { if let Some(s) = w.as_str() { ctx.warnings.push(s.to_owned()); } }
}

fn cmd_run(args: &[String], reuse_cmd: bool) -> i32 {
    let mut o = match opts_from(args) { Ok(o) => o, Err(e) => { eprintln!("error: {e}"); return 2 } };
    o.reuse = reuse_cmd || flag(args, "--reuse");
    if let Err(e) = std::fs::create_dir_all(&o.work).and_then(|_| std::fs::create_dir_all(&o.parity)) { eprintln!("error: {}: {e}", o.parity.display()); return 2; }
    // thresholds: the user's file (created from the defaults on the first run)
    if !o.thresholds.exists() { let _ = std::fs::write(&o.thresholds, fn_parity::config::DEFAULT_TOML); }
    let cfg = match Config::load(&o.thresholds) { Ok(c) => c, Err(e) => { eprintln!("error: {e}"); return 2 } };
    let thresholds_toml = std::fs::read_to_string(&o.thresholds).unwrap_or_default();
    let mut ctx = Ctx::new(o, cfg);
    let _lock = match take_lock(&ctx.work) { Ok(l) => l, Err(e) => { eprintln!("error: {e}"); return 3 } };
    let t0 = std::time::Instant::now();
    if ctx.o.reuse { load_runs(&mut ctx); eprintln!("[reuse] re-comparing the outputs in {}", ctx.work.display()); }
    else {
        // partial runs (--skip-old / --skip-new / --only) keep the records and warnings of the earlier full run
        if ctx.o.skip_old || ctx.o.skip_new || ctx.o.only.is_some() { load_runs(&mut ctx); }
        run::stage_build(&mut ctx);
        run::stage_old(&mut ctx);
        run::stage_new(&mut ctx);
        // only after a child had to be killed (the helper already compares the clip before / after each child); never touch another app's clip otherwise
        if ctx.runs.iter().any(|r| r.timed_out || r.stragglers_killed > 0) && ctx.release_cursor() { ctx.warn("a cursor clip was still active after a killed child and has been released"); }
        save_runs(&ctx);
    }
    eprintln!("[compare] analysing...");
    let (items, crows, gaps) = analyze::analyze(&mut ctx);
    let generated = fmt_time(std::time::SystemTime::now());
    let rep = report::Report {
        generated, items: &items, runs: &ctx.runs, warnings: &ctx.warnings, gaps: &gaps, consts: &crows, cfg: &ctx.cfg, thresholds_toml,
        old_exe_info: old_exe_info(&ctx), new_exe_info: exe_info(&ctx.new_exe("fortnite-rs")),
    };
    let html = report::html(&rep);
    let json = report::summary_json(&rep);
    let (hp, jp) = (ctx.o.parity.join("report.html"), ctx.o.parity.join("summary.json"));
    if let Err(e) = std::fs::write(&hp, &html).and_then(|_| std::fs::write(&jp, serde_json::to_string_pretty(&json).unwrap_or_default())) { eprintln!("error writing the report: {e}"); return 1; }
    if flag(args, "--baseline") {
        let b = ctx.o.parity.join("baseline");
        let _ = std::fs::remove_dir_all(&b);
        let _ = std::fs::create_dir_all(&b);
        let _ = std::fs::copy(&hp, b.join("report.html")); let _ = std::fs::copy(&jp, b.join("summary.json")); let _ = std::fs::copy(&ctx.o.thresholds, b.join("thresholds.toml"));
        copy_dir(&ctx.evidence, &b.join("evidence"));
        println!("baseline recorded in {}", b.display());
    }
    // console summary
    println!("\nParity summary ({:.0}s):", t0.elapsed().as_secs_f64());
    for a in json["areas"].as_array().unwrap_or(&vec![]) {
        println!("  {:<12} {:>6}  {:<6} ({} scored / {} items)", a["area"].as_str().unwrap_or(""), a["score"].as_f64().map_or("n/a".to_owned(), |s| format!("{s:.1}")), a["grade"].as_str().unwrap_or(""), a["scored_items"], a["items"]);
        for w in a["worst"].as_array().unwrap_or(&vec![]).iter().take(2) { println!("      worst: {} [{}] {}", w["label"].as_str().unwrap_or(""), w["score"].as_f64().map_or("n/a".to_owned(), |s| format!("{s:.0}")), w["headline"].as_str().unwrap_or("")); }
    }
    println!("\nreport : {}\nsummary: {}", hp.display(), jp.display());
    0
}

fn copy_dir(from: &Path, to: &Path) {
    let _ = std::fs::create_dir_all(to);
    if let Ok(rd) = std::fs::read_dir(from) { for e in rd.flatten() { let (p, q) = (e.path(), to.join(e.file_name())); if p.is_dir() { copy_dir(&p, &q); } else { let _ = std::fs::copy(&p, &q); } } }
}

/// The reference exe as actually used: source path, size, build time and a content fingerprint of the scratch copy that was run.
fn old_exe_info(ctx: &Ctx) -> String {
    let j = std::fs::read_to_string(ctx.work.join("old-exe-source.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    match j {
        Some(j) => format!("{} ({} KB, built {}, fnv64 {}; run from a scratch copy)", j["source"].as_str().unwrap_or("?"), j["size"].as_u64().unwrap_or(0) / 1024, j["source_mtime_unix"].as_u64().map(|s| fmt_time(std::time::UNIX_EPOCH + std::time::Duration::from_secs(s))).unwrap_or_default(), j["scratch_fnv64"].as_str().unwrap_or("?")),
        None => format!("{} (scratch copy)", exe_info(&ctx.old_exe())),
    }
}