// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, anyhow, bail};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use xdvdfs::blockdev::OffsetWrapper;

/// Extracts all content from an XISO image (original Xbox or Xbox 360)
/// to `dest_dir`. Pure Rust equivalent of extract-xiso.
///
/// `progress(done, total)` is called at each extracted file.
pub fn extract_iso(
    source_iso: &Path,
    dest_dir: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let file = File::open(source_iso)
        .with_context(|| format!("opening {}", source_iso.display()))?;
    let mut dev = OffsetWrapper::new(BufReader::new(file))
        .map_err(|e| anyhow!("invalid XDVDFS image: {e}"))?;

    let volume = xdvdfs::read::read_volume(&mut dev)
        .map_err(|e| anyhow!("reading XDVDFS volume: {e}"))?;

    let tree = volume
        .root_table
        .file_tree(&mut dev)
        .map_err(|e| anyhow!("reading file tree: {e}"))?;

    let total = tree
        .iter()
        .filter(|(_, node)| !node.node.dirent.is_directory())
        .count() as u64;
    let mut done: u64 = 0;
    progress(0, total);

    for (dir, node) in &tree {
        let name = node
            .name_str::<std::io::Error>()
            .map_err(|e| anyhow!("invalid file name: {e}"))?;
        let relative = format!("{}/{}", dir.trim_start_matches('/'), name);
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
            let copied = std::io::copy(&mut dev.get_mut().by_ref().take(size), &mut out)
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
