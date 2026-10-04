//! Resolving shader specs: a `.dxbc` file, an inflated dump, or `<package>#<id>` (package = game path such as
//! `/Game/UI/Foundation/Border/Materials/M-Border-1pxOutline`, or `GlobalShaderCache`; id = zlib stream offset as printed by `extract`).

use crate::extract::{scan_blobs, Blob};
use fn_pak::PakSet;
use std::path::{Path, PathBuf};

pub const GLOBAL_CACHE: &str = "Engine/GlobalShaderCache-PCD3D_SM5.bin";

pub fn default_install() -> PathBuf {
    std::env::var_os("FN_INSTALL").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame"))
}

/// The file whose zlib streams hold the shaders of `package`.
pub fn package_bytes(set: &PakSet, package: &str) -> Result<Vec<u8>, String> {
    if package.eq_ignore_ascii_case("GlobalShaderCache") || package.to_ascii_lowercase().contains("globalshadercache") {
        return set.read(GLOBAL_CACHE).map(|c| c.into_owned()).map_err(|e| format!("{GLOBAL_CACHE}: {e}"));
    }
    if let Ok(b) = set.read_game(package, "uexp") { return Ok(b.into_owned()); }
    set.read_game(package, "uasset").map(|c| c.into_owned()).map_err(|e| format!("{package}: {e}"))
}

pub fn load_blob(install: &Path, spec: &str) -> Result<Blob, String> {
    if let Some((pkg, id)) = spec.rsplit_once('#') {
        let id: usize = id.trim_start_matches("shader_").parse().map_err(|_| format!("bad shader id in {spec}"))?;
        let set = PakSet::open_install(install).map_err(|e| e.to_string())?;
        let bytes = package_bytes(&set, pkg)?;
        return scan_blobs(&bytes).into_iter().find(|b| b.id == id).ok_or_else(|| format!("{pkg}: no shader stream at {id}"));
    }
    let bytes = std::fs::read(spec).map_err(|e| format!("{spec}: {e}"))?;
    if bytes.starts_with(b"DXBC") { return Ok(Blob { id: 0, prefix: 0, header: vec![], trailer: vec![], hash_was_zero: false, bytes }); }
    scan_blobs(&bytes).into_iter().next().ok_or_else(|| format!("{spec}: no DXBC found"))
}
