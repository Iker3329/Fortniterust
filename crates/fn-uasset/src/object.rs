//! Export object reading: tagged properties of any export plus the untouched native tail.

use crate::error::{Error, Result};
use crate::package::Package;
use crate::reader::Reader;
use crate::value::{PropertyList, Value};
use serde_json::{json, Value as Json};

/// A decoded export. `props` is the tagged-property list. Non-CDO objects carry a 4-byte `bHasGuid` flag
/// right after the `None` terminator (`UObject::Serialize`; followed by an FGuid when set) - it is consumed and
/// reported as `object_guid`/`had_guid_flag`. `trailing` is whatever follows: the *native* serialisation of the
/// object (UStruct/UClass/UFunction bodies, texture/mesh payloads, `LODData` counts, ...).
#[derive(Clone, Debug)]
pub struct ExportObject<'a> {
    pub index: usize,
    pub name: String,
    pub class: String,
    pub props: PropertyList,
    /// True when the 4-byte `bHasGuid` flag was present (every export without `RF_ClassDefaultObject`).
    pub had_guid_flag: bool,
    pub object_guid: Option<[u8; 16]>,
    /// Native payload after the tagged properties and the guid flag.
    pub trailing: &'a [u8],
    /// Set if the property list could not be read to its terminator (`props` holds what was read before).
    pub error: Option<Error>,
}

impl<'a> ExportObject<'a> {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.props.get(name)
    }
    /// Number of properties that came back as [`Value::Unparsed`] (recursively).
    pub fn unparsed_count(&self) -> usize {
        fn walk(v: &Value) -> usize {
            match v {
                Value::Unparsed { .. } => 1,
                Value::Array(a) | Value::Set(a) => a.iter().map(walk).sum(),
                Value::Map(m) => m.iter().map(|(k, v)| walk(k) + walk(v)).sum(),
                Value::Struct { props, .. } => props.iter().map(|p| walk(&p.value)).sum(),
                _ => 0,
            }
        }
        self.props.iter().map(|p| walk(&p.value)).sum()
    }
    pub fn to_json(&self, pkg: &Package) -> Json {
        let mut j = json!({
            "index": self.index,
            "name": self.name,
            "class": self.class,
            "properties": self.props.to_json(pkg),
            "nativeBytes": self.trailing.len(),
        });
        if let Some(e) = &self.error {
            j["error"] = json!(e.to_string());
        }
        j
    }
}

impl<'a> Package<'a> {
    /// Read export `i`'s tagged properties. Errors only for a bad index or an out-of-range export;
    /// property-level problems land in `ExportObject::error` / `Value::Unparsed`.
    pub fn read_export(&self, i: usize) -> Result<ExportObject<'a>> {
        let ex = self.exports.get(i).ok_or_else(|| Error::not_found(format!("export {i}")))?;
        let bytes = self.export_bytes(i)?;
        let mut r = Reader::new(bytes);
        let mut props = PropertyList::new();
        let error = self.read_tagged_into(&mut r, &mut props).err();
        let mut had_guid_flag = false;
        let mut object_guid = None;
        let mut error = error;
        if error.is_none() && ex.object_flags & crate::package::RF_CLASS_DEFAULT_OBJECT == 0 && r.remaining() >= 4 {
            let flag = r.i32().unwrap_or(0);
            had_guid_flag = true;
            if flag != 0 {
                match r.guid() {
                    Ok(g) => object_guid = Some(g),
                    Err(e) => error = Some(e),
                }
            }
        }
        let trailing = if error.is_none() { &bytes[r.pos()..] } else { &bytes[0..0] };
        Ok(ExportObject { index: i, name: ex.name.clone(), class: self.export_class_name(i), props, had_guid_flag, object_guid, trailing, error })
    }

    /// Read every export.
    pub fn read_all_exports(&self) -> Vec<Result<ExportObject<'a>>> {
        (0..self.exports.len()).map(|i| self.read_export(i)).collect()
    }

    /// Class default object (`Default__Foo_C`) of a Blueprint class package, if present.
    pub fn read_default_object(&self) -> Option<Result<ExportObject<'a>>> {
        self.default_object_export().map(|i| self.read_export(i))
    }

    /// Read an export by name.
    pub fn read_export_named(&self, name: &str) -> Option<Result<ExportObject<'a>>> {
        self.find_export(name).map(|i| self.read_export(i))
    }
}

