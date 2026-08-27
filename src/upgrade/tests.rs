//! Unit tests for `wanted::upgrade`: release discovery, strict checksum
//! verification, and the rename-dance swap with its rollback path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::engine::{Downloader, Fs, MemFs, Url};
use crate::report::SilentReporter;
use crate::upgrade::{RELEASES_URL, Release, UpgradeOutcome, Upgrader};

const OLD: &[u8] = b"old binary";
const NEW: &[u8] = b"new binary";

/// SHA-256 hex digest of `bytes`.
fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// A release payload with one asset per platform, plus a windows `.sha256` sibling.
fn release_json(tag: &str) -> String {
    format!(
        r#"{{
          "tag_name": "{tag}",
          "assets": [
            {{"name": "wanted-{tag}-linux-x86_64", "browser_download_url": "https://ex/l-{tag}"}},
            {{"name": "wanted-{tag}-windows-x86_64.exe", "browser_download_url": "https://ex/w-{tag}.exe"}},
            {{"name": "wanted-{tag}-macos-x86_64", "browser_download_url": "https://ex/m-{tag}"}}
          ]
        }}"#
    )
}

/// Downloader stub serving canned bytes per URL.
struct StubDownloader {
    files: HashMap<String, Vec<u8>>,
}

impl StubDownloader {
    fn new(files: HashMap<String, Vec<u8>>) -> Self {
        StubDownloader { files }
    }
}

impl Downloader for StubDownloader {
    fn fetch(
        &self,
        url: &Url,
        _progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> crate::Result<Vec<u8>> {
        self.files
            .get(url.as_str())
            .cloned()
            .ok_or_else(|| crate::Error::Network(format!("no stub for {}", url.as_str())))
    }
}

/// A release that downloads `NEW` and serves its real checksum for `os`.
fn stub_for(tag: &str, os: &str) -> StubDownloader {
    let mut files = HashMap::new();
    files.insert(RELEASES_URL.to_string(), release_json(tag).into_bytes());
    let exe = format!("https://ex/w-{tag}.exe");
    files.insert(exe.clone(), NEW.to_vec());
    files.insert(
        format!("{exe}.sha256"),
        format!("{}  wanted.exe", digest(NEW)).into_bytes(),
    );
    let _ = os;
    StubDownloader::new(files)
}

const VTAG: &str = "wanted-v0.2.0-windows-x86_64.exe";

#[test]
fn parse_release_picks_platform_asset_and_strips_v() {
    let release = Release::parse(&release_json("v0.2.0"), "windows").expect("parses");
    assert_eq!(release.version.to_string(), "0.2.0");
    assert_eq!(release.asset.name, VTAG);
    assert_eq!(release.asset.url, "https://ex/w-v0.2.0.exe");
}

#[test]
fn parse_release_errors_on_missing_platform_asset() {
    let err = Release::parse(&release_json("v0.2.0"), "freebsd").expect_err("no freebsd asset");
    assert!(err.to_string().contains("unsupported platform"));
}

#[test]
fn parse_checksum_extracts_lowercase_hex() {
    assert_eq!(
        Upgrader::parse_checksum("AABB CC  wanted.exe").expect("parses"),
        "aabb"
    );
    assert_eq!(Upgrader::parse_checksum("deadbeef").unwrap(), "deadbeef");
}

#[test]
fn parse_checksum_rejects_non_hex() {
    assert!(Upgrader::parse_checksum("not-a-hex").is_err());
    assert!(Upgrader::parse_checksum("").is_err());
}

#[test]
fn verify_bytes_accepts_matching_digest() {
    assert!(Upgrader::verify_bytes(&digest(NEW), NEW).is_ok());
}

#[test]
fn verify_bytes_rejects_mismatched_digest() {
    assert!(Upgrader::verify_bytes(&digest(OLD), NEW).is_err());
}

#[test]
fn upgrade_is_up_to_date_without_changes() {
    let fs = MemFs::new();
    fs.write(Path::new("bin/wanted.exe"), OLD).unwrap();
    let current = semver::Version::new(0, 9, 0);

    let outcome = Upgrader::new(&fs, &stub_for("v0.2.0", "windows"), &SilentReporter)
        .run(Path::new("bin/wanted.exe"), &current)
        .expect("runs");

    assert_eq!(outcome, UpgradeOutcome::UpToDate);
    assert_eq!(fs.read(Path::new("bin/wanted.exe")).unwrap(), OLD);
    assert!(!fs.exists(Path::new("bin/wanted.exe.part")).unwrap());
    assert!(!fs.exists(Path::new("bin/wanted.exe.old")).unwrap());
}

#[test]
fn upgrade_swaps_binary_and_keeps_backup() {
    let fs = MemFs::new();
    fs.write(Path::new("bin/wanted.exe"), OLD).unwrap();
    let current = semver::Version::new(0, 1, 0);

    let outcome = Upgrader::new(&fs, &stub_for("v0.2.0", "windows"), &SilentReporter)
        .run(Path::new("bin/wanted.exe"), &current)
        .expect("runs");

    assert_eq!(
        outcome,
        UpgradeOutcome::Upgraded(semver::Version::new(0, 2, 0))
    );
    assert_eq!(fs.read(Path::new("bin/wanted.exe")).unwrap(), NEW);
    assert_eq!(fs.read(Path::new("bin/wanted.exe.old")).unwrap(), OLD);
    assert!(!fs.exists(Path::new("bin/wanted.exe.part")).unwrap());
}

#[test]
fn upgrade_rejects_checksum_mismatch_and_leaves_binary_untouched() {
    let mut files = HashMap::new();
    files.insert(
        RELEASES_URL.to_string(),
        release_json("v0.2.0").into_bytes(),
    );
    let exe = "https://ex/w-v0.2.0.exe";
    files.insert(exe.to_string(), NEW.to_vec());
    files.insert(
        format!("{exe}.sha256"),
        format!("{}  wanted.exe", digest(OLD)).into_bytes(),
    );
    let downloader = StubDownloader::new(files);

    let fs = MemFs::new();
    fs.write(Path::new("bin/wanted.exe"), OLD).unwrap();

    let err = Upgrader::new(&fs, &downloader, &SilentReporter)
        .run(Path::new("bin/wanted.exe"), &semver::Version::new(0, 1, 0))
        .expect_err("checksum mismatch");

    assert!(err.to_string().contains("checksum mismatch"));
    assert_eq!(fs.read(Path::new("bin/wanted.exe")).unwrap(), OLD);
    assert!(!fs.exists(Path::new("bin/wanted.exe.part")).unwrap());
}

/// A filesystem that fails to rename a specific source path, for rollback tests.
struct FailingFs {
    inner: MemFs,
    deny_from: PathBuf,
}

impl Fs for FailingFs {
    fn read(&self, path: &Path) -> crate::Result<Vec<u8>> {
        self.inner.read(path)
    }
    fn write(&self, path: &Path, data: &[u8]) -> crate::Result<()> {
        self.inner.write(path, data)
    }
    fn exists(&self, path: &Path) -> crate::Result<bool> {
        self.inner.exists(path)
    }
    fn create_dir_all(&self, path: &Path) -> crate::Result<()> {
        self.inner.create_dir_all(path)
    }
    fn rename(&self, from: &Path, _to: &Path) -> crate::Result<()> {
        if from == self.deny_from {
            return Err(crate::Error::Other("denied rename".to_string()));
        }
        self.inner.rename(from, _to)
    }
    fn remove_dir_all(&self, path: &Path) -> crate::Result<()> {
        self.inner.remove_dir_all(path)
    }
    fn remove_file(&self, path: &Path) -> crate::Result<()> {
        self.inner.remove_file(path)
    }
    fn read_dir(&self, path: &Path) -> crate::Result<Vec<(String, bool)>> {
        self.inner.read_dir(path)
    }
}

#[test]
fn failed_swap_restores_previous_binary() {
    let exe = PathBuf::from("bin/wanted.exe");
    let fs = FailingFs {
        inner: MemFs::new(),
        deny_from: PathBuf::from("bin/wanted.exe.part"),
    };
    fs.inner.write(&exe, OLD).unwrap();

    let err = Upgrader::new(&fs, &stub_for("v0.2.0", "windows"), &SilentReporter)
        .run(&exe, &semver::Version::new(0, 1, 0))
        .expect_err("rename of .part denied");

    assert!(err.to_string().contains("denied rename"));
    assert_eq!(fs.read(&exe).unwrap(), OLD);
    assert!(!fs.exists(Path::new("bin/wanted.exe.old")).unwrap());
}
