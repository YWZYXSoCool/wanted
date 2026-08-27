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

/// Build an in-memory tar.xz from the given `(path, content)` entries.
fn tar_xz_with(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, *name, content.as_bytes())
                .unwrap();
        }
        builder.finish().unwrap();
    }
    let mut xz = xz2::write::XzEncoder::new(Vec::new(), 6);
    xz.write_all(&tar_bytes).unwrap();
    xz.finish().unwrap()
}

#[test]
fn extracts_tar_xz() {
    let fs = MemFs::new();
    let dest = Path::new("/apps/llvm");
    let bytes = tar_xz_with(&[("llvm/bin/clang.exe", "clang"), ("llvm/LICENSE", "apache")]);

    extract(&bytes, dest, &fs).unwrap();

    assert!(fs.exists(&dest.join("bin/clang.exe")).unwrap());
    assert!(fs.exists(&dest.join("LICENSE")).unwrap());
    assert!(!fs.exists(&dest.join("llvm/bin/clang.exe")).unwrap());
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
