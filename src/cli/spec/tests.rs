//! Tests for tool spec parsing.

use super::{ToolSpec, ToolSpecError};
use crate::Version;
use std::str::FromStr;

#[test]
fn test_pinned_name_and_version() {
    let spec = ToolSpec::from_str("go@1.22.5").unwrap();
    assert_eq!(spec.name().as_str(), "go");
    assert_eq!(spec.version(), &Version::parse("1.22.5").unwrap());
}

#[test]
fn test_bare_name_defaults_to_latest() {
    let spec = ToolSpec::from_str("go").unwrap();
    assert_eq!(spec.name().as_str(), "go");
    assert_eq!(spec.version(), &Version::Latest);
}

#[test]
fn test_whitespace_is_trimmed() {
    let spec = ToolSpec::from_str("  go@1.2.0  ").unwrap();
    assert_eq!(spec.name().as_str(), "go");
    assert_eq!(spec.version(), &Version::parse("1.2.0").unwrap());
}

#[test]
fn test_empty_spec_is_rejected() {
    assert_eq!(ToolSpec::from_str(""), Err(ToolSpecError::Empty));
}

#[test]
fn test_multi_version_spec_is_rejected() {
    assert!(matches!(
        ToolSpec::from_str("go@1@2"),
        Err(ToolSpecError::InvalidVersion { .. })
    ));
}

#[test]
fn test_non_semver_version_is_rejected() {
    assert!(matches!(
        ToolSpec::from_str("go@banana"),
        Err(ToolSpecError::InvalidVersion { .. })
    ));
}

#[test]
fn test_missing_name_is_rejected() {
    assert_eq!(
        ToolSpec::from_str("@1.22.0"),
        Err(ToolSpecError::MissingName {
            raw: "@1.22.0".into()
        })
    );
}

#[test]
fn test_missing_version_is_rejected() {
    assert!(matches!(
        ToolSpec::from_str("go@"),
        Err(ToolSpecError::InvalidVersion { .. })
    ));
}

#[test]
fn test_display_round_trips_with_at() {
    let spec = ToolSpec::from_str("go@1.22.5").unwrap();
    assert_eq!(spec.to_string(), "go@1.22.5");
}
