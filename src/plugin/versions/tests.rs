//! Unit tests for `VersionsSource::parse`.

use super::VersionsSource;

/// A source specifier with only the rules that matter for parsing; the URL is
/// never used by `parse`.
fn source(field: Option<&str>, strip: Option<&str>, stable_only: bool) -> VersionsSource {
    VersionsSource {
        url: "https://example.invalid/versions.json".into(),
        field: field.map(str::to_owned),
        strip: strip.map(str::to_owned),
        stable_only,
    }
}

#[test]
fn go_array_of_objects_field_and_strip() {
    let spec = source(Some("version"), Some("go"), true);
    let body = r#"[
        {"version": "go1.27.0", "stable": true},
        {"version": "go1.26.7", "stable": true},
        {"version": "go1.27.0rc1", "stable": false}
    ]"#;
    let versions = spec.parse(body).unwrap();
    assert!(versions.contains(&"1.27.0".to_string()));
    assert!(versions.contains(&"1.26.7".to_string()));
    assert!(!versions.iter().any(|v| v.starts_with("go")));
    assert!(!versions.iter().any(|v| v.contains("rc")));
}

#[test]
fn node_array_of_objects_strips_v_prefix() {
    let spec = source(Some("version"), Some("v"), false);
    let body = r#"[
        {"version": "v26.8.1"},
        {"version": "v24.20.0"}
    ]"#;
    let versions = spec.parse(body).unwrap();
    assert!(versions.contains(&"26.8.1".to_string()));
    assert!(versions.contains(&"24.20.0".to_string()));
}

#[test]
fn llvm_array_of_objects_uses_tag_name_and_drops_rc() {
    let spec = source(Some("tag_name"), Some("llvmorg-"), true);
    let body = r#"[
        {"tag_name": "llvmorg-23.1.0"},
        {"tag_name": "llvmorg-22.1.8"},
        {"tag_name": "llvmorg-23.1.0rc1"}
    ]"#;
    let versions = spec.parse(body).unwrap();
    assert!(versions.contains(&"23.1.0".to_string()));
    assert!(versions.contains(&"22.1.8".to_string()));
    assert!(!versions.iter().any(|v| v.contains("rc")));
}

#[test]
fn pypi_nested_object_field_names_the_version_map() {
    let spec = source(Some("releases"), None, true);
    let body = r#"{
        "info": {"version": "3.14.7"},
        "releases": {
            "3.14.7": {},
            "3.14.6": {}
        }
    }"#;
    let versions = spec.parse(body).unwrap();
    assert!(versions.contains(&"3.14.7".to_string()));
    assert!(versions.contains(&"3.14.6".to_string()));
}

#[test]
fn object_without_field_uses_top_level_keys() {
    let spec = source(None, None, true);
    let body = r#"{ "3.14.7": {}, "3.14.6": {} }"#;
    let versions = spec.parse(body).unwrap();
    assert!(versions.contains(&"3.14.7".to_string()));
    assert!(versions.contains(&"3.14.6".to_string()));
}

#[test]
fn nested_field_with_missing_key_errors() {
    let spec = source(Some("releases"), None, false);
    let body = r#"{ "info": {} }"#;
    assert!(spec.parse(body).is_err());
}

#[test]
fn bare_string_array_without_field() {
    let spec = source(None, Some("v"), false);
    let body = r#"["v1.2.3", "v1.2.4"]"#;
    let versions = spec.parse(body).unwrap();
    assert_eq!(versions, vec!["1.2.3".to_string(), "1.2.4".to_string()]);
}

#[test]
fn skips_non_semver_entries() {
    let spec = source(None, None, false);
    let body = r#"["1.2.3", "not-a-version", "go1.4.0"]"#;
    let versions = spec.parse(body).unwrap();
    assert_eq!(versions, vec!["1.2.3".to_string()]);
}

#[test]
fn skips_entries_missing_declared_field() {
    let spec = source(Some("version"), None, false);
    let body = r#"[{"version": "1.2.3"}, {"name": "1.2.4"}]"#;
    let versions = spec.parse(body).unwrap();
    assert_eq!(versions, vec!["1.2.3".to_string()]);
}

#[test]
fn errors_when_nothing_survives_filtering() {
    let spec = source(Some("version"), Some("go"), true);
    let body = r#"[
        {"version": "go1.27.0rc1", "stable": false},
        {"version": "not-a-version"}
    ]"#;
    assert!(spec.parse(body).is_err());
}

#[test]
fn errors_on_non_json_body() {
    let spec = source(None, None, false);
    assert!(spec.parse("hello").is_err());
}

#[test]
fn errors_on_scalar_body() {
    let spec = source(None, None, false);
    assert!(spec.parse("42").is_err());
}
