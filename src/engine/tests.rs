//! Execution engine tests: planning is pure, and execution rolls back by
//! replaying compensations against the in-memory filesystem.

use std::cell::RefCell;
use std::io::Cursor;
use std::io::Write;
use std::path::Path;

use crate::Error;
use crate::Version;
use crate::engine::execute;
use crate::engine::fs::{Fs, MemFs};
use crate::engine::ops::Op;
use crate::engine::plan::Selection;
use crate::engine::{Ctx, Downloader, Url};
use crate::env::{EnvStore, EnvVar, MemEnvStore};
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

/// A manifest declaring per-platform env overrides: Windows flattens the install
/// to the base (`.`) while other platforms keep the binaries under `bin/`.
/// A manifest whose asset URL carries both `{version}` and `{date}` (the
/// python-build-standalone shape: the date is the version's build metadata).
const PBS_TOML: &str = r#"
[meta]
name = "python3"
version = "1.1.0"

[install]
method = "download"
asset = { "x86_64-pc-windows-msvc" = { default = "https://ex/python-build-standalone/releases/download/{date}/cpython-{version}-x86_64-pc-windows-msvc-install_only.tar.gz" } }
base_dir = "python3"
"#;

const ENV_PLATFORM_TOML: &str = r#"
[meta]
name = "node"
version = "1.0.0"

[install]
method = "download"
base_dir = "node"

[install.asset]
"x86_64-pc-windows-msvc" = { default = "https://n/{version}-win.zip" }
"x86_64-unknown-linux-gnu" = { default = "https://n/{version}-linux.tar.gz" }

[env]
PATH = "bin"

[env_by_platform."x86_64-pc-windows-msvc"]
PATH = "."
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
        _url: &Url,
        _on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> crate::Result<Vec<u8>> {
        Ok(self.0.clone())
    }
}

/// A process runner that accepts every invocation (used where installing is not
/// the behavior under test).
struct NoopProcess;

impl crate::engine::ProcessRunner for NoopProcess {
    fn run(&self, _program: &Path, _args: &[String]) -> crate::Result<()> {
        Ok(())
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
        _url: &Url,
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

fn vsn() -> Version {
    Version::parse("1.23.0").unwrap()
}

fn plan_for(manifest: &Manifest, root: &Path) -> crate::engine::plan::Plan {
    manifest
        .plan(root, &win_target(), &vsn(), &Selection::default())
        .unwrap()
}

/// A `Selection` naming exactly the given components against the `default` source.
fn selection(components: &[String]) -> Selection<'_> {
    Selection {
        source: None,
        components,
    }
}

#[test]
fn plan_is_pure_and_well_structured() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = plan_for(&manifest, Path::new("/root"));

    assert_eq!(plan.name.as_str(), "golang");
    assert_eq!(plan.version.to_string(), "1.23.0");
    assert_eq!(plan.staged_ops.len(), 2);
    assert!(matches!(plan.staged_ops[0], Op::Download { .. }));
    assert!(matches!(plan.staged_ops[1], Op::Unpack { .. }));
    assert_eq!(plan.commit_ops.len(), 1);
    assert!(matches!(plan.commit_ops[0], Op::WriteEnv { .. }));
    assert_eq!(plan.dest_dir, Path::new("/root/golang"));
}

#[test]
fn env_is_overridden_per_platform() {
    let manifest = Manifest::parse(ENV_PLATFORM_TOML).unwrap();

    let root = Path::new("/root");
    let node_dir = root.join("node");

    let windows = manifest
        .plan(root, &win_target(), &vsn(), &Selection::default())
        .unwrap()
        .env_deltas()
        .into_iter()
        .find(|d| d.name.as_str() == "PATH")
        .unwrap();
    assert_eq!(windows.value, node_dir.to_string_lossy());

    let linux = manifest
        .plan(
            root,
            &Target::parts("x86_64", "linux", "gnu"),
            &vsn(),
            &Selection::default(),
        )
        .unwrap()
        .env_deltas()
        .into_iter()
        .find(|d| d.name.as_str() == "PATH")
        .unwrap();
    assert_eq!(linux.value, node_dir.join("bin").to_string_lossy());
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
        runner: &NoopProcess,
        env: &env,
        reporter: &SilentReporter,
    };
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = plan_for(&manifest, root);
    let staging_dir = plan.staging_dir.clone();

    execute::execute(&plan, &ctx).unwrap();

    let app_file = root.join("golang/bin/go.exe");
    assert_eq!(fs.read(&app_file).unwrap(), b"binary");
    assert!(!fs.exists(&staging_dir).unwrap());

    let expected_path = root.join("golang").join("bin");
    let actual = env.read(&EnvVar::from("PATH")).unwrap().unwrap();
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
        runner: &NoopProcess,
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
            _ => None,
        })
        .collect();
    assert_eq!(phases, ["Extracting", "Configuring env"]);
}

#[test]
fn execute_reports_download_source_url() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let reporter = RecordingReporter(RefCell::new(Vec::new()));
    let root = Path::new("/root");
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(tool_zip()),
        runner: &NoopProcess,
        env: &env,
        reporter: &reporter,
    };
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = plan_for(&manifest, root);

    execute::execute(&plan, &ctx).unwrap();

    let sources: Vec<_> = reporter
        .0
        .borrow()
        .iter()
        .filter_map(|event| match event {
            Progress::DownloadSource { url } => Some(url.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(sources, ["https://go.dev/d/1.23.0.zip"]);
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
        runner: &NoopProcess,
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
            _ => None,
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
    fn read(&self, name: &EnvVar) -> crate::Result<Option<String>> {
        self.inner.read(name)
    }
    fn write(&self, name: &EnvVar, value: &str) -> crate::Result<()> {
        if name.is_path() {
            Err(Error::Other("simulated env write failure".into()))
        } else {
            self.inner.write(name, value)
        }
    }
    fn remove(&self, name: &EnvVar) -> crate::Result<()> {
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
        runner: &NoopProcess,
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
            .plan(Path::new("/root"), &sparc, &vsn(), &Selection::default())
            .is_err()
    );
}

/// Extract the URL of the plan's Download op, for asserting source selection.
fn download_url(plan: &crate::engine::plan::Plan) -> &str {
    match &plan.staged_ops[0] {
        Op::Download { url, .. } => url.as_str(),
        _ => unreachable!("first staged op is always Download"),
    }
}

#[test]
fn plan_defaults_to_the_default_asset_source() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let plan = plan_for(&manifest, Path::new("/root"));
    assert_eq!(download_url(&plan), "https://go.dev/d/1.23.0.zip");
}

#[test]
fn plan_url_expands_version_build_metadata_into_date() {
    let manifest = Manifest::parse(PBS_TOML).unwrap();
    let version = Version::parse("3.13.0+20260825").unwrap();
    let plan = manifest
        .plan(
            Path::new("/root"),
            &win_target(),
            &version,
            &Selection::default(),
        )
        .unwrap();
    assert_eq!(
        download_url(&plan),
        "https://ex/python-build-standalone/releases/download/20260825/cpython-3.13.0+20260825-x86_64-pc-windows-msvc-install_only.tar.gz"
    );
}

#[test]
fn plan_uses_requested_asset_source() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let selection = Selection {
        source: Some("mirror"),
        components: &[],
    };
    let plan = manifest
        .plan(Path::new("/root"), &win_target(), &vsn(), &selection)
        .unwrap();
    assert_eq!(download_url(&plan), "https://mirror/h/1.23.0.zip");
}

#[test]
fn plan_rejects_unknown_asset_source() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    let selection = Selection {
        source: Some("nope"),
        components: &[],
    };
    let err = manifest
        .plan(Path::new("/root"), &win_target(), &vsn(), &selection)
        .unwrap_err();
    assert!(err.to_string().contains("no asset source nope"));
}

const COMPONENT_TOML: &str = r#"
[meta]
name = "llvm"
version = "1.0.0"

[install]
method = "download"
base_dir = "llvm"
asset = { "x86_64-pc-windows-msvc" = { default = "https://ex/llvm-{version}.zip", mirror = "https://mirror/l-{version}.zip" } }

[install.component]
"clang" = { "x86_64-pc-windows-msvc" = { default = "https://ex/clang-{version}.zip", mirror = "https://mirror/c-{version}.zip" } }

[env]
PATH = "bin"
CLANG = "clang/bin"
"#;

/// Extract the Unpack target path of the i-th staged op.
fn unpack_target(plan: &crate::engine::plan::Plan, index: usize) -> &std::path::Path {
    match &plan.staged_ops[index] {
        Op::Unpack { to, .. } => to,
        _ => unreachable!("staged op {index} is not Unpack"),
    }
}

#[test]
fn plan_downloads_no_components_by_default() {
    let manifest = Manifest::parse(COMPONENT_TOML).unwrap();
    let plan = manifest
        .plan(
            Path::new("/root"),
            &win_target(),
            &vsn(),
            &Selection::default(),
        )
        .unwrap();
    assert_eq!(plan.staged_ops.len(), 2);
}

#[test]
fn plan_appends_download_and_unpack_for_requested_component() {
    let manifest = Manifest::parse(COMPONENT_TOML).unwrap();
    let plan = manifest
        .plan(
            Path::new("/root"),
            &win_target(),
            &vsn(),
            &selection(&[String::from("clang")]),
        )
        .unwrap();
    assert_eq!(plan.staged_ops.len(), 4);
    assert!(
        matches!(&plan.staged_ops[2], Op::Download { url, .. } if url.as_str() == "https://ex/clang-1.23.0.zip")
    );
    let unpack_to = unpack_target(&plan, 3);
    assert_eq!(
        unpack_to.file_name().and_then(|s| s.to_str()),
        Some("clang")
    );
    assert_eq!(
        unpack_to
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str()),
        Some("app")
    );
}

#[test]
fn plan_dedupes_repeated_component_requests() {
    let manifest = Manifest::parse(COMPONENT_TOML).unwrap();
    let plan = manifest
        .plan(
            Path::new("/root"),
            &win_target(),
            &vsn(),
            &selection(&[String::from("clang"), String::from("clang")]),
        )
        .unwrap();
    assert_eq!(plan.staged_ops.len(), 4);
}

#[test]
fn plan_rejects_unknown_component() {
    let manifest = Manifest::parse(COMPONENT_TOML).unwrap();
    let err = manifest
        .plan(
            Path::new("/root"),
            &win_target(),
            &vsn(),
            &selection(&[String::from("bogus")]),
        )
        .unwrap_err();
    assert!(err.to_string().contains("no component 'bogus'"));
}

#[test]
fn plan_component_uses_requested_asset_source() {
    let manifest = Manifest::parse(COMPONENT_TOML).unwrap();
    let selection = Selection {
        source: Some("mirror"),
        components: &["clang".to_string()],
    };
    let plan = manifest
        .plan(Path::new("/root"), &win_target(), &vsn(), &selection)
        .unwrap();
    assert!(
        matches!(&plan.staged_ops[2], Op::Download { url, .. } if url.as_str() == "https://mirror/c-1.23.0.zip")
    );
}
