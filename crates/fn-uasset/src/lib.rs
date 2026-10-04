//! # fn-uasset
//!
//! Pure-Rust parser for Unreal Engine 4.16 cooked packages (`.uasset` + `.uexp`) as shipped in Fortnite 1.8
//! (and, with `ParseOptions { assumed_ue4_version: 514 }`, Fortnite 1.10). No unsafe code, no dependencies beyond
//! `serde_json` (JSON export) and `fn-pak` (loading from paks).
//!
//! * [`Package`] - header: summary (file/licensee version, custom versions), name map, import/export maps,
//!   depends, preload dependencies, soft package references, thumbnail table (index only, image data is never
//!   read), object path resolution ([`Package::object_path`], [`Package::resolve`] -> [`ObjectRef`]),
//!   class helpers ([`Package::class_export`], [`Package::super_class`], [`Package::default_object_export`]).
//! * [`Package::read_export`] / [`ExportObject`] - tagged-property reader (reflection-light: every value is
//!   decoded from its own `FPropertyTag`; anything undecodable becomes [`Value::Unparsed`] with the raw bytes and
//!   the reason, and reading continues with the next tag). Every [`Property`] also carries the byte offset of its
//!   tag in `uasset ++ uexp`.
//! * [`Value`] / [`PropertyList`] - generic value tree with typed accessors and JSON export (`to_json`).
//!   Covers bool/int/float/double, name/str/text, object/class/weak/lazy/soft references, enum/byte, arrays,
//!   sets, maps, delegates, and structs (tagged *and* the native-serialized ones: Vector, Rotator, Quat,
//!   LinearColor, Color, Transform, Guid, RichCurveKey, GameplayTagContainer, material pin inputs, SmartName,
//!   Sequencer segments/templates, ... see [`native`]).
//! * [`classes`] - native payload of reflection exports: `UClass`/Blueprint/Widget/AnimBlueprint generated
//!   classes, `UFunction` (script bytes), `UProperty` declarations, `UEnum`, `UScriptStruct`, `DataTable` and
//!   `CurveTable` rows.
//! * [`kismet`] - Kismet bytecode reader/disassembler: [`kismet::disassemble`] / [`kismet::parse`] give an
//!   [`kismet::Expr`] AST per statement and the Node tool's exact text format via [`kismet::Expr::render`].
//! * [`PackageData`] - convenience loader that pulls `.uasset`/`.uexp` out of a [`fn_pak::PakSet`].
//!
//! ## Format facts worth knowing
//!
//! * Cooked packages are **unversioned**: `FileVersionUE4 == 0`, no custom versions, empty engine version. The
//!   reader substitutes [`package::DEFAULT_ASSUMED_UE4_VERSION`] (513) for 1.8; 1.10 needs 514 (soft object
//!   paths are `FName + FString` from 514, `UField::Next` is gone and `UStruct::Children` becomes an array).
//! * Export `SerialOffset`s address the concatenation `uasset ++ uexp` (offsets already include the header size).
//! * Every export except class default objects carries a 4-byte `bHasGuid` flag right after the property list.
//! * Soft object references in 1.8 are plain `FString`s (`AssetObjectProperty`), never in the name table.
//! * `FRichCurveKey` is a 27-byte binary struct, `GameplayTagContainer` is `i32 n + n x FName`, material
//!   `ExpressionInput`s are native, and `TMap` struct values do not name their struct in the property tag.
//!
//! ```no_run
//! use fn_pak::PakSet;
//! use fn_uasset::PackageData;
//! let set = PakSet::open_install(r"C:\path\to\FortniteGame").unwrap();
//! let data = PackageData::load(&set, "/Game/TimeOfDay/TODM/TODM_A_Child_Athena").unwrap();
//! let pkg = data.package().unwrap();
//! let cdo = pkg.read_default_object().unwrap().unwrap();
//! println!("{}", cdo.to_json(&pkg));
//! ```
pub mod classes;
pub mod error;
pub mod kismet;
pub mod native;
pub mod object;
pub mod package;
pub mod props;
pub mod reader;
pub mod value;

pub use error::{Error, ErrorKind, Result};
pub use object::ExportObject;
pub use package::{Export, Import, ObjectRef, Package, PackageIndex, ParseOptions, Summary};
pub use reader::Reader;
pub use value::{Property, PropertyList, Text, Value};

use std::borrow::Cow;

/// Owned/borrowed file bytes of one package (`.uasset` + optional `.uexp`), loaded from a pak set.
pub struct PackageData<'a> {
    /// Long package name (`/Game/...`).
    pub name: String,
    pub asset: Cow<'a, [u8]>,
    pub uexp: Cow<'a, [u8]>,
}

impl<'a> PackageData<'a> {
    /// Load by long package path (`/Game/Foo/Bar`, `/Engine/...`) or by pak path of the `.uasset`
    /// (`FortniteGame/Content/Foo/Bar.uasset`).
    pub fn load(set: &'a fn_pak::PakSet, path: &str) -> Result<PackageData<'a>> {
        let (long, asset_path) = if path.starts_with("/Game/") || path.starts_with("/Engine/") {
            let ap = fn_pak::game_path_to_pak(path, "uasset").ok_or_else(|| Error::not_found(path.to_string()))?;
            (path.split('.').next().unwrap_or(path).to_string(), ap)
        } else {
            let lower = path.to_lowercase();
            let ap = if lower.ends_with(".uasset") || lower.ends_with(".umap") { path.to_string() } else { format!("{path}.uasset") };
            (fn_pak::pak_path_to_game(&ap).unwrap_or_default(), ap)
        };
        let mut asset_path = asset_path;
        if !set.contains(&asset_path) {
            if let Some(stem) = asset_path.strip_suffix(".uasset") {
                let m = format!("{stem}.umap");
                if set.contains(&m) {
                    asset_path = m;
                }
            }
        }
        let asset = set.read(&asset_path).map_err(|e| Error::not_found(format!("{asset_path}: {e}")))?;
        let stem = asset_path.rsplit_once('.').map(|x| x.0).unwrap_or(&asset_path);
        let uexp = match set.read(&format!("{stem}.uexp")) {
            Ok(b) => b,
            Err(_) => Cow::Borrowed(&[][..]),
        };
        Ok(PackageData { name: long, asset, uexp })
    }

    /// Parse the header (borrowing from this object).
    pub fn package(&self) -> Result<Package<'_>> {
        Ok(Package::parse(&self.asset, &self.uexp)?.with_name(self.name.clone()))
    }
}



