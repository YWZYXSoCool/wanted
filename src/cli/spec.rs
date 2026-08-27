//! Tool specification parsing: `name@version` for the install command.

use std::fmt;
use std::str::FromStr;

/// A tool spec from the command line, e.g. `go@1.22.5`.
///
/// A bare `name` with no version defaults to `latest`.
///
/// ```rust
/// use std::str::FromStr;
/// use wanted::cli::spec::ToolSpec;
///
/// let pinned = ToolSpec::from_str("go@1.22.5").unwrap();
/// assert_eq!(pinned.name(), "go");
/// assert_eq!(pinned.version(), "1.22.5");
///
/// let latest = ToolSpec::from_str("go").unwrap();
/// assert_eq!(latest.version(), "latest");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ToolSpec {
    name: String,
    version: String,
}

impl ToolSpec {
    /// The tool name, the part before `@`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The requested version, or `latest` when not pinned.
    pub fn version(&self) -> &str {
        &self.version
    }
}

impl FromStr for ToolSpec {
    type Err = ToolSpecError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let spec = raw.trim();
        if spec.is_empty() {
            return Err(ToolSpecError::Empty);
        }
        match spec.split_once('@') {
            None => Ok(ToolSpec {
                name: spec.to_string(),
                version: "latest".to_string(),
            }),
            Some((name, version)) => Self::pinned(name, version, spec),
        }
    }
}

impl fmt::Display for ToolSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

impl ToolSpec {
    fn pinned(name: &str, version: &str, raw: &str) -> Result<Self, ToolSpecError> {
        if name.is_empty() {
            return Err(ToolSpecError::MissingName { raw: raw.into() });
        }
        if version.is_empty() || version.contains('@') {
            return Err(ToolSpecError::InvalidVersion { raw: raw.into() });
        }
        Ok(ToolSpec {
            name: name.to_string(),
            version: version.to_string(),
        })
    }
}

/// An invalid tool spec.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ToolSpecError {
    /// The spec is empty or whitespace.
    #[error("tool spec must not be empty")]
    Empty,
    /// The spec has nothing before `@`.
    #[error("missing tool name before '@' in {raw:?}")]
    MissingName { raw: String },
    /// The version after `@` is empty or contains another `@`.
    #[error("tool spec {raw:?} must pin a single version after '@'")]
    InvalidVersion { raw: String },
}

#[cfg(test)]
mod tests;
