//! Plugin manifest parsing tests.

use super::{EnvBox, InstallMethod, Manifest};

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
    assert_eq!(manifest.meta.name, "golang");
    assert_eq!(manifest.meta.version, "1.0.0");
    assert_eq!(manifest.meta.url.as_deref(), Some("https://go.dev"));
    assert_eq!(manifest.install.method, InstallMethod::Download);
    assert_eq!(manifest.install.base_dir, "golang");
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
