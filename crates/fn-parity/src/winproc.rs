//! Process helper (replaces the former `ps/exec.ps1`): runs ONE executable with a timeout, measures it (wall / CPU / peak working set /
//! peak private bytes), kills stragglers of the same executable and releases a leaked cursor clip. No input injection of any kind; the
//! child only gets its own command line and the stdout/stderr handed in by the caller. Win32 is reached through a handful of raw
//! `extern "system"` declarations (no extra crates); this is the only module of the crate that uses `unsafe`.
#![allow(unsafe_code)]
use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

type Handle = *mut c_void;

#[repr(C)] #[derive(Default, Clone, Copy)] struct Rect { l: i32, t: i32, r: i32, b: i32 }
#[repr(C)] #[derive(Default, Clone, Copy)] struct FileTime { lo: u32, hi: u32 }
impl FileTime { fn v(self) -> u64 { ((self.hi as u64) << 32) | self.lo as u64 } }
#[repr(C)] #[derive(Default)] struct MemCounters { cb: u32, page_faults: u32, peak_ws: usize, ws: usize, qpp: usize, qp: usize, qpnp: usize, qnp: usize, pagefile: usize, peak_pagefile: usize, private: usize }
#[repr(C)] struct ProcEntry { size: u32, usage: u32, pid: u32, heap: usize, module: u32, threads: u32, parent: u32, pri: i32, flags: u32, exe: [u16; 260] }

#[link(name = "user32")]
extern "system" {
    fn GetClipCursor(r: *mut Rect) -> i32;
    fn ClipCursor(r: *const Rect) -> i32;
    fn GetSystemMetrics(i: i32) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn K32GetProcessMemoryInfo(h: Handle, c: *mut MemCounters, cb: u32) -> i32;
    fn GetProcessTimes(h: Handle, create: *mut FileTime, exit: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
    fn GetSystemTimeAsFileTime(t: *mut FileTime);
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Process32FirstW(s: Handle, e: *mut ProcEntry) -> i32;
    fn Process32NextW(s: Handle, e: *mut ProcEntry) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn QueryFullProcessImageNameW(h: Handle, flags: u32, buf: *mut u16, len: *mut u32) -> i32;
    fn CloseHandle(h: Handle) -> i32;
}

const INVALID_HANDLE: isize = -1;

/// Release a ClipCursor that a killed / crashed child left behind. Only acts when the active clip is smaller than the virtual screen
/// (never touches the default, unclipped state). Returns true when a clip was released.
pub fn release_cursor_clip() -> bool {
    unsafe {
        let mut r = Rect::default();
        GetClipCursor(&mut r);
        let (vw, vh) = (GetSystemMetrics(78), GetSystemMetrics(79)); // SM_CXVIRTUALSCREEN / SM_CYVIRTUALSCREEN
        let (w, h) = (r.r - r.l, r.b - r.t);
        if w > 0 && h > 0 && (w < vw || h < vh) { ClipCursor(std::ptr::null()); return true; }
    }
    false
}

/// Measurement of one child (the former `exec.ps1` result JSON).
#[derive(Clone, Debug, Default)]
pub struct ProcResult {
    pub started: bool,
    pub exit: Option<i32>,
    pub timed_out: bool,
    pub wall_s: f64,
    pub cpu_s: f64,
    pub user_s: f64,
    pub peak_ws_mb: f64,
    pub peak_private_mb: f64,
    pub stragglers_killed: u32,
    pub cursor_clip_released: bool,
    pub error: Option<String>,
}

fn round(x: f64, d: i32) -> f64 { let k = 10f64.powi(d); (x * k).round() / k }
fn kill_tree(pid: u32) { let _ = Command::new("taskkill.exe").args(["/PID", &pid.to_string(), "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status(); }
fn norm(p: &str) -> String { p.trim_start_matches(r"\\?\").replace('/', "\\").to_lowercase() }

/// Everything `exec.ps1 -Exe -ArgFile -Cwd -TimeoutSec -EnvFile` did. `env_unset` entries may end in `*` (case-insensitive wildcard prefix).
pub fn run(exe: &Path, args: &[String], cwd: &Path, timeout_s: u64, env_set: &[(&str, &str)], env_unset: &[&str], stdout: Stdio, stderr: Stdio) -> ProcResult {
    let mut res = ProcResult::default();
    let mut filetime0 = FileTime::default();
    unsafe { GetSystemTimeAsFileTime(&mut filetime0); }
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(stdout).stderr(stderr);
    if !cwd.as_os_str().is_empty() { cmd.current_dir(cwd); }
    for pat in env_unset.iter().filter(|p| !p.is_empty()) {
        let pl = pat.to_lowercase();
        for (k, _) in std::env::vars_os() {
            let kl = k.to_string_lossy().to_lowercase();
            let hit = match pl.strip_suffix('*') { Some(prefix) => kl.starts_with(prefix), None => kl == pl };
            if hit { cmd.env_remove(&k); }
        }
    }
    for (k, v) in env_set { cmd.env(k, v); }
    match cmd.spawn() {
        Err(e) => res.error = Some(e.to_string()),
        Ok(mut child) => {
            res.started = true;
            let h = child.as_raw_handle() as Handle;
            let sw = Instant::now();
            let (mut peak_ws, mut peak_priv) = (0usize, 0usize);
            let poll = |peak_ws: &mut usize, peak_priv: &mut usize| unsafe {
                let mut c = MemCounters { cb: std::mem::size_of::<MemCounters>() as u32, ..Default::default() };
                if K32GetProcessMemoryInfo(h, &mut c, c.cb) != 0 { *peak_ws = (*peak_ws).max(c.peak_ws); *peak_priv = (*peak_priv).max(c.private); }
            };
            let mut exited = false;
            loop {
                match child.try_wait() { Ok(Some(_)) => { exited = true; break; } Ok(None) => {} Err(_) => break }
                if sw.elapsed().as_secs() >= timeout_s { res.timed_out = true; break; }
                std::thread::sleep(Duration::from_millis(100));
                poll(&mut peak_ws, &mut peak_priv);
            }
            if res.timed_out {
                kill_tree(child.id());
                let t = Instant::now();
                while t.elapsed() < Duration::from_secs(5) { if matches!(child.try_wait(), Ok(Some(_))) { exited = true; break; } std::thread::sleep(Duration::from_millis(50)); }
            }
            res.wall_s = round(sw.elapsed().as_secs_f64(), 3);
            unsafe {
                let (mut c, mut e, mut k, mut u) = (FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
                if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0 {
                    res.cpu_s = round((k.v() + u.v()) as f64 / 1e7, 3); res.user_s = round(u.v() as f64 / 1e7, 3);
                }
            }
            poll(&mut peak_ws, &mut peak_priv);
            res.peak_ws_mb = round(peak_ws as f64 / 1048576.0, 1);
            res.peak_private_mb = round(peak_priv as f64 / 1048576.0, 1);
            if exited { if let Ok(Some(s)) = child.try_wait() { res.exit = s.code(); } }
        }
    }
    // stragglers: any other process running this very executable that started after us (a hung leftover, or the child respawning itself)
    res.stragglers_killed = kill_stragglers(exe, filetime0.v().saturating_sub(10_000_000));
    if res.timed_out || res.stragglers_killed > 0 || res.exit.map_or(false, |c| c != 0) { res.cursor_clip_released = release_cursor_clip(); }
    res
}

fn kill_stragglers(exe: &Path, since_ft: u64) -> u32 {
    let want = norm(&exe.to_string_lossy());
    let mut killed = 0;
    unsafe {
        let snap = CreateToolhelp32Snapshot(2, 0); // TH32CS_SNAPPROCESS
        if snap as isize == INVALID_HANDLE || snap.is_null() { return 0; }
        let mut e: ProcEntry = std::mem::zeroed();
        e.size = std::mem::size_of::<ProcEntry>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        let me = std::process::id();
        while ok != 0 {
            if e.pid != me && e.pid != 0 {
                let p = OpenProcess(0x1000, 0, e.pid); // PROCESS_QUERY_LIMITED_INFORMATION
                if !p.is_null() {
                    let mut buf = [0u16; 1024]; let mut len = buf.len() as u32;
                    if QueryFullProcessImageNameW(p, 0, buf.as_mut_ptr(), &mut len) != 0 && norm(&String::from_utf16_lossy(&buf[..len as usize])) == want {
                        let (mut c, mut x, mut k, mut u) = (FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
                        if GetProcessTimes(p, &mut c, &mut x, &mut k, &mut u) != 0 && c.v() >= since_ft { kill_tree(e.pid); killed += 1; }
                    }
                    CloseHandle(p);
                }
            }
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    killed
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cmd_exe() -> std::path::PathBuf { std::path::PathBuf::from(std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into())) }
    #[test] fn measures_exit_code_time_and_memory_and_passes_the_environment() {
        let r = run(&cmd_exe(), &["/c".into(), "exit %FN_PARITY_T%".into()], Path::new(""), 30, &[("FN_PARITY_T", "3")], &[], Stdio::null(), Stdio::null());
        assert!(r.started && !r.timed_out && r.exit == Some(3), "{r:?}");
        assert!(r.wall_s > 0.0 && r.peak_ws_mb > 0.0 && r.error.is_none(), "{r:?}");
    }
    #[test] fn env_unset_wildcards_remove_matching_variables() {
        std::env::set_var("FN_PARITY_WILD_A", "x");
        let r = run(&cmd_exe(), &["/c".into(), "if defined FN_PARITY_WILD_A (exit 7) else (exit 0)".into()], Path::new(""), 30, &[], &["fn_parity_wild*"], Stdio::null(), Stdio::null());
        assert_eq!(r.exit, Some(0), "{r:?}");
    }
    #[test] fn a_hung_child_is_killed_with_its_tree_at_the_timeout() {
        let r = run(Path::new(r"C:\Windows\System32\PING.EXE"), &["-n".into(), "60".into(), "127.0.0.1".into()], Path::new(""), 1, &[], &[], Stdio::null(), Stdio::null());
        assert!(r.timed_out && r.wall_s >= 1.0 && r.wall_s < 20.0, "{r:?}");
    }
    #[test] fn a_missing_executable_is_reported_not_panicked() {
        let r = run(Path::new(r"C:\definitely\not\here.exe"), &[], Path::new(""), 5, &[], &[], Stdio::null(), Stdio::null());
        assert!(!r.started && r.error.is_some());
    }
}
