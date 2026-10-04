//! Small `serde_json::Value` accessors shared by the loaders.
use serde_json::Value;
use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(String),
    Json(String),
    /// A required field / file of the schema is missing or has the wrong type.
    Schema(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(s) => write!(f, "io: {s}"),
            Error::Json(s) => write!(f, "json: {s}"),
            Error::Schema(s) => write!(f, "schema: {s}"),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn read_json(path: &std::path::Path) -> Result<Value> {
    let s = std::fs::read_to_string(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&s).map_err(|e| Error::Json(format!("{}: {e}", path.display())))
}

pub(crate) fn schema<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Schema(msg.into()))
}

pub(crate) fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(String::from)
}

pub(crate) fn req_str(v: &Value, key: &str, ctx: &str) -> Result<String> {
    str_of(v, key).ok_or_else(|| Error::Schema(format!("{ctx}: missing string '{key}'")))
}

pub(crate) fn f32_of(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(|x| x.as_f64()).map(|x| x as f32)
}

pub(crate) fn bool_of(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(|x| x.as_bool())
}

pub(crate) fn u32_of(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(|x| x.as_u64()).map(|x| x as u32)
}

pub(crate) fn arr_f32<const N: usize>(v: Option<&Value>) -> Option<[f32; N]> {
    let a = v?.as_array()?;
    if a.len() < N {
        return None;
    }
    let mut out = [0.0f32; N];
    for i in 0..N {
        out[i] = a[i].as_f64()? as f32;
    }
    Some(out)
}
