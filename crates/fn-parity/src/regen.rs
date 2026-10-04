//! `fn-parity regen-old-logs` (the former tools\regen-old-logs.ps1): regenerates the golden logs in testdata\old-logs (per-frame mode, FNRT_SIM_HZ=0)
//! and testdata\old-logs-30hz (the default fixed 30 Hz tick) from the OLD prototype executable, built from a SNAPSHOT COPY of ..\FortniteRuntime (the
//! old tree itself is never touched and the Fortnite client is never run).
//!
//!   fn-parity regen-old-logs [--snapshot DIR] [--build] [--exe PATH] [--only mode[,mode]]
//!
//!   --snapshot  directory holding src\, Cargo.toml, Cargo.lock and junctions reports\, assets\, verification\ (default: %TEMP%\fn-snap-resync2; created from the old tree when missing)
//!   --build     `cargo build --release` the snapshot first (CARGO_TARGET_DIR = %TEMP%\cargo-old-resync2)
//!   --exe       a prebuilt old exe (default: <target dir>\release\fortnite-runtime.exe)
//! Window modes (--camera-test, --camera-video, --camera-prop-test, --pickaxe-test, --dir-test) read the real keyboard while their window is foreground, so every
//! run is repeated and a golden is only kept when two consecutive runs are byte-identical (README of testdata\old-logs). Clean verification folder per run
//! (dir-frames.txt is appended to by the exe). FNRT_* variables are cleared first (FNRT_GRAVITY_SCALE / FNRT_JUMPZ would change the physics).
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Mode { n: &'static str, a: &'static [&'static str], f: &'static [&'static str], hz: bool, also: Option<(&'static [&'static str], &'static [&'static str])> }

// name, flags, files, per-hz (the tick-sensitive modes are produced for both rates)
const MODES: &[Mode] = &[
    Mode { n: "movement-test", a: &["--movement-test"], f: &["movement-test.txt"], hz: false, also: None },
    Mode { n: "jump-air-test", a: &["--jump-air-test"], f: &["jump-air-test.txt"], hz: false, also: None },
    Mode { n: "jump-feel-test", a: &["--jump-feel-test"], f: &["jump-feel-test.txt"], hz: false, also: None },
    Mode { n: "sim-tick-test", a: &["--sim-tick-test"], f: &["sim-tick-test.txt"], hz: false, also: None },
    Mode { n: "camera-prop-test", a: &["--camera-prop-test"], f: &["camprop-after.txt"], hz: false, also: Some((&["--camera-prop-test", "--no-prop-collision"], &["camprop-before.txt"])) },
    Mode { n: "camera-video", a: &["--camera-video"], f: &["camera-video-v2.txt"], hz: false, also: None },
    Mode { n: "face-test", a: &["--face-test"], f: &["face-test.txt"], hz: false, also: Some((&["--face-test", "--legacy"], &["face-test-legacy.txt"])) },
    Mode { n: "camera-test", a: &["--camera-test"], f: &["camera-test.txt"], hz: true, also: None },
    Mode { n: "pickaxe-test", a: &["--pickaxe-test"], f: &["pickaxe-test.txt", "pickaxe-frames.txt"], hz: true, also: None },
    Mode { n: "dir-test", a: &["--dir-test"], f: &["pickaxe-test.txt", "pickaxe-frames.txt", "dir-frames.txt"], hz: true, also: None },
];

fn junction(link: &Path, target: &Path) -> Result<(), String> {
    let st = Command::new("cmd").args(["/c", "mklink", "/J"]).arg(link).arg(target).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| e.to_string())?;
    if st.success() { Ok(()) } else { Err(format!("mklink /J {} -> {} failed", link.display(), target.display())) }
}

/// Run the old exe once in a clean verification folder (600 s limit).
fn invoke_old(run: &Path, ver: &Path, flags: &[&str], sim_hz0: bool) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(ver);
    std::fs::create_dir_all(ver).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(run.join("FortniteRuntime.exe"));
    cmd.args(flags).current_dir(run);
    if sim_hz0 { cmd.env("FNRT_SIM_HZ", "0"); }
    let mut child = cmd.spawn().map_err(|e| format!("cannot start the old exe: {e}"))?;
    let t0 = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(s) => { if !s.success() { eprintln!("warning: exit code {:?} for {}", s.code(), flags.join(" ")); } return Ok(()); }
            None => { if t0.elapsed() > Duration::from_secs(600) { let _ = child.kill(); return Err(format!("timeout: {}", flags.join(" "))); } std::thread::sleep(Duration::from_millis(200)); }
        }
    }
}

fn read_text(p: &Path) -> Result<String, String> {
    let b = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let s = String::from_utf8_lossy(&b).into_owned();
    Ok(s.strip_prefix('\u{feff}').map(str::to_owned).unwrap_or(s))
}

fn collect(run: &Path, ver: &Path, flags: &[&str], files: &[&str], dest: &Path, sim_hz0: bool) -> Result<(), String> {
    let mut texts: Vec<Vec<String>> = vec![];
    for _ in 0..2 {
        invoke_old(run, ver, flags, sim_hz0)?;
        texts.push(files.iter().map(|f| read_text(&ver.join(f))).collect::<Result<_, _>>()?);
    }
    for (k, f) in files.iter().enumerate() {
        if texts[0][k] != texts[1][k] { return Err(format!("two runs of '{}' differ in {f}; re-run (a stray key press or a focus change reaches the window modes)", flags.join(" "))); }
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for (k, f) in files.iter().enumerate() { std::fs::write(dest.join(f), &texts[0][k]).map_err(|e| e.to_string())?; }
    println!("{:<18} {} -> {}", flags.join(" "), files.join(","), dest.display());
    Ok(())
}

pub fn run(args: &[String], ws: &Path) -> Result<(), String> {
    let arg = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1).cloned());
    let temp = std::env::temp_dir();
    let snapshot = arg("--snapshot").map(PathBuf::from).unwrap_or_else(|| temp.join("fn-snap-resync2"));
    let only: Option<Vec<String>> = arg("--only").map(|s| s.split(',').map(str::to_owned).collect());
    let old_tree = ws.parent().ok_or("workspace has no parent folder")?.join("FortniteRuntime");
    let target = temp.join("cargo-old-resync2");
    if !snapshot.join("src").exists() {
        std::fs::create_dir_all(&snapshot).map_err(|e| e.to_string())?;
        if !crate::exec::mirror_dir(&old_tree.join("src"), &snapshot.join("src")) { return Err("robocopy of the old src failed".into()); }
        for f in ["Cargo.toml", "Cargo.lock"] { std::fs::copy(old_tree.join(f), snapshot.join(f)).map_err(|e| format!("{f}: {e}"))?; }
        for j in ["reports", "assets", "verification"] { junction(&snapshot.join(j), &old_tree.join(j))?; }
    }
    if args.iter().any(|a| a == "--build") {
        let st = Command::new("cargo").args(["build", "--release"]).current_dir(&snapshot).env("CARGO_TARGET_DIR", &target).status().map_err(|e| e.to_string())?;
        if !st.success() { return Err("old build failed".into()); }
    }
    let exe = arg("--exe").map(PathBuf::from).unwrap_or_else(|| target.join("release").join("fortnite-runtime.exe"));
    let fnrt: Vec<String> = std::env::vars().map(|(k, _)| k).filter(|k| k.to_uppercase().starts_with("FNRT_")).collect();
    for k in fnrt { std::env::remove_var(k); }
    let run_dir = temp.join("fn-oldrun");
    let _ = std::fs::remove_dir_all(&run_dir);
    std::fs::create_dir_all(&run_dir).map_err(|e| e.to_string())?;
    std::fs::copy(&exe, run_dir.join("FortniteRuntime.exe")).map_err(|e| format!("{}: {e}", exe.display()))?;
    junction(&run_dir.join("assets"), &old_tree.join("assets"))?;
    let ver = run_dir.join("verification");
    for m in MODES {
        if only.as_ref().map_or(false, |o| !o.iter().any(|x| x == m.n)) { continue; }
        let mut variants: Vec<(&[&str], &[&str])> = vec![(m.a, m.f)];
        if let Some(a) = m.also { variants.push(a); }
        for (a, f) in variants {
            if m.hz {
                collect(&run_dir, &ver, a, f, &ws.join("testdata").join("old-logs").join(m.n), true)?;
                collect(&run_dir, &ver, a, f, &ws.join("testdata").join("old-logs-30hz").join(m.n), false)?;
            } else { collect(&run_dir, &ver, a, f, &ws.join("testdata").join("old-logs").join(m.n), false)?; }
        }
    }
    println!("regen-old-logs: done");
    Ok(())
}
