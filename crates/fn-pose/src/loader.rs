//! Asset loading helpers: the three character meshes (body, head, master) either from the old tree's glTF exports or straight from
//! cooked packages (raw files or the pak) through `fn-mesh`, and the SK_Pick handle positions for the grip diagnostics.
use crate::clip::ClipSourceDir;
use crate::part::Part;
use std::path::{Path, PathBuf};

pub const BODY_PKG: &str = "/Game/Characters/Player/Male/Medium/Bodies/M_Med_Soldier_04/Meshes/SK_M_Med_Soldier_04";
pub const HEAD_PKG: &str = "/Game/Athena/Heroes/Meshes/Heads/SK_M_MED_Head_01_ATH";
pub const MASTER_PKG: &str = "/Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player";
pub const PICK_PKG: &str = "/Game/Weapons/FORT_Melee/Meshes/SK_Pick";

/// All files below `root` (recursive), for case-insensitive lookup by file name.
pub struct AssetIndex { pub files: Vec<PathBuf> }
impl AssetIndex {
    pub fn scan(root: &Path) -> AssetIndex { let mut files = vec![]; walk(root, &mut files); AssetIndex { files } }
    pub fn find(&self, name: &str) -> Result<&Path, String> {
        self.files.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))).map(|p| p.as_path()).ok_or_else(|| format!("Missing asset: {name}"))
    }
}
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() { let p = e.path(); if p.is_dir() { walk(&p, out); } else { out.push(p); } }
}

/// The parts the old runtime poses, from the old tree's glTF exports: `[body, head, master]`. Body and head get the master's IK bones appended
/// (`Model::load`); the master is the "extra mesh sharing the master skeleton".
pub fn old_tree_parts(idx: &AssetIndex) -> Result<Vec<Part>, String> {
    let master = Part::from_gltf_path(idx.find("Fortnite_M_Avg_Player.gltf")?)?;
    let body = Part::from_gltf_path(idx.find("SK_M_Med_Soldier_04.gltf")?)?.with_master_ik_bones(&master.tree);
    let head = Part::from_gltf_path(idx.find("SK_M_MED_Head_01_ATH.gltf")?)?.with_master_ik_bones(&master.tree);
    Ok(vec![body, head, master])
}
/// Vertex positions of SK_Pick (old glTF export) for the handle-axis diagnostics.
pub fn old_tree_pick_positions(idx: &AssetIndex) -> Result<Vec<[f32; 3]>, String> {
    let p = Part::from_gltf_path(idx.find("SK_Pick.gltf")?)?;
    Ok(p.prims.iter().flat_map(|p| p.positions.iter().copied()).collect())
}

/// Parses a package by long name from raw files / the pak.
pub fn package(src: &ClipSourceDir, long: &str) -> Result<fn_ueio::Package, String> {
    match src {
        ClipSourceDir::RawPackages(root) => {
            let base = root.join("FortniteGame/Content").join(long.trim_start_matches("/Game/"));
            let rd = |e: &str| std::fs::read(format!("{}.{e}", base.display())).unwrap_or_default();
            let a = std::fs::read(format!("{}.uasset", base.display())).map_err(|e| format!("{}: {e}", base.display()))?;
            fn_ueio::Package::parse(&a, &rd("uexp"), &rd("ubulk")).map_err(|e| format!("{long}: {e}"))
        }
        ClipSourceDir::Pak(set) => {
            let a = set.read_game(long, "uasset").map_err(|e| format!("{long}: {e}"))?;
            let x = set.read_game(long, "uexp").map(|c| c.into_owned()).unwrap_or_default();
            let b = set.read_game(long, "ubulk").map(|c| c.into_owned()).unwrap_or_default();
            fn_ueio::Package::parse(&a, &x, &b).map_err(|e| format!("{long}: {e}"))
        }
        ClipSourceDir::FnAnimDir(_) => Err("a .fnanim directory has no meshes".into()),
    }
}

/// `[body, head, master]` decoded from cooked packages (LOD 0), identical to [`old_tree_parts`] up to the exporter's float formatting.
pub fn package_parts(src: &ClipSourceDir) -> Result<Vec<Part>, String> {
    let load = |long: &str, name: &str| -> Result<Part, String> {
        let pkg = package(src, long)?;
        let mesh = fn_mesh::SkeletalMesh::parse(&pkg).map_err(|e| format!("{long}: {e}"))?;
        Part::from_skeletal_mesh(name, &mesh, 0)
    };
    let master = load(MASTER_PKG, "Fortnite_M_Avg_Player")?;
    let body = load(BODY_PKG, "SK_M_Med_Soldier_04")?.with_master_ik_bones(&master.tree);
    let head = load(HEAD_PKG, "SK_M_MED_Head_01_ATH")?.with_master_ik_bones(&master.tree);
    Ok(vec![body, head, master])
}
pub fn package_pick_positions(src: &ClipSourceDir) -> Result<Vec<[f32; 3]>, String> {
    let pkg = package(src, PICK_PKG)?;
    let mesh = fn_mesh::SkeletalMesh::parse(&pkg).map_err(|e| e.to_string())?;
    let p = Part::from_skeletal_mesh("SK_Pick", &mesh, 0)?;
    Ok(p.prims.iter().flat_map(|p| p.positions.iter().copied()).collect())
}

/// `%FN_TESTDATA%` or `%LOCALAPPDATA%\Temp\fn-testdata` (extracted raw packages) when it exists.
pub fn default_package_root() -> Option<PathBuf> {
    let p = std::env::var_os("FN_TESTDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("Temp").join("fn-testdata"));
    if p.join("FortniteGame/Content").is_dir() { Some(p) } else { None }
}
