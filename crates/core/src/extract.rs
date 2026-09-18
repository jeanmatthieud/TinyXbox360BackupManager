// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, anyhow, bail};
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use xdvdfs::blockdev::OffsetWrapper;
use xdvdfs::layout::DirectoryEntryNode;

/// Extracts all content from an XISO image (original Xbox or Xbox 360)
/// to `dest_dir`. Pure Rust equivalent of extract-xiso.
///
/// `progress(done, total)` is called at each extracted file.
pub fn extract_iso(
    source_iso: &Path,
    dest_dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    extract_filtered(source_iso, dest_dir, None, &[], false, cancel, progress)
}

/// Extracts an image except for the named root entries.
///
/// Used when something else installs part of the disc to its proper place, and
/// leaving that part in the extracted folder too would only duplicate it.
pub fn extract_iso_excluding(
    source_iso: &Path,
    dest_dir: &Path,
    exclude: &[&str],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    extract_filtered(source_iso, dest_dir, None, exclude, false, cancel, progress)
}

/// Extracts only the named root entries of an image, leaving everything else
/// behind. Names are matched case-insensitively, and both files and folders
/// qualify.
///
/// This exists so one disc of a set can contribute a couple of folders to
/// another disc's extracted game without also dropping its own `default.xex`
/// on top of it — see [`crate::quirks::DiscQuirk::ContributesFolders`].
/// Extracts what the named root folders *contain* straight into `dest_dir`,
/// dropping the folder level itself: `installation1/foo/bar.dat` is written as
/// `foo/bar.dat`. That is what an installation disc means by "copy the contents
/// of these folders into the game" — the folders are a wrapper, not part of the
/// layout the game expects.
///
/// Every named root must exist and hold at least one file, or this fails: a
/// caller asking for specific folders has been promised they are there, and
/// silently extracting nothing would report success over an empty folder.
pub fn extract_iso_subtree_contents(
    source_iso: &Path,
    dest_dir: &Path,
    roots: &[&str],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    extract_filtered(source_iso, dest_dir, Some(roots), &[], true, cancel, progress)
}

fn extract_filtered(
    source_iso: &Path,
    dest_dir: &Path,
    roots: Option<&[&str]>,
    exclude: &[&str],
    strip_root: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let file = File::open(source_iso)
        .with_context(|| format!("opening {}", source_iso.display()))?;
    let mut dev = OffsetWrapper::new(BufReader::new(file))
        .map_err(|e| anyhow!("invalid XDVDFS image: {e}"))?;

    let volume = xdvdfs::read::read_volume(&mut dev)
        .map_err(|e| anyhow!("reading XDVDFS volume: {e}"))?;

    let mut tree = volume
        .root_table
        .file_tree(&mut dev)
        .map_err(|e| anyhow!("reading file tree: {e}"))?;

    // Filtering here rather than inside the loop keeps `total` honest: the
    // progress must count what will be written, not what the image holds. An
    // entry belongs to a subtree when it *is* one of the named roots, or when
    // it sits under one.
    let subtree_of = |dir: &str, node: &DirectoryEntryNode, names: &[&str]| {
        let first = dir.trim_start_matches('/').split('/').next().unwrap_or("");
        if !first.is_empty() {
            return names.iter().any(|n| n.eq_ignore_ascii_case(first));
        }
        node.name_str::<std::io::Error>()
            .is_ok_and(|name| names.iter().any(|n| n.eq_ignore_ascii_case(&name)))
    };

    if !exclude.is_empty() {
        tree.retain(|(dir, node)| !subtree_of(dir, node, exclude));
    }

    if let Some(roots) = roots {
        tree.retain(|(dir, node)| subtree_of(dir, node, roots));

        // A root that holds no file means the image is not shaped the way the
        // caller was told. Returning `Ok` here would report success over a
        // folder nothing was written to, so this fails the way
        // `convert::install_bundled_content` does on a bonus disc that turns
        // out to carry no package.
        let missing: Vec<&str> = roots
            .iter()
            .copied()
            .filter(|root| {
                !tree.iter().any(|(dir, node)| {
                    !node.node.dirent.is_directory() && subtree_of(dir, node, &[root])
                })
            })
            .collect();
        if !missing.is_empty() {
            bail!(
                "expected folder(s) {} hold no file in {}",
                missing.join(", "),
                source_iso.display()
            );
        }
    }

    let total = tree
        .iter()
        .filter(|(_, node)| !node.node.dirent.is_directory())
        .count() as u64;
    let mut done: u64 = 0;
    progress(0, total);

    for (dir, node) in &tree {
        if crate::convert::is_cancelled(cancel) {
            bail!(crate::convert::CONVERSION_CANCELLED);
        }
        let name = node
            .name_str::<std::io::Error>()
            .map_err(|e| anyhow!("invalid file name: {e}"))?;
        let relative = format!("{}/{}", dir.trim_start_matches('/'), name);
        let relative = if strip_root {
            match relative.trim_start_matches('/').split_once('/') {
                Some((_, under_root)) => under_root.to_string(),
                // The named root folder itself: only what is inside it is
                // wanted, so it contributes no directory of its own.
                None => continue,
            }
        } else {
            relative
        };
        let target = join_secure(dest_dir, &relative)?;

        if node.node.dirent.is_directory() {
            fs::create_dir_all(&target)
                .with_context(|| format!("creating {}", target.display()))?;
            continue;
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        let size = node.node.dirent.data.size() as u64;
        let mut out = File::create(&target)
            .with_context(|| format!("creating {}", target.display()))?;

        if size > 0 {
            node.node
                .dirent
                .seek_to(&mut dev)
                .map_err(|e| anyhow!("seeking in image: {e}"))?;
            let copied = copy_cancellable(dev.get_mut(), &mut out, size, cancel)
                .with_context(|| format!("extracting {relative}"))?;
            if copied != size {
                bail!("incomplete extraction of {relative} ({copied}/{size} bytes)");
            }
        }

        done += 1;
        progress(done, total);
    }

    Ok(())
}

/// Chunk size used to stream a file out of the image, and thus the granularity
/// at which a cancellation is noticed. A single file can be several gigabytes,
/// so waiting for it to finish is not an option.
const COPY_CHUNK: usize = 1 << 20;

/// Copies exactly `size` bytes from `reader` to `writer`, bailing out as soon as
/// `cancel` is raised. Returns the number of bytes copied, which is short of
/// `size` only if the reader hit EOF early.
fn copy_cancellable(
    reader: &mut impl Read,
    writer: &mut impl Write,
    size: u64,
    cancel: &AtomicBool,
) -> Result<u64> {
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut copied: u64 = 0;

    while copied < size {
        if crate::convert::is_cancelled(cancel) {
            bail!(crate::convert::CONVERSION_CANCELLED);
        }
        let want = (COPY_CHUNK as u64).min(size - copied) as usize;
        let read = reader.read(&mut buf[..want])?;
        if read == 0 {
            break;
        }
        writer.write_all(&buf[..read])?;
        copied += read as u64;
    }

    Ok(copied)
}

/// Joins a relative path from the image, refusing any traversal
/// outside the destination folder.
fn join_secure(base: &Path, relative: &str) -> Result<std::path::PathBuf> {
    let mut path = base.to_path_buf();
    for part in relative.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(['\\', ':']) {
            bail!("suspicious path in image: {relative}");
        }
        path.push(part);
    }
    Ok(path)
}
