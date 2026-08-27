//! Extraction tests: uniform leading-directory stripping, flat archives kept
//! intact, and path sanitization.

use std::io::Cursor;
use std::io::Write;
use std::path::Path;

use super::extract;
use crate::engine::fs::{Fs, MemFs};

/// Build an in-memory zip from the given `(path, content)` entries.
fn zip_with(entries: &[(&str, &str)]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let cursor = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let options = SimpleFileOptions::default();
    for (name, content) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn strips_uniform_leading_dir() {
    let fs = MemFs::new();
    let dest = Path::new("/apps/golang");
    let bytes = zip_with(&[("go/bin/go.exe", "binary"), ("go/LICENSE", "mit")]);

    extract(&bytes, dest, &fs).unwrap();

    assert!(fs.exists(&dest.join("bin/go.exe")).unwrap());
    assert!(fs.exists(&dest.join("LICENSE")).unwrap());
    assert!(!fs.exists(&dest.join("go/bin/go.exe")).unwrap());
}

#[test]
fn keeps_flat_archive_untouched() {
    let fs = MemFs::new();
    let dest = Path::new("/apps/foo");
    let bytes = zip_with(&[("bin/foo.exe", "binary"), ("LICENSE", "mit")]);

    extract(&bytes, dest, &fs).unwrap();

    assert!(fs.exists(&dest.join("bin/foo.exe")).unwrap());
    assert!(fs.exists(&dest.join("LICENSE")).unwrap());
}
