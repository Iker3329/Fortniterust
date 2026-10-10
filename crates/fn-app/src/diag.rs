//! Start-up and failure UX of the game binary: console attach for the GUI-subsystem release exe, the log file beside the exe, native error boxes and
//! the panic hook (the old runtime's `install_panic_hook` / `error` / `fatal`, over `sys.rs`).
//!
//! * [`init`] (first thing in `main`): attaches to the parent console when started from a terminal, opens `fortnite-rs.log` beside the exe (falls back to
//!   the temp folder when that is read-only; the previous run is kept as `fortnite-rs.previous.log`) and tees **everything written with `eprintln!`**
//!   into it (the process stderr is swapped for a pipe; nothing in the other modules has to change), then installs the panic hook.
//! * [`show_error`]: log + native box. Boxes are suppressed in automated runs (`--smoke-test`, `--script`, `--screenshot`, `--live-test`, see
//!   [`set_dialogs`]) and whenever the env var `FN_NO_DIALOG` is set, so a harness never hangs on a modal dialog; the message still goes to stderr / log.
//! * [`last_error`]: the last `error: ...` line that went to stderr. `main` uses it to show a box when the app loop exits with failure before any
//!   in-window error page could be drawn (window creation failure, GPU failure).
//! * [`settings_load_note`] / [`log_settings_load`]: the human-readable note for a `fn_settings::LoadReport` (corrupt file moved to `.corrupt`,
//!   repaired values). `app.rs` builds a `settings_note` string that was never printed; call [`log_settings_load`] there (see README, Handoff).
use crate::sys;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

pub const TITLE: &str = "Fortnite 1.8 - standalone Rust runtime";
pub const LOG_NAME: &str = "fortnite-rs.log";

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
static DIALOGS: AtomicBool = AtomicBool::new(true);
static CONSOLE: OnceLock<sys::Console> = OnceLock::new();

pub fn exe_dir() -> PathBuf { std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from(".")) }

/// Where the log goes: beside the exe when that folder is writable, else the temp folder.
pub fn log_path_candidates(exe_dir: &Path, temp: &Path) -> Vec<PathBuf> { vec![exe_dir.join(LOG_NAME), temp.join(LOG_NAME)] }
pub fn log_path() -> Option<&'static Path> { LOG_PATH.get().map(|p| p.as_path()) }
/// True when something visible (a terminal or a redirect) receives stderr, false for a double-clicked GUI exe.
pub fn stderr_visible() -> bool { !matches!(CONSOLE.get(), Some(sys::Console::None)) }

/// `YYYY-MM-DD HH:MM:SS UTC` of a unix time (civil-from-days, no time-zone database needed).
pub fn utc_stamp(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let (y, doy) = (yoe + era * 400, doe - (365 * yoe + yoe / 4 - yoe / 100));
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", if m <= 2 { y + 1 } else { y }, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Extracts complete `error: ...` lines from a stream of stderr chunks (keeps the unfinished tail in `partial`).
pub fn scan_error_lines(partial: &mut String, chunk: &[u8]) -> Vec<String> {
    partial.push_str(&String::from_utf8_lossy(chunk));
    let mut out = vec![];
    while let Some(i) = partial.find('\n') {
        let line: String = partial.drain(..=i).collect();
        if let Some(rest) = line.trim_end().strip_prefix("error:") { out.push(rest.trim().to_owned()); }
    }
    if partial.len() > 64 * 1024 { partial.clear(); }
    out
}

fn open_log(exe_dir: &Path) -> Option<(std::fs::File, PathBuf)> {
    for p in log_path_candidates(exe_dir, &std::env::temp_dir()) {
        if p.exists() { let _ = std::fs::rename(&p, p.with_file_name("fortnite-rs.previous.log")); }
        if let Ok(f) = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&p) { return Some((f, p)); }
    }
    None
}

/// Console attach + log file + stderr tee + panic hook. Call once, first thing in `main`.
pub fn init() {
    let console = sys::console_init();
    let _ = CONSOLE.set(console);
    if let Some((mut file, path)) = open_log(&exe_dir()) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let args: Vec<String> = std::env::args().skip(1).collect();
        let _ = writeln!(file, "fortnite-rs {} started {}  console: {:?}  args: {:?}", env!("CARGO_PKG_VERSION"), utc_stamp(now), console, args);
        *LOG_FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(file);
        let _ = LOG_PATH.set(path);
        let mut partial = String::new();
        sys::tee_stderr(move |chunk| {
            if let Ok(mut g) = LOG_FILE.lock() { if let Some(f) = g.as_mut() { let _ = f.write_all(chunk); } }
            for line in scan_error_lines(&mut partial, chunk) { if let Ok(mut g) = LAST_ERROR.lock() { *g = Some(line); } }
        });
    }
    install_panic_hook();
}

/// Waits until everything printed so far has reached the log file and the terminal. `main` calls it before returning.
pub fn flush() { sys::tee_flush(std::time::Duration::from_millis(500)); }

/// Allow / forbid native boxes (interactive runs only). `FN_NO_DIALOG` always wins.
pub fn set_dialogs(on: bool) { DIALOGS.store(on, Ordering::Relaxed); }
pub fn dialogs_enabled() -> bool { DIALOGS.load(Ordering::Relaxed) && fn_live::var_os("FN_NO_DIALOG").is_none() }
pub fn last_error() -> Option<String> { LAST_ERROR.lock().ok().and_then(|g| g.clone()) }

/// Logs `text` and (when dialogs are enabled) shows it in a native error box. The log path is appended to the box.
pub fn show_error(title: &str, text: &str) {
    eprintln!("[fatal] {title}: {text}");
    if dialogs_enabled() {
        let tail = log_path().map(|p| format!("\n\nLog file: {}", p.display())).unwrap_or_default();
        sys::message_box(title, &format!("{text}{tail}"));
    }
}

fn panic_text(info: &std::panic::PanicHookInfo<'_>) -> (String, String) {
    let what = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown error".into());
    let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_else(|| "unknown location".into());
    (what, at)
}

/// Any panic names its location in the log and a native box before the process dies (the release profile is `panic = "abort"`, which used to end the
/// game silently with 0xC0000409). Release the mouse first: a clipped cursor could not reach the box.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let (what, at) = panic_text(info);
        let thread = std::thread::current().name().unwrap_or("<unnamed>").to_owned();
        sys::release_cursor_clip();
        eprintln!("[panic] thread '{thread}' at {at}: {what}\n{}", std::backtrace::Backtrace::capture());
        sys::tee_flush(std::time::Duration::from_millis(500)); // the report is in the log / terminal before anything else can go wrong
        let fatal = cfg!(panic = "abort") || thread == "main";
        if fatal {
            let tail = log_path().map(|p| format!("\n\nLog file: {}", p.display())).unwrap_or_default();
            if dialogs_enabled() { sys::message_box(TITLE, &format!("The game hit an unexpected error and must close.\n\n{what}\n({at}){tail}")); }
            if cfg!(panic = "abort") { std::process::exit(101); }
        }
    }));
}

/// One human-readable note for a settings load, `None` when nothing happened worth telling (no file yet, or a clean load).
pub fn settings_load_note(r: &fn_settings::LoadReport) -> Option<String> {
    use fn_settings::LoadStatus as S;
    let shown = |w: &[String]| { let mut s = w.iter().take(6).cloned().collect::<Vec<_>>().join("; "); if w.len() > 6 { s.push_str(&format!("; ... {} more", w.len() - 6)); } s };
    match r.status {
        S::Loaded | S::Missing => None,
        S::Repaired => Some(format!("{} had values that were repaired: {}", r.path.display(), shown(&r.warnings))),
        S::Corrupt => Some(format!("{} could not be used ({}); {}; defaults are in effect", r.path.display(), shown(&r.warnings),
            r.backup.as_ref().map_or("the file was left in place".to_owned(), |b| format!("the bad file was moved to {}", b.display())))),
    }
}
/// Prints (and so logs) [`settings_load_note`] as a `[settings]` line.
pub fn log_settings_load(r: &fn_settings::LoadReport) { if let Some(n) = settings_load_note(r) { eprintln!("[settings] {n}"); } }

#[cfg(test)]
mod tests {
    use super::*;
    use fn_settings::{LoadReport, LoadStatus};

    #[test] fn utc_stamp_is_correct_for_known_instants() {
        assert_eq!(utc_stamp(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc_stamp(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(utc_stamp(1_759_449_599), "2025-10-02 23:59:59 UTC");
    }
    #[test] fn error_lines_are_found_across_chunk_boundaries() {
        let mut p = String::new();
        assert!(scan_error_lines(&mut p, b"[load] x\nerr").is_empty());
        assert_eq!(scan_error_lines(&mut p, b"or: Could not create the window: boom\r\nmore\n"), vec!["Could not create the window: boom".to_owned()]);
        assert!(scan_error_lines(&mut p, b"warning: error: not at the start\n").is_empty());
    }
    #[test] fn log_goes_beside_the_exe_then_to_temp() {
        let c = log_path_candidates(Path::new("C:/game"), Path::new("C:/tmp"));
        assert_eq!(c, vec![PathBuf::from("C:/game").join(LOG_NAME), PathBuf::from("C:/tmp").join(LOG_NAME)]);
    }
    #[test] fn corrupt_settings_note_names_the_moved_file_and_clean_loads_are_silent() {
        let mut r = LoadReport { status: LoadStatus::Corrupt, path: "x/FortniteSettings.ini".into(), file_version: None, warnings: vec!["settings file unusable: bad".into()], backup: Some("x/FortniteSettings.ini.corrupt".into()) };
        let n = settings_load_note(&r).unwrap();
        assert!(n.contains("FortniteSettings.ini") && n.contains("moved to") && n.contains(".corrupt") && n.contains("defaults"), "{n}");
        r.backup = None; assert!(settings_load_note(&r).unwrap().contains("left in place"));
        r.status = LoadStatus::Repaired; r.warnings = (0..9).map(|i| format!("w{i}")).collect(); assert!(settings_load_note(&r).unwrap().contains("3 more"));
        r.status = LoadStatus::Loaded; assert!(settings_load_note(&r).is_none()); r.status = LoadStatus::Missing; assert!(settings_load_note(&r).is_none());
    }
    #[test] fn dialogs_can_be_switched_off() { set_dialogs(false); assert!(!dialogs_enabled()); set_dialogs(true); }
    /// The crate lint is `deny(unsafe_code)`; `sys.rs` is the single opt-out. Nothing else may contain the keyword as code.
    #[test] fn unsafe_is_confined_to_sys_rs() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.extension().map_or(true, |x| x != "rs") || p.file_name().map_or(false, |n| n == "sys.rs") { continue; }
            let text = std::fs::read_to_string(&p).unwrap();
            let kw = ["un", "safe"].concat(); // built at run time so this test does not trip over itself
            let needles = [format!("{kw} "), format!("{kw}{{"), format!("allow({kw}_code")];
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                assert!(!needles.iter().any(|k| code.contains(k.as_str())), "{}:{}: {kw} outside sys.rs: {line}", p.display(), n + 1);
            }
        }
    }
}
