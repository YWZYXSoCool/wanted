//! Self-upgrade: replace the running `wanted` binary from a GitHub release.
//!
//! Distribution runs through the `YWZYXSoCool/wanted` Releases API (see
//! [`RELEASES_URL`]). CI ships one portable binary per platform, named
//! `wanted-<tag>-<os>-<arch>[.exe]` (see `.github/workflows/ci.yml`). Discovery
//! resolves the latest release, picks this platform's asset, enforces its
//! `.sha256` companion, and swaps the binary in. The swap uses a rename dance so
//! a running executable is never overwritten in place; the previous binary is
//! kept as a `.old` sibling and cleared on the next start, giving each upgrade a
//! rollback path.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Result;
use crate::engine::{Downloader, Fs};
use crate::error::io_err;
use crate::report::{Progress, Reporter};

/// Base GitHub API endpoint for the `wanted` project's latest release.
pub const RELEASES_URL: &str = "https://api.github.com/repos/YWZYXSoCool/wanted/releases/latest";

/// A single release asset: this platform's binary and its download URL.
#[derive(Clone, Debug)]
pub struct Asset {
    /// The binary's asset name on the release (e.g. `wanted-v0.2.0-linux-x86_64`).
    pub name: String,
    /// Download URL for the binary.
    pub url: String,
}

/// A discovered latest release, resolved to the running platform.
#[derive(Clone, Debug)]
pub struct Release {
    /// The latest release version.
    pub version: semver::Version,
    /// This platform's binary asset.
    pub asset: Asset,
}

impl Release {
    /// Parse the GitHub `releases/latest` JSON, picking the asset for platform `os`.
    pub fn parse(json: &str, os: &str) -> Result<Release> {
        let raw: RawRelease = serde_json::from_str(json)
            .map_err(|e| crate::Error::Other(format!("bad release payload: {e}")))?;
        let version = Self::parse_tag(&raw.tag_name)?;
        let asset = Self::select_asset(&raw.assets, os).ok_or_else(|| {
            crate::Error::UnsupportedPlatform {
                target: os.to_string(),
            }
        })?;
        Ok(Release { version, asset })
    }

    /// Whether this release is strictly newer than `current`.
    pub fn is_upgrade_over(&self, current: &semver::Version) -> bool {
        self.version > *current
    }

    /// Parse a release tag into a semantic version, tolerating a leading `v`.
    fn parse_tag(tag: &str) -> Result<semver::Version> {
        let stripped = tag.strip_prefix('v').unwrap_or(tag);
        semver::Version::parse(stripped)
            .map_err(|e| crate::Error::Other(format!("unparsable release tag '{tag}': {e}")))
    }

    /// Pick the asset for `os` from a release, skipping checksum companion files.
    fn select_asset(assets: &[RawAsset], os: &str) -> Option<Asset> {
        let marker = format!("-{os}-");
        assets
            .iter()
            .filter(|asset| !asset.name.ends_with(".sha256"))
            .find(|asset| asset.name.contains(&marker))
            .map(|asset| Asset {
                name: asset.name.clone(),
                url: asset.browser_download_url.clone(),
            })
    }
}

/// How an upgrade attempt ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpgradeOutcome {
    /// No version newer than the running one.
    UpToDate,
    /// The running binary was replaced with `version`.
    Upgraded(semver::Version),
}

/// Raw shape of the GitHub `releases/latest` response.
#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    assets: Vec<RawAsset>,
}

/// Raw shape of one release asset entry.
#[derive(Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
}

/// Drive a self-upgrade against injected backends.
///
/// The in-flight methods take `&self` (they read the seams); the pure helpers are
/// associated functions, because they need no receiver state.
pub struct Upgrader<'a> {
    fs: &'a dyn Fs,
    downloader: &'a dyn Downloader,
    reporter: &'a dyn Reporter,
}

impl<'a> Upgrader<'a> {
    /// An upgrader over the given backends.
    pub fn new(fs: &'a dyn Fs, downloader: &'a dyn Downloader, reporter: &'a dyn Reporter) -> Self {
        Upgrader {
            fs,
            downloader,
            reporter,
        }
    }

    /// Resolve, verify, and swap in the latest release if it is newer than `current`.
    pub fn run(self, exe: &Path, current: &semver::Version) -> Result<UpgradeOutcome> {
        self.reporter
            .report(Progress::Phase("Checking for updates"));
        let release = self.fetch_release()?;
        if !release.is_upgrade_over(current) {
            return Ok(UpgradeOutcome::UpToDate);
        }

        self.reporter.report(Progress::Phase("Downloading"));
        let bytes =
            self.downloader
                .fetch(&release.asset.url.clone().into(), &mut |done, total| {
                    self.reporter.report(Progress::Bytes { done, total });
                })?;

        let checksum = self.fetch_checksum(&release.asset)?;
        Self::verify_bytes(&checksum, &bytes)?;

        self.reporter.report(Progress::Phase("Replacing binary"));
        self.replace_binary(exe, &bytes)?;

        Ok(UpgradeOutcome::Upgraded(release.version))
    }

    /// Fetch and decode the JSON release payload for the running platform.
    fn fetch_release(&self) -> Result<Release> {
        let json = self
            .downloader
            .fetch(&RELEASES_URL.to_string().into(), &mut |_, _| {})?;
        let text = std::str::from_utf8(&json)
            .map_err(|e| crate::Error::Other(format!("release payload is not UTF-8: {e}")))?;
        Release::parse(text, std::env::consts::OS)
    }

    /// Fetch and parse the release's `.sha256` companion file.
    fn fetch_checksum(&self, asset: &Asset) -> Result<String> {
        let url: String = format!("{}.sha256", asset.url);
        let text = self.downloader.fetch(&url.into(), &mut |_, _| {})?;
        Self::parse_checksum(&String::from_utf8_lossy(&text))
    }

    /// Stage the new binary and rename it over the running exe, keeping a backup.
    fn replace_binary(&self, exe: &Path, bytes: &[u8]) -> Result<()> {
        let part = Self::sibling(exe, ".part");
        let old = Self::sibling(exe, ".old");
        self.fs.write(&part, bytes)?;
        if self.fs.exists(&old)? {
            self.fs.remove_file(&old)?;
        }
        self.fs.rename(exe, &old)?;
        if let Err(cause) = self.fs.rename(&part, exe) {
            self.restore_backup(&old, exe);
            return Err(cause);
        }
        Ok(())
    }

    /// Move the backup binary back onto the exe path after a failed swap.
    fn restore_backup(&self, old: &Path, exe: &Path) {
        let _ = self.fs.rename(old, exe);
    }

    /// Resolve the running exe and version, then run a full self-upgrade.
    pub fn upgrade(
        fs: &dyn Fs,
        downloader: &dyn Downloader,
        reporter: &dyn Reporter,
    ) -> Result<UpgradeOutcome> {
        let exe = std::env::current_exe().map_err(|e| io_err(PathBuf::new(), e))?;
        let current = Release::parse_tag(env!("CARGO_PKG_VERSION"))?;
        Upgrader::new(fs, downloader, reporter).run(&exe, &current)
    }

    /// Remove a stale `.old` backup beside the running exe (best effort on success).
    pub fn cleanup_stale(fs: &dyn Fs) -> Result<()> {
        let Ok(exe) = std::env::current_exe() else {
            return Ok(());
        };
        let old = Self::sibling(&exe, ".old");
        if fs.exists(&old)? {
            fs.remove_file(&old)?;
        }
        Ok(())
    }
}

impl Upgrader<'_> {
    /// The path `original` with `tag` appended to its file name.
    fn sibling(path: &Path, tag: &str) -> PathBuf {
        PathBuf::from(format!("{}{}", path.display(), tag))
    }

    /// Extract the lowercase hex digest from a `.sha256` file body.
    fn parse_checksum(text: &str) -> Result<String> {
        text.split_whitespace()
            .next()
            .map(str::to_ascii_lowercase)
            .filter(|token| Self::is_hex(token))
            .ok_or_else(|| crate::Error::Other("malformed .sha256 file".to_string()))
    }

    /// Whether every character of `token` is a hexadecimal digit.
    fn is_hex(token: &str) -> bool {
        !token.is_empty() && token.chars().all(|c| c.is_ascii_hexdigit())
    }

    /// Reject `data` unless its SHA-256 digest equals `expected_hex`.
    fn verify_bytes(expected_hex: &str, data: &[u8]) -> Result<()> {
        use sha2::{Digest, Sha256};
        let actual = format!("{:x}", Sha256::digest(data));
        if actual == expected_hex {
            Ok(())
        } else {
            Err(crate::Error::Other(format!(
                "checksum mismatch: expected {expected_hex}, got {actual}"
            )))
        }
    }
}

#[cfg(test)]
mod tests;
