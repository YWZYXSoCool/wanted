//! The crate's error type.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// The crate-wide error.
#[derive(Debug, Error)]
pub enum Error {
    /// Plugin manifest parse / validation failure.
    #[error("manifest: {0}")]
    Manifest(String),

    /// The manifest is missing a required field.
    #[error("manifest missing field: {field}")]
    MissingField { field: &'static str },

    /// File / directory I/O failure, with the offending path.
    #[error("i/o at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// Network download failure.
    #[error("network: {0}")]
    Network(String),

    /// Archive extraction failure.
    #[error("archive: {0}")]
    Archive(String),

    /// The current platform has no matching asset.
    #[error("unsupported platform: {target}")]
    UnsupportedPlatform { target: String },

    /// The current platform lacks the requested asset source.
    #[error("no asset source {name} for platform {target}")]
    SourceNotFound { target: String, name: String },

    /// A capability that is not yet supported.
    #[error("unsupported on this platform yet: {0}")]
    Unsupported(&'static str),

    /// A compensation step failed during a rollback.
    #[error("rollback failed: {0}")]
    Rollback(String),

    /// Any other error.
    #[error("{0}")]
    Other(String),
}

/// Bind an arbitrary std error to the path where it occurred.
pub fn io_err(path: PathBuf, source: io::Error) -> Error {
    Error::Io { path, source }
}
