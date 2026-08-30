//! The `.wanted` directory layout.

use std::path::PathBuf;

use crate::Result;
use crate::engine::fs::Fs;
use crate::fs_path::DirName;

/// Layout accessor for the persistent root directory.
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Construct around a root directory (under which `.wanted` lives).
    pub fn new(root: PathBuf) -> Self {
        Store { root }
    }

    /// The project root directory.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// The `.wanted` directory.
    pub fn store_dir(&self) -> PathBuf {
        self.root.join(".wanted")
    }

    /// Where installed tools are placed.
    pub fn apps_dir(&self) -> PathBuf {
        self.store_dir().join("apps")
    }

    /// Where install receipts live.
    pub fn installed_dir(&self) -> PathBuf {
        self.store_dir().join("installed")
    }

    /// The receipt path for one tool.
    pub fn receipt_path(&self, name: &DirName) -> PathBuf {
        self.installed_dir().join(name).join("receipt.toml")
    }

    /// List installed tool names (direct subdirectories of `apps`).
    pub fn list_installed(&self, fs: &dyn Fs) -> Result<Vec<String>> {
        let apps = self.apps_dir();
        if !fs.exists(&apps)? {
            return Ok(Vec::new());
        }
        let entries = fs.read_dir(&apps)?;
        let mut names: Vec<String> = entries
            .into_iter()
            .filter(|(_, is_dir)| *is_dir)
            .map(|(name, _)| name)
            .collect();
        names.sort();
        Ok(names)
    }
}
