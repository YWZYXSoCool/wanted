//! Persistent write-back backend for environment variables.

use crate::Result;
use crate::env::EnvStore;
use crate::error::Error;

/// A real persistent backend based on the Windows user-level registry
/// (`HKCU\Environment`).
///
/// M0 lands on Windows first; POSIX rc-file writing is left for later.
pub struct RealEnvStore;

impl RealEnvStore {
    /// Construct the real backend.
    pub fn new() -> Self {
        Self
    }
}

impl EnvStore for RealEnvStore {
    #[cfg(windows)]
    fn read(&self, name: &crate::env::EnvVar) -> Result<Option<String>> {
        match winreg_read(name.as_str()) {
            Ok(value) => Ok(Some(value)),
            Err(io) if io.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(io) => Err(Error::Other(format!("read env {name}: {io}"))),
        }
    }
    #[cfg(windows)]
    fn write(&self, name: &crate::env::EnvVar, value: &str) -> Result<()> {
        winreg_write(name.as_str(), value)
    }
    #[cfg(windows)]
    fn remove(&self, name: &crate::env::EnvVar) -> Result<()> {
        winreg_remove(name.as_str())
    }

    #[cfg(not(windows))]
    fn read(&self, _name: &crate::env::EnvVar) -> Result<Option<String>> {
        Err(Error::Unsupported("persistent env write on POSIX"))
    }
    #[cfg(not(windows))]
    fn write(&self, _name: &crate::env::EnvVar, _value: &str) -> Result<()> {
        Err(Error::Unsupported("persistent env write on POSIX"))
    }
    #[cfg(not(windows))]
    fn remove(&self, _name: &crate::env::EnvVar) -> Result<()> {
        Err(Error::Unsupported("persistent env write on POSIX"))
    }
}

impl Default for RealEnvStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
const ENV_KEY: &str = "Environment";

#[cfg(windows)]
fn env_key() -> Result<winreg::RegKey> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let flags = KEY_QUERY_VALUE | KEY_SET_VALUE;
    hkcu.open_subkey_with_flags(ENV_KEY, flags)
        .or_else(|_| hkcu.create_subkey(ENV_KEY).map(|(k, _)| k))
        .map_err(|e| Error::Other(format!("open env key: {e}")))
}

#[cfg(windows)]
fn winreg_read(name: &str) -> std::io::Result<String> {
    let key = env_key().map_err(std::io::Error::other)?;
    key.get_value::<String, _>(name)
}

#[cfg(windows)]
fn winreg_write(name: &str, value: &str) -> Result<()> {
    let key = env_key()?;
    key.set_value(name, &value.to_string())
        .map_err(|e| Error::Other(format!("set env {name}: {e}")))
}

#[cfg(windows)]
fn winreg_remove(name: &str) -> Result<()> {
    let key = env_key()?;
    key.delete_value(name)
        .map_err(|e| Error::Other(format!("delete env {name}: {e}")))
}
