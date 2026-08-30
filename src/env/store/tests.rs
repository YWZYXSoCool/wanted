//! POSIX `RealEnvStore` file-backend tests: writes parse/upsert lines and are
//! shell-escaped, and removals drop exactly the named variable.

use std::fs;
use std::path::Path;

use super::RealEnvStore;
use crate::env::EnvVar;
use tempfile::NamedTempFile;

/// Read a backend file's lines for assertions.
fn file_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .expect("env file readable")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn store_at(file: &NamedTempFile) -> RealEnvStore {
    RealEnvStore::at(file.path().to_path_buf())
}

#[test]
fn test_write_writes_export_line() {
    let file = NamedTempFile::new().unwrap();
    let store = store_at(&file);
    let path = EnvVar::from("PATH");
    store.write(&path, "/opt/go/bin").unwrap();
    assert_eq!(file_lines(file.path()), vec!["export PATH=\"/opt/go/bin\""]);
}

#[test]
fn test_write_replaces_same_name() {
    let file = NamedTempFile::new().unwrap();
    let store = store_at(&file);
    let path = EnvVar::from("PATH");
    store.write(&path, "/one").unwrap();
    store.write(&path, "/two").unwrap();
    let lines = file_lines(file.path());
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("export PATH="))
            .count(),
        1
    );
    assert!(lines.contains(&"export PATH=\"/two\"".to_owned()));
}

#[test]
fn test_write_keeps_other_names() {
    let file = NamedTempFile::new().unwrap();
    let store = store_at(&file);
    store.write(&EnvVar::from("A"), "/a").unwrap();
    store.write(&EnvVar::from("B"), "/b").unwrap();
    store.write(&EnvVar::from("A"), "/a2").unwrap();
    let lines = file_lines(file.path());
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("export A=")).count(),
        1
    );
    assert!(lines.contains(&"export A=\"/a2\"".to_owned()));
    assert!(lines.contains(&"export B=\"/b\"".to_owned()));
}

#[test]
fn test_remove_drops_only_named_line() {
    let file = NamedTempFile::new().unwrap();
    let store = store_at(&file);
    store.write(&EnvVar::from("A"), "/a").unwrap();
    store.write(&EnvVar::from("B"), "/b").unwrap();
    store.remove(&EnvVar::from("A")).unwrap();
    assert_eq!(file_lines(file.path()), vec!["export B=\"/b\""]);
    store.remove(&EnvVar::from("A")).unwrap();
    assert_eq!(file_lines(file.path()), vec!["export B=\"/b\""]);
}

#[test]
fn test_escaping_keeps_value_on_one_line() {
    let file = NamedTempFile::new().unwrap();
    let store = store_at(&file);
    let value = "a\"b\\c";
    store.write(&EnvVar::from("WEIRD"), value).unwrap();
    let lines = file_lines(file.path());
    assert_eq!(lines, vec!["export WEIRD=\"a\\\"b\\\\c\""]);
}

#[test]
fn test_remove_absent_file_is_noop() {
    let file = NamedTempFile::new().unwrap();
    store_at(&file).remove(&EnvVar::from("A")).unwrap();
    assert!(!file.path().exists() || file_lines(file.path()).is_empty());
}
