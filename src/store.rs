//! The `.wanted` record directory.
//!
//! `.wanted` holds only records — one receipt per installed tool. The apps
//! themselves live in the directory where `wanted install` runs (the store's root).

use std::path::PathBuf;

use crate::Result;
use crate::engine::fs::Fs;
use crate::fs_path::DirName;

/// Layout accessor for the record directory.
///
/// Installed apps live directly under `root` (the run directory); `.wanted`
/// under it persists per-tool receipts so `list`, `uninstall`, and upgrade can
/// reconstruct what was installed and where.
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Construct around a run directory (under which `.wanted` lives).
    pub fn new(root: PathBuf) -> Self {
        Store { root }
    }

    /// The run directory where apps are installed.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// The `.wanted` record directory.
    pub fn store_dir(&self) -> PathBuf {
        self.root.join(".wanted")
    }

    /// Where install receipts live.
    pub fn installed_dir(&self) -> PathBuf {
        self.store_dir().join("installed")
    }

    /// The receipt path for one tool.
    pub fn receipt_path(&self, name: &DirName) -> PathBuf {
        self.installed_dir().join(name).join("receipt.toml")
    }

    /// List installed tool names (one record subdirectory of `installed`).
    pub fn list_installed(&self, fs: &dyn Fs) -> Result<Vec<String>> {
        let installed = self.installed_dir();
        if !fs.exists(&installed)? {
            return Ok(Vec::new());
        }
        let entries = fs.read_dir(&installed)?;
        let mut names: Vec<String> = entries
            .into_iter()
            .filter(|(_, is_dir)| *is_dir)
            .map(|(name, _)| name)
            .collect();
        names.sort();
        Ok(names)
    }
}
