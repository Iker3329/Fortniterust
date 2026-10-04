#![allow(dead_code)]
use fn_dxbc_cli::extract::{scan_blobs, Blob};
use fn_dxbc_cli::spec::{default_install, package_bytes};
use fn_pak::PakSet;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The 1.8 install, or `None` (tests skip) when it is not on this machine.
pub fn install() -> Option<PathBuf> {
    let p = default_install();
    if p.join("Content").join("Paks").is_dir() { Some(p) } else { None }
}

pub fn open() -> Option<PakSet> { install().and_then(|p| PakSet::open_install(p).ok()) }

/// Same tolerant normalisation as `fn-dxbc/tests/oracle.rs`.
pub fn normalize(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut acc = String::new();
    for l in text.replace(['\0', '\u{feff}'], "").lines() {
        let t = l.trim();
        if t.is_empty() && acc.is_empty() { continue; }
        acc = if acc.is_empty() { t.to_string() } else { format!("{acc}{t}") };
        let bal = acc.matches('(').count() as i32 - acc.matches(')').count() as i32;
        let braces = acc.matches('{').count() as i32 - acc.matches('}').count() as i32;
        if (bal > 0 || braces > 0) && !acc.starts_with("//") { continue; }
        out.push(acc.split_whitespace().collect::<Vec<_>>().join(" "));
        acc.clear();
    }
    out
}

pub fn files(root: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v = vec![];
    fn rec(d: &Path, ext: &str, v: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { rec(&p, ext, v); } else if p.extension().is_some_and(|x| x == ext) { v.push(p); }
        }
    }
    rec(root, ext, &mut v);
    v.sort();
    v
}

/// file-name (lowercase, no extension) -> game paths of every `.uasset` in the paks
pub struct PathIndex { map: HashMap<String, Vec<String>> }
impl PathIndex {
    pub fn new(set: &PakSet) -> Self {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (_, e) in set.effective() {
            let p = fn_pak::normalize_path(&e.path);
            if !p.to_ascii_lowercase().ends_with(".uasset") { continue; }
            let Some(game) = fn_pak::pak_path_to_game(&p) else { continue };
            let name = p.rsplit('/').next().unwrap().trim_end_matches(".uasset").to_ascii_lowercase();
            map.entry(name).or_default().push(game);
        }
        PathIndex { map }
    }
    pub fn find(&self, name: &str) -> Option<String> { self.map.get(&name.to_ascii_lowercase()).and_then(|v| v.first().cloned()) }
}

pub struct PkgCache { pub map: HashMap<String, Vec<Blob>> }
impl PkgCache {
    pub fn new() -> Self { PkgCache { map: HashMap::new() } }
    pub fn blobs(&mut self, set: &PakSet, game: &str) -> &Vec<Blob> {
        self.map.entry(game.to_string()).or_insert_with(|| package_bytes(set, game).map(|b| scan_blobs(&b)).unwrap_or_default())
    }
}


/// FNV-1a of the program chunk (same function the CLI sweep uses to name unique shaders).
pub fn program_hash(bytes: &[u8]) -> u64 { fn_dxbc_cli::extract::program_hash(bytes) }

/// Every unique shader of the old corpus (`reports/**/*.dxbc`): (name, bytes), sorted by name.
pub fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut seen = std::collections::BTreeMap::new();
    for f in files(&fn_testdata::reports(), "dxbc") {
        let Ok(bytes) = std::fs::read(&f) else { continue };
        seen.entry(program_hash(&bytes)).or_insert(bytes);
    }
    seen.into_iter().map(|(h, b)| (format!("s_{h:016x}"), b)).collect()
}
