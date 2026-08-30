//! Command-method tests: running external package-manager commands in fallback
//! order into `apps/<base_dir>`, treating a missing tool or a failing command
//! alike (both fall through to the next candidate).

use std::cell::RefCell;
use std::path::Path;

use crate::Version;
use crate::engine::execute;
use crate::engine::fs::{Fs, MemFs};
use crate::engine::ops::Op;
use crate::engine::plan::Selection;
use crate::engine::{Ctx, Downloader, ProcessRunner, Url};
use crate::env::{EnvStore, EnvVar, MemEnvStore};
use crate::plugin::{Manifest, Target};
use crate::report::{Progress, Reporter, SilentReporter};

/// A manifest installing `bat` via cargo first, npm second (fallback order).
const COMMAND_TOML: &str = r#"
[meta]
name = "bat"
version = "1.0.0"

[install]
method = "command"
base_dir = "bat"

[install.command]
"x86_64-pc-windows-msvc" = [
  { tool = "cargo", args = ["install", "--root", "{base}", "bat"], env = { CARGO_INSTALL_ROOT = "{base}" } },
  { tool = "npm", args = ["install", "--prefix", "{base}", "bat"] },
]

[env]
PATH = "bin"
"#;

/// A stable Windows platform target (independent of the host) for reproducibility.
fn target() -> Target {
    Target::parts("x86_64", "windows", "msvc")
}

/// The app's PATH segment declared by `[env]`.
fn expected_path(root: &Path) -> String {
    root.join(".wanted")
        .join("apps")
        .join("bat")
        .join("bin")
        .to_string_lossy()
        .into_owned()
}

fn vsn() -> Version {
    Version::parse("1.23.0").unwrap()
}

fn plan_for(root: &Path) -> crate::engine::plan::Plan {
    let manifest = Manifest::parse(COMMAND_TOML).unwrap();
    manifest
        .plan(root, &target(), &vsn(), &Selection::default())
        .unwrap()
}

/// A downloader that always returns the given bytes (unused by the command method).
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

/// A recorded child-process invocation: program, args, and extra environment.
type ProcessCall = (String, Vec<String>, Vec<(String, String)>);

/// A process runner that fails the first `remaining` invocations (driving the
/// fallback for a missing tool or a non-zero exit alike), then succeeds, while
/// recording every call.
struct FlakyProcess {
    remaining: RefCell<usize>,
    calls: RefCell<Vec<ProcessCall>>,
}

impl FlakyProcess {
    fn new(remaining: usize) -> Self {
        FlakyProcess {
            remaining: RefCell::new(remaining),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl ProcessRunner for FlakyProcess {
    fn run(&self, program: &Path, args: &[String]) -> crate::Result<()> {
        self.run_with_env(program, args, &[])
    }
    fn run_with_env(
        &self,
        program: &Path,
        args: &[String],
        env: &[(String, String)],
    ) -> crate::Result<()> {
        self.calls.borrow_mut().push((
            program.to_string_lossy().into_owned(),
            args.to_vec(),
            env.to_vec(),
        ));
        if *self.remaining.borrow() > 0 {
            *self.remaining.borrow_mut() -= 1;
            Err(crate::Error::Process("simulated failure".into()))
        } else {
            Ok(())
        }
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

/// A reporter that records the sequence of phase labels.
#[derive(Default)]
struct RecordingReporter {
    phases: RefCell<Vec<&'static str>>,
}

impl Reporter for RecordingReporter {
    fn report(&self, event: Progress) {
        if let Progress::Phase(label) = event {
            self.phases.borrow_mut().push(label);
        }
    }
}

#[test]
fn plan_builds_run_command_op() {
    let plan = plan_for(Path::new("/root"));
    assert_eq!(plan.staged_ops.len(), 1);
    assert_eq!(plan.app_dir, plan.dest_dir);
    assert!(matches!(plan.commit_ops[0], Op::WriteEnv { .. }));

    let Op::RunCommand { commands, base } = &plan.staged_ops[0] else {
        panic!("command plan should run a RunCommand op");
    };
    assert_eq!(commands.len(), 2);
    assert_eq!(base, &plan.dest_dir);
    let base_str = base.to_string_lossy();

    assert_eq!(commands[0].tool.as_str(), "cargo");
    let expected_args = vec![
        "install".to_string(),
        "--root".to_string(),
        base_str.to_string(),
        "bat".to_string(),
    ];
    assert_eq!(commands[0].args, expected_args);
    assert_eq!(
        commands[0].env,
        vec![("CARGO_INSTALL_ROOT".to_string(), base_str.to_string())]
    );

    assert_eq!(commands[1].tool.as_str(), "npm");
    assert!(commands[1].env.is_empty());
}

#[test]
fn plan_rejects_command_without_platform_entry() {
    let manifest = Manifest::parse(COMMAND_TOML).unwrap();
    let linux = Target::parts("x86_64", "linux", "gnu");
    let err = manifest
        .plan(Path::new("/root"), &linux, &vsn(), &Selection::default())
        .unwrap_err();
    assert!(err.to_string().contains("unsupported platform"));
}

#[test]
fn plan_rejects_components_for_command_method() {
    let manifest = Manifest::parse(COMMAND_TOML).unwrap();
    let selection = Selection {
        source: None,
        components: &["clang".to_string()],
    };
    let err = manifest
        .plan(Path::new("/root"), &target(), &vsn(), &selection)
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("only supported for the 'download' method")
    );
}

#[test]
fn execute_falls_back_to_second_command_on_first_failure() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let root = Path::new("/root");
    let runner = FlakyProcess::new(1);
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(Vec::new()),
        runner: &runner,
        env: &env,
        reporter: &SilentReporter,
    };
    let plan = plan_for(root);

    execute::execute(&plan, &ctx).unwrap();

    let calls = runner.calls.borrow();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "cargo");
    assert_eq!(calls[1].0, "npm");
    assert!(fs.exists(&plan.dest_dir).unwrap());
    assert_eq!(
        env.read(&EnvVar::from("PATH")).unwrap().unwrap(),
        expected_path(root)
    );
}

#[test]
fn execute_aggregates_errors_when_all_commands_fail() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let root = Path::new("/root");
    let runner = FlakyProcess::new(2);
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(Vec::new()),
        runner: &runner,
        env: &env,
        reporter: &SilentReporter,
    };
    let plan = plan_for(root);
    let keep = plan.dest_dir.join("keep.txt");
    fs.write(&keep, b"existing").unwrap();

    let err = execute::execute(&plan, &ctx).unwrap_err();

    assert!(
        err.to_string().contains("all install commands failed"),
        "{err}"
    );
    assert!(err.to_string().contains("cargo"), "{err}");
    assert!(err.to_string().contains("npm"), "{err}");
    assert!(fs.exists(&keep).unwrap(), "a prior install must survive");
}

#[test]
fn command_install_reports_single_installing_phase() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let root = Path::new("/root");
    let reporter = RecordingReporter::default();
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(Vec::new()),
        runner: &FlakyProcess::new(0),
        env: &env,
        reporter: &reporter,
    };
    let plan = plan_for(root);

    execute::execute(&plan, &ctx).unwrap();

    assert_eq!(
        reporter.phases.borrow().as_slice(),
        ["Installing", "Configuring env"]
    );
}

#[test]
fn command_install_rolls_back_dest_on_env_failure() {
    let fs = MemFs::new();
    let env = FailingEnv {
        inner: MemEnvStore::new(),
    };
    let root = Path::new("/root");
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(Vec::new()),
        runner: &FlakyProcess::new(0),
        env: &env,
        reporter: &SilentReporter,
    };
    let plan = plan_for(root);

    let err = execute::execute(&plan, &ctx).unwrap_err();

    assert!(err.to_string().contains("simulated"));
    assert!(!fs.exists(&plan.dest_dir).unwrap());
}

const CHAIN_TOML: &str = r#"
[meta]
name = "go"
version = "1.23.0"

[install]
method = "command"
base_dir = "golang"
fallback = ["download"]

[install.asset]
"x86_64-pc-windows-msvc" = { default = "https://go.dev/d/{version}.zip" }
"x86_64-unknown-linux-gnu" = { default = "https://go.dev/d/{version}.linux.zip" }

[install.command]
"x86_64-pc-windows-msvc" = [
  { tool = "winget", args = ["install", "GoLang.Go", "-h"] },
]

[env]
PATH = "bin"
"#;

fn chain_plans(root: &Path) -> Vec<crate::engine::plan::Plan> {
    let manifest = Manifest::parse(CHAIN_TOML).unwrap();
    manifest.plan_chain(root, &target(), &vsn(), &Selection::default())
}

/// A valid zip containing `go/bin/go.exe`, for the download fallback to unpack.
fn go_zip() -> Vec<u8> {
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    let mut zip_writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip_writer.start_file("go/bin/go.exe", options).unwrap();
    zip_writer.write_all(b"binary").unwrap();
    zip_writer.finish().unwrap().into_inner()
}

#[test]
fn plan_chain_orders_command_before_download() {
    let plans = chain_plans(Path::new("/root"));
    assert_eq!(plans.len(), 2);
    assert!(matches!(plans[0].staged_ops[0], Op::RunCommand { .. }));
    assert!(matches!(plans[1].staged_ops[0], Op::Download { .. }));
}

#[test]
fn plan_chain_skips_platform_methods_without_data() {
    let manifest = Manifest::parse(CHAIN_TOML).unwrap();
    let linux = Target::parts("x86_64", "linux", "gnu");
    let plans = manifest.plan_chain(Path::new("/root"), &linux, &vsn(), &Selection::default());
    assert_eq!(plans.len(), 1);
    assert!(matches!(plans[0].staged_ops[0], Op::Download { .. }));

    let sparc = Target::parts("sparc64", "linux", "gnu");
    assert!(
        manifest
            .plan_chain(Path::new("/root"), &sparc, &vsn(), &Selection::default())
            .is_empty()
    );
}

#[test]
fn execute_chain_falls_back_to_download_when_command_fails() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let root = Path::new("/root");
    let runner = FlakyProcess::new(1);
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(go_zip()),
        runner: &runner,
        env: &env,
        reporter: &SilentReporter,
    };
    let plans = chain_plans(root);

    let index = execute::execute_chain(&plans, &ctx).unwrap();

    assert_eq!(index, 1);
    assert!(fs.exists(&plans[1].dest_dir).unwrap());
    let expected = root.join(".wanted").join("apps").join("golang").join("bin");
    assert_eq!(
        env.read(&EnvVar::from("PATH")).unwrap().unwrap(),
        expected.to_string_lossy()
    );
}

#[test]
fn execute_chain_errors_when_every_attempt_fails() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let root = Path::new("/root");
    let runner = FlakyProcess::new(1);
    let ctx = Ctx {
        root: root.to_path_buf(),
        fs: &fs,
        downloader: &StubDownloader(b"not a zip".to_vec()),
        runner: &runner,
        env: &env,
        reporter: &SilentReporter,
    };
    let plans = chain_plans(root);

    let err = execute::execute_chain(&plans, &ctx).unwrap_err();

    assert!(
        err.to_string().contains("all install attempts failed"),
        "{err}"
    );
}
