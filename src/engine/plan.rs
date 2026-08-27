//! Planning (pure function).
//!
//! Manifest + platform target + version -> a lazy [`Plan`]. This module does no
//! I/O, so it is unit-testable, `--dry-run`-able, and auditable. Side effects
//! are deferred to the execution layer.

use std::path::{Path, PathBuf};

use crate::Result;
use crate::engine::ops::Op;
use crate::engine::staging::Staging;
use crate::env::EnvDelta;
use crate::plugin::{DEFAULT_SOURCE, InstallMethod, Manifest, Target};

/// A complete install plan (two phases: staged ops + commit ops).
#[derive(Clone, Debug)]
pub struct Plan {
    /// Tool name.
    pub name: String,
    /// Version to install.
    pub version: String,
    /// Staging directory.
    pub staging_dir: PathBuf,
    /// File the archive is downloaded to.
    pub download_to: PathBuf,
    /// Extracted app root (to be moved to `dest_dir`).
    pub app_dir: PathBuf,
    /// Final home directory (`apps/<base_dir>`).
    pub dest_dir: PathBuf,
    /// Staging-phase operations (Download / Unpack).
    pub staged_ops: Vec<Op>,
    /// Commit-phase operations (WriteEnv). Not atomic; backed by compensations.
    pub commit_ops: Vec<Op>,
}

impl Plan {
    /// Environment deltas the commit phase will write, so the installer can
    /// snapshot old values and build a receipt.
    pub fn env_deltas(&self) -> Vec<EnvDelta> {
        let mut deltas = Vec::new();
        for op in &self.commit_ops {
            if let Op::WriteEnv { deltas: d } = op {
                deltas.extend(d.clone());
            }
        }
        deltas
    }
}

impl Manifest {
    /// Build an install plan for the current platform, version, and download
    /// source (pure calculation).
    pub fn plan(
        &self,
        root: &Path,
        target: &Target,
        version: &str,
        source: Option<&str>,
    ) -> Result<Plan> {
        let url = self.install_url(target, version, source)?;
        let base_dir = PathBuf::from(&self.install.base_dir);
        let staging = Staging::new(&root.join(".wanted"), &self.meta.name);
        let staging_dir = staging.dir().to_path_buf();
        let download_to = staging_dir.join("downloads").join(archive_name(&url));
        let app_dir = staging_dir.join("app");
        let dest_dir = root.join(".wanted").join("apps").join(&base_dir);
        let deltas = self.env_deltas(dest_dir.parent().unwrap_or(Path::new("")), version)?;

        Ok(Plan {
            name: self.meta.name.clone(),
            version: version.to_string(),
            staging_dir,
            app_dir: app_dir.clone(),
            dest_dir,
            staged_ops: vec![
                Op::Download {
                    url,
                    to: download_to.clone(),
                },
                Op::Unpack {
                    from: download_to.clone(),
                    to: app_dir,
                },
            ],
            download_to,
            commit_ops: vec![Op::WriteEnv { deltas }],
        })
    }

    /// Pick the asset URL for the current platform and source, substituting `{version}`.
    fn install_url(&self, target: &Target, version: &str, source: Option<&str>) -> Result<String> {
        if self.install.method != InstallMethod::Download {
            return Err(crate::Error::Unsupported(
                "install method 'system' not yet wired",
            ));
        }
        let sources = self.install.assets.get(&target.triplet()).ok_or_else(|| {
            crate::Error::UnsupportedPlatform {
                target: target.triplet(),
            }
        })?;
        let name = source.unwrap_or(DEFAULT_SOURCE);
        let template = sources
            .get(name)
            .ok_or_else(|| crate::Error::SourceNotFound {
                target: target.triplet(),
                name: name.to_string(),
            })?;
        Ok(template.replace("{version}", version))
    }

    /// Compute the pure deltas from the manifest's env declarations (no side effects).
    fn env_deltas(&self, apps_root: &Path, version: &str) -> Result<Vec<EnvDelta>> {
        let mut deltas = Vec::new();
        let base = apps_root.join(&self.install.base_dir);
        let user_home = crate::env::user_home();
        for (name, template) in &self.env {
            let op = if name == "PATH" {
                match self.install.env_box {
                    crate::plugin::EnvBox::Prepend => crate::env::EnvOp::Prepend,
                    crate::plugin::EnvBox::Append => crate::env::EnvOp::Append,
                }
            } else {
                crate::env::EnvOp::Set
            };
            deltas.push(EnvDelta {
                name: name.clone(),
                value: resolve_template(template, &base, &user_home, version),
                op,
            });
        }
        Ok(deltas)
    }
}

/// Expand template placeholders, joining relative paths under `base`.
fn resolve_template(template: &str, base: &Path, user_home: &str, version: &str) -> String {
    let substituted = template
        .replace("{version}", version)
        .replace("{user}", user_home);
    let value_path = Path::new(&substituted);
    if template.starts_with('$') || value_path.is_absolute() {
        substituted
    } else {
        base.join(value_path).to_string_lossy().into_owned()
    }
}

/// Extract a file name from the URL for local persistence.
fn archive_name(url: &str) -> String {
    url.rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("archive.bin")
        .to_string()
}
