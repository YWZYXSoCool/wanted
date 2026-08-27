//! Uninstall restore: verify receipt-based rollback against the in-memory
//! filesystem and env backend.

use std::path::Path;

use super::{apply_receipt, remove_receipt};
use crate::Receipt;
use crate::VarSnapshot;
use crate::engine::fs::{Fs, MemFs};
use crate::env::{EnvStore, MemEnvStore};

#[test]
fn apply_receipt_removes_app_and_restores_env() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let app_root = Path::new("/root/.wanted/apps/golang");
    fs.write(&app_root.join("go/bin/go.exe"), b"binary")
        .unwrap();
    env.write("PATH", "/root/.wanted/apps/golang/bin;/usr/bin")
        .unwrap();
    env.write("GOROOT", "/root/.wanted/apps/golang").unwrap();

    let receipt = Receipt {
        name: "golang".to_string(),
        version: "1.23.0".to_string(),
        app_dir: app_root.to_string_lossy().into_owned(),
        vars: vec![
            VarSnapshot {
                name: "PATH".to_string(),
                old: Some("/usr/bin".to_string()),
            },
            VarSnapshot {
                name: "GOROOT".to_string(),
                old: None,
            },
        ],
    };

    apply_receipt(&receipt, &fs, &env).unwrap();

    assert!(!fs.exists(app_root).unwrap());
    assert_eq!(env.read("PATH").unwrap().unwrap(), "/usr/bin");
    assert_eq!(env.read("GOROOT").unwrap(), None);
}

#[test]
fn remove_receipt_deletes_subtree() {
    let fs = MemFs::new();
    let path = Path::new("/root/.wanted/installed/golang/receipt.toml");
    let receipt = Receipt {
        name: "golang".to_string(),
        version: "1.23.0".to_string(),
        app_dir: "/root/.wanted/apps/golang".to_string(),
        vars: vec![],
    };
    receipt.write(&fs, path).unwrap();

    remove_receipt(&fs, path).unwrap();

    assert!(!fs.exists(path).unwrap());
    assert!(!fs.exists(path.parent().unwrap()).unwrap());
}
