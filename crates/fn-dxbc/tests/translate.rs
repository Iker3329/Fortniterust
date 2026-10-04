//! (3c) the translators on the old corpus: every unique `.dxbc` below reports/ is translated to WGSL (validated with naga) and to Rust
//! (the committed `tests/generated/corpus.rs` is compiled and compared with the interpreter in `generated_rust.rs`).  The corpus is the set of
//! unique shaders of the old reports, vendored in `tests/corpus/` (refresh with `FN_DXBC_REGEN=1`).
mod common;
use common::*;
use fn_dxbc::codegen::{translate_rust_module, translate_wgsl};
use std::collections::BTreeMap;

pub fn validate_wgsl(src: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(src).map_err(|e| e.emit_to_string(src))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

#[test]
fn corpus_translates_and_validates() {
    let c = corpus();
    let shaders = parse_all(&c);
    let (_, failed) = translate_rust_module(&shaders.iter().map(|(n, s)| (n.clone(), s)).collect::<Vec<_>>());
    let (mut wgsl_ok, mut naga_ok) = (0, 0);
    let mut unsupported: BTreeMap<String, usize> = BTreeMap::new();
    let mut naga_fail: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for (name, sh) in &shaders {
        match translate_wgsl(sh, "shader") {
            Ok(w) => {
                wgsl_ok += 1;
                match validate_wgsl(&w) {
                    Ok(()) => naga_ok += 1,
                    Err(e) => {
                        let key: String = e.lines().next().unwrap_or("").chars().take(120).collect();
                        let ent = naga_fail.entry(key).or_insert((0, format!("{name}\n{w}")));
                        ent.0 += 1;
                    }
                }
            }
            Err(e) => *unsupported.entry(format!("wgsl: {e}")).or_default() += 1,
        }
    }
    eprintln!("corpus: {} unique shaders; Rust translated {}; WGSL translated {wgsl_ok}, naga-valid {naga_ok}", shaders.len(), shaders.len() - failed.len());
    for (n, e) in &failed { eprintln!("  rust unsupported {n}: {e}"); }
    for (k, n) in &unsupported { eprintln!("  unsupported x{n}: {k}"); }
    for (k, (n, sample)) in &naga_fail { eprintln!("  naga x{n}: {k}\n{}", sample.chars().take(3000).collect::<String>()); }
    assert!(failed.is_empty(), "rust translation failures");
    assert!(naga_fail.is_empty(), "{} naga failures", naga_fail.len());
}

/// `tests/generated/corpus.rs` must be what the translator produces today (regenerate with `FN_DXBC_REGEN=1`).
#[test]
fn generated_rust_is_up_to_date() {
    let regen = std::env::var_os("FN_DXBC_REGEN").is_some();
    if regen { refresh_corpus(); }
    let c = corpus();
    let shaders = parse_all(&c);
    let (text, failed) = translate_rust_module(&shaders.iter().map(|(n, s)| (n.clone(), s)).collect::<Vec<_>>());
    assert!(failed.is_empty());
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("generated").join("corpus.rs");
    if regen { std::fs::write(&path, &text).unwrap(); eprintln!("wrote {} ({} bytes)", path.display(), text.len()); return; }
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(old.replace("\r\n", "\n") == text, "tests/generated/corpus.rs is stale: run `FN_DXBC_REGEN=1 cargo test -p fn-dxbc --test translate`");
}


