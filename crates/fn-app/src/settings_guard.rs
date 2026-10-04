//! Looks at the user's `FortniteSettings.ini` once, before the app loads it, so that a saved window size that the GPU cannot present never reaches the
//! window / swap chain, and so that the bad value is repaired **in the file** (the old crash repeated on every launch because nothing ever rewrote it).
//!
//! * a `Video.Resolution` above [`MAX_CLIENT`] (e.g. `9000x720`, `1280x9000`, `99999x1`) is clamped per axis and written back;
//! * a malformed one (`abc`) is reported by `fn_settings` as repaired-to-default and the file is rewritten too;
//! * a corrupt file is moved to `.corrupt` by `Settings::load` (unchanged); here its note is returned so it can be printed / logged.
//!
//! `fn_settings` clamps to its own `MAX_RES` when it parses; this guard is independent of that constant on purpose (it re-checks against the renderer's
//! limit) and the renderer clamps once more to the real device limit (`gfx.rs`), so a hand-edited ini can never crash the window.
use crate::platform::{MAX_CLIENT, MIN_CLIENT};
use fn_settings::{LoadReport, LoadStatus, Settings, Value};
use std::path::Path;

/// The client size the window may be created with for a saved `(w, h)`; `(0, 0)` stays "unset" (the caller falls back to its default).
pub fn clamp_resolution(w: u32, h: u32) -> (u32, u32) {
    if w == 0 && h == 0 { return (0, 0); }
    (w.clamp(MIN_CLIENT.0, MAX_CLIENT.0), h.clamp(MIN_CLIENT.1, MAX_CLIENT.1))
}

pub struct Checked {
    pub settings: Settings,
    pub report: LoadReport,
    /// `(saved, repaired)` when the resolution had to be changed (and the file was rewritten).
    pub repaired_resolution: Option<((u32, u32), (u32, u32))>,
    /// A line for the log / terminal, None when the file was fine (or absent).
    pub note: Option<String>,
}

/// Loads `path` like `Settings::load` and repairs the saved resolution in place (see the module docs).
pub fn check_and_repair(path: &Path) -> Checked {
    let (mut settings, mut report) = Settings::load(path);
    let saved = (settings.video.width, settings.video.height);
    let fixed = clamp_resolution(saved.0, saved.1);
    let malformed = report.status == LoadStatus::Repaired && report.warnings.iter().any(|w| w.starts_with("Video.Resolution"));
    let mut repaired_resolution = None;
    if fixed != saved || malformed {
        if fixed != saved { let _ = settings.set("video.resolution", Value::Size(fixed.0, fixed.1)); repaired_resolution = Some((saved, fixed)); }
        match settings.save(path) {
            Ok(()) => report.warnings.push(format!("Video.Resolution {}x{} repaired to {}x{} and written back to {}", saved.0, saved.1, settings.video.width, settings.video.height, path.display())),
            Err(e) => report.warnings.push(format!("could not write the repaired Video.Resolution back to {}: {e}", path.display())),
        }
        if report.status == LoadStatus::Loaded { report.status = LoadStatus::Repaired; }
    }
    let note = crate::diag::settings_load_note(&report);
    Checked { settings, report, repaired_resolution, note }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tmp(name: &str) -> std::path::PathBuf { let d = std::env::temp_dir().join(format!("fn-app-guard-{name}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }
    fn ini(res: &str) -> String { format!("[Settings]\nVersion=1\n[Video]\nResolution={res}\nWindowMode=Windowed\n") }

    #[test] fn clamping_is_per_axis_and_keeps_unset() {
        assert_eq!(clamp_resolution(9000, 720), (8192, 720)); assert_eq!(clamp_resolution(1280, 9000), (1280, 8192)); assert_eq!(clamp_resolution(99999, 1), (8192, MIN_CLIENT.1));
        assert_eq!(clamp_resolution(0, 0), (0, 0)); assert_eq!(clamp_resolution(1920, 1080), (1920, 1080)); assert_eq!(clamp_resolution(u32::MAX, u32::MAX), (8192, 8192));
    }
    #[test] fn the_reported_crash_inputs_are_repaired_in_the_file() {
        for (bad, want) in [("9000x720", (8192, 720)), ("1280x9000", (1280, 8192)), ("99999x1", (8192, 360)), ("99999x99999", (8192, 8192))] {
            let d = tmp("crash"); let p = d.join("FortniteSettings.ini"); std::fs::write(&p, ini(bad)).unwrap();
            let c = check_and_repair(&p);
            assert_eq!((c.settings.video.width, c.settings.video.height), want, "{bad}");
            assert!(c.note.as_deref().map_or(false, |n| n.contains("written back")), "{bad}: {:?}", c.note);
            let again = check_and_repair(&p);
            assert_eq!(again.report.status, LoadStatus::Loaded, "{bad}: the file is clean on the next launch: {:?}", again.report.warnings);
            assert_eq!((again.settings.video.width, again.settings.video.height), want); assert!(again.note.is_none() && again.repaired_resolution.is_none());
            let _ = std::fs::remove_dir_all(d);
        }
    }
    #[test] fn a_malformed_resolution_is_rewritten_a_good_one_and_a_missing_file_are_left_alone() {
        let d = tmp("other"); let p = d.join("FortniteSettings.ini");
        std::fs::write(&p, ini("abc")).unwrap(); let c = check_and_repair(&p); assert!(c.repaired_resolution.is_none() && c.note.is_some());
        assert!(!std::fs::read_to_string(&p).unwrap().contains("abc"), "the garbage value is gone from the file");
        let good = ini("1920x1080"); std::fs::write(&p, &good).unwrap(); let c = check_and_repair(&p); assert!(c.note.is_none()); assert_eq!(std::fs::read_to_string(&p).unwrap(), good, "untouched");
        let missing = d.join("nope.ini"); let c = check_and_repair(&missing); assert!(c.note.is_none() && !missing.exists(), "no file is created for a first launch");
        let _ = std::fs::remove_dir_all(d);
    }
    #[test] fn a_corrupt_file_is_moved_aside_and_the_note_says_so() {
        let d = tmp("corrupt"); let p = d.join("FortniteSettings.ini"); std::fs::write(&p, [0xFFu8, 0xFE, 0x00, 0x01, 0x80, 0x81]).unwrap();
        let c = check_and_repair(&p);
        let note = c.note.expect("a corrupt file must leave a note");
        assert!(note.contains("moved to") && note.contains(".corrupt"), "{note}");
        assert!(d.join("FortniteSettings.ini.corrupt").exists() && !p.exists());
        let _ = std::fs::remove_dir_all(d);
    }
}
