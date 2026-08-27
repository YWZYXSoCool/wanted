//! Tests for tool spec parsing.

use super::{ToolSpec, ToolSpecError};
use std::str::FromStr;

#[test]
fn test_pinned_name_and_version() {
    let spec = ToolSpec::from_str("go@1.22.5").unwrap();
    assert_eq!(spec.name(), "go");
    assert_eq!(spec.version(), "1.22.5");
}

#[test]
fn test_bare_name_defaults_to_latest() {
    let spec = ToolSpec::from_str("go").unwrap();
    assert_eq!(spec.name(), "go");
    assert_eq!(spec.version(), "latest");
}

#[test]
fn test_whitespace_is_trimmed() {
    let spec = ToolSpec::from_str("  go@1.2  ").unwrap();
    assert_eq!(spec.name(), "go");
    assert_eq!(spec.version(), "1.2");
}

#[test]
fn test_empty_spec_is_rejected() {
    assert_eq!(ToolSpec::from_str(""), Err(ToolSpecError::Empty));
}

#[test]
fn test_multi_version_spec_is_rejected() {
    assert_eq!(
        ToolSpec::from_str("go@1@2"),
        Err(ToolSpecError::InvalidVersion {
            raw: "go@1@2".into()
        })
    );
}

#[test]
fn test_missing_name_is_rejected() {
    assert_eq!(
        ToolSpec::from_str("@1.22"),
        Err(ToolSpecError::MissingName {
            raw: "@1.22".into()
        })
    );
}

#[test]
fn test_missing_version_is_rejected() {
    assert_eq!(
        ToolSpec::from_str("go@"),
        Err(ToolSpecError::InvalidVersion { raw: "go@".into() })
    );
}

#[test]
fn test_display_round_trips_with_at() {
    let spec = ToolSpec::from_str("go@1.22.5").unwrap();
    assert_eq!(spec.to_string(), "go@1.22.5");
}
