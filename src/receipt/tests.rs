//! Receipt round-trips: serialize -> write -> read back with unchanged semantics.

use std::path::Path;

use super::{Receipt, VarSnapshot};
use crate::Version;
use crate::engine::fs::MemFs;
use crate::env::{EnvOp, EnvVar};
use crate::fs_path::AppDir;

fn sample() -> Receipt {
    Receipt {
        name: "golang".to_string(),
        version: Version::parse("1.23.0").unwrap(),
        app_dir: AppDir::from("/root/golang"),
        vars: vec![
            VarSnapshot {
                name: EnvVar::from("PATH"),
                op: EnvOp::Prepend,
                value: "C:\\apps\\golang\\bin".to_string(),
                old: Some("C:\\before".to_string()),
            },
            VarSnapshot {
                name: EnvVar::from("GOROOT"),
                op: EnvOp::Set,
                value: "C:\\apps\\golang".to_string(),
                old: None,
            },
        ],
    }
}

#[test]
fn receipt_to_and_from_toml_round_trips() {
    let receipt = sample();
    let text = receipt.to_toml().unwrap();
    let back = Receipt::from_toml(&text).unwrap();
    assert_eq!(receipt, back);
}

#[test]
fn receipt_write_and_read_round_trips() {
    let fs = MemFs::new();
    let path = Path::new("/w/.wanted/installed/golang/receipt.toml");
    let receipt = sample();
    receipt.write(&fs, path).unwrap();
    let back = Receipt::read(&fs, path).unwrap().unwrap();
    assert_eq!(receipt, back);
}

#[test]
fn receipt_read_missing_returns_none() {
    let fs = MemFs::new();
    let back = Receipt::read(&fs, Path::new("/nope/receipt.toml")).unwrap();
    assert!(back.is_none());
}
