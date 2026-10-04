//! Locates the READ-ONLY old prototype tree (`FortniteRuntime`) for parity tests: recovered-data reports, exported assets and the
//! logs the old executable wrote. Override with env `FN_OLD_ROOT`. Tests that need data that is absent call `require_*` and are
//! skipped (return None) instead of failing, so the workspace still tests on a machine without the old tree.
use std::path::{Path, PathBuf};

/// `<workspace>/../FortniteRuntime` unless `FN_OLD_ROOT` is set.
pub fn old_root() -> PathBuf {
    if let Some(p) = std::env::var_os("FN_OLD_ROOT") { return PathBuf::from(p); }
    // crates/<name> -> workspace root -> its parent -> FortniteRuntime
    let here = std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    here.ancestors().nth(3).map(|p| p.join("FortniteRuntime")).unwrap_or_else(|| PathBuf::from("FortniteRuntime"))
}
pub fn reports() -> PathBuf { old_root().join("reports") }
pub fn assets() -> PathBuf { old_root().join("assets") }
pub fn verification() -> PathBuf { old_root().join("verification") }
/// Contents of `reports/<name>`; panics with a clear message when the old tree is missing (tests that need it are parity tests).
pub fn report_text(name: &str) -> String {
    let p = reports().join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("old-tree report {} not readable ({e}); set FN_OLD_ROOT", p.display()))
}
/// All files below `root`, recursively (unreadable directories yield nothing).
pub fn files(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let Ok(dir) = std::fs::read_dir(root) else { return out };
    for e in dir.flatten() { let p = e.path(); if p.is_dir() { out.extend(files(&p)); } else { out.push(p); } }
    out
}
/// Case-insensitive lookup of a file by name in a file list.
pub fn find(files: &[PathBuf], name: &str) -> Result<PathBuf, String> {
    files.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))).cloned().ok_or_else(|| format!("Missing asset: {name}"))
}
#[cfg(test)]
mod tests { #[test] fn old_root_exists() { assert!(super::old_root().join("src").is_dir(), "{:?}", super::old_root()); } }
