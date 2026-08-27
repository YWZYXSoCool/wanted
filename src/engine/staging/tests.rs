//! Staging area lifecycle tests.

use std::path::Path;

use super::Staging;
use crate::engine::fs::Fs;
use crate::engine::fs::MemFs;

#[test]
fn abort_removes_staged_content() {
    let fs = MemFs::new();
    let dir = Staging::new(Path::new("/root"), "go");
    dir.ensure_clean(&fs).unwrap();
    fs.write(&dir.dir().join("bin/go.exe"), b"x").unwrap();

    dir.abort(&fs).unwrap();
    assert!(!fs.exists(dir.dir()).unwrap());
    assert!(!fs.exists(&dir.dir().join("bin/go.exe")).unwrap());
}
