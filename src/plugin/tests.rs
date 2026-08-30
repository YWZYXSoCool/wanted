//! Plugin manifest parsing tests.

use super::{EnvBox, InstallMethod, Manifest, Target};

const COMPONENT_TOML: &str = r#"
[meta]
name = "llvm"
version = "1.0.0"

[install]
method = "download"
base_dir = "llvm"
asset = { "x86_64-pc-windows-msvc" = { default = "https://ex/llvm-{version}.zip" } }

[install.component]
"clang" = { "x86_64-pc-windows-msvc" = { default = "https://ex/clang-{version}.zip" } }
"#;

#[test]
fn parses_optional_components() {
    let manifest = Manifest::parse(COMPONENT_TOML).unwrap();
    let clang = &manifest.install.components["clang"];
    assert_eq!(
        clang["x86_64-pc-windows-msvc"]["default"],
        "https://ex/clang-{version}.zip"
    );
}

#[test]
fn components_default_to_empty_when_absent() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    assert!(manifest.install.components.is_empty());
}

const GOLANG_TOML: &str = r#"
[meta]
name = "golang"
version = "1.0.0"
url = "https://go.dev"

[install]
method = "download"
base_dir = "golang"
env_box = "prepend"

[install.asset]
"x86_64-pc-windows-msvc" = { default = ".../go{version}.windows-amd64.zip" }
"x86_64-unknown-linux-gnu" = { default = ".../go{version}.linux-amd64.tar.gz" }

[env]
PATH = "bin"
GOROOT = "."
"#;

#[test]
fn parses_golang_manifest() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    assert_eq!(manifest.meta.name.as_str(), "golang");
    assert_eq!(manifest.meta.version, "1.0.0");
    assert_eq!(manifest.meta.url.as_deref(), Some("https://go.dev"));
    assert_eq!(manifest.install.method, InstallMethod::Download);
    assert_eq!(manifest.install.base_dir.as_str(), "golang");
    assert_eq!(manifest.install.env_box, EnvBox::Prepend);
    assert_eq!(manifest.install.assets.len(), 2);
    assert!(manifest.install.assets["x86_64-pc-windows-msvc"].contains_key("default"));
    assert_eq!(manifest.env.len(), 2);
}

#[test]
fn rejects_missing_meta_name() {
    let source =
        "[meta]\nversion = \"1.0.0\"\n[install]\nmethod = \"download\"\nbase_dir = \"go\"\n";
    assert!(Manifest::parse(source).is_err());
}

#[test]
fn rejects_download_without_assets() {
    let source = r#"
[meta]
name = "x"
version = "1.0.0"
[install]
method = "download"
base_dir = "x"
"#;
    assert!(Manifest::parse(source).is_err());
}

const STRATEGY_TOML: &str = r#"
[meta]
name = "llvm"
version = "1.0.0"

[install]
method = "download"
base_dir = "llvm"
asset = { "x86_64-pc-windows-msvc" = { default = "https://ex/LLVM-{version}-win64.exe" }, "x86_64-unknown-linux-gnu" = { default = "https://ex/llvm-{version}.tar.xz" } }

[install.strategy]
"x86_64-pc-windows-msvc" = { method = "installer", args = ["/S", "/DIR={base}"] }
"#;

#[test]
fn parses_per_platform_strategy() {
    let manifest = Manifest::parse(STRATEGY_TOML).unwrap();
    assert_eq!(manifest.install.strategy.len(), 1);
    assert_eq!(
        manifest.install.strategy["x86_64-pc-windows-msvc"].method,
        InstallMethod::Installer
    );
    assert_eq!(
        manifest.install.strategy["x86_64-pc-windows-msvc"].args,
        ["/S", "/DIR={base}"]
    );
}

#[test]
fn method_for_hits_strategy_on_windows_and_falls_back_on_linux() {
    let manifest = Manifest::parse(STRATEGY_TOML).unwrap();
    let windows = Target::parts("x86_64", "windows", "msvc");
    let linux = Target::parts("x86_64", "linux", "gnu");
    let (win_method, win_args) = manifest.install.method_for(&windows);
    let (linux_method, linux_args) = manifest.install.method_for(&linux);
    assert_eq!(*win_method, InstallMethod::Installer);
    assert_eq!(win_args, ["/S", "/DIR={base}"]);
    assert_eq!(*linux_method, InstallMethod::Download);
    assert!(linux_args.is_empty());
}

#[test]
fn installer_also_requires_assets() {
    let source = r#"
[meta]
name = "llvm"
version = "1.0.0"
[install]
method = "installer"
base_dir = "llvm"
"#;
    assert!(Manifest::parse(source).is_err());
}

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
"#;

#[test]
fn parses_command_install_method() {
    let manifest = Manifest::parse(COMMAND_TOML).unwrap();
    assert_eq!(manifest.install.method, InstallMethod::Command);
    assert_eq!(manifest.install.commands.len(), 1);
    let commands = &manifest.install.commands["x86_64-pc-windows-msvc"];
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].tool, "cargo");
    assert_eq!(commands[0].args, ["install", "--root", "{base}", "bat"]);
    assert_eq!(commands[0].env["CARGO_INSTALL_ROOT"], "{base}");
    assert_eq!(commands[1].tool, "npm");
    assert!(commands[1].env.is_empty());
}

#[test]
fn rejects_command_without_commands() {
    let source = r#"
[meta]
name = "bat"
version = "1.0.0"
[install]
method = "command"
base_dir = "bat"
"#;
    assert!(Manifest::parse(source).is_err());
}

const CHAIN_TOML: &str = r#"
[meta]
name = "go"
version = "1.0.0"

[install]
method = "command"
base_dir = "golang"
fallback = ["download"]
asset = { "x86_64-pc-windows-msvc" = { default = "https://ex/go{version}.zip" } }

[install.command]
"x86_64-pc-windows-msvc" = [
  { tool = "choco", args = ["install", "golang", "-y", "--params", "/InstallDir:{base}"] },
]
"#;

#[test]
fn parses_fallback_chain() {
    let manifest = Manifest::parse(CHAIN_TOML).unwrap();
    assert_eq!(manifest.install.method, InstallMethod::Command);
    assert_eq!(manifest.install.fallback, [InstallMethod::Download]);
}

#[test]
fn fallback_download_still_requires_assets() {
    let source = r#"
[meta]
name = "go"
version = "1.0.0"
[install]
method = "command"
base_dir = "golang"
fallback = ["download"]
[install.command]
"x86_64-pc-windows-msvc" = [{ tool = "choco", args = [] }]
"#;
    assert!(Manifest::parse(source).is_err());
}

const VERSIONS_TOML: &str = r#"
[meta]
name = "go"
version = "1.0.0"

[install]
method = "download"
base_dir = "golang"
asset = { "x86_64-pc-windows-msvc" = { default = "https://ex/go{version}.zip", "go.dev" = "https://go.dev/dl/go{version}.zip" } }

[install.versions]
default = ["1.21.0", "1.22.5"]
"go.dev" = ["1.20.0", "1.22.5", "1.23.1"]
"#;

#[test]
fn parses_versions_per_source() {
    let manifest = Manifest::parse(VERSIONS_TOML).unwrap();
    assert_eq!(manifest.install.versions["default"], ["1.21.0", "1.22.5"]);
    assert_eq!(
        manifest.install.versions["go.dev"],
        ["1.20.0", "1.22.5", "1.23.1"]
    );
}

#[test]
fn versions_absent_are_empty() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    assert!(manifest.install.versions.is_empty());
}

#[test]
fn versions_for_defaults_to_default_source() {
    let manifest = Manifest::parse(VERSIONS_TOML).unwrap();
    assert_eq!(
        manifest.install.versions_for(None),
        Some(&vec!["1.21.0".into(), "1.22.5".into()])
    );
    assert_eq!(
        manifest.install.versions_for(Some("go.dev")),
        Some(&vec!["1.20.0".into(), "1.22.5".into(), "1.23.1".into()])
    );
    assert_eq!(manifest.install.versions_for(Some("missing")), None);
}

#[test]
fn latest_for_resolves_newest_version_of_the_source() {
    let manifest = Manifest::parse(VERSIONS_TOML).unwrap();
    let latest = manifest.install.latest_for(None).unwrap().unwrap();
    assert_eq!(latest, crate::Version::parse("1.22.5").unwrap());
    let latest_dev = manifest
        .install
        .latest_for(Some("go.dev"))
        .unwrap()
        .unwrap();
    assert_eq!(latest_dev, crate::Version::parse("1.23.1").unwrap());
}

#[test]
fn latest_for_is_none_when_source_declares_no_versions() {
    let manifest = Manifest::parse(GOLANG_TOML).unwrap();
    assert!(manifest.install.latest_for(None).is_none());
}

#[test]
fn versions_source_for_exposes_declared_endpoint() {
    const SOURCE: &str = r#"
[meta]
name = "go"
version = "1.0.0"
[install]
method = "download"
base_dir = "golang"
asset = { "x86_64-pc-windows-msvc" = { default = "https://ex/{version}.zip" } }
[install.versions_source]
default = { url = "https://ex/go/index.json", field = "version", strip = "go", stable_only = true }
"#;
    let manifest = Manifest::parse(SOURCE).unwrap();
    let source = manifest.install.versions_source_for(None).unwrap();
    assert_eq!(source.url, "https://ex/go/index.json");
    assert_eq!(source.field.as_deref(), Some("version"));
    assert_eq!(source.strip.as_deref(), Some("go"));
    assert!(source.stable_only);
    assert!(
        manifest
            .install
            .versions_source_for(Some("missing"))
            .is_none()
    );
}

#[test]
fn versions_source_parses_suffix_and_nested_fields() {
    const SOURCE: &str = r#"
[meta]
name = "python3"
version = "1.1.0"
[install]
method = "download"
base_dir = "python3"
asset = { "x86_64-unknown-linux-gnu" = { default = "https://ex/{date}/{version}.tar.gz" } }
[install.versions_source]
default = { url = "https://ex/pbs/latest", field = "assets[].name", strip = "cpython-", suffix = "-x86_64-unknown-linux-gnu-install_only.tar.gz", stable_only = true }
"#;
    let manifest = Manifest::parse(SOURCE).unwrap();
    let source = manifest.install.versions_source_for(None).unwrap();
    assert_eq!(source.field.as_deref(), Some("assets[].name"));
    assert_eq!(source.strip.as_deref(), Some("cpython-"));
    assert_eq!(
        source.suffix.as_deref(),
        Some("-x86_64-unknown-linux-gnu-install_only.tar.gz")
    );
}

#[test]
fn versions_source_fills_no_inline_list() {
    const SOURCE: &str = r#"
[meta]
name = "go"
version = "1.0.0"
[install]
method = "download"
base_dir = "golang"
asset = { "x86_64-pc-windows-msvc" = { default = "https://ex/{version}.zip" } }
[install.versions_source]
default = { url = "https://ex/go/index.json" }
"#;
    let manifest = Manifest::parse(SOURCE).unwrap();
    assert!(manifest.install.versions_for(None).is_none());
    assert!(manifest.install.latest_for(None).is_none());
    assert!(manifest.install.versions_source_for(None).is_some());
}

#[test]
fn latest_for_errors_on_non_semver_version() {
    const SOURCE: &str = r#"
[meta]
name = "go"
version = "1.0.0"
[install]
method = "download"
base_dir = "golang"
asset = { "x86_64-pc-windows-msvc" = { default = "https://ex/{version}.zip" } }
[install.versions]
default = ["1.2.0", "go1.3.0"]
"#;
    let manifest = Manifest::parse(SOURCE).unwrap();
    assert!(manifest.install.latest_for(None).unwrap().is_err());
}
