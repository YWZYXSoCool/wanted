//! Uninstall restore: verify receipt-based rollback against the in-memory
//! filesystem and env backend.

use std::path::Path;

use super::{apply_receipt, remove_receipt};
use crate::Receipt;
use crate::VarSnapshot;
use crate::Version;
use crate::engine::fs::{Fs, MemFs};
use crate::env::{EnvDelta, EnvOp, EnvStore, EnvVar, MemEnvStore, PATH_SEP};
use crate::fs_path::AppDir;

/// The PATH variable, for store calls that take a name.
fn path() -> EnvVar {
    EnvVar::from("PATH")
}

#[test]
fn apply_receipt_removes_app_and_restores_env() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    let app_root = Path::new("/root/golang");
    fs.write(&app_root.join("go/bin/go.exe"), b"binary")
        .unwrap();
    env.write(
        &path(),
        &format!("/root/golang/bin{PATH_SEP}/root/gcc/bin{PATH_SEP}/usr/bin"),
    )
    .unwrap();
    env.write(&EnvVar::from("GOROOT"), "/root/golang").unwrap();

    let receipt = Receipt {
        name: "golang".to_string(),
        version: Version::parse("1.23.0").unwrap(),
        app_dir: AppDir::from_path(app_root),
        vars: vec![
            VarSnapshot {
                name: path(),
                op: EnvOp::Prepend,
                value: "/root/golang/bin".to_string(),
                old: Some("/usr/bin".to_string()),
            },
            VarSnapshot {
                name: EnvVar::from("GOROOT"),
                op: EnvOp::Set,
                value: "/root/golang".to_string(),
                old: None,
            },
        ],
    };

    apply_receipt(&receipt, &fs, &env).unwrap();

    assert!(!fs.exists(app_root).unwrap());
    assert_eq!(
        env.read(&path()).unwrap().unwrap(),
        format!("/root/gcc/bin{PATH_SEP}/usr/bin")
    );
    assert_eq!(env.read(&EnvVar::from("GOROOT")).unwrap(), None);
}

/// Install A, then B on top, then uninstall A out of order. A's removal must
/// leave B's segment intact — the original snapshot-restore approach would have
/// clobbered it.
#[test]
fn uninstall_of_earlier_app_preserves_later_app_path() {
    let fs = MemFs::new();
    let env = MemEnvStore::new();
    env.write(&path(), "/usr/bin").unwrap();

    // Receipt A: installed when PATH was `orig`.
    let receipt_a = Receipt {
        name: "gcc".to_string(),
        version: Version::parse("13.0.0").unwrap(),
        app_dir: AppDir::from("/root/gcc"),
        vars: vec![VarSnapshot {
            name: path(),
            op: EnvOp::Prepend,
            value: "/root/gcc/bin".to_string(),
            old: Some("/usr/bin".to_string()),
        }],
    };
    // Simulate install A's merge.
    crate::env::apply_deltas(
        &[EnvDelta {
            name: path(),
            value: "/root/gcc/bin".into(),
            op: EnvOp::Prepend,
        }],
        &env,
    )
    .unwrap();

    // Receipt B: installed later, on top of A.
    let receipt_b = Receipt {
        name: "golang".to_string(),
        version: Version::parse("1.23.0").unwrap(),
        app_dir: AppDir::from("/root/golang"),
        vars: vec![VarSnapshot {
            name: path(),
            op: EnvOp::Prepend,
            value: "/root/golang/bin".to_string(),
            old: Some(format!("/root/gcc/bin{PATH_SEP}/usr/bin")),
        }],
    };
    crate::env::apply_deltas(
        &[EnvDelta {
            name: path(),
            value: "/root/golang/bin".into(),
            op: EnvOp::Prepend,
        }],
        &env,
    )
    .unwrap();
    assert_eq!(
        env.read(&path()).unwrap().unwrap(),
        format!("/root/golang/bin{PATH_SEP}/root/gcc/bin{PATH_SEP}/usr/bin")
    );

    // Uninstall A (the earlier one) first.
    apply_receipt(&receipt_a, &fs, &env).unwrap();

    // B's segment must survive.
    assert_eq!(
        env.read(&path()).unwrap().unwrap(),
        format!("/root/golang/bin{PATH_SEP}/usr/bin")
    );
    // Uninstall B leaves only the original PATH.
    apply_receipt(&receipt_b, &fs, &env).unwrap();
    assert_eq!(env.read(&path()).unwrap().unwrap(), "/usr/bin");
}

#[test]
fn remove_receipt_deletes_subtree() {
    let fs = MemFs::new();
    let path = Path::new("/root/.wanted/installed/golang/receipt.toml");
    let receipt = Receipt {
        name: "golang".to_string(),
        version: Version::parse("1.23.0").unwrap(),
        app_dir: AppDir::from("/root/golang"),
        vars: vec![],
    };
    receipt.write(&fs, path).unwrap();

    remove_receipt(&fs, path).unwrap();

    assert!(!fs.exists(path).unwrap());
    assert!(!fs.exists(path.parent().unwrap()).unwrap());
}
