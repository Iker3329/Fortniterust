//! Shared fixtures of the fn-pose parity tests. Everything that needs the old tree (`FortniteRuntime/assets`, `verification`) or the extracted
//! packages (`%LOCALAPPDATA%\Temp\fn-testdata`) returns None / skips with a note when the data is missing.
#![allow(dead_code)]
use fn_animgraph::{ClipLibrary, Skeleton};
use fn_legacy::Md5Clip;
use fn_pose::clip::{load_player_skeleton_info, ClipSourceDir, ClipStore, SampleMode};
use fn_pose::loader::{self, AssetIndex};
use fn_pose::part::Part;
use glam::Vec3;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());
/// The pick script and the scenario runner flip process-wide switches.
pub fn lock() -> MutexGuard<'static, ()> { LOCK.lock().unwrap_or_else(|e| e.into_inner()) }

pub fn golden(rel: &str) -> String {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "testdata", "old-logs", rel].iter().collect();
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("golden log {} missing: {e}", p.display()))
}

/// Everything the old-tree based tests need.
pub struct Old {
    pub idx: AssetIndex,
    pub parts: Vec<Part>,
    pub md5: HashMap<String, Md5Clip>,
    pub reference: Skeleton,
}
impl Old {
    pub fn load() -> Option<Old> {
        let assets = fn_testdata::assets();
        let all = fn_legacy::files(&assets);
        if all.is_empty() { eprintln!("SKIPPED: old tree assets not found at {}", assets.display()); return None; }
        let idx = AssetIndex::scan(&assets);
        let parts = loader::old_tree_parts(&idx).unwrap_or_else(|e| panic!("{e}"));
        let md5 = fn_legacy::load_clip_set(&all, &fn_legacy::prototype_clip_names()).unwrap_or_else(|e| panic!("{e}"));
        let reference = fn_legacy::load_soldier_skeleton(&all).unwrap_or_else(|e| panic!("{e}"));
        Some(Old { idx, parts, md5, reference })
    }
}

/// Decoded clips (from the extracted packages) in `mode`, plus the player USkeleton.
pub fn package_clips(mode: SampleMode) -> Option<(ClipStore, ClipSourceDir<'static>)> {
    let root = loader::default_package_root().or_else(|| { eprintln!("SKIPPED: extracted packages not found (FN_TESTDATA)"); None })?;
    let src = ClipSourceDir::RawPackages(Box::leak(root.into_boxed_path()));
    let skel = load_player_skeleton_info(&src).unwrap_or_else(|e| panic!("{e}"));
    let store = ClipStore::load(&src, &fn_legacy::prototype_clip_names(), &skel, mode).unwrap_or_else(|e| panic!("{e}"));
    Some((store, src))
}

pub fn lib_of<'a, C: fn_animgraph::ClipSource>(m: &'a HashMap<String, C>) -> &'a dyn ClipLibrary { m }

/// max |a-b| over components
pub fn max_abs(a: Vec3, b: Vec3) -> f32 { (a - b).abs().max_element() }

/// Per-name running maximum.
#[derive(Default)]
pub struct Errs { pub per: Vec<(String, f32, u32)> }
impl Errs {
    pub fn add(&mut self, name: &str, e: f32, frame: u32) {
        match self.per.iter_mut().find(|x| x.0 == name) { Some(x) => { if e > x.1 { x.1 = e; x.2 = frame; } } None => self.per.push((name.to_owned(), e, frame)) }
    }
    pub fn max(&self) -> f32 { self.per.iter().map(|x| x.1).fold(0., f32::max) }
    pub fn report(&self, what: &str) { for (n, e, f) in &self.per { println!("  {what} {n:<12} max error {e:.3e} m (frame {f})"); } println!("  {what} OVERALL max error {:.3e} m", self.max()); }
}
