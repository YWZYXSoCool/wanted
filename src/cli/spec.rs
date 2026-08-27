//! Tool specification parsing: `name@version` for the install command.

use std::fmt;
use std::str::FromStr;

use crate::Version;

/// A tool spec from the command line, e.g. `go@1.22.5`.
///
/// A bare `name` with no version defaults to `latest`.
///
/// ```rust
/// use std::str::FromStr;
/// use wanted::Version;
/// use wanted::cli::spec::ToolSpec;
///
/// let pinned = ToolSpec::from_str("go@1.22.5").unwrap();
/// assert_eq!(pinned.name(), "go");
/// assert_eq!(pinned.version(), &Version::parse("1.22.5").unwrap());
///
/// let latest = ToolSpec::from_str("go").unwrap();
/// assert_eq!(latest.version(), &Version::Latest);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ToolSpec {
    name: String,
    version: Version,
}

impl ToolSpec {
    /// The tool name, the part before `@`.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The requested version, or `latest` when not pinned.
    #[inline]
    pub fn version(&self) -> &Version {
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
                version: Version::Latest,
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
        let version =
            version
                .parse::<Version>()
                .map_err(|detail| ToolSpecError::InvalidVersion {
                    raw: raw.into(),
                    detail: detail.to_string(),
                })?;
        Ok(ToolSpec {
            name: name.to_string(),
            version,
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
    /// The version after `@` is not a valid SemVer.
    #[error("tool spec {raw:?} has an invalid version: {detail}")]
    InvalidVersion { raw: String, detail: String },
}

#[cfg(test)]
mod tests;
