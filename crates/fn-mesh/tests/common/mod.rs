//! Shared helpers for the verification tests: locating the extracted test packages and the old UEViewer exports,
//! and a tiny glTF accessor reader.
#![allow(dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};

/// Root of the extracted `FortniteGame/Content` test packages (`FN_TESTDATA` overrides the default).
pub fn testdata() -> Option<PathBuf> {
    let root = std::env::var("FN_TESTDATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(r"C:\Users\Brady\AppData\Local\Temp\fn-testdata"));
    let content = root.join("FortniteGame").join("Content");
    if content.is_dir() {
        Some(content)
    } else {
        None
    }
}

/// Root of the old UEViewer exports (`assets/exported/Game`) (`FN_OLD_EXPORTS` overrides the default).
pub fn old_exports() -> Option<PathBuf> {
    let p = std::env::var("FN_OLD_EXPORTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Users\Brady\Documents\Codex\2026-10-02\im-seeing-titkoks-where-people-are-2\FortniteRuntime\assets\exported\Game"));
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

/// Root of the old sky assets (`assets/sky`).
pub fn old_sky() -> Option<PathBuf> {
    let p = std::env::var("FN_OLD_SKY")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Users\Brady\Documents\Codex\2026-10-02\im-seeing-titkoks-where-people-are-2\FortniteRuntime\assets\sky"));
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

pub fn load_package(base: &Path) -> fn_ueio::Package {
    let rd = |e: &str| std::fs::read(base.with_extension(e)).unwrap_or_default();
    fn_ueio::Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")).unwrap_or_else(|e| panic!("{}: {e}", base.display()))
}

/// A glTF document + its .bin.
pub struct Gltf {
    pub json: Value,
    pub bin: Vec<u8>,
}

impl Gltf {
    pub fn from_parts(json: &str, bin: Vec<u8>) -> Gltf {
        Gltf { json: serde_json::from_str(json).expect("valid gltf json"), bin }
    }
    pub fn load(base: &Path) -> Gltf {
        let json = std::fs::read_to_string(base.with_extension("gltf")).unwrap();
        let bin = std::fs::read(base.with_extension("bin")).unwrap();
        Gltf::from_parts(&json, bin)
    }
    pub fn accessor(&self, i: usize) -> (usize, usize, u64, usize) {
        let a = &self.json["accessors"][i];
        let ncomp = match a["type"].as_str().unwrap() {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            t => panic!("type {t}"),
        };
        let view = &self.json["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        (ncomp, a["count"].as_u64().unwrap() as usize, a["componentType"].as_u64().unwrap(), view["byteOffset"].as_u64().unwrap_or(0) as usize)
    }
    /// All components as f64 (any component type, no normalisation applied).
    pub fn values(&self, i: usize) -> (usize, Vec<f64>) {
        let (ncomp, count, comp, off) = self.accessor(i);
        let size = match comp {
            5126 | 5125 => 4,
            5123 | 5122 => 2,
            _ => 1,
        };
        let mut out = Vec::with_capacity(ncomp * count);
        for k in 0..ncomp * count {
            let o = off + k * size;
            let b = &self.bin[o..o + size];
            out.push(match comp {
                5126 => f32::from_le_bytes(b.try_into().unwrap()) as f64,
                5125 => u32::from_le_bytes(b.try_into().unwrap()) as f64,
                5123 => u16::from_le_bytes(b.try_into().unwrap()) as f64,
                5121 => b[0] as f64,
                c => panic!("component {c}"),
            });
        }
        (ncomp, out)
    }
}

/// Maximum absolute difference of two equally long slices (infinity when the lengths differ).
pub fn max_abs_err(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() {
        return f64::INFINITY;
    }
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max)
}
