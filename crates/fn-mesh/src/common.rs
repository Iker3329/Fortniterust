//! Shared low-level helpers used by the skeleton / skeletal mesh / static mesh decoders.

use fn_ueio::props::{read_props, Ctx as PropCtx, PropList};
use fn_ueio::{invalid, Package, Reader, Result};

/// Read an `FName` (index + number) and resolve it to text (`Base_N-1` for number > 0).
pub(crate) fn fname(r: &mut Reader, names: &[String]) -> Result<String> {
    let idx = r.i32()?;
    let num = r.i32()?;
    let base = match names.get(idx as usize) {
        Some(b) => b.clone(),
        None => return invalid(format!("bad FName index {idx} at {}", r.tell())),
    };
    Ok(if num > 0 { format!("{}_{}", base, num - 1) } else { base })
}

/// UE `bool` as serialised by `FArchive& operator<<(bool&)`: a 32 bit integer.
pub(crate) fn bool32(r: &mut Reader) -> Result<bool> {
    Ok(r.i32()? != 0)
}

/// `FStripDataFlags`: two bytes (global, class) written before most cooked structures.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StripFlags {
    pub global: u8,
    pub class: u8,
}

impl StripFlags {
    pub fn read(r: &mut Reader) -> Result<StripFlags> {
        Ok(StripFlags { global: r.u8()?, class: r.u8()? })
    }
    /// Editor-only data (source models, soft vertices, raw point indices ...) is absent.
    pub fn editor_stripped(&self) -> bool {
        self.global & 1 != 0
    }
    pub fn server_stripped(&self) -> bool {
        self.global & 2 != 0
    }
    pub fn class_stripped(&self, flag: u8) -> bool {
        self.class & flag != 0
    }
}

/// `TArray::BulkSerialize` header: `(element_size, element_count)`; the raw elements follow.
pub(crate) fn bulk_header(r: &mut Reader) -> Result<(usize, usize)> {
    let size = r.i32()?;
    let count = r.i32()?;
    if size < 0 || count < 0 {
        return invalid(format!("bad bulk array header size={size} count={count} at {}", r.tell()));
    }
    let (size, count) = (size as usize, count as usize);
    if size.checked_mul(count).map_or(true, |n| n > r.remaining()) {
        return invalid(format!("bulk array {count}x{size} exceeds data at {}", r.tell()));
    }
    Ok((size, count))
}

/// Skip a whole bulk array (`SkipBulkArrayData`).
pub(crate) fn skip_bulk_array(r: &mut Reader) -> Result<()> {
    let (size, count) = bulk_header(r)?;
    r.skip(size * count)
}

/// Plain `TArray` element count with a sanity bound against the remaining bytes (each element at least `min` bytes).
pub(crate) fn array_count(r: &mut Reader, min_elem: usize) -> Result<usize> {
    let n = r.i32()?;
    if n < 0 || (n as usize).checked_mul(min_elem).map_or(true, |b| b > r.remaining()) {
        return invalid(format!("bad array count {n} at {}", r.tell()));
    }
    Ok(n as usize)
}

/// `TArray<uint16>`
pub(crate) fn read_u16_array(r: &mut Reader) -> Result<Vec<u16>> {
    let n = array_count(r, 2)?;
    (0..n).map(|_| r.u16()).collect()
}

/// `TArray<int16>`
pub(crate) fn read_i16_array(r: &mut Reader) -> Result<Vec<i16>> {
    let n = array_count(r, 2)?;
    (0..n).map(|_| r.i16()).collect()
}

/// `FBoxSphereBounds` (7 floats in UE 4.16).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bounds {
    pub origin: [f32; 3],
    pub extent: [f32; 3],
    pub radius: f32,
}

impl Bounds {
    pub(crate) fn read(r: &mut Reader) -> Result<Bounds> {
        Ok(Bounds { origin: r.vec3()?, extent: r.vec3()?, radius: r.f32()? })
    }
    /// Axis aligned box `(min, max)` = origin -/+ extent.
    pub fn box_min_max(&self) -> ([f32; 3], [f32; 3]) {
        let mut mn = [0.0; 3];
        let mut mx = [0.0; 3];
        for i in 0..3 {
            mn[i] = self.origin[i] - self.extent[i];
            mx[i] = self.origin[i] + self.extent[i];
        }
        (mn, mx)
    }
}

/// `FTransform` as serialised natively in UE 4.16: rotation quat (xyzw), translation, scale3D (10 floats).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}

impl Transform {
    pub const IDENTITY: Transform = Transform { rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0; 3], scale: [1.0; 3] };
    pub(crate) fn read(r: &mut Reader) -> Result<Transform> {
        Ok(Transform { rotation: r.quat()?, translation: r.vec3()?, scale: r.vec3()? })
    }
}

/// Resolve a package index (as found in object properties / native object references) to a short description.
/// Imports yield their full object path (`/Game/Dir/Asset.Asset`), exports yield `ExportName` (same package).
pub(crate) fn object_path(pkg: &Package, index: i32) -> Option<String> {
    if index == 0 {
        None
    } else if index < 0 {
        let i = (-index) as usize - 1;
        if i < pkg.imports.len() {
            Some(pkg.import_path(index))
        } else {
            None
        }
    } else {
        pkg.exports.get(index as usize - 1).map(|e| e.object_name.clone())
    }
}

/// Short object name (last component) of a package index.
pub(crate) fn object_name(pkg: &Package, index: i32) -> Option<String> {
    if index == 0 {
        return None;
    }
    let n = pkg.index_name(index);
    if n.is_empty() {
        None
    } else {
        Some(n)
    }
}

/// Everything needed to start decoding the native part of an export: its tagged properties and a reader
/// positioned right after the `bSerializeGuid` field (and optional `FGuid`) that follows the property list.
pub(crate) struct ExportBody<'a> {
    pub props: PropList,
    pub reader: Reader<'a>,
    /// Absolute end offset (in `pkg.data`) of the export.
    pub end: usize,
}

pub(crate) fn export_body(pkg: &Package, export: usize) -> Result<ExportBody<'_>> {
    let (start, end) = pkg.export_range(export);
    if end > pkg.data.len() || start > end {
        return invalid(format!("export {export} range {start}..{end} outside package data"));
    }
    let mut r = Reader::at(&pkg.data[..end], start);
    let c = PropCtx { names: &pkg.names, file_version: pkg.summary.file_version };
    let props = read_props(&mut r, &c)?;
    // UObject::Serialize: `bool bHasGuid` (int32) followed by an FGuid when set.
    let has_guid = r.i32()?;
    if has_guid != 0 {
        r.skip(16)?;
    }
    Ok(ExportBody { props, reader: r, end })
}

/// Find the export for a class name, with a helpful error.
pub(crate) fn find_class(pkg: &Package, class: &str) -> Result<usize> {
    match pkg.find_export(class) {
        Some(i) => Ok(i),
        None => invalid(format!("package has no {class} export")),
    }
}

/// Decode one FPackedNormal byte exactly as the engine does (`x / 127.5 - 1`, see `Common.usf` `TangentBias`).
pub fn unpack_byte_engine(b: u8) -> f32 {
    b as f32 / 127.5 - 1.0
}

/// Decode one FPackedNormal byte the way UEViewer does (`(b ^ 0x80) as i8 / 127`).
pub fn unpack_byte_uev(b: u8) -> f32 {
    ((b ^ 0x80) as i8) as f32 * (1.0f32 / 127.0f32)
}

/// Decode an `FPackedNormal` (4 bytes x,y,z,w as stored) into xyz using the engine formula.
pub fn unpack_normal(p: [u8; 4]) -> [f32; 3] {
    [unpack_byte_engine(p[0]), unpack_byte_engine(p[1]), unpack_byte_engine(p[2])]
}

/// Normalised vector from packed bytes (engine decode, then normalised; zero stays zero).
pub fn unpack_normal_unit(p: [u8; 4]) -> [f32; 3] {
    let v = unpack_normal(p);
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        v
    }
}

/// Binormal sign encoded in the w byte of `TangentZ` (+1 / -1).
pub fn tangent_sign(z: [u8; 4]) -> f32 {
    if z[3] >= 128 {
        1.0
    } else {
        -1.0
    }
}
