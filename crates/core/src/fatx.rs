// SPDX-License-Identifier: GPL-3.0-only

//! The console's own hard drive, read and written directly through its FATX
//! filesystem.
//!
//! An Xbox 360 hard drive carries no partition table and no filesystem any
//! desktop OS can mount: its partitions sit at fixed offsets and hold FATX, a
//! filesystem of Microsoft's own. So the disk is opened as a raw block device
//! and driven by the `fatx` crate rather than through the OS.
//!
//! Only the `data` partition matters here — the one the console calls `Hdd1`
//! and where every game, DLC and Aurora install lives. The session therefore
//! presents that partition as a single device named `Hdd1` at the root of a
//! `/Device/dir/file` tree, which is exactly the shape Aurora's FTP server
//! serves. That is what lets the scanner, the layout resolution and the
//! install step be written once (see [`crate::remote_fs`]) and run unchanged
//! over the network or over SATA.

use crate::ftp::RemoteEntry;
use crate::remote_fs::RemoteFs;
use anyhow::{Context, Result, anyhow, bail};
use fatx::{FatxFs, FatxFsConfig, FatxFsHandle};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Device name the FATX partition is exposed under. The console's internal
/// drive is `Hdd1` to Aurora, and its databases store scan paths as
/// `Hdd1:\Content\…`, so using the same name here makes those paths resolve
/// without a special case.
pub const FATX_VOLUME: &str = "Hdd1";

/// Xbox 360 partition holding user content (`Hdd1`).
pub const DEFAULT_PARTITION: &str = "data";

/// How often a long file copy reports its progress.
const PROGRESS_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(200);

/// Copy buffer. Each `write` on a FATX file rewrites the file's directory
/// entry, so copying in large chunks matters: a small buffer would spend most
/// of the transfer seeking back to the parent directory.
const COPY_BUFFER: usize = 1 << 20;

/// A FATX target: one partition of one raw device (or of a disk image).
#[derive(Debug, Clone, Serialize, Deserialize)]
// A config written before a field existed still loads, with that field at its
// default — the app must never refuse to start over its own settings file.
#[serde(default)]
pub struct FatxConfig {
    /// Block device (`/dev/sdb`, `\\.\PhysicalDrive1`, `/dev/rdisk3`) or the
    /// path of a raw disk image.
    pub device: PathBuf,
    /// Name of the Xbox 360 partition to open, per the `fatx` crate's
    /// partition map. Effectively always [`DEFAULT_PARTITION`].
    pub partition: String,
}

impl Default for FatxConfig {
    fn default() -> Self {
        Self {
            device: PathBuf::new(),
            partition: DEFAULT_PARTITION.to_string(),
        }
    }
}

impl FatxConfig {
    pub fn new(device: PathBuf) -> Self {
        Self {
            device,
            ..Default::default()
        }
    }

    /// Partition name to open, falling back to the user-content partition for
    /// a config written before the field existed (or emptied by hand).
    fn partition_name(&self) -> &str {
        let name = self.partition.trim();
        if name.is_empty() {
            DEFAULT_PARTITION
        } else {
            name
        }
    }
}

/// An open FATX filesystem, presented as a console-shaped `/Hdd1/…` tree.
pub struct FatxSession {
    fs: FatxFsHandle,
    writable: bool,
    aurora_data_dir_cache: Option<Option<String>>,
}

impl FatxSession {
    /// Opens the target's partition. `writable` must be false for anything
    /// that only reads: a read-only session opens the device without write
    /// access at all, so a bug here cannot damage the user's games.
    pub fn open(config: &FatxConfig, writable: bool) -> Result<Self> {
        if config.device.as_os_str().is_empty() {
            bail!("no FATX device selected");
        }
        if fatx::PartitionMapEntry::from_x360_name(config.partition_name()).is_none() {
            bail!("unknown Xbox 360 partition '{}'", config.partition_name());
        }

        let fs_config = FatxFsConfig::new(config.device.to_string_lossy().to_string())
            .x360_partition(config.partition_name())
            .variant(fatx::Variant::X360)
            .writable(writable);

        let fs = FatxFs::open_device(&fs_config).map_err(|e| open_error(config, writable, e))?;

        Ok(Self {
            fs,
            writable,
            aurora_data_dir_cache: None,
        })
    }

    /// Total and free space of the partition.
    pub fn space(&mut self) -> Result<fatx::Space> {
        self.fs.space().context("reading FATX free space")
    }

    /// Flushes everything still held in memory (the FAT above all) to the
    /// device. Called at the end of every write; a session that is only read
    /// has nothing to flush.
    pub fn sync(&mut self) -> Result<()> {
        if !self.writable {
            return Ok(());
        }
        self.fs.sync().context("flushing the FATX filesystem")
    }

    /// Closes the filesystem, mirroring [`crate::ftp::FtpSession::quit`].
    /// Each write already flushed on its way out, so this is the belt to their
    /// braces — but it is still reported, since a failure here means something
    /// the user was told was written is not on the disk.
    pub fn quit(mut self) -> Result<()> {
        self.sync()
    }

    /// Maps a console path (`/Hdd1/Content/…`) to a path inside the open
    /// partition (`/Content/…`). Returns `None` for a path on another device,
    /// which this session simply does not have.
    fn resolve(&self, path: &str) -> Option<String> {
        let normalized = path.replace('\\', "/");
        let trimmed = normalized.trim_matches('/');
        if trimmed.is_empty() {
            return None;
        }
        let (device, rest) = match trimmed.split_once('/') {
            Some((device, rest)) => (device, rest),
            None => (trimmed, ""),
        };
        if !device.eq_ignore_ascii_case(FATX_VOLUME) {
            return None;
        }
        // Collapse the empty segments a caller's `format!("{dir}/{name}")` can
        // leave behind; FATX itself has no notion of them.
        let rest: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        Some(format!("/{}", rest.join("/")))
    }

    /// Whether a console path names the volume a FATX session exposes. The
    /// manifest a console writes is shared between the FTP and the FATX
    /// target, and it may well point at a folder on another device
    /// (`/Usb0/Content/…`) — which this connection simply does not have.
    pub fn path_is_on_volume(path: &str) -> bool {
        let normalized = path.replace('\\', "/");
        let device = normalized.trim_matches('/').split('/').next().unwrap_or("");
        device.eq_ignore_ascii_case(FATX_VOLUME)
    }

    fn resolve_or_err(&self, path: &str) -> Result<String> {
        self.resolve(path)
            .ok_or_else(|| anyhow!("{path} is not on the connected FATX drive"))
    }

    /// Whether a path exists, and whether it is a directory.
    fn stat(&mut self, path: &str) -> Option<fatx::DirectoryEntry> {
        let resolved = self.resolve(path)?;
        self.fs.stat(&resolved).ok()
    }

    pub fn is_dir(&mut self, path: &str) -> bool {
        self.stat(path).is_some_and(|e| e.is_directory())
    }

    /// Creates one directory, tolerating a directory that already exists.
    fn mkdir_one(&mut self, resolved: &str) -> Result<()> {
        match self.fs.stat(resolved) {
            Ok(entry) if entry.is_directory() => return Ok(()),
            Ok(_) => bail!("{resolved} already exists as a file"),
            Err(_) => {}
        }
        self.fs
            .mkdir(resolved)
            .with_context(|| format!("creating {resolved}"))
    }

    /// Opens a file for writing, replacing whatever was there. FATX has no
    /// create-or-truncate call, so an existing file is emptied first and then
    /// reopened — which also releases its clusters before the new content
    /// claims any.
    fn create_file(&mut self, resolved: &str) -> Result<fatx::File> {
        self.ensure_writable()?;
        match self.fs.stat(resolved) {
            Ok(entry) if entry.is_directory() => bail!("{resolved} is a directory"),
            Ok(_) => {
                self.fs
                    .truncate(resolved, 0)
                    .with_context(|| format!("truncating {resolved}"))?;
                self.fs
                    .open(resolved)
                    .with_context(|| format!("opening {resolved}"))
            }
            Err(_) => self
                .fs
                .create(resolved)
                .with_context(|| format!("creating {resolved}")),
        }
    }

    fn ensure_writable(&self) -> Result<()> {
        if self.writable {
            Ok(())
        } else {
            bail!("this FATX drive is open read-only")
        }
    }

    /// Recursive file count, used to scale deletion progress.
    fn count_files(&mut self, dir: &str) -> u64 {
        let mut count = 0;
        for entry in self.list_dir(dir) {
            if entry.is_dir {
                count += self.count_files(&format!("{dir}/{}", entry.name));
            } else {
                count += 1;
            }
        }
        count
    }

    fn remove_dir_inner(
        &mut self,
        dir: &str,
        done: &mut u64,
        total: u64,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()> {
        for entry in self.list_dir(dir) {
            if cancel.load(Ordering::Relaxed) {
                bail!(crate::target::DELETION_CANCELLED);
            }

            let child = format!("{dir}/{}", entry.name);
            if entry.is_dir {
                self.remove_dir_inner(&child, done, total, cancel, progress)?;
            } else {
                let resolved = self.resolve_or_err(&child)?;
                self.fs
                    .unlink(&resolved)
                    .with_context(|| format!("removing {child}"))?;
                *done += 1;
                progress(*done, total);
            }
        }

        let resolved = self.resolve_or_err(dir)?;
        self.fs
            .rmdir(&resolved)
            .with_context(|| format!("removing {dir}"))
    }

    /// Copies one local file into the open filesystem, reporting progress as
    /// it goes. `base` is how many bytes of the whole copy came before it.
    fn copy_file(
        &mut self,
        local_path: &Path,
        dest_path: &str,
        cancel: &AtomicBool,
        base: u64,
        total: u64,
        progress: &mut dyn FnMut(u64, u64, Option<f64>),
    ) -> Result<u64> {
        let resolved = self.resolve_or_err(dest_path)?;
        let mut source = std::fs::File::open(local_path)
            .with_context(|| format!("opening {}", local_path.display()))?;
        let mut dest = self.create_file(&resolved)?;

        let mut buf = vec![0u8; COPY_BUFFER];
        let mut copied: u64 = 0;
        let started = Instant::now();
        let mut last_notify = started;

        loop {
            if cancel.load(Ordering::Relaxed) {
                bail!(crate::convert::CONVERSION_CANCELLED);
            }
            let read = source
                .read(&mut buf)
                .with_context(|| format!("reading {}", local_path.display()))?;
            if read == 0 {
                break;
            }
            dest.write_all(&buf[..read])
                .with_context(|| format!("writing {dest_path}"))?;
            copied += read as u64;

            let now = Instant::now();
            if now.duration_since(last_notify) >= PROGRESS_DEBOUNCE {
                last_notify = now;
                let secs = started.elapsed().as_secs_f64();
                let speed = (secs > 0.0).then(|| copied as f64 / 1e6 / secs);
                progress(base + copied, total, speed);
            }
        }

        // Pushes the file's own entry and the FAT out to the device: a copy
        // that is reported as done must actually be on the disk, since the
        // user may unplug it as soon as the job ends.
        dest.flush().with_context(|| format!("flushing {dest_path}"))?;

        let secs = started.elapsed().as_secs_f64();
        let speed = (secs > 0.0).then(|| copied as f64 / 1e6 / secs);
        progress(base + copied, total, speed);
        Ok(copied)
    }

    fn copy_dir_inner(
        &mut self,
        local_dir: &Path,
        dest_dir: &str,
        cancel: &AtomicBool,
        sent: &mut u64,
        total: u64,
        progress: &mut dyn FnMut(u64, u64, Option<f64>),
    ) -> Result<()> {
        self.ensure_dir(dest_dir)?;

        let entries = std::fs::read_dir(local_dir)
            .with_context(|| format!("reading {}", local_dir.display()))?;
        for entry in entries.flatten() {
            if cancel.load(Ordering::Relaxed) {
                bail!(crate::convert::CONVERSION_CANCELLED);
            }

            let local_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let dest_path = format!("{}/{name}", dest_dir.trim_end_matches('/'));

            if local_path.is_dir() {
                self.copy_dir_inner(&local_path, &dest_path, cancel, sent, total, progress)?;
            } else {
                let base = *sent;
                let copied =
                    self.copy_file(&local_path, &dest_path, cancel, base, total, progress)?;
                *sent = base + copied;
            }
        }
        Ok(())
    }
}

/// Turns an open failure into something the user can act on. A raw block
/// device is only readable by root (or by the `disk`/`operator` group), which
/// is by far the most common reason this fails, and the error the OS returns
/// on its own ("permission denied") says nothing about what to do next.
fn open_error(config: &FatxConfig, writable: bool, error: fatx::Error) -> anyhow::Error {
    let device = config.device.display();
    if let fatx::Error::Io(io) = &error {
        match io.kind() {
            std::io::ErrorKind::PermissionDenied => {
                return anyhow!(
                    "no permission to open {device}{}.\n\n\
                     Raw disk access is reserved to the administrator. On Linux, add \
                     yourself to the `disk` group (`sudo usermod -aG disk $USER`, then log \
                     out and back in) or start the application with `sudo`. On Windows, run \
                     it as administrator.",
                    if writable { " for writing" } else { "" }
                );
            }
            std::io::ErrorKind::NotFound => {
                return anyhow!("{device} is no longer connected");
            }
            _ => {}
        }
    }
    if matches!(error, fatx::Error::InvalidFilesystemSignature) {
        return anyhow!(
            "no Xbox 360 filesystem found on {device}: its `{}` partition holds no FATX \
             filesystem. Make sure this really is an Xbox 360 hard drive, and that it is \
             the whole disk rather than one of its partitions.",
            config.partition_name()
        );
    }
    anyhow::Error::new(error).context(format!("opening {device}"))
}

impl RemoteFs for FatxSession {
    fn list_root(&mut self) -> Result<Vec<String>> {
        Ok(vec![FATX_VOLUME.to_string()])
    }

    fn list_dir(&mut self, dir: &str) -> Vec<RemoteEntry> {
        let Some(resolved) = self.resolve(dir) else {
            return Vec::new();
        };
        let Ok(entries) = self.fs.read_dir(&resolved) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|e| RemoteEntry {
                name: e.file_name(),
                is_dir: e.is_directory(),
                size: u64::from(e.file_size()),
            })
            .collect()
    }

    fn dir_size(&mut self, dir: &str, max_depth: u32) -> u64 {
        let mut total = 0;
        for entry in self.list_dir(dir) {
            if entry.is_dir {
                if max_depth > 0 {
                    total += self.dir_size(&format!("{dir}/{}", entry.name), max_depth - 1);
                }
            } else {
                total += entry.size;
            }
        }
        total
    }

    fn download_file(&mut self, path: &str) -> Result<Vec<u8>> {
        let resolved = self.resolve_or_err(path)?;
        let mut file = self
            .fs
            .open(&resolved)
            .with_context(|| format!("opening {path}"))?;
        let mut out = Vec::with_capacity(file.file_size() as usize);
        file.read_to_end(&mut out)
            .with_context(|| format!("reading {path}"))?;
        Ok(out)
    }

    fn download_prefix(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let resolved = self.resolve_or_err(path)?;
        let file = self
            .fs
            .open(&resolved)
            .with_context(|| format!("opening {path}"))?;
        let mut out = Vec::new();
        file.take(max_bytes as u64)
            .read_to_end(&mut out)
            .with_context(|| format!("reading {path}"))?;
        Ok(out)
    }

    fn ensure_dir(&mut self, dir: &str) -> Result<()> {
        self.ensure_writable()?;
        let resolved = self.resolve_or_err(dir)?;
        let mut current = String::new();
        for part in resolved.split('/').filter(|s| !s.is_empty()) {
            current.push('/');
            current.push_str(part);
            self.mkdir_one(&current)?;
        }
        Ok(())
    }

    fn put_bytes(&mut self, dir: &str, file_name: &str, bytes: &[u8]) -> Result<()> {
        self.ensure_dir(dir)?;
        let path = format!("{}/{file_name}", dir.trim_end_matches('/'));
        let resolved = self.resolve_or_err(&path)?;
        let mut file = self.create_file(&resolved)?;
        file.write_all(bytes)
            .with_context(|| format!("writing {path}"))?;
        file.flush().with_context(|| format!("flushing {path}"))?;
        Ok(())
    }

    fn upload_dir(
        &mut self,
        local_dir: &Path,
        dest_dir: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64, Option<f64>),
    ) -> Result<()> {
        self.ensure_writable()?;
        let total = crate::util::dir_size(local_dir);
        let mut sent: u64 = 0;
        progress(0, total, None);
        let result = self.copy_dir_inner(local_dir, dest_dir, cancel, &mut sent, total, progress);
        // Whether the copy finished or was interrupted, what did reach the
        // disk must be described correctly on it.
        self.sync()?;
        result
    }

    fn remove_dir_recursive(
        &mut self,
        dir: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()> {
        self.ensure_writable()?;
        let total = self.count_files(dir);
        let mut done: u64 = 0;
        progress(0, total);
        let result = self.remove_dir_inner(dir, &mut done, total, cancel, progress);
        self.sync()?;
        result
    }

    fn remove_empty_dir(&mut self, dir: &str) -> Result<bool> {
        self.ensure_writable()?;
        if !self.list_dir(dir).is_empty() {
            return Ok(false);
        }
        let resolved = self.resolve_or_err(dir)?;
        self.fs
            .rmdir(&resolved)
            .with_context(|| format!("removing {dir}"))?;
        self.sync()?;
        Ok(true)
    }

    fn remove_file(&mut self, dir: &str, file_name: &str) -> Result<()> {
        self.ensure_writable()?;
        let path = format!("{}/{file_name}", dir.trim_end_matches('/'));
        let resolved = self.resolve_or_err(&path)?;
        self.fs
            .unlink(&resolved)
            .with_context(|| format!("removing {path}"))?;
        self.sync()
    }

    fn aurora_data_dir_cache(&mut self) -> &mut Option<Option<String>> {
        &mut self.aurora_data_dir_cache
    }
}
