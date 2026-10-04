//! `*.fnaudiopack`: the sounds a game build needs, without the 1.8 paks.
//!
//! **Format.** A pack is a normal Unreal `.pak` (version 3, mount point `../../../`, every entry stored uncompressed and
//! unencrypted, SHA-1 fields zero) that holds only the files of the packages the exported SoundCues reach:
//! `FortniteGame/Content/<...>.uasset/.uexp/.ubulk`, byte for byte as they are in the install (Ogg/Vorbis and UE4OPUS payloads
//! stay in their original compressed form, nothing is decoded or re-encoded). Because it IS a pak, `fn_pak::PakSet` opens it
//! unchanged and every `fn-audio` loader (`Library`, `SoundClassTree::load`, `SoundMix::load`, `decode_soundwave`) works on it
//! with no code path of its own, so a render from the pack is bit-identical to a render from the install (checked by
//! `fn-audio-pack verify`). The `.fnaudiopack` extension only marks "audio subset, not a game pak" (the game's own paks are
//! never opened for it).
//!
//! **What goes in.** The closure over package imports of every cue given to [`collect`]: SoundCue -> SoundWave, SoundAttenuation
//! (-> CurveFloat), SoundConcurrency, SoundClass, DialogueWave (-> SoundWave), SoundMix; plus every package in
//! `/FortSoundClassesAndModes/` (the whole SoundClass tree and all SoundMixes: `SoundClassTree::load` scans that folder).

use fn_audio::events;
use fn_audio::cue::NodeKind;
use fn_audio::install::to_pak_asset_path;
use fn_audio::library::Library;
use fn_audio::{AudioError, Install, Result};
use fn_pak::PakSet;
use fn_uasset::{ObjectRef, PackageData, PackageIndex};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;
use std::path::Path;

/// Conventional file name of the pack inside a game's `assets/audio/` folder.
pub const PACK_NAME: &str = "audio.fnaudiopack";

/// Cues the old standalone runtime plays beyond `events::TABLE`.
pub const EXTRA_CUES: &[&str] = &[
    // sandbox tree = Athena_Tree_Large_1 -> `PlayHitSound` = oak impact (recovered, tree BP CDO)
    "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_OakTree_Impact_Cue",
    // Melee_Equip montage AnimNotify_PlaySound (recovered)
    "/Game/Sounds/Fort_GamePlay_Sounds/Player/Player_Equip_Melee_Cue",
];

/// Class names whose imports are followed while building the closure.
const FOLLOW: &[&str] = &["SoundWave", "SoundCue", "SoundAttenuation", "SoundConcurrency", "SoundClass", "SoundMix", "DialogueWave", "DialogueVoice", "CurveFloat"];

/// `events::TABLE` cues plus [`EXTRA_CUES`] plus `weapon_events::TABLE` (AR / pump shotgun / bolt sniper / healing consumables), de-duplicated, in table order.
pub fn default_cues() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for c in events::TABLE.iter().map(|e| e.cue).chain(EXTRA_CUES.iter().copied()).chain(fn_audio::weapon_events::TABLE.iter().map(|e| e.cue)).chain(fn_audio::music::PACK_CUES.iter().copied()) /* MUSIC HOOK */.chain(fn_audio::ambient::cue_paths().iter().copied()) /* AMBIENCE HOOK */ {
        if !v.contains(&c) {
            v.push(c);
        }
    }
    v
}

/// Result of [`collect`]: engine-relative pak paths (original casing) -> size, and the packages that were followed.
#[derive(Default, Debug)]
pub struct Closure {
    pub files: BTreeMap<String, u64>,
    pub packages: BTreeSet<String>,
    /// `(package, reason)` for referenced packages that could not be read (the pack is still written).
    pub problems: Vec<(String, String)>,
}

fn add_package_files(install: &Install, long: &str, out: &mut Closure) -> bool {
    let ap = to_pak_asset_path(long);
    let Some((_, e)) = install.paks.find(&ap) else { return false };
    let stem = e.rel_path().rsplit_once('.').map(|x| x.0.to_string()).unwrap_or_else(|| e.rel_path().to_string());
    for ext in ["uasset", "uexp", "ubulk"] {
        if let Some((_, e)) = install.paks.find(&format!("{stem}.{ext}")) {
            out.files.insert(e.rel_path().to_string(), e.size);
        }
    }
    true
}

/// Closure of everything the given cue packages (long names, `/Game/...`) need, plus the SoundClass/SoundMix folder.
/// SoundWaves are referenced from the cue graph by asset path (not through the import table), so they come from the parsed cue
/// (`SoundCue::wave_paths` and the dialogue nodes); attenuation, concurrency, SoundClass and curve assets come from imports.
pub fn collect(lib: &mut Library, cues: &[&str]) -> Closure {
    let mut out = Closure::default();
    let mut queue: VecDeque<String> = cues.iter().map(|c| c.split('.').next().unwrap_or(c).to_string()).collect();
    let mut leaves: BTreeSet<String> = BTreeSet::new();
    for c in cues {
        match lib.load_cue(c) {
            Ok(cue) => {
                let mut dialogue = Vec::new();
                if let Some(r) = &cue.root {
                    r.walk(&mut |n| {
                        if let NodeKind::Dialogue { dialogue_wave: Some(d) } = &n.kind {
                            dialogue.push(d.clone());
                        }
                    });
                }
                for d in dialogue {
                    queue.push_back(d.split('.').next().unwrap_or(&d).to_string());
                    if let Some(w) = lib.resolve_dialogue(&d) {
                        leaves.insert(w.to_ascii_lowercase());
                        queue.push_back(w);
                    }
                }
                for w in cue.wave_paths() {
                    let w = w.split('.').next().unwrap_or(&w).to_string();
                    leaves.insert(w.to_ascii_lowercase());
                    queue.push_back(w);
                }
            }
            Err(e) => out.problems.push((c.to_string(), e.to_string())),
        }
    }
    let install = &lib.install;
    for p in install.all_asset_paths().into_iter().filter(|p| p.contains("/FortSoundClassesAndModes/")) {
        if let Some(long) = fn_pak::pak_path_to_game(&p) {
            queue.push_back(long);
        }
    }
    while let Some(name) = queue.pop_front() {
        let key = name.to_ascii_lowercase();
        if !out.packages.insert(key) {
            continue;
        }
        if !add_package_files(install, &name, &mut out) {
            out.problems.push((name, "package not found".into()));
            continue;
        }
        if leaves.contains(&name.to_ascii_lowercase()) {
            continue;
        }
        let ap = to_pak_asset_path(&name);
        let data = match PackageData::load(&install.paks, &ap) {
            Ok(d) => d,
            Err(e) => {
                out.problems.push((name, e.to_string()));
                continue;
            }
        };
        let pkg = match data.package() {
            Ok(p) => p,
            Err(e) => {
                out.problems.push((name, e.to_string()));
                continue;
            }
        };
        for i in 0..pkg.imports.len() {
            if let ObjectRef::Import { class_name, package, .. } = pkg.resolve(PackageIndex(-(i as i32) - 1)) {
                if package.starts_with("/Game/") && FOLLOW.contains(&class_name.as_str()) {
                    queue.push_back(package);
                }
            }
        }
    }
    out
}

fn fstring(s: &str) -> Vec<u8> {
    let mut v = ((s.len() + 1) as i32).to_le_bytes().to_vec();
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}

/// Statistics of a written pack.
#[derive(Debug, Clone, Default)]
pub struct PackStats {
    pub entries: usize,
    pub bytes: u64,
    pub payload_bytes: u64,
}

/// Write `files` (engine-relative pak paths found in `install`) as a stored pak v3 at `out`.
pub fn write_pack(install: &Install, files: &BTreeMap<String, u64>, out: &Path) -> Result<PackStats> {
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let mut f = std::io::BufWriter::new(std::fs::File::create(out)?);
    let mut pos: u64 = 0;
    let mut index: Vec<u8> = Vec::new();
    index.extend(fstring("../../../"));
    index.extend((files.len() as i32).to_le_bytes());
    let mut payload_bytes = 0u64;
    for name in files.keys() {
        let data = install.paks.read(name)?;
        let mut h: Vec<u8> = Vec::with_capacity(53);
        h.extend(pos.to_le_bytes()); // offset of this header
        h.extend((data.len() as u64).to_le_bytes()); // compressed size (= size, stored)
        h.extend((data.len() as u64).to_le_bytes());
        h.extend(0u32.to_le_bytes()); // method: stored
        h.extend([0u8; 20]); // SHA-1 (not verified by the reader)
        h.push(0); // not encrypted
        h.extend(0u32.to_le_bytes()); // block size
        f.write_all(&h)?;
        f.write_all(&data)?;
        pos += h.len() as u64 + data.len() as u64;
        payload_bytes += data.len() as u64;
        index.extend(fstring(name));
        index.extend(&h);
    }
    let index_offset = pos;
    f.write_all(&index)?;
    let mut footer: Vec<u8> = Vec::new();
    footer.extend(0x5A6F12E1u32.to_le_bytes());
    footer.extend(3u32.to_le_bytes());
    footer.extend(index_offset.to_le_bytes());
    footer.extend((index.len() as u64).to_le_bytes());
    footer.extend([0u8; 20]);
    f.write_all(&footer)?;
    f.flush()?;
    Ok(PackStats { entries: files.len(), bytes: index_offset + index.len() as u64 + footer.len() as u64, payload_bytes })
}

/// Open a pack as an [`Install`] (use `Library::new(open_pack(p)?)`).
pub fn open_pack(path: impl AsRef<Path>) -> Result<Install> {
    let p = path.as_ref().to_path_buf();
    if !p.is_file() {
        return Err(AudioError::Malformed { what: "audio pack", detail: format!("{} is not a file", p.display()) });
    }
    Ok(Install { paks: PakSet::open_files(&[p])? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cues_cover_the_event_table_without_duplicates() {
        let c = default_cues();
        for e in events::TABLE {
            assert!(c.contains(&e.cue), "{}", e.event);
        }
        let mut s = c.clone();
        s.sort();
        s.dedup();
        assert_eq!(s.len(), c.len());
        assert!(c.iter().all(|x| x.starts_with("/Game/")));
    }

    #[test]
    fn fstring_is_nul_terminated_length_prefixed() {
        assert_eq!(fstring("ab"), vec![3, 0, 0, 0, b'a', b'b', 0]);
    }
}

