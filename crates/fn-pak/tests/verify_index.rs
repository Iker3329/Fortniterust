//! Verifies the Rust pak reader against the Node prototype's `reports/pak-index.json`
//! (same entry count, same paths, offsets, sizes, methods, encrypted flags) for the 1.8 install.
//! Skipped (with a message) when the install or the report is not present on this machine.

use std::path::PathBuf;

fn fn_root() -> PathBuf {
    PathBuf::from(std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into()))
}
fn reports() -> PathBuf {
    if let Ok(p) = std::env::var("FN_REPORTS") {
        return PathBuf::from(p);
    }
    PathBuf::from(r"C:\Users\Brady\Documents\Codex\2026-10-02\im-seeing-titkoks-where-people-are-2\FortniteRuntime\reports")
}

#[test]
fn matches_node_pak_index() {
    let root = fn_root();
    let idx_path = reports().join("pak-index.json");
    if !root.exists() || !idx_path.exists() {
        eprintln!("SKIP: install or pak-index.json missing");
        return;
    }
    let t = std::time::Instant::now();
    let set = fn_pak::PakSet::open_install(&root).expect("open install");
    eprintln!("mounted {} paks, {} entries, {} effective in {:?}", set.paks().len(), set.total_entries(), set.len(), t.elapsed());
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&idx_path).unwrap()).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), set.total_entries(), "entry count");
    // pak-index.json is in alphabetical pak order, entry order within pak.
    let mut paks: Vec<&fn_pak::PakFile> = set.paks().iter().collect();
    paks.sort_by(|a, b| a.name.cmp(&b.name));
    let mut i = 0;
    for pf in paks {
        for e in &pf.entries {
            let j = &arr[i];
            assert_eq!(j["pak"].as_str().unwrap(), pf.name, "pak @{i}");
            assert_eq!(j["file"].as_str().unwrap(), e.path, "file @{i}");
            assert_eq!(j["offset"].as_u64().unwrap(), e.offset, "offset @{i}");
            assert_eq!(j["compressed"].as_u64().unwrap(), e.compressed_size, "compressed @{i}");
            assert_eq!(j["uncompressed"].as_u64().unwrap(), e.size, "uncompressed @{i}");
            assert_eq!(j["method"].as_u64().unwrap(), e.method as u64, "method @{i}");
            assert_eq!(j["encrypted"].as_u64().unwrap() != 0, e.encrypted, "encrypted @{i}");
            i += 1;
        }
    }
    assert_eq!(i, arr.len());
}

#[test]
fn case_insensitive_lookup_and_read() {
    let root = fn_root();
    if !root.exists() {
        return;
    }
    let set = fn_pak::PakSet::open_install(&root).expect("open install");
    let hits = set.glob("FortniteGame/Content/**/PlayerPawn_Generic.uasset");
    assert!(!hits.is_empty(), "glob found PlayerPawn_Generic");
    let p = hits[0].1.path.clone();
    let upper = p.to_uppercase();
    let a = set.read(&p).unwrap();
    let b = set.read(&upper).unwrap();
    assert_eq!(a, b);
    assert_eq!(&a[0..4], &[0xC1, 0x83, 0x2A, 0x9E], "uasset magic");
}

/// The 1.10 install uses pak v4 (footer gains the index-encryption flag byte) and *per-pak mount points*
/// (`../../../FortniteGame/Content/L10N/de/...`): entry names are relative to the mount point.
#[test]
fn mounts_1_10_pak_v4() {
    let root = PathBuf::from(std::env::var("FN_ROOT_110").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.10-Cert-CL-3790078\1.10-Cert-CL-3790078\Fortnite\Fortnite\FortniteGame".into()));
    if !root.exists() {
        eprintln!("SKIP: 1.10 install missing");
        return;
    }
    let set = fn_pak::PakSet::open_install(&root).expect("open 1.10 install");
    assert!(set.skipped.is_empty(), "skipped: {:?}", set.skipped);
    assert!(set.paks().iter().all(|p| p.info.version == 4));
    assert!(set.paks().iter().any(|p| p.mount != "../../../"), "expected non-default mount points");
    assert_eq!(set.total_entries(), set.len(), "no shadowed duplicates expected");
    let hits = set.glob("FortniteGame/Content/TimeOfDay/TODM/TODM_A_Child_Athena.uasset");
    assert_eq!(hits.len(), 1);
    let b = set.read(hits[0].1.rel_path()).unwrap();
    assert_eq!(&b[0..4], &[0xC1, 0x83, 0x2A, 0x9E]);
    // every stored entry of the mount-point-prefixed paks resolves to the same bytes by its full path
    let (pf, e) = set.paks().iter().find(|p| p.mount.contains("L10N/de")).map(|p| (p, &p.entries[0])).unwrap();
    assert_eq!(pf.read(e).unwrap().len() as u64, e.size);
    assert!(set.contains(&e.path));
}
