//! A validated, typed install version.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The version to install: an unpinned `latest` or a validated semantic version.
///
/// A typed wrapper keeps `{version}` templating, receipts, and uninstall display
/// from treating a bare string as arbitrary text: a pinned value must be well-formed
/// SemVer, and the `latest` case is explicit rather than a magic string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Version {
    /// No version requested; `{version}` resolves to the literal `latest`.
    Latest,
    /// A pinned, validated semantic version.
    Pinned(semver::Version),
}

impl Version {
    /// Parse a version string, treating `latest` as the unpinned case.
    pub fn parse(source: &str) -> Result<Self, semver::Error> {
        if source == "latest" {
            Ok(Version::Latest)
        } else {
            Ok(Version::Pinned(source.parse()?))
        }
    }
}

impl FromStr for Version {
    type Err = semver::Error;

    #[inline]
    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Version::parse(source)
    }
}

impl fmt::Display for Version {
    /// Render the version verbatim (`latest`, or the semantic version).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Version::Latest => f.write_str("latest"),
            Version::Pinned(version) => version.fmt(f),
        }
    }
}

impl Serialize for Version {
    /// Serialize as the printable version string.
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Version {
    /// Deserialize from a printable version string.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let source = String::deserialize(deserializer)?;
        Version::parse(&source).map_err(serde::de::Error::custom)
    }
}
