//! Tool specification parsing: `name@version` for the install command.

use std::fmt;
use std::str::FromStr;

use crate::Version;
use crate::fs_path::DirName;

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
/// assert_eq!(pinned.name().as_str(), "go");
/// assert_eq!(pinned.version(), &Version::parse("1.22.5").unwrap());
///
/// let latest = ToolSpec::from_str("go").unwrap();
/// assert_eq!(latest.version(), &Version::Latest);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ToolSpec {
    name: DirName,
    version: Version,
}

impl ToolSpec {
    /// The tool name, the part before `@`.
    #[inline]
    pub fn name(&self) -> &DirName {
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
                name: parse_name(spec)?,
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
            name: parse_name(name)?,
            version,
        })
    }
}

/// Parse the tool name as a platform-safe directory segment.
fn parse_name(raw: &str) -> Result<DirName, ToolSpecError> {
    raw.to_string()
        .try_into()
        .map_err(|_| ToolSpecError::InvalidName {
            raw: raw.to_string(),
        })
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
    /// The tool name is not usable as a directory segment on this platform.
    #[error("tool name is not usable as a directory segment: {raw:?}")]
    InvalidName { raw: String },
    /// The version after `@` is not a valid SemVer.
    #[error("tool spec {raw:?} has an invalid version: {detail}")]
    InvalidVersion { raw: String, detail: String },
}

#[cfg(test)]
mod tests;
