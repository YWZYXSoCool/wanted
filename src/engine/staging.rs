//! Staging area lifecycle.
//!
//! Two-phase commit: downloads and extraction go entirely into the staging area
//! (discardable as a whole); only commit moves them into place. Any failure
//! before commit is rolled back by `abort` deleting the directory — no
//! compensation enum needed.

use std::path::{Path, PathBuf};

use crate::Result;
use crate::engine::fs::Fs;
use crate::fs_path::DirName;

/// Handle to a staging area.
pub struct Staging {
    dir: PathBuf,
}

impl Staging {
    /// Construct (without creating) a staging directory isolated by a unique suffix.
    pub fn new(root: &Path, name: &DirName) -> Staging {
        let nonce = unique_nonce();
        Staging::from_dir(
            root.join(".staging")
                .join(format!("{}-{nonce}", name.as_str())),
        )
    }

    /// Construct from an already-resolved path (the execution layer reproduces the
    /// plan's staging path).
    pub fn from_dir(dir: PathBuf) -> Staging {
        Staging { dir }
    }

    /// Clear any leftover staging area so work starts from a clean slate.
    pub fn ensure_clean(&self, fs: &dyn Fs) -> Result<()> {
        if fs.exists(&self.dir)? {
            fs.remove_dir_all(&self.dir)?;
        }
        Ok(())
    }

    /// The staging directory's absolute path.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Discard the staging area (the fallback rollback for a pre-commit failure).
    pub fn abort(&self, fs: &dyn Fs) -> Result<()> {
        if fs.exists(&self.dir)? {
            fs.remove_dir_all(&self.dir)?;
        }
        Ok(())
    }
}

/// Build a unique staging directory suffix from the timestamp.
fn unique_nonce() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:x}")
}

#[cfg(test)]
mod tests;
