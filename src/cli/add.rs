//! Plugin-source resolution for the `add` command.
//!
//! `wanted add` accepts either a local plugin `.toml` (an existing path) or a tool
//! name, in which case the manifest is fetched as `<name>.toml` from a plugin
//! registry. The default registry (the GitHub `wanted-registry`) keeps every
//! manifest at the repository root, so the file for a tool named `golang` lives at
//! `golang.toml`.

use std::path::PathBuf;

/// Default plugin registry: raw GitHub content for the `wanted-registry` repo's
/// `main` branch. `wanted add <name>` appends `<name>.toml` to this base.
pub const DEFAULT_REGISTRY: &str =
    "https://raw.githubusercontent.com/YWZYXSoCool/wanted-registry/main";

/// Where an `add` target's plugin content comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginSource {
    /// An existing local file: the argument is a path on disk.
    Local(PathBuf),
    /// A registry entry: fetch `<name>.toml` from `url` (derived from the base).
    Registry {
        /// Tool name, the part the `.toml` suffix is appended to.
        name: String,
        /// Fully-formed raw URL to download.
        url: String,
    },
}

/// Resolve an `add` target: an existing local path wins (backwards compatible
/// with `wanted add ./foo.toml`); anything else names a registry manifest fetched
/// as `<name>.toml` from `registry` (defaulting to [`DEFAULT_REGISTRY`]).
pub fn resolve_add_source(target: &str, registry: Option<&str>) -> PluginSource {
    let path = PathBuf::from(target);
    if path.is_file() {
        return PluginSource::Local(path);
    }
    let name = target.to_string();
    let base = registry.unwrap_or(DEFAULT_REGISTRY).trim_end_matches('/');
    let url = format!("{base}/{name}.toml");
    PluginSource::Registry { name, url }
}

#[cfg(test)]
mod tests;
