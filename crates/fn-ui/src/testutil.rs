//! Test helpers: locate files of the OLD prototype tree (reports, assets, verification captures). Tests that need them skip with a message when
//! the tree is absent. Override the location with `FN_UI_OLD_ROOT`; small evidence files that were copied are looked up in `testdata/` first.
use std::path::PathBuf;

pub fn old_root() -> PathBuf { crate::assets::DirAssets::old_root() }

/// `testdata/<file name>` of this crate (copied evidence).
pub fn testdata(rel: &str) -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join(rel) }

/// Path of `rel` (relative to the old tree root, forward slashes) if it exists: the crate's `testdata/` copy wins.
pub fn find_old(rel: &str) -> Option<PathBuf> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let td = testdata(name);
    if td.exists() { return Some(td); }
    let p = old_root().join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    if p.exists() { Some(p) } else { None }
}

/// Text of an old-tree file; prints a skip notice and returns None when absent.
pub fn read_old(rel: &str) -> Option<String> {
    match find_old(rel).and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(t) => Some(t),
        None => { eprintln!("skipping: old-tree file {rel} not found (set FN_UI_OLD_ROOT)"); None }
    }
}
#[allow(dead_code)]
pub fn read_old_bytes(rel: &str) -> Option<Vec<u8>> {
    match find_old(rel).and_then(|p| std::fs::read(p).ok()) {
        Some(t) => Some(t),
        None => { eprintln!("skipping: old-tree file {rel} not found (set FN_UI_OLD_ROOT)"); None }
    }
}

/// UI context over the old assets tree (fonts + textures), or None (skip) when it is absent.
pub fn ui_context() -> Option<crate::scene::UiContext> {
    match crate::scene::UiContext::from_default_assets() { Ok(c) => Some(c), Err(e) => { eprintln!("skipping: no UI assets/fonts ({e})"); None } }
}

/// Evidence report text (`testdata/<name>` copy, else the old tree's `reports/<name>`); panics when neither exists (the copies ship with the crate).
pub fn report(name: &str) -> String { read_old(&format!("reports/{name}")).unwrap_or_else(|| panic!("evidence file {name} missing from testdata/ and the old tree")) }
pub fn report_json(name: &str) -> serde_json::Value { serde_json::from_str(&report(name)).unwrap_or_else(|e| panic!("{name}: {e}")) }

/// Fonts of the old unit tests (Fonts::from_paths): NotoSans Regular/Bold plus the Burbank title and Roboto Bold faces. Cached; falls back to system
/// fonts (and the NotoSans Bold stand-ins) when the asset tree is absent.
pub fn fonts() -> crate::text::Fonts {
    static F: std::sync::OnceLock<crate::text::Fonts> = std::sync::OnceLock::new();
    F.get_or_init(|| {
        let src = crate::assets::DirAssets::new(crate::assets::DirAssets::default_root().join("fonts"));
        crate::text::Fonts::load(&src).expect("a NotoSans or system font")
    }).clone()
}
/// Directory of the old exported assets (for tests that read PNG dimensions); None when absent.
#[allow(dead_code)]
pub fn assets_root() -> Option<PathBuf> { let r = crate::assets::DirAssets::default_root(); if r.is_dir() { Some(r) } else { eprintln!("skipping: asset tree {} not found", r.display()); None } }