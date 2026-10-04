//! Asset folder of the prototype (`..\FortniteRuntime\assets`): file index + material (.mat) slot parsing.
use std::path::{Path, PathBuf};

/// Default asset root: `<workspace>/../FortniteRuntime/assets` (workspace = FortniteRuntime-rs), overridable with `--assets` / `FN_ASSETS`.
pub fn default_assets_root() -> PathBuf {
    if let Ok(p) = std::env::var("FN_ASSETS") { return PathBuf::from(p); }
    let candidates = [
        PathBuf::from(r"..\FortniteRuntime\assets"),
        PathBuf::from("../FortniteRuntime/assets"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../FortniteRuntime/assets"),
    ];
    candidates.into_iter().find(|p| p.exists()).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../FortniteRuntime/assets"))
}

/// Flat index of every file under the asset root (the prototype finds assets by file name).
pub struct AssetIndex {
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
}
impl AssetIndex {
    pub fn scan(root: &Path) -> Result<AssetIndex, String> {
        if !root.exists() { return Err(format!("asset folder {} does not exist (use --assets or FN_ASSETS)", root.display())); }
        let mut files = vec![];
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
                let p = entry.path();
                if p.is_dir() { stack.push(p); } else { files.push(p); }
            }
        }
        Ok(AssetIndex { root: root.to_path_buf(), files })
    }
    /// Case-insensitive lookup by file name.
    pub fn find(&self, name: &str) -> Result<&Path, String> {
        self.files.iter().find(|p| p.file_name().map_or(false, |n| n.to_string_lossy().eq_ignore_ascii_case(name))).map(|p| p.as_path()).ok_or_else(|| format!("missing asset: {name}"))
    }
    pub fn load_rgba(&self, name: &str) -> Result<image::RgbaImage, String> {
        let p = self.find(name)?;
        Ok(image::open(p).map_err(|e| format!("{}: {e}", p.display()))?.to_rgba8())
    }
}

/// Texture names of one exported `.mat` file.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct MatSlots { pub diffuse: Option<String>, pub normal: Option<String>, pub spec: Option<String>, pub mask: Option<String> }
/// Slots come from `Diffuse=`, `Normal=`, `SpecPower=` (= the UE "SpecularMasks" texture) and the `Other[n]=` entry ending in `_M`
/// (= the "M" mask). Which channel means what is inferred in the shader, not recovered.
pub fn parse_mat(text: &str) -> MatSlots {
    let mut s = MatSlots::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().to_owned();
        match key.trim() {
            "Diffuse" => s.diffuse = Some(value),
            "Normal" => s.normal = Some(value),
            "SpecPower" => s.spec = Some(value),
            k if k.starts_with("Other[") && (value.ends_with("_M") || value.ends_with("_m")) => s.mask = Some(value),
            _ => {}
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_character_slots() {
        let s = parse_mat("Diffuse=A_C\nNormal=A_N\nSpecPower=A_S\nOther[0]=A_M\nOther[1]=T_NoisePack_01\n");
        assert_eq!(s, MatSlots { diffuse: Some("A_C".into()), normal: Some("A_N".into()), spec: Some("A_S".into()), mask: Some("A_M".into()) });
    }
    #[test]
    fn noise_pack_is_not_a_mask() { assert_eq!(parse_mat("Diffuse=X\nOther[0]=T_NoisePack_01\n").mask, None); }
}
