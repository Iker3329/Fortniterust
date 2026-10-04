//! Error type shared by every module of the crate.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("pak: {0}")]
    Pak(#[from] fn_pak::PakError),
    #[error("package: {0}")]
    Package(#[from] fn_uasset::Error),
    /// Structurally invalid or truncated serialized data (never a panic).
    #[error("malformed {what}: {detail}")]
    Malformed { what: &'static str, detail: String },
    /// A format this crate does not decode (name of the compressed format).
    #[error("unsupported audio format {0:?}")]
    Unsupported(String),
    #[error("decode ({codec}): {detail}")]
    Decode { codec: &'static str, detail: String },
    #[error("cue not found: {0}")]
    CueNotFound(String),
    #[error("audio command queue full")]
    QueueFull,
    #[error("device: {0}")]
    Device(String),
}

pub type Result<T> = std::result::Result<T, AudioError>;

pub(crate) fn malformed<T>(what: &'static str, detail: impl Into<String>) -> Result<T> {
    Err(AudioError::Malformed { what, detail: detail.into() })
}
