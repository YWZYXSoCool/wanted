//! In-memory filesystem behavior tests.

use std::path::Path;

use super::{Fs, MemFs};

#[test]
fn mem_fs_write_read_roundtrip() {
    let fs = MemFs::new();
    fs.write(Path::new("/a/b/c.txt"), b"hello").unwrap();
    assert_eq!(fs.read(Path::new("/a/b/c.txt")).unwrap(), b"hello");
    assert!(fs.exists(Path::new("/a/b/c.txt")).unwrap());
    assert!(fs.exists(Path::new("/a/b")).unwrap());
}

#[test]
fn mem_fs_rename_moves_subtree() {
    let fs = MemFs::new();
    fs.write(Path::new("/staging/go/bin/go.exe"), b"x").unwrap();
    fs.write(Path::new("/staging/go/LICENSE"), b"y").unwrap();
    fs.rename(Path::new("/staging/go"), Path::new("/apps/golang"))
        .unwrap();

    assert!(!fs.exists(Path::new("/staging/go")).unwrap());
    assert_eq!(fs.read(Path::new("/apps/golang/bin/go.exe")).unwrap(), b"x");
    assert_eq!(fs.read(Path::new("/apps/golang/LICENSE")).unwrap(), b"y");
}

#[test]
fn mem_fs_remove_dir_all_prunes_subtree() {
    let fs = MemFs::new();
    fs.write(Path::new("/a/b/c.txt"), b"x").unwrap();
    fs.remove_dir_all(Path::new("/a")).unwrap();
    assert!(!fs.exists(Path::new("/a")).unwrap());
}

#[test]
fn mem_fs_rename_replaces_existing_target() {
    let fs = MemFs::new();
    fs.write(Path::new("/from.txt"), b"new").unwrap();
    fs.write(Path::new("/to.txt"), b"old").unwrap();
    fs.rename(Path::new("/from.txt"), Path::new("/to.txt"))
        .unwrap();
    assert_eq!(fs.read(Path::new("/to.txt")).unwrap(), b"new");
}
