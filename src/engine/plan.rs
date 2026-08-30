//! Planning (pure function).
//!
//! Manifest + platform target + version -> a lazy [`Plan`]. This module does no
//! I/O, so it is unit-testable, `--dry-run`-able, and auditable. Side effects
//! are deferred to the execution layer.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::Version;
use crate::engine::expand::{expand_template, resolve_template};
use crate::engine::ops::{CommandInvocation, Op};
use crate::engine::staging::Staging;
use crate::engine::url::Url;
use crate::env::{EnvDelta, EnvVar};
use crate::fs_path::DirName;
use crate::plugin::{AssetMap, DEFAULT_SOURCE, InstallMethod, Manifest, Target};

/// Install-time choices supplied by the user: the asset source and any optional
/// components to download on top of the base asset.
#[derive(Clone, Debug, Default)]
pub struct Selection<'a> {
    /// Named asset source; `None` picks the plugin's `default` source.
    pub source: Option<&'a str>,
    /// Names of optional components to download, in declaration-independent order.
    pub components: &'a [String],
}

/// A complete install plan (two phases: staged ops + commit ops).
#[derive(Clone, Debug)]
pub struct Plan {
    /// Tool name (a validated directory segment, also used for the receipt).
    pub name: DirName,
    /// Version to install.
    pub version: Version,
    /// Staging directory.
    pub staging_dir: PathBuf,
    /// File the archive is downloaded to.
    pub download_to: PathBuf,
    /// Extracted app root (to be moved to `dest_dir`).
    pub app_dir: PathBuf,
    /// Final home directory (`<base_dir>` in the run directory).
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
        version: &Version,
        selection: &Selection,
    ) -> Result<Plan> {
        let (method, args) = self.install.method_for(target);
        self.plan_method(method, args, root, target, version, selection)
    }

    /// Build the ordered fallback chain of buildable plans: `[primary]` then
    /// every declared `fallback` method that resolves for this platform. A
    /// method whose data is unavailable for `target` is skipped, so the caller
    /// (`execute_chain`) can fall through to the next one instead of aborting.
    pub fn plan_chain(
        &self,
        root: &Path,
        target: &Target,
        version: &Version,
        selection: &Selection,
    ) -> Vec<Plan> {
        let (method, args) = self.install.method_for(target);
        let mut plans = Vec::new();
        if let Ok(plan) = self.plan_method(method, args, root, target, version, selection) {
            plans.push(plan);
        }
        for fallback in self.install.fallback.iter().filter(|m| *m != method) {
            let attempts: &[String] = if *fallback == InstallMethod::Installer {
                &self.install.args
            } else {
                &[]
            };
            if let Ok(plan) = self.plan_method(fallback, attempts, root, target, version, selection)
            {
                plans.push(plan);
            }
        }
        plans
    }

    /// Build a plan for one explicit method, dispatching on its data.
    fn plan_method(
        &self,
        method: &InstallMethod,
        args: &[String],
        root: &Path,
        target: &Target,
        version: &Version,
        selection: &Selection,
    ) -> Result<Plan> {
        match method {
            InstallMethod::Installer => self.plan_installer(root, target, version, selection, args),
            InstallMethod::System => Err(crate::Error::Unsupported(
                "install method 'system' not yet wired",
            )),
            InstallMethod::Download => self.plan_download(root, target, version, selection),
            InstallMethod::Command => self.plan_command(root, target, version, selection),
        }
    }

    /// Plan a download-and-extract install (unpack a vendored archive).
    fn plan_download(
        &self,
        root: &Path,
        target: &Target,
        version: &Version,
        selection: &Selection,
    ) -> Result<Plan> {
        let url = self.install_url(target, version, selection.source)?;
        let base_dir = &self.install.base_dir;
        let staging = Staging::new(&root.join(".wanted"), &self.meta.name);
        let staging_dir = staging.dir().to_path_buf();
        let download_to = staging_dir.join("downloads").join(url.file_name());
        let app_dir = staging_dir.join("app");
        let dest_dir = root.join(base_dir);
        let deltas =
            self.env_deltas(dest_dir.parent().unwrap_or(Path::new("")), target, version)?;

        let mut staged_ops = vec![
            Op::Download {
                url,
                to: download_to.clone(),
            },
            Op::Unpack {
                from: download_to.clone(),
                to: app_dir.clone(),
            },
        ];
        let mut seen = BTreeSet::new();
        for component in selection.components {
            if !seen.insert(component) {
                continue;
            }
            self.component_ops(
                component,
                target,
                version,
                selection.source,
                &staging_dir,
                &mut staged_ops,
            )?;
        }

        Ok(Plan {
            name: self.meta.name.clone(),
            version: version.clone(),
            staging_dir,
            app_dir,
            dest_dir,
            staged_ops,
            download_to,
            commit_ops: vec![Op::WriteEnv { deltas }],
        })
    }

    /// Plan a silent-installer install: download it, then run it into `dest_dir`.
    fn plan_installer(
        &self,
        root: &Path,
        target: &Target,
        version: &Version,
        selection: &Selection,
        args: &[String],
    ) -> Result<Plan> {
        if !selection.components.is_empty() {
            return Err(crate::Error::Unsupported(
                "components are only supported for the 'download' method",
            ));
        }
        let url = self.install_url(target, version, selection.source)?;
        let base_dir = &self.install.base_dir;
        let staging = Staging::new(&root.join(".wanted"), &self.meta.name);
        let staging_dir = staging.dir().to_path_buf();
        let download_to = staging_dir.join("downloads").join(url.file_name());
        let dest_dir = root.join(base_dir);
        let expanded = args
            .iter()
            .map(|arg| expand_template(arg, &dest_dir, version))
            .collect();
        let deltas =
            self.env_deltas(dest_dir.parent().unwrap_or(Path::new("")), target, version)?;

        let staged_ops = vec![
            Op::Download {
                url,
                to: download_to.clone(),
            },
            Op::RunInstaller {
                exe: download_to.clone(),
                args: expanded,
                base: dest_dir.clone(),
            },
        ];
        Ok(Plan {
            name: self.meta.name.clone(),
            version: version.clone(),
            staging_dir,
            app_dir: dest_dir.clone(),
            dest_dir,
            staged_ops,
            download_to,
            commit_ops: vec![Op::WriteEnv { deltas }],
        })
    }

    /// Plan a command install: run external package-manager commands in fallback
    /// order, writing the tool straight into `<base_dir>` in the run directory.
    fn plan_command(
        &self,
        root: &Path,
        target: &Target,
        version: &Version,
        selection: &Selection,
    ) -> Result<Plan> {
        if !selection.components.is_empty() {
            return Err(crate::Error::Unsupported(
                "components are only supported for the 'download' method",
            ));
        }
        let raw_commands = self
            .install
            .commands
            .get(&target.triplet())
            .ok_or_else(|| crate::Error::UnsupportedPlatform {
                target: target.triplet(),
            })?;
        let base_dir = &self.install.base_dir;
        let staging = Staging::new(&root.join(".wanted"), &self.meta.name);
        let staging_dir = staging.dir().to_path_buf();
        let dest_dir = root.join(base_dir);
        let deltas =
            self.env_deltas(dest_dir.parent().unwrap_or(Path::new("")), target, version)?;
        let commands = raw_commands
            .iter()
            .map(|raw| CommandInvocation::from_raw(raw, &dest_dir, version))
            .collect();
        let base = dest_dir.clone();
        let download_to = staging_dir.join("downloads");

        Ok(Plan {
            name: self.meta.name.clone(),
            version: version.clone(),
            staging_dir,
            download_to,
            app_dir: dest_dir.clone(),
            dest_dir,
            staged_ops: vec![Op::RunCommand { commands, base }],
            commit_ops: vec![Op::WriteEnv { deltas }],
        })
    }

    /// Pick the asset URL for the current platform and source, substituting `{version}`.
    fn install_url(&self, target: &Target, version: &Version, source: Option<&str>) -> Result<Url> {
        let template = self.source_template(&self.install.assets, target, source)?;
        Ok(Url::from(
            template.replace("{version}", &version.to_string()),
        ))
    }

    /// Push a Download + Unpack pair for one enabled component, extracted under
    /// `staging_dir/app/<name>`.
    fn component_ops(
        &self,
        component: &str,
        target: &Target,
        version: &Version,
        source: Option<&str>,
        staging_dir: &Path,
        staged_ops: &mut Vec<Op>,
    ) -> Result<()> {
        let assets = self.install.components.get(component).ok_or_else(|| {
            crate::Error::UnknownComponent {
                name: component.to_string(),
            }
        })?;
        let template = self.source_template(assets, target, source)?;
        let url = Url::from(template.replace("{version}", &version.to_string()));
        let to = staging_dir
            .join("downloads")
            .join(format!("{component}-{}", url.file_name()));
        staged_ops.push(Op::Download {
            url,
            to: to.clone(),
        });
        staged_ops.push(Op::Unpack {
            from: to,
            to: staging_dir.join("app").join(component),
        });
        Ok(())
    }

    /// Resolve the URL template for the current platform and source across any
    /// platform-keyed asset map (`assets` or a component's).
    fn source_template<'a>(
        &self,
        assets: &'a AssetMap,
        target: &Target,
        source: Option<&str>,
    ) -> Result<&'a str> {
        let sources =
            assets
                .get(&target.triplet())
                .ok_or_else(|| crate::Error::UnsupportedPlatform {
                    target: target.triplet(),
                })?;
        let name = source.unwrap_or(DEFAULT_SOURCE);
        sources
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| crate::Error::SourceNotFound {
                target: target.triplet(),
                name: name.to_string(),
            })
    }

    /// Compute the pure deltas from the manifest's env declarations. A matching
    /// `env_by_platform` entry for `target` overrides the variable's template.
    fn env_deltas(
        &self,
        run_dir: &Path,
        target: &Target,
        version: &Version,
    ) -> Result<Vec<EnvDelta>> {
        let mut deltas = Vec::new();
        let base = run_dir.join(&self.install.base_dir);
        let platform_env = self.env_by_platform.get(&target.triplet());
        for (raw_name, template) in &self.env {
            let name = EnvVar::from(raw_name.as_str());
            let template = platform_env
                .and_then(|vars| vars.get(raw_name))
                .unwrap_or(template);
            let op = if name.is_path() {
                match self.install.env_box {
                    crate::plugin::EnvBox::Prepend => crate::env::EnvOp::Prepend,
                    crate::plugin::EnvBox::Append => crate::env::EnvOp::Append,
                }
            } else {
                crate::env::EnvOp::Set
            };
            deltas.push(EnvDelta {
                name,
                value: resolve_template(template, &base, version),
                op,
            });
        }
        Ok(deltas)
    }
}
