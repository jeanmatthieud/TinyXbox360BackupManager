// SPDX-License-Identifier: GPL-3.0-only

//! Extraction of archives: game archives (.7z / .zip / .rar), wrapping an ISO
//! or XBLA packages, and the Toolbox components.

use crate::convert::{CONVERSION_CANCELLED, is_cancelled};
use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Why a volume of a split RAR set is refused, at pick time
/// ([`is_rar_volume`]) and again at extraction.
pub const MULTI_VOLUME_RAR: &str = "multi-volume RAR archives are not supported yet";

/// True if the extension is a supported game archive format (.7z / .zip /
/// .rar). Multi-volume RAR sets are not supported yet: see [`is_rar_volume`].
pub fn is_supported_archive(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        ext.eq_ignore_ascii_case("7z")
            || ext.eq_ignore_ascii_case("zip")
            || ext.eq_ignore_ascii_case("rar")
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
    // `Rar!\x1A\x07` opens both RAR 1.5-4 (then `\x00`) and RAR 5 (then
    // `\x01\x00`) archives; `RE~^` is RAR 1.3/1.4.
    &magic == b"7z\xBC\xAF\x27\x1C"
        || &magic[..4] == b"PK\x03\x04"
        || &magic == b"Rar!\x1A\x07"
        || &magic[..4] == b"RE~^"
}

/// True if `path` is a RAR archive that is one volume of a split set
/// (`.partN.rar`, or a `.rar` followed by `.r00`, `.r01`…). Reads only the
/// headers, so it is cheap enough to reject such a file when it is picked
/// rather than as a failed job, once per volume, after the queue started.
pub fn is_rar_volume(path: &Path) -> bool {
    // `read_path` scans up to 8 MiB for a self-extracting stub: not worth
    // doing for a file that cannot be a RAR archive anyway.
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("rar")) {
        return false;
    }
    rars::ArchiveReader::read_path(path).is_ok_and(|archive| rar_is_volume(&archive))
}

/// The main header's volume flag is set on every volume of a split set,
/// including the parts whose boundaries fall between two members: the member
/// split flags alone would let those through, and extracting one would then
/// install part of the content and report success.
fn rar_is_volume(archive: &rars::Archive) -> bool {
    let main_flag = match archive {
        rars::Archive::Rar13(a) => a.main.is_volume(),
        rars::Archive::Rar15To40(a) => a.main.is_volume(),
        rars::Archive::Rar50Plus(a) => a.main.is_volume(),
        // A family added by a later `rars`: the member flags still catch
        // most split sets.
        _ => false,
    };
    main_flag
        || archive
            .members()
            .any(|m| m.meta.is_split_before || m.meta.is_split_after)
}

/// Extracts the archive into `dest`. `progress` receives
/// (extracted bytes, total uncompressed bytes), reported per chunk so
/// large solid archives still show smooth progress.
///
/// `cancel` is polled per chunk, so a multi-gigabyte archive stops within a
/// megabyte of the request instead of running to completion.
pub fn extract_to(
    path: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    if ext.eq_ignore_ascii_case("7z") {
        extract_7z(path, dest, cancel, progress)
    } else if ext.eq_ignore_ascii_case("rar") {
        extract_rar(path, dest, cancel, progress)
    } else {
        extract_zip(path, dest, cancel, progress)
    }
}

/// Copies `reader` to `out`, adding the copied bytes to `done` chunk by
/// chunk and reporting (done, total) after each one. Bails out with
/// [`CONVERSION_CANCELLED`] as soon as `cancel` is raised.
fn copy_with_progress(
    reader: &mut dyn Read,
    out: &Path,
    done: &mut u64,
    total: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let mut out_file = File::create(out)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if is_cancelled(cancel) {
            bail!(CONVERSION_CANCELLED);
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        out_file.write_all(&buf[..n])?;
        *done += n as u64;
        progress(*done, total);
    }
}

fn extract_zip(
    path: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("reading zip archive")?;
    let total: u64 = archive
        .decompressed_size()
        .and_then(|s| u64::try_from(s).ok())
        .unwrap_or(0);
    let mut done: u64 = 0;
    progress(0, total);

    for i in 0..archive.len() {
        // Also checked between entries, so an archive of many small files
        // stops just as fast as one holding a single huge file.
        if is_cancelled(cancel) {
            bail!(CONVERSION_CANCELLED);
        }
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
            copy_with_progress(&mut entry, &out, &mut done, total, cancel, progress)
                .with_context(|| format!("extracting {}", out.display()))?;
        }
    }
    Ok(())
}

fn extract_7z(
    path: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let mut reader =
        sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty())
            .context("reading 7z archive")?;
    let total: u64 = reader.archive().files.iter().map(|e| e.size()).sum();
    let mut done: u64 = 0;
    progress(0, total);

    let result = reader.for_each_entries(|entry, entry_reader| {
        // A cancellation is surfaced as an error rather than by returning
        // `Ok(false)`: stopping the iteration silently would look like a
        // successful extraction to the caller. The message carries the
        // `CONVERSION_CANCELLED` marker the GUI matches on.
        if is_cancelled(cancel) {
            return Err(sevenz_rust2::Error::Other(CONVERSION_CANCELLED.into()));
        }
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
            copy_with_progress(entry_reader, &out, &mut done, total, cancel, progress)
                .map_err(|e| sevenz_rust2::Error::Other(format!("{e:#}").into()))?;
        }
        Ok(true)
    });
    result.context("extracting 7z archive")?;
    Ok(())
}

/// RAR 1.3 through 7, decoded by `rars`, a pure-Rust implementation under
/// Apache-2.0. The reference UnRAR source is not an option: its licence
/// forbids reusing the code to recreate the compressor, a restriction the
/// GPL does not allow on top of it.
///
/// `rars` asks for one writer per entry and takes ownership of it (a
/// `Box<dyn Write>` is `'static`), so the writer cannot borrow `cancel` or
/// `progress`. The decoding therefore runs on a scoped thread, while this one
/// relays in both directions through shared atomics: it mirrors `cancel`
/// into a flag the writer checks on every write, and reports the byte count
/// the writer keeps. That keeps a single multi-gigabyte ISO responsive, where
/// servicing both only between entries would freeze the progress bar and
/// ignore the cancel button until the very end.
///
/// One exception: a compressed RAR 5 member of up to 512 MiB is decoded
/// whole in memory before `rars` writes any of it, so for such a member (an
/// XBLA package, typically) progress and cancellation only take effect once
/// it is decoded. Lowering that limit through `ArchiveReadOptions` is not an
/// option: above it, `rars` refuses filtered members instead of streaming
/// them, which would turn a slow extraction into a failed one.
fn extract_rar(
    path: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let archive = rars::ArchiveReader::read_path(path).context("reading rar archive")?;
    // Already refused at pick time; checked again for the Toolbox, whose
    // downloads are never picked.
    if rar_is_volume(&archive) {
        bail!(MULTI_VOLUME_RAR);
    }
    let total: u64 = archive
        .members()
        .filter(|m| !m.meta.is_directory)
        .map(|m| m.meta.unpacked_size)
        .sum();
    let done = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    progress(0, total);

    let result = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            archive.extract_to(None, |meta| {
                // Surfaced as an error, like in `extract_7z`, so that stopping
                // does not look like a successful extraction. Rewritten below.
                if stop.load(Ordering::Relaxed) {
                    return Err(io::Error::other(CONVERSION_CANCELLED).into());
                }
                let name = meta.name_lossy();
                let rel = sanitized_relative_path(&name)
                    .ok_or_else(|| io::Error::other(format!("unsafe path in archive: {name}")))?;
                let out = dest.join(rel);
                if meta.is_directory {
                    std::fs::create_dir_all(&out)?;
                    return Ok(Box::new(io::sink()));
                }
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                Ok(Box::new(CountingWriter {
                    file: File::create(&out)?,
                    written: Arc::clone(&done),
                    stop: Arc::clone(&stop),
                }))
            })
        });
        while !worker.is_finished() {
            if is_cancelled(cancel) {
                stop.store(true, Ordering::Relaxed);
            }
            progress(done.load(Ordering::Relaxed), total);
            std::thread::sleep(RAR_POLL_INTERVAL);
        }
        worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    });
    // A cancellation comes back wrapped in `rars`' I/O error; report it with
    // the bare marker the callers match on instead.
    if stop.load(Ordering::Relaxed) || is_cancelled(cancel) {
        bail!(CONVERSION_CANCELLED);
    }
    result.context("extracting rar archive")?;
    progress(total, total);
    Ok(())
}

/// How often [`extract_rar`] relays progress and cancellation between the
/// decoding thread and its caller. Also the most it adds to an extraction.
const RAR_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// A file writer that adds what it writes to a counter shared with
/// [`extract_rar`], which owns no other view of the bytes going out, and
/// fails as soon as that function raises `stop`.
struct CountingWriter {
    file: File,
    written: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(io::Error::other(CONVERSION_CANCELLED));
        }
        let n = self.file.write(buf)?;
        self.written.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Rejects absolute paths and `..` components (zip-slip protection).
fn sanitized_relative_path(name: &str) -> Option<PathBuf> {
    // 7z and RAR entry names may use backslashes regardless of the host
    // platform.
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
