//! A validated, typed install version.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The version to install: an unpinned `latest` or a validated semantic version.
///
/// A typed wrapper keeps `{version}` templating, receipts, and uninstall display
/// from treating a bare string as arbitrary text: a pinned value must be well-formed
/// SemVer, and the `latest` case is explicit rather than a magic string.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
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
            return Ok(Version::Latest);
        }
        Ok(Version::Pinned(source.parse()?))
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

/// Resolve a source's declared versions to the newest by semantic ordering.
///
/// Requires every version string to be a valid SemVer so the comparison is
/// unambiguous; an empty list or a non-SemVer entry is an error rather than a
/// best-effort guess.
pub fn pick_latest(versions: &[String]) -> crate::Result<Version> {
    if versions.is_empty() {
        return Err(crate::Error::Other(
            "source declares no versions to resolve latest".into(),
        ));
    }
    let mut parsed = Vec::with_capacity(versions.len());
    for raw in versions {
        let version = raw.parse::<semver::Version>().map_err(|detail| {
            crate::Error::Other(format!(
                "cannot resolve latest: source version {raw:?} is not a comparable SemVer ({detail})"
            ))
        })?;
        parsed.push(version);
    }
    parsed.sort();
    parsed
        .into_iter()
        .max()
        .map(Version::Pinned)
        .ok_or_else(|| crate::Error::Other("source declares no versions".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pick_latest_selects_max_semver() {
        let picked = pick_latest(&versions(&["1.2.0", "1.10.0", "1.3.1"])).unwrap();
        assert_eq!(picked, Version::parse("1.10.0").unwrap());
    }

    #[test]
    fn pick_latest_single_entry() {
        assert_eq!(
            pick_latest(&versions(&["0.1.0"])).unwrap(),
            Version::parse("0.1.0").unwrap()
        );
    }

    #[test]
    fn pick_latest_errors_on_empty_list() {
        assert!(pick_latest(&[]).is_err());
    }

    #[test]
    fn pick_latest_errors_on_non_semver_entry() {
        let err = pick_latest(&versions(&["1.2.0", "go1.3.0"])).unwrap_err();
        assert!(err.to_string().contains("go1.3.0"));
    }
}
