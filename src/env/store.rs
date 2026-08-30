//! Persistent write-back backend for environment variables.

use std::path::PathBuf;
#[cfg(not(windows))]
use std::sync::atomic::{AtomicU64, Ordering};

use crate::Result;
use crate::env::EnvStore;
use crate::error::Error;

/// A real persistent backend. On Windows it writes the user-level registry
/// (`HKCU\Environment`); on POSIX it maintains an `export`-line script at
/// `~/.wanted/env.sh` that the user sources once from their shell rc, so a fresh
/// terminal picks up every later write automatically.
pub struct RealEnvStore {
    #[cfg_attr(windows, allow(dead_code))]
    file: PathBuf,
}

impl RealEnvStore {
    /// Construct the real backend for the current user.
    #[cfg(windows)]
    pub fn new() -> Result<Self> {
        Ok(RealEnvStore {
            file: PathBuf::new(),
        })
    }
    /// Construct the real backend for the current user.
    #[cfg(not(windows))]
    pub fn new() -> Result<Self> {
        let home = std::env::var("HOME")
            .map_err(|_| Error::Other("cannot resolve $HOME for the env file".into()))?;
        Ok(RealEnvStore::at(
            PathBuf::from(home).join(".wanted").join("env.sh"),
        ))
    }

    /// Construct a backend writing to an explicit file, for tests and tooling.
    #[cfg(not(windows))]
    pub fn at(file: PathBuf) -> Self {
        RealEnvStore { file }
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
    fn read(&self, name: &crate::env::EnvVar) -> Result<Option<String>> {
        Ok(std::env::var(name.as_str()).ok())
    }
    #[cfg(not(windows))]
    fn write(&self, name: &crate::env::EnvVar, value: &str) -> Result<()> {
        let lines = match read_lines(&self.file)? {
            Some(lines) => upsert(lines, name.as_str(), value),
            None => vec![export_line(name.as_str(), value)],
        };
        write_lines_atomic(&self.file, &lines)
    }
    #[cfg(not(windows))]
    fn remove(&self, name: &crate::env::EnvVar) -> Result<()> {
        let existing = match read_lines(&self.file)? {
            None => return Ok(()),
            Some(lines) => lines,
        };
        let kept: Vec<String> = existing
            .into_iter()
            .filter(|l| !is_var_line(l, name.as_str()))
            .collect();
        write_lines_atomic(&self.file, &kept)
    }
}

impl Default for RealEnvStore {
    fn default() -> Self {
        match Self::new() {
            Ok(store) => store,
            Err(_) => Self {
                file: PathBuf::from(".wanted").join("env.sh"),
            },
        }
    }
}

#[cfg(not(windows))]
fn export_line(name: &str, value: &str) -> String {
    format!("export {name}=\"{}\"", escape_value(value))
}

/// Replace every line declaring `name` with a single `export name="value"`,
/// appending when none exists.
#[cfg(not(windows))]
fn upsert(lines: Vec<String>, name: &str, value: &str) -> Vec<String> {
    let mut changed = false;
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        if is_var_line(&line, name) {
            if changed {
                continue;
            }
            out.push(export_line(name, value));
            changed = true;
        } else {
            out.push(line);
        }
    }
    if !changed {
        out.push(export_line(name, value));
    }
    out
}

/// Whether a line is an `export <name>=...` declaration for the given name.
#[cfg(not(windows))]
fn is_var_line(line: &str, name: &str) -> bool {
    line.strip_prefix("export ")
        .and_then(|rest| rest.split('=').next())
        == Some(name)
}

/// Shell-escape a value so it stays inside a single double-quoted string.
#[cfg(not(windows))]
fn escape_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Read the file's lines, or `None` when it does not exist.
#[cfg(not(windows))]
fn read_lines(path: &PathBuf) -> Result<Option<Vec<String>>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(Error::Io {
                path: path.clone(),
                source: err,
            });
        }
    };
    Ok(Some(text.lines().map(str::to_owned).collect()))
}

/// Monotonic sequence granting each in-process write its own temp path. A
/// pid-only temp name would let one writer's `rename` evict another writer's
/// still-pending file, so its `rename` would then fail with `NotFound`; pid plus
/// sequence stays unique both within and across processes.
#[cfg(not(windows))]
static WRITE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write the script atomically (temp file + rename) so a reader never sees a
/// partially-written file.
#[cfg(not(windows))]
fn write_lines_atomic(path: &PathBuf, lines: &[String]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| Error::Io {
        path: parent.to_path_buf(),
        source: e,
    })?;
    let seq = WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".env.sh.{}.{seq}.tmp", std::process::id()));
    let write = std::fs::File::create(&tmp).and_then(|mut f| {
        for line in lines {
            writeln!(f, "{line}")?;
        }
        Ok(())
    });
    if let Err(err) = write {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::Io {
            path: tmp,
            source: err,
        });
    }
    std::fs::rename(&tmp, path).map_err(|e| Error::Io {
        path: path.clone(),
        source: e,
    })
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

#[cfg(all(test, not(windows)))]
mod tests;
