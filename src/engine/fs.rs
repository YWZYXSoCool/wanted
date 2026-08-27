//! Abstract filesystem layer.
//!
//! The engine never touches `std::fs` directly: it accesses the filesystem
//! through the [`Fs`] trait. That way rollback logic can run on the real
//! filesystem or be unit-tested by replaying compensations against the in-memory
//! [`MemFs`] — "testing rollback tests real rollback".

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::Result;
use crate::error::io_err;

/// Filesystem operations available to the engine.
///
/// Methods take `&self`: the real implementation has no internal state, while the
/// in-memory one carries state through interior mutability.
pub trait Fs: Send {
    /// Read all bytes of a file.
    fn read(&self, path: &Path) -> Result<Vec<u8>>;
    /// Overwrite a file (creating it if missing, implicitly creating parents).
    fn write(&self, path: &Path, data: &[u8]) -> Result<()>;
    /// Whether the path exists (file or directory).
    fn exists(&self, path: &Path) -> Result<bool>;
    /// Recursively create directories.
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    /// Move a file or directory to the target (removing an existing target first).
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    /// Recursively remove a directory.
    fn remove_dir_all(&self, path: &Path) -> Result<()>;
    /// Remove a single file.
    fn remove_file(&self, path: &Path) -> Result<()>;
    /// List the direct children as `(name, is_dir)`.
    fn read_dir(&self, path: &Path) -> Result<Vec<(String, bool)>>;
}

/// The real filesystem, backed by `std::fs`.
pub struct RealFs;

impl Fs for RealFs {
    fn read(&self, path: &Path) -> Result<Vec<u8>> {
        std::fs::read(path).map_err(|e| io_err(path.to_path_buf(), e))
    }
    fn write(&self, path: &Path, data: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err(parent.to_path_buf(), e))?;
        }
        std::fs::write(path, data).map_err(|e| io_err(path.to_path_buf(), e))
    }
    fn exists(&self, path: &Path) -> Result<bool> {
        Ok(path.exists())
    }
    fn create_dir_all(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path).map_err(|e| io_err(path.to_path_buf(), e))
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let contains = Self::exists(self, to)?;
        if contains {
            Self::remove_dir_all(self, to)?;
        }
        std::fs::rename(from, to).map_err(|e| io_err(to.to_path_buf(), e))
    }
    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        std::fs::remove_dir_all(path).map_err(|e| io_err(path.to_path_buf(), e))
    }
    fn remove_file(&self, path: &Path) -> Result<()> {
        std::fs::remove_file(path).map_err(|e| io_err(path.to_path_buf(), e))
    }
    fn read_dir(&self, path: &Path) -> Result<Vec<(String, bool)>> {
        let mut out = Vec::new();
        let entries = std::fs::read_dir(path).map_err(|e| io_err(path.to_path_buf(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_err(path.to_path_buf(), e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry
                .file_type()
                .map_err(|e| io_err(path.to_path_buf(), e))?
                .is_dir();
            out.push((name, is_dir));
        }
        Ok(out)
    }
}

#[derive(Clone, Debug)]
enum MemNode {
    File(Vec<u8>),
    Dir,
}

/// A purely in-memory filesystem, used to safely replay rollback actions in tests.
#[derive(Default, Debug)]
pub struct MemFs {
    entries: RefCell<BTreeMap<PathBuf, MemNode>>,
}

impl MemFs {
    /// Construct an empty in-memory filesystem.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconstruct from bytes, handy for snapshot-style comparison.
    pub fn snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        for (path, node) in self.entries.borrow().iter() {
            if let MemNode::File(data) = node {
                out.push((path.clone(), data.clone()));
            }
        }
        out
    }

    fn key(path: &Path) -> PathBuf {
        let mut key = PathBuf::new();
        for comp in path.components() {
            match comp {
                Component::Normal(part) => key.push(part),
                Component::CurDir => {}
                Component::ParentDir => {
                    key.pop();
                }
                Component::RootDir | Component::Prefix(_) => {}
            }
        }
        key
    }

    fn ensure_ancestors(&self, path: &Path) {
        let mut map = self.entries.borrow_mut();
        let mut cur = PathBuf::new();
        for comp in path.components() {
            if let Component::Normal(part) = comp {
                cur.push(part);
                map.entry(cur.clone()).or_insert(MemNode::Dir);
            }
        }
    }
}

impl Fs for MemFs {
    fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let key = Self::key(path);
        match self.entries.borrow().get(&key) {
            Some(MemNode::File(data)) => Ok(data.clone()),
            Some(MemNode::Dir) => Err(crate::Error::Other(format!("is a directory: {path:?}"))),
            None => Err(crate::Error::Other(format!("no such file: {path:?}"))),
        }
    }
    fn write(&self, path: &Path, data: &[u8]) -> Result<()> {
        self.ensure_ancestors(path);
        self.entries
            .borrow_mut()
            .insert(Self::key(path), MemNode::File(data.to_vec()));
        Ok(())
    }
    fn exists(&self, path: &Path) -> Result<bool> {
        Ok(self.entries.borrow().contains_key(&Self::key(path)))
    }
    fn create_dir_all(&self, path: &Path) -> Result<()> {
        let mut map = self.entries.borrow_mut();
        let mut cur = PathBuf::new();
        for comp in path.components() {
            if let Component::Normal(part) = comp {
                cur.push(part);
                map.entry(cur.clone()).or_insert(MemNode::Dir);
            }
        }
        Ok(())
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let from_key = Self::key(from);
        let to_key = Self::key(to);
        let mut map = self.entries.borrow_mut();
        let doomed: Vec<PathBuf> = map
            .keys()
            .filter(|k| *k == &to_key || k.starts_with(&to_key))
            .cloned()
            .collect();
        for key in doomed {
            map.remove(&key);
        }
        let moving: Vec<(PathBuf, MemNode)> = map
            .iter()
            .filter(|(k, _)| *k == &from_key || k.starts_with(&from_key))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (key, node) in moving {
            map.remove(&key);
            let rel = key.strip_prefix(&from_key).expect("prefix checked above");
            map.insert(to_key.join(rel), node);
        }
        Ok(())
    }
    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let key = Self::key(path);
        let mut map = self.entries.borrow_mut();
        let doomed: Vec<PathBuf> = map
            .keys()
            .filter(|k| *k == &key || k.starts_with(&key))
            .cloned()
            .collect();
        for d in doomed {
            map.remove(&d);
        }
        Ok(())
    }
    fn remove_file(&self, path: &Path) -> Result<()> {
        self.entries.borrow_mut().remove(&Self::key(path));
        Ok(())
    }
    fn read_dir(&self, path: &Path) -> Result<Vec<(String, bool)>> {
        let base = Self::key(path);
        let map = self.entries.borrow();
        let mut out = Vec::new();
        for key in map.keys() {
            if key == &base {
                continue;
            }
            let Ok(rel) = key.strip_prefix(&base) else {
                continue;
            };
            if rel.components().count() == 1 {
                let name = rel
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let is_dir = matches!(map.get(key), Some(MemNode::Dir));
                out.push((name, is_dir));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
