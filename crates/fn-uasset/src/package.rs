//! UE 4.16 package header: summary, name map, import/export maps, depends, thumbnails.
//!
//! A cooked package is two files: `Foo.uasset` (header: summary + tables, `TotalHeaderSize` bytes) and
//! `Foo.uexp` (export payloads). **Export `SerialOffset`s address the concatenation `uasset ++ uexp`**, so a
//! [`Package`] borrows both slices and maps offsets across the boundary without copying.

use crate::error::{Error, ErrorKind, Result};
use crate::reader::Reader;

pub const PACKAGE_TAG: u32 = 0x9E2A_83C1;

/// `EUnrealEngineObjectUE4Version` values this parser branches on.
pub mod ver {
    pub const SERIALIZE_TEXT_IN_PACKAGES: i32 = 459;
    pub const ADD_STRING_ASSET_REFERENCES_MAP: i32 = 384;
    pub const ADDED_SEARCHABLE_NAMES: i32 = 510;
    pub const ENGINE_VERSION_OBJECT: i32 = 336;
    pub const PACKAGE_SUMMARY_HAS_COMPATIBLE_ENGINE_VERSION: i32 = 444;
    pub const WORLD_LEVEL_INFO: i32 = 224;
    pub const CHANGED_CHUNKID_TO_BE_AN_ARRAY_OF_CHUNKIDS: i32 = 326;
    pub const ADDED_CHUNKID_TO_ASSETDATA_AND_UPACKAGE: i32 = 278;
    pub const PRELOAD_DEPENDENCIES_IN_COOKED_EXPORTS: i32 = 507;
    pub const TEMPLATE_INDEX_IN_COOKED_EXPORTS: i32 = 508;
    pub const NAME_HASHES_SERIALIZED: i32 = 504;
    pub const SIXTY_FOUR_BIT_EXPORTMAP_SERIALSIZES: i32 = 326;
    pub const LOAD_FOR_EDITOR_GAME: i32 = 365;
    pub const COOKED_ASSETS_IN_EDITOR_SUPPORT: i32 = 485;
    pub const STRUCT_GUID_IN_PROPERTY_TAG: i32 = 441;
    pub const PROPERTY_GUID_IN_PROPERTY_TAG: i32 = 503;
    pub const ARRAY_PROPERTY_INNER_TAGS: i32 = 282;
    pub const PROPERTY_TAG_SET_MAP_SUPPORT: i32 = 509;
    pub const FTEXT_HISTORY: i32 = 354;
    pub const SERIALIZE_RICH_CURVE_KEY: i32 = 399;
    pub const ADDED_SOFT_OBJECT_PATH: i32 = 514;
}

/// Index into the import/export tables: `0` = null, `>0` = export `n-1`, `<0` = import `-n-1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct PackageIndex(pub i32);

impl PackageIndex {
    pub fn is_null(self) -> bool {
        self.0 == 0
    }
    pub fn is_import(self) -> bool {
        self.0 < 0
    }
    pub fn is_export(self) -> bool {
        self.0 > 0
    }
    pub fn import_index(self) -> Option<usize> {
        if self.0 < 0 {
            Some((-(self.0 as i64) - 1) as usize)
        } else {
            None
        }
    }
    pub fn export_index(self) -> Option<usize> {
        if self.0 > 0 {
            Some(self.0 as usize - 1)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EngineVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    pub changelist: u32,
    pub branch: String,
}

#[derive(Clone, Debug)]
pub struct CustomVersion {
    pub guid: [u8; 16],
    pub version: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub legacy_file_version: i32,
    pub legacy_ue3_version: i32,
    pub file_version_ue4: i32,
    pub file_version_licensee_ue4: i32,
    pub custom_versions: Vec<CustomVersion>,
    pub total_header_size: i32,
    pub folder_name: String,
    pub package_flags: u32,
    pub name_count: i32,
    pub name_offset: i32,
    pub gatherable_text_data_count: i32,
    pub gatherable_text_data_offset: i32,
    pub export_count: i32,
    pub export_offset: i32,
    pub import_count: i32,
    pub import_offset: i32,
    pub depends_offset: i32,
    pub string_asset_references_count: i32,
    pub string_asset_references_offset: i32,
    pub searchable_names_offset: i32,
    pub thumbnail_table_offset: i32,
    pub guid: [u8; 16],
    pub generations: Vec<(i32, i32)>,
    pub saved_by_engine_version: EngineVersion,
    pub compatible_with_engine_version: EngineVersion,
    pub compression_flags: u32,
    pub package_source: u32,
    pub additional_packages_to_cook: Vec<String>,
    pub num_texture_allocations: i32,
    pub asset_registry_data_offset: i32,
    pub bulk_data_start_offset: i64,
    pub world_tile_info_data_offset: i32,
    pub chunk_ids: Vec<i32>,
    pub preload_dependency_count: i32,
    pub preload_dependency_offset: i32,
}

#[derive(Clone, Debug)]
pub struct Import {
    pub class_package: String,
    pub class_name: String,
    pub outer: PackageIndex,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Export {
    pub class_index: PackageIndex,
    pub super_index: PackageIndex,
    pub template_index: PackageIndex,
    pub outer_index: PackageIndex,
    pub name: String,
    pub object_flags: u32,
    pub serial_size: i64,
    pub serial_offset: i64,
    pub forced_export: bool,
    pub not_for_client: bool,
    pub not_for_server: bool,
    pub package_guid: [u8; 16],
    pub package_flags: u32,
    pub not_always_loaded_for_editor_game: bool,
    pub is_asset: bool,
    pub first_export_dependency: i32,
    pub serialization_before_serialization: i32,
    pub create_before_serialization: i32,
    pub serialization_before_create: i32,
    pub create_before_create: i32,
}

#[derive(Clone, Debug)]
pub struct ThumbnailEntry {
    pub class_name: String,
    pub object_path: String,
    pub file_offset: i32,
}

pub const RF_CLASS_DEFAULT_OBJECT: u32 = 0x0000_0010;

/// A resolved object reference (see [`Package::resolve`]).
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectRef {
    Null,
    /// An object living in another package.
    Import {
        index: usize,
        name: String,
        /// Class name of the imported object (`BlueprintGeneratedClass`, `Texture2D`, ...).
        class_name: String,
        /// Class package (`/Script/Engine`, `/Script/CoreUObject`).
        class_package: String,
        /// The package that contains it (`/Game/Characters/Foo`, `/Script/Engine`).
        package: String,
        /// Full path (`/Game/Foo/Bar.Bar_C`).
        path: String,
    },
    /// An object defined in this package.
    Export { index: usize, name: String, class_name: String, path: String },
}

impl ObjectRef {
    pub fn path(&self) -> &str {
        match self {
            ObjectRef::Null => "",
            ObjectRef::Import { path, .. } | ObjectRef::Export { path, .. } => path,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            ObjectRef::Null => "",
            ObjectRef::Import { name, .. } | ObjectRef::Export { name, .. } => name,
        }
    }
}

/// Parse options.
#[derive(Clone, Copy, Debug)]
pub struct ParseOptions {
    /// `EUnrealEngineObjectUE4Version` assumed for unversioned (FileVersionUE4 == 0) packages. Default
    /// [`DEFAULT_ASSUMED_UE4_VERSION`] (UE 4.16/4.17 era, right for Fortnite 1.8).
    pub assumed_ue4_version: i32,
}
impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions { assumed_ue4_version: DEFAULT_ASSUMED_UE4_VERSION }
    }
}
/// Fortnite 1.8 cooked packages are unversioned; their layout matches `VER_UE4_LEVEL_STREAMING_DRAW_COLOR_TYPE_CHANGE`
/// (513): preload dependencies, template index, set/map tags, searchable names, 64-bit export sizes; soft object
/// paths are still plain strings (< 514).
pub const DEFAULT_ASSUMED_UE4_VERSION: i32 = 513;

/// A parsed package header borrowing its file bytes.
pub struct Package<'a> {
    pub summary: Summary,
    pub names: Vec<String>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    /// Per export: packages it depends on (empty in cooked output).
    pub depends: Vec<Vec<PackageIndex>>,
    pub soft_package_references: Vec<String>,
    pub preload_dependencies: Vec<PackageIndex>,
    pub thumbnails: Vec<ThumbnailEntry>,
    /// Non-fatal oddities found while parsing the header.
    pub warnings: Vec<String>,
    /// Long package name (`/Game/...`) if known, else empty. Used by object-path resolution.
    pub name: String,
    /// The UE4 file version used for format decisions (`file_version_ue4`, or the assumed one if unversioned).
    pub effective_version: i32,
    pub(crate) asset: &'a [u8],
    pub(crate) uexp: &'a [u8],
}

impl<'a> Package<'a> {
    /// Parse a package from its `.uasset` bytes and the `.uexp` bytes (may be empty when the package is not split).
    pub fn parse(asset: &'a [u8], uexp: &'a [u8]) -> Result<Package<'a>> {
        Self::parse_with(asset, uexp, ParseOptions::default())
    }

    /// Like [`Package::parse`] with explicit options (see [`ParseOptions`]).
    pub fn parse_with(asset: &'a [u8], uexp: &'a [u8], opts: ParseOptions) -> Result<Package<'a>> {
        let mut r = Reader::new(asset);
        let tag = r.u32()?;
        if tag != PACKAGE_TAG {
            return Err(Error::new(ErrorKind::BadMagic, format!("bad package tag {tag:#x}")));
        }
        let mut s = Summary { legacy_file_version: r.i32()?, ..Default::default() };
        if s.legacy_file_version >= 0 || s.legacy_file_version > -6 {
            return Err(Error::new(ErrorKind::UnsupportedVersion, format!("legacy file version {}", s.legacy_file_version)));
        }
        s.legacy_ue3_version = r.i32()?;
        s.file_version_ue4 = r.i32()?;
        s.file_version_licensee_ue4 = r.i32()?;
        let ncv = r.count(20)?;
        for _ in 0..ncv {
            let guid = r.guid()?;
            let version = r.i32()?;
            s.custom_versions.push(CustomVersion { guid, version });
        }
        // Cooked Fortnite packages are *unversioned*: FileVersionUE4 == 0 and the loader assumes the engine's
        // current version. We substitute `opts.assumed_ue4_version`.
        let v = if s.file_version_ue4 == 0 { opts.assumed_ue4_version } else { s.file_version_ue4 };
        s.total_header_size = r.i32()?;
        s.folder_name = r.fstring()?;
        s.package_flags = r.u32()?;
        s.name_count = r.i32()?;
        s.name_offset = r.i32()?;
        if v >= ver::SERIALIZE_TEXT_IN_PACKAGES {
            s.gatherable_text_data_count = r.i32()?;
            s.gatherable_text_data_offset = r.i32()?;
        }
        s.export_count = r.i32()?;
        s.export_offset = r.i32()?;
        s.import_count = r.i32()?;
        s.import_offset = r.i32()?;
        s.depends_offset = r.i32()?;
        if v >= ver::ADD_STRING_ASSET_REFERENCES_MAP {
            s.string_asset_references_count = r.i32()?;
            s.string_asset_references_offset = r.i32()?;
        }
        if v >= ver::ADDED_SEARCHABLE_NAMES {
            s.searchable_names_offset = r.i32()?;
        }
        s.thumbnail_table_offset = r.i32()?;
        s.guid = r.guid()?;
        let ngen = r.count(8)?;
        for _ in 0..ngen {
            let ec = r.i32()?;
            let nc = r.i32()?;
            s.generations.push((ec, nc));
        }
        let read_ev = |r: &mut Reader| -> Result<EngineVersion> {
            Ok(EngineVersion { major: r.u16()?, minor: r.u16()?, patch: r.u16()?, changelist: r.u32()?, branch: r.fstring()? })
        };
        if v >= ver::ENGINE_VERSION_OBJECT {
            s.saved_by_engine_version = read_ev(&mut r)?;
        } else {
            s.saved_by_engine_version.changelist = r.u32()?;
        }
        if v >= ver::PACKAGE_SUMMARY_HAS_COMPATIBLE_ENGINE_VERSION {
            s.compatible_with_engine_version = read_ev(&mut r)?;
        }
        s.compression_flags = r.u32()?;
        let ncc = r.i32()?;
        if ncc != 0 {
            return Err(Error::unsupported("package has compressed chunks"));
        }
        s.package_source = r.u32()?;
        let nap = r.count(4)?;
        for _ in 0..nap {
            s.additional_packages_to_cook.push(r.fstring()?);
        }
        if s.legacy_file_version > -7 {
            s.num_texture_allocations = r.i32()?;
        }
        s.asset_registry_data_offset = r.i32()?;
        s.bulk_data_start_offset = r.i64()?;
        if v >= ver::WORLD_LEVEL_INFO {
            s.world_tile_info_data_offset = r.i32()?;
        }
        if v >= ver::CHANGED_CHUNKID_TO_BE_AN_ARRAY_OF_CHUNKIDS {
            let n = r.count(4)?;
            for _ in 0..n {
                s.chunk_ids.push(r.i32()?);
            }
        } else if v >= ver::ADDED_CHUNKID_TO_ASSETDATA_AND_UPACKAGE {
            s.chunk_ids.push(r.i32()?);
        }
        if v >= ver::PRELOAD_DEPENDENCIES_IN_COOKED_EXPORTS {
            s.preload_dependency_count = r.i32()?;
            s.preload_dependency_offset = r.i32()?;
        }

        // ---- name map
        let mut warnings = Vec::new();
        // sanity: every table entry occupies at least this many bytes of the header, so counts are bounded by its size
        if s.name_count < 0 || s.name_offset < 0 || s.name_offset as usize > asset.len() || s.name_count as usize > asset.len() / 5 {
            return Err(Error::malformed("bad name map location"));
        }
        let mut names = Vec::with_capacity(s.name_count as usize);
        let mut nr = Reader::at(asset, s.name_offset as usize);
        for _ in 0..s.name_count {
            names.push(nr.fstring()?);
            if v >= ver::NAME_HASHES_SERIALIZED {
                nr.skip(4)?;
            }
        }
        let name_of = |idx: i32, num: i32| -> Result<String> {
            let base = names.get(idx as usize).ok_or_else(|| Error::malformed(format!("name index {idx} out of range")))?;
            Ok(if num > 0 { format!("{}_{}", base, num - 1) } else { base.clone() })
        };

        // ---- imports
        if s.import_count < 0 || s.import_offset < 0 || s.import_count as usize > asset.len() / 28 {
            return Err(Error::malformed("bad import map location"));
        }
        let mut imports = Vec::with_capacity(s.import_count as usize);
        let mut ir = Reader::at(asset, s.import_offset as usize);
        for _ in 0..s.import_count {
            let (cp, cpn) = (ir.i32()?, ir.i32()?);
            let (cn, cnn) = (ir.i32()?, ir.i32()?);
            let outer = PackageIndex(ir.i32()?);
            let (on, onn) = (ir.i32()?, ir.i32()?);
            imports.push(Import { class_package: name_of(cp, cpn)?, class_name: name_of(cn, cnn)?, outer, name: name_of(on, onn)? });
        }

        // ---- exports
        if s.export_count < 0 || s.export_offset < 0 || s.export_count as usize > asset.len() / 90 {
            return Err(Error::malformed("bad export map location"));
        }
        let mut exports = Vec::with_capacity(s.export_count as usize);
        let mut er = Reader::at(asset, s.export_offset as usize);
        for _ in 0..s.export_count {
            let class_index = PackageIndex(er.i32()?);
            let super_index = PackageIndex(er.i32()?);
            let template_index = if v >= ver::TEMPLATE_INDEX_IN_COOKED_EXPORTS { PackageIndex(er.i32()?) } else { PackageIndex(0) };
            let outer_index = PackageIndex(er.i32()?);
            let (n, nn) = (er.i32()?, er.i32()?);
            let object_flags = er.u32()?;
            let (serial_size, serial_offset) = if v >= ver::SIXTY_FOUR_BIT_EXPORTMAP_SERIALSIZES { (er.i64()?, er.i64()?) } else { (er.i32()? as i64, er.i32()? as i64) };
            let forced_export = er.bool32()?;
            let not_for_client = er.bool32()?;
            let not_for_server = er.bool32()?;
            let package_guid = er.guid()?;
            let package_flags = er.u32()?;
            let not_always_loaded_for_editor_game = if v >= ver::LOAD_FOR_EDITOR_GAME { er.bool32()? } else { true };
            let is_asset = if v >= ver::COOKED_ASSETS_IN_EDITOR_SUPPORT { er.bool32()? } else { false };
            let (mut a, mut b, mut c, mut d, mut e) = (-1, 0, 0, 0, 0);
            if v >= ver::PRELOAD_DEPENDENCIES_IN_COOKED_EXPORTS {
                a = er.i32()?;
                b = er.i32()?;
                c = er.i32()?;
                d = er.i32()?;
                e = er.i32()?;
            }
            exports.push(Export {
                class_index,
                super_index,
                template_index,
                outer_index,
                name: name_of(n, nn)?,
                object_flags,
                serial_size,
                serial_offset,
                forced_export,
                not_for_client,
                not_for_server,
                package_guid,
                package_flags,
                not_always_loaded_for_editor_game,
                is_asset,
                first_export_dependency: a,
                serialization_before_serialization: b,
                create_before_serialization: c,
                serialization_before_create: d,
                create_before_create: e,
            });
        }

        // ---- depends (non fatal)
        let mut depends = Vec::new();
        if s.depends_offset > 0 && (s.depends_offset as usize) < asset.len() {
            let mut dr = Reader::at(asset, s.depends_offset as usize);
            let res: Result<()> = (|| {
                for _ in 0..exports.len() {
                    let n = dr.count(4)?;
                    let mut v = Vec::with_capacity(n);
                    for _ in 0..n {
                        v.push(PackageIndex(dr.i32()?));
                    }
                    depends.push(v);
                }
                Ok(())
            })();
            if let Err(e) = res {
                warnings.push(format!("depends map unreadable: {e}"));
                depends.clear();
            }
        }

        // ---- soft package references
        let mut soft_package_references = Vec::new();
        if s.string_asset_references_count > 0 && s.string_asset_references_offset > 0 {
            let mut sr = Reader::at(asset, s.string_asset_references_offset as usize);
            let res: Result<()> = (|| {
                for _ in 0..s.string_asset_references_count {
                    soft_package_references.push(sr.fstring()?);
                }
                Ok(())
            })();
            if let Err(e) = res {
                warnings.push(format!("string asset references unreadable: {e}"));
            }
        }

        // ---- preload dependencies
        let mut preload_dependencies = Vec::new();
        if s.preload_dependency_count > 0 && s.preload_dependency_offset > 0 {
            let mut pr = Reader::at(asset, s.preload_dependency_offset as usize);
            let res: Result<()> = (|| {
                for _ in 0..s.preload_dependency_count {
                    preload_dependencies.push(PackageIndex(pr.i32()?));
                }
                Ok(())
            })();
            if let Err(e) = res {
                warnings.push(format!("preload dependencies unreadable: {e}"));
                preload_dependencies.clear();
            }
        }

        // ---- thumbnail table (index only; image data is never read)
        let mut thumbnails = Vec::new();
        if s.thumbnail_table_offset > 0 && (s.thumbnail_table_offset as usize) < asset.len() {
            let mut tr = Reader::at(asset, s.thumbnail_table_offset as usize);
            let res: Result<()> = (|| {
                let n = tr.count(12)?;
                for _ in 0..n {
                    let class_name = tr.fstring()?;
                    let object_path = tr.fstring()?;
                    let file_offset = tr.i32()?;
                    thumbnails.push(ThumbnailEntry { class_name, object_path, file_offset });
                }
                Ok(())
            })();
            if let Err(e) = res {
                warnings.push(format!("thumbnail table unreadable: {e}"));
                thumbnails.clear();
            }
        }

        Ok(Package { summary: s, names, imports, exports, depends, soft_package_references, preload_dependencies, thumbnails, warnings, name: String::new(), effective_version: v, asset, uexp })
    }

    /// Set the long package name (`/Game/Foo/Bar`) used when resolving object paths.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn asset_bytes(&self) -> &'a [u8] {
        self.asset
    }
    pub fn uexp_bytes(&self) -> &'a [u8] {
        self.uexp
    }

    /// File version (UE4) of this package.
    pub fn version(&self) -> i32 {
        self.effective_version
    }

    /// Resolve an FName (index, number) to its string (`Name_N` when number > 0).
    pub fn fname(&self, idx: i32, num: i32) -> Result<String> {
        let base = self.names.get(idx as usize).ok_or_else(|| Error::malformed(format!("name index {idx} out of range ({})", self.names.len())))?;
        Ok(if num > 0 { format!("{}_{}", base, num - 1) } else { base.clone() })
    }

    /// Borrowed base string for a name index (no number suffix).
    pub fn name_base(&self, idx: i32) -> Option<&str> {
        self.names.get(idx as usize).map(|s| s.as_str())
    }

    /// Bytes of an export's serialized payload.
    pub fn export_bytes(&self, i: usize) -> Result<&'a [u8]> {
        let e = self.exports.get(i).ok_or_else(|| Error::not_found(format!("export {i}")))?;
        self.range_bytes(e.serial_offset, e.serial_size)
    }

    /// Absolute offset (in `uasset ++ uexp`) -> slice.
    pub fn range_bytes(&self, offset: i64, size: i64) -> Result<&'a [u8]> {
        if offset < 0 || size < 0 {
            return Err(Error::malformed("negative export range"));
        }
        let (off, size) = (offset as usize, size as usize);
        if off.checked_add(size).is_none() {
            return Err(Error::malformed("export range overflows"));
        }
        let hs = self.asset.len();
        if self.uexp.is_empty() && off + size <= hs {
            return Ok(&self.asset[off..off + size]);
        }
        if off >= hs {
            let o = off - hs;
            if o + size <= self.uexp.len() {
                return Ok(&self.uexp[o..o + size]);
            }
        } else if off + size <= hs {
            return Ok(&self.asset[off..off + size]);
        }
        Err(Error::truncated(&format!("export range {offset}+{size}"), off + size, hs + self.uexp.len()))
    }

    /// Class name of an export (`"BlueprintGeneratedClass"`, `"Foo_C"`, ...). Empty for the `Class` export root
    /// (class_index = 0 means the export *is* a UClass-like object whose class is `Class`).
    pub fn export_class_name(&self, i: usize) -> String {
        match self.exports.get(i) {
            Some(e) => self.index_name(e.class_index).unwrap_or_else(|| "Class".to_string()),
            None => String::new(),
        }
    }

    /// Short object name for a package index (import name or export name); `None` for null / out of range.
    pub fn index_name(&self, idx: PackageIndex) -> Option<String> {
        if let Some(i) = idx.import_index() {
            self.imports.get(i).map(|x| x.name.clone())
        } else if let Some(i) = idx.export_index() {
            self.exports.get(i).map(|x| x.name.clone())
        } else {
            None
        }
    }

    /// Full path of an imported/exported object: `/Script/Engine.CameraComponent`,
    /// `/Game/Foo/Bar.Bar_C`, nested sub-objects joined with `:`.
    pub fn object_path(&self, idx: PackageIndex) -> String {
        if idx.is_null() {
            return String::new();
        }
        let mut chain: Vec<String> = Vec::new();
        let mut cur = idx;
        let mut package = String::new();
        for _ in 0..64 {
            if let Some(i) = cur.import_index() {
                match self.imports.get(i) {
                    Some(im) => {
                        if im.outer.is_null() {
                            package = im.name.clone();
                            break;
                        }
                        chain.push(im.name.clone());
                        cur = im.outer;
                    }
                    None => {
                        chain.push(format!("<bad import {i}>"));
                        break;
                    }
                }
            } else if let Some(i) = cur.export_index() {
                match self.exports.get(i) {
                    Some(ex) => {
                        chain.push(ex.name.clone());
                        if ex.outer_index.is_null() {
                            package = self.name.clone();
                            break;
                        }
                        cur = ex.outer_index;
                    }
                    None => {
                        chain.push(format!("<bad export {i}>"));
                        break;
                    }
                }
            } else {
                break;
            }
        }
        chain.reverse();
        let mut out = package;
        for (n, part) in chain.iter().enumerate() {
            out.push(if n == 0 { '.' } else { ':' });
            out.push_str(part);
        }
        out
    }

    /// Resolve a package index to its import/export record with class and full path.
    pub fn resolve(&self, idx: PackageIndex) -> ObjectRef {
        if let Some(i) = idx.import_index() {
            let Some(im) = self.imports.get(i) else { return ObjectRef::Null };
            // outermost package
            let mut cur = im;
            for _ in 0..64 {
                match cur.outer.import_index().and_then(|o| self.imports.get(o)) {
                    Some(o) => cur = o,
                    None => break,
                }
            }
            ObjectRef::Import { index: i, name: im.name.clone(), class_name: im.class_name.clone(), class_package: im.class_package.clone(), package: cur.name.clone(), path: self.object_path(idx) }
        } else if let Some(i) = idx.export_index() {
            if i >= self.exports.len() {
                return ObjectRef::Null;
            }
            ObjectRef::Export { index: i, name: self.exports[i].name.clone(), class_name: self.export_class_name(i), path: self.object_path(idx) }
        } else {
            ObjectRef::Null
        }
    }

    /// Index of the generated-class export (`Foo_C` of a Blueprint, `Class` of a native-less package), if any.
    pub fn class_export(&self) -> Option<usize> {
        (0..self.exports.len()).find(|&i| {
            let c = self.export_class_name(i);
            (c == "Class" || c.ends_with("GeneratedClass")) && !self.exports[i].name.starts_with("Default__")
        })
    }

    /// Resolved super class of this package's class export (`Pawn`, `FortPlayerPawnAthena`, another `Foo_C`).
    pub fn super_class(&self) -> Option<ObjectRef> {
        let i = self.class_export()?;
        match self.resolve(self.exports[i].super_index) {
            ObjectRef::Null => None,
            r => Some(r),
        }
    }

    /// First export with the given name (case-sensitive).
    pub fn find_export(&self, name: &str) -> Option<usize> {
        self.exports.iter().position(|e| e.name == name)
    }

    /// All export indices whose class name equals `class`.
    pub fn exports_of_class<'s>(&'s self, class: &'s str) -> impl Iterator<Item = usize> + 's {
        (0..self.exports.len()).filter(move |&i| self.export_class_name(i) == class)
    }

    /// The class default object export (`Default__*`, flagged `RF_ClassDefaultObject`).
    pub fn default_object_export(&self) -> Option<usize> {
        self.exports.iter().position(|e| e.object_flags & RF_CLASS_DEFAULT_OBJECT != 0)
    }
}


