//! Archive extraction.
//!
//! Does not depend on the real disk layout: it first reads bytes, parses them
//! into `[(relative_path, content)]`, then the caller writes via
//! [`Fs`](crate::engine::fs::Fs). All paths are sanitized to prevent traversal.

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use crate::Result;
use crate::engine::fs::Fs;

/// Extract bytes to the `dest` directory. If the archive has one uniform
/// leading root directory (e.g. Go's official `go/` package), it is stripped so
/// the contents land directly under `dest`.
pub fn extract(bytes: &[u8], dest: &Path, fs: &dyn Fs) -> Result<()> {
    fs.create_dir_all(dest)?;
    let mut entries = archive_entries(bytes)?;
    strip_leading_dir(&mut entries);
    for (rel, data) in entries {
        if rel.as_os_str().is_empty() || data.is_empty() {
            continue;
        }
        let target = dest.join(&rel);
        if let Some(parent) = target.parent() {
            fs.create_dir_all(parent)?;
        }
        fs.write(&target, &data)?;
    }
    Ok(())
}

/// Parse the archive into a `(relative_path, content)` list; gzip is treated as
/// tar.gz, xz as tar.xz, anything else as a zip.
fn archive_entries(bytes: &[u8]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        tar_gz_entries(bytes)
    } else if bytes.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        tar_xz_entries(bytes)
    } else if bytes.starts_with(b"PK\x03\x04") {
        zip_entries(bytes)
    } else {
        Err(crate::Error::Archive("unknown archive magic".into()))
    }
}

fn zip_entries(bytes: &[u8]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let reader = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| crate::Error::Archive(e.to_string()))?;
    let mut out = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| crate::Error::Archive(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        push_entry(&mut out, &name, &mut entry)?;
    }
    Ok(out)
}

fn tar_gz_entries(bytes: &[u8]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    tar_entries(decoder)
}

fn tar_xz_entries(bytes: &[u8]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let decoder = xz2::read::XzDecoder::new(bytes);
    tar_entries(decoder)
}

fn tar_entries<R: std::io::Read>(decoder: R) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut archive = tar::Archive::new(decoder);
    let mut out = Vec::new();
    for entry in archive
        .entries()
        .map_err(|e| crate::Error::Archive(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| crate::Error::Archive(e.to_string()))?;
        let name = entry
            .path()
            .map_err(|e| crate::Error::Archive(e.to_string()))?
            .to_string_lossy()
            .into_owned();
        push_entry(&mut out, &name, &mut entry)?;
    }
    Ok(out)
}

/// Append one archive entry to the result, skipping paths sanitized to `None`.
fn push_entry<R: Read>(
    out: &mut Vec<(PathBuf, Vec<u8>)>,
    name: &str,
    reader: &mut R,
) -> Result<()> {
    let Some(rel) = sanitize_rel(name) else {
        return Ok(());
    };
    let mut data = Vec::new();
    reader
        .read_to_end(&mut data)
        .map_err(|e| crate::Error::Archive(e.to_string()))?;
    out.push((rel, data));
    Ok(())
}

/// Sanitize an archive-relative path; `..`, roots, and drive letters return `None`.
fn sanitize_rel(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for comp in Path::new(name).components() {
        match comp {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Strip one leading directory when all entries share it; keep flat archives
/// untouched so nothing is mangled.
fn strip_leading_dir(entries: &mut [(PathBuf, Vec<u8>)]) {
    let roots: Vec<Component<'_>> = entries
        .iter()
        .filter_map(|(rel, _)| rel.components().next())
        .collect();
    let Some(first) = roots.first() else {
        return;
    };
    if !roots.iter().all(|root| root == first) {
        return;
    }
    for (rel, _) in entries.iter_mut() {
        let stripped: PathBuf = rel.components().skip(1).collect();
        *rel = stripped;
    }
}

#[cfg(test)]
mod tests;
