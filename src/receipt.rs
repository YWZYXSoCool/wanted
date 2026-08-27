//! Install receipts: persist a snapshot of the side effects produced by an
//! install so uninstall can restore them precisely.
//!
//! A receipt records two things: the **delta** of every environment variable the
//! install wrote (op + value + pre-apply value), and the **tool directory** to
//! remove on uninstall. This lets uninstall undo exactly what the install did,
//! without needing the plugin manifest.
//!
//! PATH-like variables are uninstalled **relatively**: only the segment this
//! install added is removed from the current value, so a later install added on
//! top (or an earlier uninstall) never clobbers the others.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::Version;
use crate::engine::fs::Fs;
use crate::env::EnvOp;

/// A single environment variable written by an install, with enough to reverse it.
///
/// For `Prepend`/`Append`, uninstall drops the applied `value` segment from the
/// *current* value so later installs survive any uninstall order. For `Set` it
/// restores the pre-apply `old` value (which owns the whole variable).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VarSnapshot {
    /// Variable name.
    pub name: String,
    /// How the install merged this variable.
    pub op: EnvOp,
    /// The segment/value the install applied (removed on uninstall for PATH-like vars).
    pub value: String,
    /// Value before install; meaningful only for `Set` (`None` means it did not exist).
    pub old: Option<String>,
}

/// One install receipt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    /// Tool name.
    pub name: String,
    /// Installed version.
    pub version: Version,
    /// The installed tool root (under `apps`), removed wholesale on uninstall.
    pub app_dir: String,
    /// Environment variables written by the install, with their old values.
    pub vars: Vec<VarSnapshot>,
}

impl Receipt {
    /// Serialize to TOML text.
    #[inline]
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).map_err(|e| crate::Error::Manifest(e.to_string()))
    }

    /// Parse from TOML text.
    #[inline]
    pub fn from_toml(source: &str) -> Result<Self> {
        toml::from_str(source).map_err(|e| crate::Error::Manifest(e.to_string()))
    }

    /// Write to the filesystem (implicitly creating parent directories).
    #[inline]
    pub fn write(&self, fs: &dyn Fs, path: &Path) -> Result<()> {
        fs.write(path, self.to_toml()?.as_bytes())
    }

    /// Read from the filesystem; returns `None` when the file does not exist.
    pub fn read(fs: &dyn Fs, path: &Path) -> Result<Option<Self>> {
        if !fs.exists(path)? {
            return Ok(None);
        }
        let bytes = fs.read(path)?;
        Ok(Some(Self::from_toml(&String::from_utf8_lossy(&bytes))?))
    }
}

#[cfg(test)]
mod tests;
