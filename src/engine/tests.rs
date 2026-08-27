//! Execution engine tests: planning is pure, and execution rolls back by
//! replaying compensations against the in-memory filesystem.

use std::cell::RefCell;
use std::io::Cursor;
use std::io::Write;
use std::path::Path;

use crate::Error;
use crate::engine::execute;
use crate::engine::fs::{Fs, MemFs};
use crate::engine::ops::Op;
use crate::engine::{Ctx, Downloader};
use crate::env::{EnvStore, MemEnvStore};
use crate::plugin::{Manifest, Target};
use crate::report::{Progress, Reporter, SilentReporter};

const GOLANG_TOML: &str = r#"
[meta]
name = "golang"
version = "1.0.0"

[install]
method = "download"
asset = { "x86_64-pc-windows-msvc" = { default = "https://go.dev/d/{version}.zip", mirror = "https://mirror/h/{version}.zip" } }
base_dir = "golang"

[env]
PATH = "bin"
GOROOT = "."
"#;

/// A stable Windows platform target (independent of the host) for reproducibility.
fn win_target() -> Target {
    Target::parts("x86_64", "windows", "msvc")
}

/// Build a zip containing `go/bin/go.exe` and `go/LICENSE`.
fn tool_zip() -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let cursor = Cursor::new(Vec::new());
    let mut zip_writer = zip::ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip_writer.start_file("go/bin/go.exe", options).unwrap();
    zip_writer.write_all(b"binary").unwrap();
    zip_writer.start_file("go/LICENSE", options).unwrap();
    zip_writer.write_all(b"mit").unwrap();

    zip_writer.finish().unwrap().into_inner()
}

struct StubDownloader(Vec<u8>);

impl Downloader for StubDownloader {
    fn fetch(
        &self,
        _url: &str,
        _on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> crate::Result<Vec<u8>> {
        Ok(self.0.clone())
    }
}

/// Records received progress events to verify the reporting wiring fires.
struct RecordingReporter(RefCell<Vec<Progress>>);

impl Reporter for RecordingReporter {
    fn report(&self, event: Progress) {
        self.0.borrow_mut().push(event);
    }
}

/// Reports progress in quarter chunks then returns the full archive, to verify
/// the byte stream really reaches the reporter from the downloader.
struct ChunkingDownloader;

impl Downloader for ChunkingDownloader {
    fn fetch(
        &self,
        _url: &str,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> crate::Result<Vec<u8>> {
        let body = tool_zip();
        let total = body.len() as u64;
        for done in [total / 4, total / 2, 3 * total / 4, total] {
            on_progress(done, Some(total));
        }
        Ok(body)
    }
}

fn plan_for(manifest: &Manifest, root: &Path) -> crate::engine::plan::Plan {
    manifest.plan(root, &win_target(), "1.23", None).unwrap()
}

#[test]
fn plan_is_pure_and_well_structured() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = plan_for(&manifest, Path::new("/root"));

    assert_eq!(plan.name, "golang");
    assert_eq!(plan.version, "1.23");
    assert_eq!(plan.staged_ops.len(), 2);
    assert!(matches!(plan.staged_ops[0], Op::Download { .. }));
    assert!(matches!(plan.staged_ops[1], Op::Unpack { .. }));
    assert_eq!(plan.commit_ops.len(), 1);
    assert!(matches!(plan.commit_ops[0], Op::WriteEnv { .. }));
    assert_eq!(plan.dest_dir, Path::new("/root/.wanted/apps/golang"));
}

#[test]
fn execute_commits_and_persists_env() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let root = Path::new("/root");
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(tool_zip()),
        env: &env,
        reporter: &SilentReporter,
    };
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = plan_for(&manifest, root);
    let staging_dir = plan.staging_dir.clone();

    execute::execute(&plan, &ctx).unwrap();

    let app_file = root.join(".wanted/apps/golang/bin/go.exe");
    assert_eq!(fs.read(&app_file).unwrap(), b"binary");
    assert!(!fs.exists(&staging_dir).unwrap());

    let expected_path = root.join(".wanted").join("apps").join("golang").join("bin");
    let actual = env.read("PATH").unwrap().unwrap();
    assert_eq!(actual, expected_path.to_string_lossy());
}

#[test]
fn execute_reports_phase_progress() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let reporter = RecordingReporter(RefCell::new(Vec::new()));
    let root = Path::new("/root");
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(tool_zip()),
        env: &env,
        reporter: &reporter,
    };
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = plan_for(&manifest, root);

    execute::execute(&plan, &ctx).unwrap();

    let phases: Vec<_> = reporter
        .0
        .borrow()
        .iter()
        .filter_map(|event| match event {
            Progress::Phase(label) => Some(*label),
            Progress::Bytes { .. } => None,
        })
        .collect();
    assert_eq!(phases, ["Downloading", "Extracting", "Configuring env"]);
}

#[test]
fn execute_reports_byte_progress_monotonically() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let reporter = RecordingReporter(RefCell::new(Vec::new()));
    let root = Path::new("/root");
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &ChunkingDownloader,
        env: &env,
        reporter: &reporter,
    };
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = plan_for(&manifest, root);

    execute::execute(&plan, &ctx).unwrap();

    let total = tool_zip().len() as u64;
    let expected = [
        (total / 4, Some(total)),
        (total / 2, Some(total)),
        (3 * total / 4, Some(total)),
        (total, Some(total)),
    ];
    let bytes: Vec<_> = reporter
        .0
        .borrow()
        .iter()
        .filter_map(|event| match event {
            Progress::Bytes { done, total } => Some((*done, *total)),
            Progress::Phase(_) => None,
        })
        .collect();
    assert_eq!(bytes, expected);
}

/// An env backend that deliberately fails to write PATH, to verify that a commit
/// failure rolls back the whole apps directory.
struct FailingEnv {
    inner: MemEnvStore,
}

impl EnvStore for FailingEnv {
    fn read(&self, name: &str) -> crate::Result<Option<String>> {
        self.inner.read(name)
    }
    fn write(&self, name: &str, value: &str) -> crate::Result<()> {
        if name == "PATH" {
            Err(Error::Other("simulated env write failure".into()))
        } else {
            self.inner.write(name, value)
        }
    }
    fn remove(&self, name: &str) -> crate::Result<()> {
        self.inner.remove(name)
    }
}

#[test]
fn execute_rolls_back_when_commit_fails() {
    let fs = MemFs::new();
    let env = FailingEnv {
        inner: MemEnvStore::new(),
    };
    let root = Path::new("/root");
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(tool_zip()),
        env: &env,
        reporter: &SilentReporter,
    };
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = plan_for(&manifest, root);

    let err = execute::execute(&plan, &ctx).unwrap_err();

    assert!(err.to_string().contains("simulated"));
    assert!(!fs.exists(&plan.dest_dir).unwrap());
    assert!(!fs.exists(&plan.app_dir).unwrap());
}

#[test]
fn plan_rejects_unsupported_platform() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let sparc = Target::parts("sparc64", "sparc", "");
    assert!(
        manifest
            .plan(Path::new("/root"), &sparc, "1.23", None)
            .is_err()
    );
}

/// Extract the URL of the plan's Download op, for asserting source selection.
fn download_url(plan: &crate::engine::plan::Plan) -> &str {
    match &plan.staged_ops[0] {
        Op::Download { url, .. } => url,
        _ => unreachable!("first staged op is always Download"),
    }
}

#[test]
fn plan_defaults_to_the_default_asset_source() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = plan_for(&manifest, Path::new("/root"));
    assert_eq!(download_url(&plan), "https://go.dev/d/1.23.zip");
}

#[test]
fn plan_uses_requested_asset_source() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = manifest
        .plan(Path::new("/root"), &win_target(), "1.23", Some("mirror"))
        .unwrap();
    assert_eq!(download_url(&plan), "https://mirror/h/1.23.zip");
}

#[test]
fn plan_rejects_unknown_asset_source() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let err = manifest
        .plan(Path::new("/root"), &win_target(), "1.23", Some("nope"))
        .unwrap_err();
    assert!(err.to_string().contains("no asset source nope"));
}
