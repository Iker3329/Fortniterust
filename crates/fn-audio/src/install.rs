//! Reading audio packages out of a mounted Fortnite install (never copies or writes game data).

use crate::codec::decode_asset;
use crate::error::{AudioError, Result};
use crate::soundwave::{parse_soundwave, SoundWaveAsset};
use crate::wave::DecodedWave;
use fn_pak::PakSet;
use fn_uasset::PackageData;
use std::path::Path;

pub struct Install {
    pub paks: PakSet,
}

/// Normalise anything the user may type to a pak path of the `.uasset`
/// (`FortniteGame/Content/...`), accepting `/Game/...` long names with or without `.ext`.
pub fn to_pak_asset_path(p: &str) -> String {
    if p.starts_with("/Game/") || p.starts_with("/Engine/") {
        let long = p.split('.').next().unwrap_or(p);
        fn_pak::game_path_to_pak(long, "uasset").unwrap_or_else(|| p.to_string())
    } else if p.to_ascii_lowercase().ends_with(".uasset") {
        p.to_string()
    } else {
        format!("{p}.uasset")
    }
}

/// Read the optional `.ubulk` sibling of a `.uasset` pak path.
pub fn read_ubulk(set: &PakSet, asset_path: &str) -> Vec<u8> {
    let stem = asset_path.rsplit_once('.').map(|x| x.0).unwrap_or(asset_path);
    set.read(&format!("{stem}.ubulk")).map(|c| c.into_owned()).unwrap_or_default()
}

impl Install {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Install { paks: PakSet::open_install(path)? })
    }

    /// Every effective `.uasset` pak path whose directory looks like audio (`Sounds`, `Audio`).
    pub fn sound_paths(&self) -> Vec<String> {
        self.paks
            .effective()
            .filter_map(|(_, e)| {
                let p = e.rel_path();
                let l = p.to_ascii_lowercase();
                (l.ends_with(".uasset") && (l.contains("/sound") || l.contains("/audio"))).then(|| p.to_owned())
            })
            .collect()
    }

    /// Every `.uasset` pak path (for whole-install sweeps).
    pub fn all_asset_paths(&self) -> Vec<String> {
        self.paks.effective().filter_map(|(_, e)| e.rel_path().ends_with(".uasset").then(|| e.rel_path().to_owned())).collect()
    }

    /// Parse the (first) `SoundWave` export of a package.
    pub fn load_soundwave(&self, path: &str) -> Result<SoundWaveAsset> {
        let ap = to_pak_asset_path(path);
        let data = PackageData::load(&self.paks, &ap)?;
        let pkg = data.package()?;
        let idx = pkg.exports_of_class("SoundWave").next().ok_or_else(|| AudioError::Malformed { what: "package", detail: format!("{ap} has no SoundWave export") })?;
        let ub = read_ubulk(&self.paks, &ap);
        parse_soundwave(&pkg, idx, &ub)
    }

    /// Parse + decode a SoundWave package to f32.
    pub fn decode_soundwave(&self, path: &str) -> Result<(DecodedWave, SoundWaveAsset)> {
        let a = self.load_soundwave(path)?;
        let w = decode_asset(&a)?;
        Ok((w, a))
    }
}
