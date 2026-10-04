use crate::{invalid, Error, Reader, Result};

/// VER_UE4_16 of UEViewer's table: the effective file version of unversioned UE 4.16 packages.
pub const UE4_16_FILE_VERSION: i32 = 513;
pub const BULK_AT_END: u32 = 0x1;
pub const BULK_ZLIB: u32 = 0x2;
pub const BULK_UNUSED: u32 = 0x20;
pub const BULK_FORCE_INLINE: u32 = 0x40;
pub const BULK_SEPARATE: u32 = 0x100;
pub const BULK_OPTIONAL: u32 = 0x800;

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub legacy_version: i32,
    pub file_version: i32,
    pub licensee_version: i32,
    pub unversioned: bool,
    pub total_header_size: i32,
    pub package_flags: u32,
    pub name_count: i32,
    pub name_offset: i32,
    pub export_count: i32,
    pub export_offset: i32,
    pub import_count: i32,
    pub import_offset: i32,
    pub bulk_data_start_offset: i64,
    pub custom_versions: Vec<([u8; 16], i32)>,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub class_package: String,
    pub class_name: String,
    pub outer_index: i32,
    pub object_name: String,
}

#[derive(Debug, Clone)]
pub struct Export {
    pub class_index: i32,
    pub super_index: i32,
    pub outer_index: i32,
    pub object_name: String,
    pub object_flags: u32,
    pub serial_size: i64,
    pub serial_offset: i64,
}

/// Parsed `FByteBulkData` header.
#[derive(Debug, Clone, Copy, Default)]
pub struct BulkData {
    pub flags: u32,
    pub element_count: i32,
    pub size_on_disk: i32,
    /// Raw `BulkDataOffsetInFile` as stored (no `BulkDataStartOffset` fix-up applied).
    pub offset_in_file: i64,
    /// Position in the reader just after the header (inline payload, if any, starts here).
    pub header_end: usize,
}

impl BulkData {
    pub fn read(r: &mut Reader) -> Result<BulkData> {
        let flags = r.u32()?;
        let element_count = r.i32()?;
        let size_on_disk = r.i32()?;
        let offset_in_file = r.i64()?;
        Ok(BulkData { flags, element_count, size_on_disk, offset_in_file, header_end: r.tell() })
    }
    pub fn is_unused(&self) -> bool {
        self.flags & BULK_UNUSED != 0 || self.element_count == 0
    }
    pub fn in_ubulk(&self) -> bool {
        self.flags & (BULK_SEPARATE | BULK_OPTIONAL) != 0
    }
}

pub struct Package {
    pub summary: Summary,
    pub names: Vec<String>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    /// `.uasset ++ .uexp`
    pub data: Vec<u8>,
    pub ubulk: Vec<u8>,
}

fn read_fname(r: &mut Reader, names: &[String]) -> Result<String> {
    let idx = r.i32()?;
    let num = r.i32()?;
    let base = names.get(idx as usize).cloned().ok_or_else(|| Error::Invalid(format!("bad name index {idx}")))?;
    Ok(if num > 0 { format!("{}_{}", base, num - 1) } else { base })
}

impl Package {
    /// Parse a package from the three file images. `uexp` may be empty for packages without one.
    pub fn parse(uasset: &[u8], uexp: &[u8], ubulk: &[u8]) -> Result<Package> {
        let mut data = Vec::with_capacity(uasset.len() + uexp.len());
        data.extend_from_slice(uasset);
        data.extend_from_slice(uexp);
        let mut r = Reader::new(&data);
        let tag = r.u32()?;
        if tag != 0x9E2A83C1 {
            return invalid(format!("bad package tag {tag:#x}"));
        }
        let mut s = Summary { legacy_version: r.i32()?, ..Default::default() };
        if s.legacy_version != -4 {
            r.i32()?; // UE3 version
        }
        s.file_version = r.i32()?;
        s.licensee_version = r.i32()?;
        if s.file_version == 0 && s.licensee_version == 0 {
            // Unversioned cooked package: the engine assumes its own latest version. Fortnite 1.8 = UE 4.16 = 513.
            s.unversioned = true;
            s.file_version = UE4_16_FILE_VERSION;
        }
        if s.legacy_version <= -2 {
            let n = r.i32()?;
            for _ in 0..n {
                let g = r.guid()?;
                let v = r.i32()?;
                s.custom_versions.push((g, v));
            }
        }
        s.total_header_size = r.i32()?;
        r.fstring()?; // folder name
        s.package_flags = r.u32()?;
        s.name_count = r.i32()?;
        s.name_offset = r.i32()?;
        let filter_editor_only = s.package_flags & 0x8000_0000 != 0;
        if s.file_version >= 516 && !filter_editor_only {
            r.fstring()?; // localization id
        }
        if s.file_version >= 459 {
            r.i32()?;
            r.i32()?; // gatherable text
        }
        s.export_count = r.i32()?;
        s.export_offset = r.i32()?;
        s.import_count = r.i32()?;
        s.import_offset = r.i32()?;
        r.i32()?; // depends offset
        if s.file_version >= 384 {
            r.i32()?;
            r.i32()?;
        }
        if s.file_version >= 510 {
            r.i32()?; // searchable names
        }
        r.i32()?; // thumbnail table
        r.guid()?;
        if !filter_editor_only && s.file_version >= 518 {
            r.guid()?;
            if s.file_version < 520 {
                r.guid()?;
            }
        }
        let gens = r.i32()?;
        for _ in 0..gens {
            r.skip(8)?;
        }
        if s.file_version >= 336 {
            r.skip(2 + 2 + 2 + 4)?;
            r.fstring()?;
        } else {
            r.i32()?;
        }
        if s.file_version >= 444 {
            r.skip(2 + 2 + 2 + 4)?;
            r.fstring()?;
        }
        r.u32()?; // compression flags
        let chunks = r.i32()?;
        r.skip(chunks as usize * 16)?;
        r.u32()?; // package source
        let addl = r.i32()?;
        for _ in 0..addl {
            r.fstring()?;
        }
        if s.legacy_version > -7 {
            r.i32()?;
        }
        if s.file_version >= 112 {
            r.i32()?; // asset registry data offset
        }
        if s.file_version >= 212 {
            s.bulk_data_start_offset = r.i64()?;
        }

        // names
        let mut names = Vec::with_capacity(s.name_count as usize);
        r.seek(s.name_offset as usize)?;
        for _ in 0..s.name_count {
            names.push(r.fstring()?);
            if s.file_version >= 504 {
                r.skip(4)?;
            }
        }
        // imports
        r.seek(s.import_offset as usize)?;
        let mut imports = Vec::new();
        for _ in 0..s.import_count {
            let class_package = read_fname(&mut r, &names)?;
            let class_name = read_fname(&mut r, &names)?;
            let outer_index = r.i32()?;
            let object_name = read_fname(&mut r, &names)?;
            imports.push(Import { class_package, class_name, outer_index, object_name });
        }
        // exports
        r.seek(s.export_offset as usize)?;
        let mut exports = Vec::new();
        for _ in 0..s.export_count {
            let class_index = r.i32()?;
            let super_index = r.i32()?;
            if s.file_version >= 508 {
                r.i32()?; // template
            }
            let outer_index = r.i32()?;
            let object_name = read_fname(&mut r, &names)?;
            let object_flags = r.u32()?;
            let (serial_size, serial_offset) = if s.file_version >= 511 {
                (r.i64()?, r.i64()?)
            } else {
                (r.i32()? as i64, r.i32()? as i64)
            };
            r.skip(12)?; // forced export / not for client / not for server
            r.guid()?;
            r.u32()?; // package flags
            if s.file_version >= 365 {
                r.i32()?;
            }
            if s.file_version >= 485 {
                r.i32()?;
            }
            if s.file_version >= 507 {
                r.skip(20)?;
            }
            exports.push(Export { class_index, super_index, outer_index, object_name, object_flags, serial_size, serial_offset });
        }
        Ok(Package { summary: s, names, imports, exports, data, ubulk: ubulk.to_vec() })
    }

    /// Resolve a package index (positive export, negative import, 0 null) to `Class'Name'`-like text.
    pub fn index_name(&self, idx: i32) -> String {
        if idx > 0 {
            self.exports.get(idx as usize - 1).map(|e| e.object_name.clone()).unwrap_or_default()
        } else if idx < 0 {
            self.imports.get((-idx) as usize - 1).map(|i| i.object_name.clone()).unwrap_or_default()
        } else {
            String::new()
        }
    }

    /// Class name of export `i` (resolved through its class import / export).
    pub fn export_class(&self, i: usize) -> String {
        self.index_name(self.exports[i].class_index)
    }

    /// Full object path of an import (outer chain joined with '.'), e.g. `/Game/.../Skeleton.Skeleton`.
    pub fn import_path(&self, idx: i32) -> String {
        let mut parts = Vec::new();
        let mut cur = idx;
        while cur < 0 {
            let imp = &self.imports[(-cur) as usize - 1];
            parts.push(imp.object_name.clone());
            cur = imp.outer_index;
        }
        parts.reverse();
        parts.join(".")
    }

    pub fn export_bytes(&self, i: usize) -> Result<&[u8]> {
        let e = &self.exports[i];
        let (o, s) = (e.serial_offset as usize, e.serial_size as usize);
        self.data.get(o..o + s).ok_or_else(|| Error::Invalid(format!("export {i} out of range")))
    }

    /// Start offset of export `i` inside `self.data` (the reader position base).
    pub fn export_range(&self, i: usize) -> (usize, usize) {
        let e = &self.exports[i];
        (e.serial_offset as usize, (e.serial_offset + e.serial_size) as usize)
    }

    pub fn find_export(&self, class: &str) -> Option<usize> {
        (0..self.exports.len()).find(|&i| self.export_class(i).eq_ignore_ascii_case(class))
    }

    /// Resolve the payload bytes of a bulk block. For inline payloads the caller passes the reader position data;
    /// this handles end-of-file and `.ubulk` payloads. Returns `None` for unused blocks.
    pub fn bulk_payload(&self, b: &BulkData) -> Result<Option<&[u8]>> {
        if b.is_unused() {
            return Ok(None);
        }
        let size = b.size_on_disk as usize;
        if b.flags & BULK_FORCE_INLINE != 0 {
            let s = self.data.get(b.header_end..b.header_end + size).ok_or_else(|| Error::Invalid("inline bulk out of range".into()))?;
            return Ok(Some(s));
        }
        if b.in_ubulk() {
            let o = (b.offset_in_file + self.summary.bulk_data_start_offset) as usize;
            let s = self.ubulk.get(o..o + size).ok_or_else(|| Error::Invalid(format!("ubulk bulk out of range ({o}+{size} > {})", self.ubulk.len())))?;
            return Ok(Some(s));
        }
        if b.flags & BULK_AT_END != 0 {
            let o = (b.offset_in_file + self.summary.bulk_data_start_offset) as usize;
            let s = self.data.get(o..o + size).ok_or_else(|| Error::Invalid("end-of-file bulk out of range".into()))?;
            return Ok(Some(s));
        }
        // inline right after header (legacy)
        let s = self.data.get(b.header_end..b.header_end + size).ok_or_else(|| Error::Invalid("legacy inline bulk out of range".into()))?;
        Ok(Some(s))
    }
}

