//! Plugin manifest domain model and parsing.
//!
//! TOML manifest -> validated internal model [`Manifest`]. The parsed result can
//! be consumed by the planning layer.

pub mod target;

pub mod raw;

use std::collections::BTreeMap;
use std::path::Path;

use crate::Result;
use crate::error::Error;

use raw::RawManifest;
pub use raw::{AssetMap, EnvBox, InstallMethod, RawCommand, RawStrategy};

pub use target::Target;

/// The default source name used when none is explicitly requested.
pub(crate) const DEFAULT_SOURCE: &str = "default";

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
    /// Default install method.
    pub method: InstallMethod,
    /// Platform triplet -> source name -> URL template (with `{version}`).
    pub assets: AssetMap,
    /// Optional components (platform-keyed like `assets`), downloaded only when enabled.
    pub components: BTreeMap<String, AssetMap>,
    /// Placement path relative to `apps`.
    pub base_dir: String,
    /// The PATH box.
    pub env_box: EnvBox,
    /// Default silent-install arguments (with `{base}`), used by the `installer` method.
    pub args: Vec<String>,
    /// Per-platform override of method and silent arguments.
    pub strategy: BTreeMap<String, RawStrategy>,
    /// External install commands for the `command` method, keyed by platform
    /// triplet; the ordered list is tried in fallback order until one succeeds.
    pub commands: BTreeMap<String, Vec<RawCommand>>,
    /// Fallback install methods, tried in order after the primary `method`
    /// fails; each reuses this section's already-declared data.
    pub fallback: Vec<InstallMethod>,
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
        let mut attempts = raw
            .install
            .fallback
            .iter()
            .copied()
            .chain(std::iter::once(raw.install.method));
        let archive_attempt = {
            let mut it = attempts.clone();
            it.any(|m| matches!(m, InstallMethod::Download | InstallMethod::Installer))
        };
        if archive_attempt && raw.install.asset.is_empty() {
            return Err(Error::MissingField {
                field: "install.asset",
            });
        }
        if attempts.any(|m| m == InstallMethod::Command) && raw.install.command.is_empty() {
            return Err(Error::MissingField {
                field: "install.command",
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
                components: raw.install.component,
                base_dir: raw.install.base_dir,
                env_box: raw.install.env_box.unwrap_or(EnvBox::Prepend),
                args: raw.install.args,
                strategy: raw.install.strategy,
                commands: raw.install.command,
                fallback: raw.install.fallback,
            },
            env: raw.env,
        })
    }
}

impl Install {
    /// Resolve the install method and silent args for a platform: a matching
    /// `strategy` entry wins, otherwise the install-level defaults apply.
    pub fn method_for(&self, target: &Target) -> (&InstallMethod, &[String]) {
        let Some(entry) = self.strategy.get(&target.triplet()) else {
            return (&self.method, &self.args);
        };
        (&entry.method, &entry.args)
    }
}

#[cfg(test)]
mod tests;
