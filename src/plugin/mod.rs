//! Plugin manifest domain model and parsing.
//!
//! TOML manifest -> validated internal model [`Manifest`]. The parsed result can
//! be consumed by the planning layer.

pub mod target;

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::Result;
use crate::error::Error;

pub use target::Target;

/// The default source name used when none is explicitly requested.
pub(crate) const DEFAULT_SOURCE: &str = "default";

/// Install method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallMethod {
    /// Extract the vendor-distributed archive directly.
    Download,
    /// Delegate to a system package manager (`winget` / `brew`); not wired in M0.
    System,
}

/// The PATH box (prepend or append).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvBox {
    /// Prepend.
    Prepend,
    /// Append.
    Append,
}

/// Manifest metadata.
#[derive(Clone, Debug)]
pub struct Meta {
    /// Tool name.
    pub name: String,
    /// Manifest's own version (SemVer).
    pub version: String,
    /// Source / update origin.
    pub url: Option<String>,
}

/// Install section.
#[derive(Clone, Debug)]
pub struct Install {
    /// Install method.
    pub method: InstallMethod,
    /// Platform triplet -> source name -> URL template (with `{version}`).
    pub assets: BTreeMap<String, BTreeMap<String, String>>,
    /// Placement path relative to `apps`.
    pub base_dir: String,
    /// The PATH box.
    pub env_box: EnvBox,
}

/// A validated plugin manifest.
#[derive(Clone, Debug)]
pub struct Manifest {
    /// Metadata.
    pub meta: Meta,
    /// Install section.
    pub install: Install,
    /// Environment variables to configure (name -> template).
    pub env: BTreeMap<String, String>,
}

impl Manifest {
    /// Parse and validate from inline TOML text.
    pub fn parse(source: &str) -> Result<Self> {
        let raw: RawManifest =
            toml::from_str(source).map_err(|e| Error::Manifest(e.to_string()))?;
        Self::from_raw(raw)
    }

    /// Read and parse from a file path.
    pub fn load(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| crate::error::io_err(path.to_path_buf(), e))?;
        Self::parse(&source)
    }

    fn from_raw(raw: RawManifest) -> Result<Self> {
        let name = raw
            .meta
            .name
            .ok_or(Error::MissingField { field: "meta.name" })?;
        let version = raw.meta.version.ok_or(Error::MissingField {
            field: "meta.version",
        })?;
        if raw.install.method == InstallMethod::Download && raw.install.asset.is_empty() {
            return Err(Error::MissingField {
                field: "install.asset",
            });
        }
        Ok(Manifest {
            meta: Meta {
                name,
                version,
                url: raw.meta.url,
            },
            install: Install {
                method: raw.install.method,
                assets: raw.install.asset,
                base_dir: raw.install.base_dir,
                env_box: raw.install.env_box.unwrap_or(EnvBox::Prepend),
            },
            env: raw.env,
        })
    }
}

#[derive(Deserialize)]
struct RawManifest {
    meta: RawMeta,
    install: RawInstall,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct RawMeta {
    name: Option<String>,
    version: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct RawInstall {
    method: InstallMethod,
    #[serde(default)]
    asset: BTreeMap<String, BTreeMap<String, String>>,
    base_dir: String,
    #[serde(default)]
    env_box: Option<EnvBox>,
}

#[cfg(test)]
mod tests;
