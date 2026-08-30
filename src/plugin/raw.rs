// Raw deserialization types shared by two consumers so they can never drift:
//
// - the library's `Manifest::parse`, which deserializes TOML into them via serde;
// - the build script (`build.rs`), which `include!`s this file and derives the
//   plugin JSON Schema from the exact same types.
//
// Keep every field that affects `parse` here; the generated schema mirrors it 1:1.
//
// NOTE: file-level `//!` doc comments are intentionally avoided because this file
// is `include!`d inside build.rs mid-file, where inner doc comments are illegal.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;

/// Install method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InstallMethod {
    /// Extract the vendor-distributed archive directly.
    Download,
    /// Run the downloaded asset as a silent installer, targeting `apps/<base_dir>`.
    Installer,
    /// Delegate to a system package manager (`winget` / `brew`); not wired in M0.
    System,
    /// Run external package-manager commands (e.g. `cargo install`, `npm i -g`)
    /// in order, falling back to the next on a missing tool or non-zero exit.
    Command,
}

/// A per-platform override of the install method and its silent arguments.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct RawStrategy {
    /// Install method for this platform.
    pub method: InstallMethod,
    /// Silent-install arguments, with `{base}` = install dir under `apps`.
    #[serde(default)]
    pub args: Vec<String>,
}

/// One external install command, tried in order until one succeeds.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct RawCommand {
    /// The program to invoke, resolved via PATH (e.g. "cargo", "npm").
    pub tool: String,
    /// Argument templates; `{base}` / `{version}` / `{user}` are expanded.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables (name -> template), layered over the
    /// inherited environment of the child process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// The PATH box (prepend or append).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EnvBox {
    /// Prepend.
    Prepend,
    /// Append.
    Append,
}

/// Asset map shape: platform triplet -> source name -> URL template (with `{version}`).
pub type AssetMap = BTreeMap<String, BTreeMap<String, String>>;

/// A plugin manifest exactly as it appears in a TOML file.
#[derive(Deserialize, JsonSchema)]
pub struct RawManifest {
    /// Metadata.
    pub meta: RawMeta,
    /// Install section.
    pub install: RawInstall,
    /// Environment variables to configure (name -> template).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Per-platform overrides of the `env` values above, keyed by platform
    /// triplet. A matching entry redefines a variable on that platform only;
    /// variables without a platform entry keep their global `env` value.
    #[serde(default)]
    pub env_by_platform: BTreeMap<String, BTreeMap<String, String>>,
}

/// Manifest metadata.
#[derive(Deserialize, JsonSchema)]
pub struct RawMeta {
    /// Tool name.
    pub name: Option<String>,
    /// Manifest's own version (SemVer).
    pub version: Option<String>,
    /// Source / update origin.
    #[serde(default)]
    pub url: Option<String>,
}

/// Install section.
#[derive(Deserialize, JsonSchema)]
pub struct RawInstall {
    /// Install method.
    pub method: InstallMethod,
    /// Asset map: platform triplet -> source name -> URL template (with `{version}`).
    #[serde(default)]
    pub asset: AssetMap,
    /// Optional components (platform-keyed like `asset`), downloaded only when enabled.
    #[serde(default)]
    pub component: BTreeMap<String, AssetMap>,
    /// Placement path relative to `apps`.
    pub base_dir: String,
    /// The PATH box.
    #[serde(default)]
    pub env_box: Option<EnvBox>,
    /// Default silent-install arguments (with `{base}`), used by the `installer` method.
    #[serde(default)]
    pub args: Vec<String>,
    /// Per-platform override of method and silent arguments.
    #[serde(default)]
    pub strategy: BTreeMap<String, RawStrategy>,
    /// External install commands for the `command` method, keyed by platform
    /// triplet; the ordered list is tried in fallback order until one succeeds.
    #[serde(default)]
    pub command: BTreeMap<String, Vec<RawCommand>>,
    /// Fallback install methods, tried in order after the primary `method`
    /// fails. Each entry reuses this section's already-declared data
    /// (`asset`, `command`, `args`).
    #[serde(default)]
    pub fallback: Vec<InstallMethod>,
}
