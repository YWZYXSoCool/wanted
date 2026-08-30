//! Filesystem path primitives whose validity depends on the platform.
//!
//! A name that is a safe directory segment on one OS can be unusable on
//! another. These newtypes make that platform rule a property of the type, so
//! it cannot be forgotten at the point of use.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

/// A single filesystem directory segment, validated against the *current*
/// platform's rules.
///
/// POSIX forbids only `/` and NUL; Windows additionally forbids `<>:"\|?*`,
/// trailing dots/spaces, and reserved device names (`CON`, `NUL`, `COM1-9`, …).
/// Construction **rejects** such names (rather than sanitizing them) so install
/// directories are predictable and collision-free on every OS.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DirName(String);

impl DirName {
    /// The directory segment as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DirName {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate(&value)?;
        Ok(DirName(value))
    }
}

impl AsRef<str> for DirName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<Path> for DirName {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl std::fmt::Display for DirName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a name was rejected as a directory segment.
fn invalid(name: &str, detail: &str) -> Error {
    Error::InvalidName {
        name: name.to_string(),
        detail: detail.to_string(),
    }
}

fn validate(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(invalid(name, "empty"));
    }
    if name == "." || name == ".." {
        return Err(invalid(name, "cannot be '.' or '..'"));
    }
    if name.contains('\0') {
        return Err(invalid(name, "contains a NUL byte"));
    }
    if name.contains('/') {
        return Err(invalid(name, "contains '/'"));
    }
    #[cfg(windows)]
    {
        if name
            .chars()
            .any(|c| matches!(c, '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*'))
        {
            return Err(invalid(
                name,
                "contains a character not allowed in a Windows directory name",
            ));
        }
        if name != name.trim_end_matches(['.', ' ']) {
            return Err(invalid(
                name,
                "ends in a '.' or space, which Windows reserves",
            ));
        }
        if is_windows_reserved_device(name) {
            return Err(invalid(name, "is a Windows reserved device name"));
        }
    }
    Ok(())
}

/// Whether `name` is a Windows reserved device name (`CON`, `PRN`, `AUX`, `NUL`,
/// `COM1`-`COM9`, `LPT1`-`LPT9`). Only meaningful on Windows.
#[cfg(windows)]
fn is_windows_reserved_device(name: &str) -> bool {
    let trailing: String = name
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let trailing: String = trailing.chars().rev().collect();
    let base = &name[..name.len() - trailing.len()];
    match base {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        "COM" | "LPT" => !trailing.is_empty(),
        _ => false,
    }
}

/// The installed tool root directory, as recorded in a receipt.
///
/// Centralizes the lossy path <-> string boundary: receipts are persisted as
/// plain strings (byte-identical to the old `String` field), and this type
/// converts to/from `PathBuf` in one place so callers never hand-roll it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AppDir(String);

impl AppDir {
    /// Wrap a native path, capturing it in its platform string form.
    pub fn from_path(path: &Path) -> Self {
        AppDir(path.to_string_lossy().into_owned())
    }

    /// The stored path, for reads and removals.
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl From<String> for AppDir {
    fn from(value: String) -> Self {
        AppDir(value)
    }
}

impl From<&str> for AppDir {
    fn from(value: &str) -> Self {
        AppDir(value.to_owned())
    }
}

impl AsRef<Path> for AppDir {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl From<AppDir> for PathBuf {
    fn from(value: AppDir) -> Self {
        PathBuf::from(value.0)
    }
}
