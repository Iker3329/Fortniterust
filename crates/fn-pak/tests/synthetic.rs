//! Synthetic paks (built in memory, written to the temp dir) covering what the shipped installs do not exercise:
//! zlib-compressed blocks (neither the 1.8 nor the 1.10 install has any compressed entry), the v4 footer with an
//! index-encryption flag, v5 relative block offsets, encrypted entries, mount points, override order.

use fn_pak::{PakError, PakFile, PakSet};
use std::path::PathBuf;

fn fstring(s: &str) -> Vec<u8> {
    let mut v = ((s.len() + 1) as i32).to_le_bytes().to_vec();
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}

struct Ent {
    name: &'static str,
    data: Vec<u8>,
    compress: bool,
    encrypted: bool,
}

/// Serialise a pak. `version` 3, 4 (encrypted-index byte before the magic) or 5 (relative block offsets).
fn build_pak(version: u32, mount: &str, ents: &[Ent]) -> Vec<u8> {
    let mut file: Vec<u8> = Vec::new();
    let mut index: Vec<u8> = Vec::new();
    index.extend(fstring(mount));
    index.extend((ents.len() as i32).to_le_bytes());
    for e in ents {
        let offset = file.len() as u64;
        // header (as stored in-pak AND in the index) -- blocks need the final absolute positions, so build data first
        let block_size: u32 = 65536;
        let mut blocks: Vec<(u64, u64)> = Vec::new();
        let mut payload: Vec<u8> = Vec::new();
        let method: u32 = if e.compress { 1 } else { 0 };
        let header_len = 8 + 8 + 8 + 4 + 20 + if e.compress { 4 + 16 * e.data.len().div_ceil(block_size as usize).max(1) } else { 0 } + 1 + 4;
        if e.compress {
            let mut pos = offset + header_len as u64;
            for chunk in e.data.chunks(block_size as usize) {
                let z = miniz_oxide::deflate::compress_to_vec_zlib(chunk, 6);
                let (s, en) = (pos, pos + z.len() as u64);
                blocks.push((s, en));
                payload.extend(z);
                pos = en;
            }
            if e.data.is_empty() {
                blocks.push((pos, pos));
            }
        } else {
            payload.extend(&e.data);
        }
        let compressed_size = if e.compress { payload.len() as u64 } else { e.data.len() as u64 };
        let mut h: Vec<u8> = Vec::new();
        h.extend(offset.to_le_bytes());
        h.extend(compressed_size.to_le_bytes());
        h.extend((e.data.len() as u64).to_le_bytes());
        h.extend(method.to_le_bytes());
        h.extend([0u8; 20]);
        if e.compress {
            h.extend((blocks.len() as u32).to_le_bytes());
            for (s, en) in &blocks {
                let (s, en) = if version >= 5 { (s - offset, en - offset) } else { (*s, *en) };
                h.extend(s.to_le_bytes());
                h.extend(en.to_le_bytes());
            }
        }
        h.push(e.encrypted as u8);
        h.extend(if e.compress { block_size } else { 0 }.to_le_bytes());
        assert_eq!(h.len(), header_len);
        file.extend(&h);
        file.extend(&payload);
        index.extend(fstring(e.name));
        index.extend(&h);
    }
    let index_offset = file.len() as u64;
    let index_size = index.len() as u64;
    file.extend(&index);
    if version >= 4 {
        file.push(0); // bEncryptedIndex
    }
    file.extend(0x5A6F12E1u32.to_le_bytes());
    file.extend(version.to_le_bytes());
    file.extend(index_offset.to_le_bytes());
    file.extend(index_size.to_le_bytes());
    file.extend([0u8; 20]);
    file
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fn-pak-test-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn sample() -> Vec<Ent> {
    let big: Vec<u8> = (0..200_000u32).map(|i| ((i * 7) ^ (i >> 5)) as u8).collect();
    vec![
        Ent { name: "FortniteGame/Content/Plain.uasset", data: b"hello stored world".to_vec(), compress: false, encrypted: false },
        Ent { name: "FortniteGame/Content/Big.bin", data: big, compress: true, encrypted: false },
        Ent { name: "FortniteGame/Content/Small.bin", data: vec![42; 10], compress: true, encrypted: false },
        Ent { name: "FortniteGame/Config/Secret.ini", data: vec![1, 2, 3], compress: false, encrypted: true },
    ]
}

#[test]
fn zlib_blocks_and_versions() {
    for version in [3u32, 4, 5] {
        let ents = sample();
        let path = tmp(&format!("v{version}.pak"));
        std::fs::write(&path, build_pak(version, "../../../", &ents)).unwrap();
        let pak = PakFile::open(&path).unwrap();
        assert_eq!(pak.info.version, version);
        assert_eq!(pak.entries.len(), ents.len());
        for (e, want) in pak.entries.iter().zip(&ents) {
            assert_eq!(e.path, format!("../../../{}", want.name));
            if want.encrypted {
                assert!(matches!(pak.read(e), Err(PakError::Encrypted(_))));
                continue;
            }
            let got = pak.read(e).unwrap();
            assert_eq!(&got[..], &want.data[..], "v{version} {}", want.name);
            // stored entries are zero-copy
            assert_eq!(matches!(got, std::borrow::Cow::Borrowed(_)), !want.compress);
        }
    }
}

#[test]
fn mount_points_and_override_order() {
    // two paks, the later (higher chunk) one overrides; a non-default mount point prefixes entry names
    let a = vec![Ent { name: "Foo/Bar.txt", data: b"old".to_vec(), compress: false, encrypted: false }, Ent { name: "Only/InA.txt", data: b"a".to_vec(), compress: false, encrypted: false }];
    let b = vec![Ent { name: "Bar.txt", data: b"new".to_vec(), compress: true, encrypted: false }];
    let dir = tmp("set");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pakchunk1-WindowsClient.pak"), build_pak(3, "../../../", &a)).unwrap();
    std::fs::write(dir.join("pakchunk0_s1-WindowsClient.pak"), build_pak(4, "../../../Foo/", &b)).unwrap();
    let set = PakSet::open_dir(&dir).unwrap();
    // order: chunk0_s1 < chunk1  => chunk1 wins
    assert_eq!(set.paks()[0].name, "pakchunk0_s1-WindowsClient.pak");
    assert_eq!(&set.read("foo/BAR.txt").unwrap()[..], b"old");
    assert_eq!(&set.read("../../../Only\\InA.txt").unwrap()[..], b"a");
    assert_eq!(set.total_entries(), 3);
    assert_eq!(set.len(), 2);
    assert_eq!(set.glob("Foo/*.txt").len(), 1);
    assert_eq!(set.glob("**/*.txt").len(), 2);
    assert!(set.read("missing").is_err());
    // reversed priority: put the compressed one in a later chunk
    let dir2 = tmp("set2");
    std::fs::create_dir_all(&dir2).unwrap();
    std::fs::write(dir2.join("pakchunk0-WindowsClient.pak"), build_pak(3, "../../../", &a)).unwrap();
    std::fs::write(dir2.join("pakchunk0_s1-WindowsClient.pak"), build_pak(4, "../../../Foo/", &b)).unwrap();
    let set2 = PakSet::open_dir(&dir2).unwrap();
    assert_eq!(&set2.read("Foo/Bar.txt").unwrap()[..], b"new");
}

/// MEMORY + LOAD TIME: a second open of the same files shares the first one's index (same `PakFile`s) and reads the same; the mount stays for
/// the process, so an open after every set was dropped shares it too.
#[test]
fn reopening_shares_the_live_index() {
    let a = vec![Ent { name: "Foo/Bar.txt", data: b"shared".to_vec(), compress: true, encrypted: false }];
    let dir = tmp("share");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pakchunk0-WindowsClient.pak"), build_pak(3, "../../../", &a)).unwrap();
    let s1 = PakSet::open_dir(&dir).unwrap();
    let s2 = PakSet::open_install(&dir).unwrap();
    assert!(std::ptr::eq(s1.paks().as_ptr(), s2.paks().as_ptr()), "the live index is shared");
    assert_eq!(&s2.read("foo/bar.txt").unwrap()[..], b"shared");
    assert_eq!(s1.glob("**/*.txt").len(), s2.glob("**/*.txt").len());
    let p1 = s1.paks().as_ptr() as usize;
    drop(s1);
    drop(s2);
    let s3 = PakSet::open_dir(&dir).unwrap();
    assert_eq!(&s3.read("Foo/Bar.txt").unwrap()[..], b"shared");
    assert_eq!(s3.paks().as_ptr() as usize, p1, "the mount stays for the process");
}
