// SPDX-License-Identifier: GPL-3.0-only

//! Extraction of game archives (.7z / .zip), used for XBLA packages.

use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// True if the extension is a supported archive format (.7z / .zip).
pub fn is_supported_archive(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        ext.eq_ignore_ascii_case("7z") || ext.eq_ignore_ascii_case("zip")
    })
}

/// Cheap validity check on the file magic.
pub fn looks_valid(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 6];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    &magic == b"7z\xBC\xAF\x27\x1C" || &magic[..4] == b"PK\x03\x04"
}

/// Extracts the archive into `dest`. `progress` receives
/// (extracted bytes, total uncompressed bytes), reported per chunk so
/// large solid archives still show smooth progress.
pub fn extract_to(
    path: &Path,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let is_7z = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("7z"));
    if is_7z {
        extract_7z(path, dest, progress)
    } else {
        extract_zip(path, dest, progress)
    }
}

/// Copies `reader` to `out`, adding the copied bytes to `done` chunk by
/// chunk and reporting (done, total) after each one.
fn copy_with_progress(
    reader: &mut dyn Read,
    out: &Path,
    done: &mut u64,
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> std::io::Result<()> {
    let mut out_file = File::create(out)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        std::io::Write::write_all(&mut out_file, &buf[..n])?;
        *done += n as u64;
        progress(*done, total);
    }
}

fn extract_zip(path: &Path, dest: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("reading zip archive")?;
    let total: u64 = archive
        .decompressed_size()
        .and_then(|s| u64::try_from(s).ok())
        .unwrap_or(0);
    let mut done: u64 = 0;
    progress(0, total);

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).context("reading zip entry")?;
        // enclosed_name rejects absolute paths and `..` traversal.
        let Some(rel) = entry.enclosed_name() else {
            bail!("unsafe path in archive: {}", entry.name());
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            copy_with_progress(&mut entry, &out, &mut done, total, progress)
                .with_context(|| format!("extracting {}", out.display()))?;
        }
    }
    Ok(())
}

fn extract_7z(path: &Path, dest: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    let mut reader =
        sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty())
            .context("reading 7z archive")?;
    let total: u64 = reader.archive().files.iter().map(|e| e.size()).sum();
    let mut done: u64 = 0;
    progress(0, total);

    let result = reader.for_each_entries(|entry, entry_reader| {
        let rel = sanitized_relative_path(entry.name()).ok_or_else(|| {
            sevenz_rust2::Error::Other(format!("unsafe path in archive: {}", entry.name()).into())
        })?;
        let out = dest.join(rel);
        if entry.is_directory() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            copy_with_progress(entry_reader, &out, &mut done, total, progress)?;
        }
        Ok(true)
    });
    result.context("extracting 7z archive")?;
    Ok(())
}

/// Rejects absolute paths and `..` components (zip-slip protection).
fn sanitized_relative_path(name: &str) -> Option<PathBuf> {
    // 7z entry names may use backslashes regardless of the host platform.
    let name = name.replace('\\', "/");
    let path = Path::new(&name);
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}
