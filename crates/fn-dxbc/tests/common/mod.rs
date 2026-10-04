#![allow(dead_code)]
pub mod asm;
use fn_dxbc::{Container, Shader};
use std::path::{Path, PathBuf};

pub fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() { collect(&p, out); } else if p.extension().is_some_and(|x| x == "dxbc") { out.push(p); }
    }
}

/// FNV-1a of the program chunk (same function the CLI sweep uses to name unique shaders).
pub fn program_hash(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    if let Ok(c) = Container::parse(bytes) {
        if let Some(p) = c.program() { for &b in p.data { h = (h ^ b as u64).wrapping_mul(0x100000001b3); } }
    }
    h
}

/// The committed corpus: one `.dxbc` per unique program of the old reports (`tests/corpus/s_<hash>.dxbc`).  Vendored so the generated Rust
/// (`tests/generated/corpus.rs`) and the tests that use it do not depend on the old tree (which other tools keep rewriting).
pub fn corpus_dir() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("corpus") }

/// Re-imports the unique shaders of `reports/**/*.dxbc` into `tests/corpus` (run through `FN_DXBC_REGEN=1 cargo test -p fn-dxbc --test translate`).
pub fn refresh_corpus() {
    let mut files = vec![];
    collect(&fn_testdata::reports(), &mut files);
    files.sort();
    let dir = corpus_dir();
    std::fs::create_dir_all(&dir).unwrap();
    for f in files {
        let Ok(bytes) = std::fs::read(&f) else { continue };
        let target = dir.join(format!("s_{:016x}.dxbc", program_hash(&bytes)));
        if !target.exists() { std::fs::write(target, bytes).unwrap(); }
    }
}

/// Every unique shader of the committed corpus plus the hand-assembled ones: (name, dxbc bytes), sorted by name.
pub fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut files = vec![];
    collect(&corpus_dir(), &mut files);
    files.sort();
    let mut v: Vec<(String, Vec<u8>)> = files.iter().filter_map(|f| Some((f.file_stem()?.to_string_lossy().to_string(), std::fs::read(f).ok()?))).collect();
    v.extend(asm::handmade());
    v
}

pub fn parse_all(c: &[(String, Vec<u8>)]) -> Vec<(String, Shader)> { c.iter().map(|(n, b)| (n.clone(), Shader::parse(b).unwrap())).collect() }
