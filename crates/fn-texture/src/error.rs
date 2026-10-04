use std::fmt;

/// Errors produced by `fn-texture`.
#[derive(Debug)]
pub enum Error {
    /// Low level package / property reading failure.
    Package(fn_ueio::Error),
    /// The package has no exportable texture (`Texture2D` / `TextureCube`), or the class is not supported
    /// (for example `VolumeTexture`, `TextureRenderTarget2D`).
    NotATexture(String),
    /// The texture data layout is not what a cooked UE 4.16 texture looks like.
    Invalid(String),
    /// A pixel format that this crate does not decode (name as stored in the package, e.g. `PF_ASTC_4x4`).
    UnsupportedFormat(String),
    /// The requested mip does not exist or has no payload (stripped, or its `.ubulk` is missing).
    MipUnavailable(String),
    /// PNG / file output failure.
    Output(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Package(e) => write!(f, "package error: {e}"),
            Error::NotATexture(s) => write!(f, "not a supported texture: {s}"),
            Error::Invalid(s) => write!(f, "invalid texture data: {s}"),
            Error::UnsupportedFormat(s) => write!(f, "unsupported pixel format {s}"),
            Error::MipUnavailable(s) => write!(f, "mip unavailable: {s}"),
            Error::Output(s) => write!(f, "output error: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<fn_ueio::Error> for Error {
    fn from(e: fn_ueio::Error) -> Self {
        Error::Package(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
