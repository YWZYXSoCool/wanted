//! Installer-method tests: downloading an executable and running it silently into
//! `apps/<base_dir>`, with the method/args selected per-platform via `install.strategy`.

use std::path::Path;

use crate::Version;
use crate::engine::execute;
use crate::engine::fs::{Fs, MemFs};
use crate::engine::ops::Op;
use crate::engine::plan::Selection;
use crate::engine::{Ctx, Downloader, ProcessRunner, Url};
use crate::env::{EnvStore, EnvVar, MemEnvStore};
use crate::plugin::{Manifest, Target};
use crate::report::SilentReporter;

/// A manifest that mixes methods per platform: Windows installs a silent `.exe`,
/// Linux just unpacks a tarball, selected by `install.strategy`.
const MIXED_TOML: &str = r#"
[meta]
name = "gcc"
version = "1.0.0"

[install]
method = "download"
base_dir = "gcc"
asset = { "x86_64-pc-windows-msvc" = { default = "https://ex/gcc-{version}-win.exe" }, "x86_64-unknown-linux-gnu" = { default = "https://ex/gcc-{version}.tar.gz" } }

[install.strategy]
"x86_64-pc-windows-msvc" = { method = "installer", args = ["/VERYSILENT", "/DIR={base}"] }

[env]
PATH = "bin"
"#;

/// A stable Windows platform target (independent of the host) for reproducibility.
fn win_target() -> Target {
    Target::parts("x86_64", "windows", "msvc")
}

/// Plan the mixed manifest against Windows, where the installer strategy applies.
fn vsn() -> Version {
    Version::parse("1.23.0").unwrap()
}

fn plan_for(root: &Path) -> crate::engine::plan::Plan {
    let manifest = Manifest::parse(MIXED_TOML).unwrap();
    manifest
        .plan(root, &win_target(), &vsn(), &Selection::default())
        .unwrap()
}

/// A downloader that always returns the given bytes back.
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

/// A process runner that accepts every invocation (unsilenced behavior).
struct NoopProcess;

impl ProcessRunner for NoopProcess {
    fn run(&self, _program: &Path, _args: &[String]) -> crate::Result<()> {
        Ok(())
    }
}

/// An env backend that deliberately fails to write PATH, to verify that a commit
/// failure rolls back the freshly installed directory.
struct FailingEnv {
    inner: MemEnvStore,
}

impl EnvStore for FailingEnv {
    fn read(&self, name: &EnvVar) -> crate::Result<Option<String>> {
        self.inner.read(name)
    }
    fn write(&self, name: &EnvVar, value: &str) -> crate::Result<()> {
        if name.is_path() {
            Err(crate::Error::Other("simulated env write failure".into()))
        } else {
            self.inner.write(name, value)
        }
    }
    fn remove(&self, name: &EnvVar) -> crate::Result<()> {
        self.inner.remove(name)
    }
}

#[test]
fn plan_selects_installer_strategy_on_windows() {
    let plan = plan_for(Path::new("/root"));
    assert_eq!(plan.staged_ops.len(), 2);
    assert!(matches!(plan.staged_ops[0], Op::Download { .. }));
    let Op::RunInstaller { exe, args, base } = &plan.staged_ops[1] else {
        panic!("windows plan should run the installer");
    };
    let expected_base = Path::new("/root").join(".wanted").join("apps").join("gcc");
    assert_eq!(base, &expected_base);
    assert_eq!(plan.dest_dir, expected_base);
    assert_eq!(plan.app_dir, expected_base);
    let base_str = expected_base.to_string_lossy();
    assert_eq!(
        args,
        &["/VERYSILENT".to_string(), format!("/DIR={base_str}")]
    );
    assert_eq!(exe, Path::new(&plan.download_to));
}

#[test]
fn plan_falls_back_to_download_off_strategy_platform() {
    let manifest = Manifest::parse(MIXED_TOML).unwrap();
    let linux = Target::parts("x86_64", "linux", "gnu");
    let plan = manifest
        .plan(Path::new("/root"), &linux, &vsn(), &Selection::default())
        .unwrap();
    assert_eq!(plan.staged_ops.len(), 2);
    assert!(matches!(plan.staged_ops[0], Op::Download { .. }));
    assert!(matches!(plan.staged_ops[1], Op::Unpack { .. }));
    assert_ne!(plan.app_dir, plan.dest_dir);
}

#[test]
fn execute_installer_keeps_dest_and_persists_env() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let root = Path::new("/root");
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(vec![0x4d, 0x5a]),
        runner: &NoopProcess,
        env: &env,
        reporter: &SilentReporter,
    };
    let plan = plan_for(root);
    let staging_dir = plan.staging_dir.clone();

    execute::execute(&plan, &ctx).unwrap();

    assert!(fs.exists(&plan.dest_dir).unwrap());
    assert!(!fs.exists(&staging_dir).unwrap());
    let expected_path = root.join(".wanted").join("apps").join("gcc").join("bin");
    assert_eq!(
        env.read(&EnvVar::from("PATH")).unwrap().unwrap(),
        expected_path.to_string_lossy()
    );
}

#[test]
fn execute_installer_rolls_back_dest_on_env_failure() {
    let fs = MemFs::new();
    let env = FailingEnv {
        inner: MemEnvStore::new(),
    };
    let root = Path::new("/root");
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(vec![0x4d, 0x5a]),
        runner: &NoopProcess,
        env: &env,
        reporter: &SilentReporter,
    };
    let plan = plan_for(root);

    let err = execute::execute(&plan, &ctx).unwrap_err();

    assert!(err.to_string().contains("simulated"));
    assert!(!fs.exists(&plan.dest_dir).unwrap());
    assert!(!fs.exists(&plan.app_dir).unwrap());
}
