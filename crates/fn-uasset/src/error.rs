//! Error type. Every error carries a coarse [`ErrorKind`] so sweeps can group failures by cause.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorKind {
    /// Reading ran past the end of a buffer.
    Truncated,
    /// Not a UE4 package (bad magic).
    BadMagic,
    /// Legacy/file version outside what this parser understands.
    UnsupportedVersion,
    /// Structurally invalid data (bad counts, bad indices).
    Malformed,
    /// Valid data of a kind this reader does not decode (reported, never guessed).
    Unsupported,
    /// Missing companion file / object.
    NotFound,
    Io,
}

#[derive(Clone, Debug)]
pub struct Error {
    pub kind: ErrorKind,
    pub msg: String,
}

impl Error {
    pub fn new(kind: ErrorKind, msg: impl Into<String>) -> Self {
        Error { kind, msg: msg.into() }
    }
    pub fn truncated(what: &str, wanted: usize, have: usize) -> Self {
        Error::new(ErrorKind::Truncated, format!("{what}: wanted offset {wanted}, buffer is {have}"))
    }
    pub fn malformed(msg: impl Into<String>) -> Self {
        Error::new(ErrorKind::Malformed, msg)
    }
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Error::new(ErrorKind::Unsupported, msg)
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Error::new(ErrorKind::NotFound, msg)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.msg)
    }
}
impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
