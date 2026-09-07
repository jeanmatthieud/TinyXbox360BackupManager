// SPDX-License-Identifier: GPL-3.0-only

//! The file operations a console-shaped target must provide.
//!
//! Two very different backends expose the same tree of `/Device/dir/file`
//! paths: an Aurora FTP server over the network ([`crate::ftp::FtpSession`])
//! and the console's own hard drive read directly through its FATX filesystem
//! ([`crate::fatx::FatxSession`]). Everything above them — the scanner, the
//! layout resolution, the Aurora database lookups, the manifest, the install
//! step — only ever needs these calls, so it is written once against this
//! trait and works on either.
//!
//! Paths are always absolute and forward-slashed, and always start with a
//! device segment (`/Hdd1/Content/0000000000000000`), exactly as Aurora's FTP
//! server presents them. The FATX backend keeps that shape by exposing its
//! partition under a single device name of its own.

use crate::ftp::{FtpSession, RemoteEntry};
use anyhow::Result;
use std::path::Path;
use std::sync::atomic::AtomicBool;

pub trait RemoteFs {
    /// Device names at the root of the target (`Hdd1`, `Usb0`, …).
    fn list_root(&mut self) -> Result<Vec<String>>;

    /// Lists a directory. An unreadable or missing directory yields an empty
    /// list rather than an error: a failed listing is never fatal to a scan.
    fn list_dir(&mut self, dir: &str) -> Vec<RemoteEntry>;

    /// Recursive size of a directory, bounded by `max_depth`.
    fn dir_size(&mut self, dir: &str, max_depth: u32) -> u64;

    /// Reads a whole file into memory.
    fn download_file(&mut self, path: &str) -> Result<Vec<u8>>;

    /// Reads at most the first `max_bytes` of a file, without pulling the rest
    /// of it. Used to read a package header out of a multi-hundred-MB file.
    fn download_prefix(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>>;

    /// SHA1 hex digest of a file's content, hashed as it streams in so the
    /// file never sits in memory as a whole. Title updates are routinely
    /// 100 MB and are only ever read to be identified by their hash.
    fn sha1_file(&mut self, path: &str) -> Result<String>;

    /// Creates a directory and every missing level above it.
    fn ensure_dir(&mut self, dir: &str) -> Result<()>;

    /// Writes an in-memory buffer as a single file, creating `dir` if needed.
    fn put_bytes(&mut self, dir: &str, file_name: &str, bytes: &[u8]) -> Result<()>;

    /// Copies a local directory tree into `dest_dir` (created if needed).
    /// `progress(sent_bytes, total_bytes, megabytes_per_second)` is called as
    /// the copy advances.
    fn upload_dir(
        &mut self,
        local_dir: &Path,
        dest_dir: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64, Option<f64>),
    ) -> Result<()>;

    /// Removes a directory and everything under it.
    /// `progress(deleted_files, total_files)` is called after each file.
    fn remove_dir_recursive(
        &mut self,
        dir: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()>;

    /// Removes a directory only if it holds nothing, reporting whether it was.
    fn remove_empty_dir(&mut self, dir: &str) -> Result<bool>;

    /// Removes a single file from a directory.
    fn remove_file(&mut self, dir: &str, file_name: &str) -> Result<()>;

    /// Memoized result of [`crate::target::find_aurora_data_dir`] for this
    /// session. Locating the Aurora install costs several round trips (or, on
    /// FATX, several directory walks) and it cannot move while the session is
    /// open, so every caller shares one lookup.
    fn aurora_data_dir_cache(&mut self) -> &mut Option<Option<String>>;
}

impl RemoteFs for FtpSession {
    fn list_root(&mut self) -> Result<Vec<String>> {
        FtpSession::list_root(self)
    }

    fn list_dir(&mut self, dir: &str) -> Vec<RemoteEntry> {
        FtpSession::list_dir(self, dir)
    }

    fn dir_size(&mut self, dir: &str, max_depth: u32) -> u64 {
        FtpSession::dir_size(self, dir, max_depth)
    }

    fn download_file(&mut self, path: &str) -> Result<Vec<u8>> {
        FtpSession::download_file(self, path)
    }

    fn download_prefix(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        FtpSession::download_prefix(self, path, max_bytes)
    }

    fn sha1_file(&mut self, path: &str) -> Result<String> {
        FtpSession::sha1_file(self, path)
    }

    fn ensure_dir(&mut self, dir: &str) -> Result<()> {
        FtpSession::ensure_dir(self, dir)
    }

    fn put_bytes(&mut self, dir: &str, file_name: &str, bytes: &[u8]) -> Result<()> {
        FtpSession::put_bytes(self, dir, file_name, bytes)
    }

    fn upload_dir(
        &mut self,
        local_dir: &Path,
        dest_dir: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64, Option<f64>),
    ) -> Result<()> {
        FtpSession::upload_dir(self, local_dir, dest_dir, cancel, progress)
    }

    fn remove_dir_recursive(
        &mut self,
        dir: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()> {
        FtpSession::remove_dir_recursive(self, dir, cancel, progress)
    }

    fn remove_empty_dir(&mut self, dir: &str) -> Result<bool> {
        FtpSession::remove_empty_dir(self, dir)
    }

    fn remove_file(&mut self, dir: &str, file_name: &str) -> Result<()> {
        FtpSession::remove_file(self, dir, file_name)
    }

    fn aurora_data_dir_cache(&mut self) -> &mut Option<Option<String>> {
        &mut self.aurora_data_dir_cache
    }
}

/// An open session on a console-shaped target, whichever way it is reached.
///
/// Callers that work on "the console" — the scanner, the deletion, the
/// install step — hold one of these rather than a concrete session, so a
/// single code path serves both the network and a hard drive on the desk.
pub enum RemoteSession {
    Ftp(FtpSession),
    Fatx(crate::fatx::FatxSession),
}

impl RemoteSession {
    fn inner(&mut self) -> &mut dyn RemoteFs {
        match self {
            RemoteSession::Ftp(session) => session,
            RemoteSession::Fatx(session) => session,
        }
    }

    /// Closes the session: an FTP `QUIT`, or a final flush of the filesystem.
    pub fn quit(self) -> Result<()> {
        match self {
            RemoteSession::Ftp(session) => {
                session.quit();
                Ok(())
            }
            // Unlike a `QUIT`, this one can fail in a way that matters: it is
            // the last chance for anything still in memory to reach the disk.
            RemoteSession::Fatx(session) => session.quit(),
        }
    }
}

impl RemoteFs for RemoteSession {
    fn list_root(&mut self) -> Result<Vec<String>> {
        self.inner().list_root()
    }

    fn list_dir(&mut self, dir: &str) -> Vec<RemoteEntry> {
        self.inner().list_dir(dir)
    }

    fn dir_size(&mut self, dir: &str, max_depth: u32) -> u64 {
        self.inner().dir_size(dir, max_depth)
    }

    fn download_file(&mut self, path: &str) -> Result<Vec<u8>> {
        self.inner().download_file(path)
    }

    fn download_prefix(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        self.inner().download_prefix(path, max_bytes)
    }

    fn sha1_file(&mut self, path: &str) -> Result<String> {
        self.inner().sha1_file(path)
    }

    fn ensure_dir(&mut self, dir: &str) -> Result<()> {
        self.inner().ensure_dir(dir)
    }

    fn put_bytes(&mut self, dir: &str, file_name: &str, bytes: &[u8]) -> Result<()> {
        self.inner().put_bytes(dir, file_name, bytes)
    }

    fn upload_dir(
        &mut self,
        local_dir: &Path,
        dest_dir: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64, Option<f64>),
    ) -> Result<()> {
        self.inner().upload_dir(local_dir, dest_dir, cancel, progress)
    }

    fn remove_dir_recursive(
        &mut self,
        dir: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()> {
        self.inner().remove_dir_recursive(dir, cancel, progress)
    }

    fn remove_empty_dir(&mut self, dir: &str) -> Result<bool> {
        self.inner().remove_empty_dir(dir)
    }

    fn remove_file(&mut self, dir: &str, file_name: &str) -> Result<()> {
        self.inner().remove_file(dir, file_name)
    }

    fn aurora_data_dir_cache(&mut self) -> &mut Option<Option<String>> {
        self.inner().aurora_data_dir_cache()
    }
}
