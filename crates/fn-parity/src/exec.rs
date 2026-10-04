//! Process execution with timeouts, measurement and cleanup. Every child runs through [`crate::winproc::run`]: no input injection, the child
//! is killed (whole tree) on timeout, stragglers of the same executable are killed, a leaked ClipCursor is released.
use crate::config::Config;
use crate::model::RunRecord;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Opts {
    pub root: PathBuf,
    pub parity: PathBuf,
    /// raw outputs of both executables (hundreds of MB of BMP captures): outside the repository by default
    pub work: PathBuf,
    pub thresholds: PathBuf,
    pub old_tree: PathBuf,
    pub scratch_old: PathBuf,
    pub target_dir: PathBuf,
    /// Use this exe as the reference instead of `<old tree>\FortniteRuntime.exe` (it is copied to the scratch folder either way).
    pub old_exe: Option<PathBuf>,
    /// Build the reference from a snapshot copy of the old tree's sources (scratch folder, separate target dir) instead of using its stale exe.
    pub old_build: bool,
    pub no_build: bool,
    pub skip_old: bool,
    pub skip_new: bool,
    /// Reuse the outputs of the last run in parity/work instead of running the executables again (re-compare only).
    pub reuse: bool,
    pub only: Option<Vec<String>>,
    pub quick: bool,
}

pub struct Ctx {
    pub o: Opts,
    pub cfg: Config,
    pub runs: Vec<RunRecord>,
    pub warnings: Vec<String>,
    pub work: PathBuf,
    pub evidence: PathBuf,
}

impl Ctx {
    pub fn new(o: Opts, cfg: Config) -> Ctx {
        let work = o.work.clone(); let evidence = o.parity.join("evidence");
        Ctx { o, cfg, runs: vec![], warnings: vec![], work, evidence }
    }
    pub fn warn(&mut self, w: impl Into<String>) { let w = w.into(); eprintln!("warning: {w}"); if !self.warnings.contains(&w) { self.warnings.push(w); } }
    /// A run replaces an earlier record of the same side + id (partial re-runs keep the other records).
    pub fn record(&mut self, rec: RunRecord) { self.runs.retain(|r| !(r.side == rec.side && r.id == rec.id)); self.runs.push(rec); }
    pub fn wanted(&self, group: &str) -> bool { self.o.only.as_ref().map_or(true, |v| v.iter().any(|g| g.eq_ignore_ascii_case(group))) }
    pub fn scale(&self) -> f64 { self.cfg.f("run.timeout_scale", 1.0).max(0.1) * if self.o.quick { 0.5 } else { 1. } }
    pub fn old_exe(&self) -> PathBuf { self.o.scratch_old.join("FortniteRuntime.exe") }
    pub fn new_exe(&self, name: &str) -> PathBuf { self.o.target_dir.join("release").join(format!("{name}.exe")) }
    pub fn work_old(&self, mode: &str) -> PathBuf { self.work.join("old").join(mode) }
    pub fn work_new(&self, what: &str) -> PathBuf { self.work.join("new").join(what) }

    /// Run one process and record it. `env_unset` entries may end in `*`. Output of the child is saved to `work/logs/<id>.log`.
    pub fn run_proc(&mut self, id: &str, side: &str, exe: &Path, args: &[String], cwd: &Path, timeout_s: f64, env_set: &[(&str, &str)], env_unset: &[&str], out_dir: &Path) -> RunRecord {
        let mut rec = RunRecord { id: id.to_owned(), side: side.to_owned(), cmd: format!("{} {}", exe.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), args.join(" ")), out_dir: out_dir.display().to_string(), ..Default::default() };
        if !exe.exists() { rec.note = format!("executable not found: {}", exe.display()); eprintln!("[{side}] {id}: {}", rec.note); self.record(rec.clone()); return rec; }
        let logdir = self.work.join("logs"); let _ = std::fs::create_dir_all(&logdir);
        let logp = logdir.join(format!("{}-{}.log", side, id.replace(['/', '\\', ':'], "_")));
        let log = std::fs::File::create(&logp).ok();
        let timeout_s = (timeout_s * self.scale()).max(5.);
        eprintln!("[{side}] {id}: {} (timeout {:.0}s)", rec.cmd, timeout_s);
        let (out, err) = match log { Some(f) => { let f2 = f.try_clone().ok(); (Stdio::from(f), f2.map_or_else(Stdio::null, Stdio::from)) } None => (Stdio::null(), Stdio::null()) };
        let r = crate::winproc::run(exe, args, cwd, timeout_s as u64, env_set, env_unset, out, err);
        rec.exit = r.exit; rec.timed_out = r.timed_out; rec.wall_s = r.wall_s; rec.cpu_s = r.cpu_s; rec.peak_ws_mb = r.peak_ws_mb;
        rec.stragglers_killed = r.stragglers_killed; rec.cursor_released = r.cursor_clip_released;
        if let Some(e) = r.error { rec.note = e; }
        rec.ok = rec.exit == Some(0) && !rec.timed_out;
        if rec.timed_out { rec.note = format!("{} TIMED OUT after {:.0}s and was killed", rec.note, timeout_s).trim().to_owned(); }
        eprintln!("[{side}] {id}: exit {:?} in {:.1}s{}{}", rec.exit, rec.wall_s, if rec.timed_out { " (timeout)" } else { "" }, if rec.stragglers_killed > 0 { " (stragglers killed)" } else { "" });
        self.record(rec.clone());
        rec
    }

    /// Release a leaked ClipCursor (checked once more at the end of every full run).
    pub fn release_cursor(&mut self) -> bool { crate::winproc::release_cursor_clip() }

    /// `cargo build --release -p ...`; returns (ok, last lines of output). Never fails the run: the caller falls back to the existing binaries.
    pub fn cargo_build(&mut self, pkgs: &[&str], timeout_s: u64) -> (bool, String) {
        let (cwd, target) = (self.o.root.clone(), self.o.target_dir.clone());
        self.cargo_build_in(&cwd, &target, pkgs, timeout_s)
    }

    /// Same in an arbitrary workspace / target dir (`pkgs` empty = the package in `cwd`).
    pub fn cargo_build_in(&mut self, cwd: &Path, target: &Path, pkgs: &[&str], timeout_s: u64) -> (bool, String) {
        let logdir = self.work.join("logs"); let _ = std::fs::create_dir_all(&logdir);
        let tag = if pkgs.is_empty() { cwd.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() } else { pkgs.join("+") };
        let logp = logdir.join(format!("build-{tag}.log"));
        let pkgs: Vec<&str> = pkgs.to_vec();
        let f = std::fs::File::create(&logp).ok();
        let mut cmd = Command::new("cargo");
        cmd.arg("build").arg("--release");
        for p in &pkgs { cmd.arg("-p").arg(p); }
        cmd.current_dir(cwd).env("CARGO_TARGET_DIR", target).env("CARGO_TERM_COLOR", "never").stdin(Stdio::null());
        match f { Some(f) => { let f2 = f.try_clone().ok(); cmd.stdout(Stdio::from(f)); if let Some(f2) = f2 { cmd.stderr(Stdio::from(f2)); } } None => { cmd.stdout(Stdio::null()).stderr(Stdio::null()); } }
        eprintln!("[build] cargo build --release {} {}", pkgs.iter().map(|p| format!("-p {p}")).collect::<Vec<_>>().join(" "), if pkgs.is_empty() { cwd.display().to_string() } else { String::new() });
        let t0 = Instant::now();
        let mut child = match cmd.spawn() { Ok(c) => c, Err(e) => return (false, format!("cannot start cargo: {e}")) };
        let ok = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s.success(),
                Ok(None) => { if t0.elapsed().as_secs() > timeout_s { let _ = Command::new("taskkill").args(["/PID", &child.id().to_string(), "/T", "/F"]).output(); let _ = child.wait(); break false; } std::thread::sleep(Duration::from_millis(250)); }
                Err(_) => break false,
            }
        };
        let tail: String = std::fs::read_to_string(&logp).unwrap_or_default().lines().rev().take(25).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        let err_lines: String = tail.lines().filter(|l| l.starts_with("error")).take(4).collect::<Vec<_>>().join(" | ");
        let mut rec = RunRecord { id: format!("build:{tag}"), side: "build".into(), cmd: if pkgs.is_empty() { format!("cargo build --release (in {})", cwd.display()) } else { format!("cargo build --release -p {}", pkgs.join(" -p ")) }, wall_s: t0.elapsed().as_secs_f64(), ok, exit: Some(if ok { 0 } else { 1 }), ..Default::default() };
        if !ok { rec.note = err_lines.clone(); }
        self.record(rec);
        (ok, err_lines)
    }
}

/// Copy `from` over `to` only when the bytes differ (idempotent preparation of the scratch copy; compared by content because other agents
/// rebuild the old exe in place and a newer scratch copy must never mask a different source).
pub fn sync_file(from: &Path, to: &Path) -> Result<bool, String> {
    let src = std::fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
    if std::fs::read(to).map_or(false, |d| d == src) { return Ok(false); }
    if let Some(d) = to.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
    std::fs::write(to, &src).map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))?;
    Ok(true)
}

/// FNV-1a 64 of a file's bytes as 16 hex digits (identifies the reference exe in the report; not a security hash).
pub fn fingerprint(p: &Path) -> String {
    let Ok(b) = std::fs::read(p) else { return "unreadable".into() };
    let mut h: u64 = 0xcbf29ce484222325;
    for x in b { h ^= x as u64; h = h.wrapping_mul(0x100000001b3); }
    format!("{h:016x}")
}
/// Mirror a folder with robocopy (copies only new/changed files, never deletes in the source). Returns false when robocopy reports a real error (>= 8).
pub fn mirror_dir(from: &Path, to: &Path) -> bool {
    let st = Command::new("robocopy").arg(from).arg(to).args(["/E", "/XO", "/NFL", "/NDL", "/NJH", "/NJS", "/NP", "/MT:8", "/R:1", "/W:1"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    matches!(st, Ok(s) if s.code().map_or(false, |c| c < 8))
}

/// Exact mirror (also deletes files that no longer exist in the source) of a folder into a scratch location.
pub fn mirror_dir_purge(from: &Path, to: &Path) -> bool {
    let st = Command::new("robocopy").arg(from).arg(to).args(["/MIR", "/NFL", "/NDL", "/NJH", "/NJS", "/NP", "/MT:8", "/R:1", "/W:1"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    matches!(st, Ok(s) if s.code().map_or(false, |c| c < 8))
}
