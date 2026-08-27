//! Install receipts: persist a snapshot of the side effects produced by an
//! install so uninstall can restore them precisely.
//!
//! A receipt records two things: the **pre-apply value** of every environment
//! variable the install wrote, and the **tool directory** to remove on uninstall.
//! This lets uninstall roll back fully without needing the plugin manifest.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::engine::fs::Fs;

/// A single environment variable's pre-apply snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VarSnapshot {
    /// Variable name.
    pub name: String,
    /// Value before install; `None` means it did not exist (uninstall deletes it).
    pub old: Option<String>,
}

/// One install receipt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    /// Tool name.
    pub name: String,
    /// Installed version.
    pub version: String,
    /// The installed tool root (under `apps`), removed wholesale on uninstall.
    pub app_dir: String,
    /// Environment variables written by the install, with their old values.
    pub vars: Vec<VarSnapshot>,
}

impl Receipt {
    /// Serialize to TOML text.
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).map_err(|e| crate::Error::Manifest(e.to_string()))
    }

    /// Parse from TOML text.
    pub fn from_toml(source: &str) -> Result<Self> {
        toml::from_str(source).map_err(|e| crate::Error::Manifest(e.to_string()))
    }

    /// Write to the filesystem (implicitly creating parent directories).
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
