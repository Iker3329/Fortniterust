//! Package access: pak set + raw `.uasset/.uexp/.ubulk` loading by long package path (`/Game/...`).
use fn_pak::PakSet;
use fn_ueio::Package;
use std::path::Path;

pub struct Source {
    pub set: PakSet,
}

pub struct Raw {
    pub uasset: Vec<u8>,
    pub uexp: Vec<u8>,
    pub ubulk: Vec<u8>,
}

/// Manifest package names are `Game/...` or `Engine/...` (no leading slash).
pub fn long_name(pkg: &str) -> String {
    if pkg.starts_with('/') {
        pkg.to_string()
    } else {
        format!("/{pkg}")
    }
}

impl Source {
    /// `dir` may be the `Paks` directory or any directory above it.
    pub fn open(dir: &Path) -> Result<Source, String> {
        let set = PakSet::open_install(dir).map_err(|e| format!("cannot open paks at {}: {e}", dir.display()))?;
        Ok(Source { set })
    }

    pub fn exists(&self, long: &str) -> bool {
        fn_pak::game_path_to_pak(long, "uasset").map_or(false, |p| self.set.contains(&p))
    }

    pub fn load_raw(&self, long: &str) -> Result<Raw, String> {
        let long = long_name(long);
        let uasset = self.set.read_game(&long, "uasset").map_err(|e| format!("{long}.uasset: {e}"))?.into_owned();
        let uexp = self.set.read_game(&long, "uexp").map(|c| c.into_owned()).unwrap_or_default();
        let ubulk = self.set.read_game(&long, "ubulk").map(|c| c.into_owned()).unwrap_or_default();
        Ok(Raw { uasset, uexp, ubulk })
    }

    pub fn package(&self, long: &str) -> Result<Package, String> {
        let r = self.load_raw(long)?;
        Package::parse(&r.uasset, &r.uexp, &r.ubulk).map_err(|e| format!("{long}: {e}"))
    }
}
