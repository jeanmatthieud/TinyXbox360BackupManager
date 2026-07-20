// SPDX-License-Identifier: GPL-3.0-only

//! FTP transfer to the console (Aurora FTP server).
//!
//! Specificities of the console server:
//! - only one connection at a time (no multiple streams);
//! - path arguments of commands (LIST, STOR, DELE...) are poorly handled:
//!   must navigate with CWD then only use relative names;
//! - NLST returns complete LIST lines.

use crate::util::dir_size;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::{Duration, Instant};
use suppaftp::FtpStream;
use suppaftp::types::FileType;

/// Minimum delay between two intra-file progress notifications, so the UI
/// isn't refreshed on every 8 KiB chunk.
const PROGRESS_DEBOUNCE: Duration = Duration::from_millis(200);

/// Wraps a reader and reports the running byte count on each read, so the
/// upload of a single (possibly large) file can be tracked as it streams.
struct ProgressReader<R, F> {
    inner: R,
    sent: u64,
    on_read: F,
}

impl<R: Read, F: FnMut(u64)> Read for ProgressReader<R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.sent += n as u64;
            (self.on_read)(self.sent);
        }
        Ok(n)
    }
}

#[derive(Debug, Clone, Default)]
pub struct FtpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub struct FtpSession {
    stream: FtpStream,
}

impl Drop for FtpSession {
    /// Best-effort FTP `QUIT` so the console frees its (single) connection
    /// slot even when a session is dropped on an error path, without an
    /// explicit `quit()` call. Bounded by `IO_TIMEOUT`, so a dead/timed-out
    /// connection cannot hang this beyond that.
    fn drop(&mut self) {
        let _ = self.stream.quit();
    }
}

fn parent_and_name(remote_dir: &str) -> (String, String) {
    let trimmed = remote_dir.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((parent, name)) if !parent.is_empty() => (parent.to_string(), name.to_string()),
        Some((_, name)) => ("/".to_string(), name.to_string()),
        None => ("/".to_string(), trimmed.to_string()),
    }
}

/// Maximum connection establishment timeout.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Maximum timeout waiting for a response on the control connection.
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

impl FtpSession {
    pub fn connect(config: &FtpConfig) -> Result<Self> {
        use std::net::ToSocketAddrs;

        let addr = (config.host.as_str(), config.port)
            .to_socket_addrs()
            .with_context(|| format!("invalid address: {}:{}", config.host, config.port))?
            .next()
            .with_context(|| format!("address not found: {}", config.host))?;

        let mut stream = FtpStream::connect_timeout(addr, CONNECT_TIMEOUT)
            .with_context(|| {
                format!(
                    "connection to {}:{} (timeout of {}s exceeded?)",
                    config.host,
                    config.port,
                    CONNECT_TIMEOUT.as_secs()
                )
            })?;
        let _ = stream.get_ref().set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.get_ref().set_write_timeout(Some(IO_TIMEOUT));

        stream
            .login(&config.user, &config.password)
            .context("FTP authentication refused")?;
        stream
            .transfer_type(FileType::Binary)
            .context("switching to binary mode")?;
        Ok(Self { stream })
    }

    /// Moves to a remote directory by descending component by component
    /// (the console server poorly handles absolute paths).
    fn cwd(&mut self, remote_dir: &str) -> Result<()> {
        self.stream.cwd("/").context("CWD /")?;
        for part in remote_dir.split('/').filter(|p| !p.is_empty()) {
            self.stream
                .cwd(part)
                .with_context(|| format!("CWD {part} (in {remote_dir})"))?;
        }
        Ok(())
    }

    /// Like `cwd`, but creates missing directories.
    fn cwd_create(&mut self, remote_dir: &str) -> Result<()> {
        self.stream.cwd("/").context("CWD /")?;
        for part in remote_dir.split('/').filter(|p| !p.is_empty()) {
            if self.stream.cwd(part).is_err() {
                self.stream
                    .mkdir(part)
                    .with_context(|| format!("MKD {part} (in {remote_dir})"))?;
                self.stream
                    .cwd(part)
                    .with_context(|| format!("CWD {part} (in {remote_dir})"))?;
            }
        }
        Ok(())
    }

    /// Lists the current directory.
    fn list_cwd(&mut self) -> Vec<RemoteEntry> {
        let Ok(lines) = self.stream.list(None) else {
            return Vec::new();
        };

        lines
            .iter()
            .filter_map(|line| {
                let file = suppaftp::list::ListParser::parse_posix(line)
                    .or_else(|_| suppaftp::list::ListParser::parse_dos(line))
                    .ok()?;
                let name = file.name().to_string();
                if name == "." || name == ".." {
                    return None;
                }
                Some(RemoteEntry {
                    name,
                    is_dir: file.is_directory(),
                    size: file.size() as u64,
                })
            })
            .collect()
    }

    /// Lists entries at the root of the console (Hdd1, Usb0, ...).
    pub fn list_root(&mut self) -> Result<Vec<String>> {
        self.cwd("/")?;
        Ok(self.list_cwd().into_iter().map(|e| e.name).collect())
    }

    /// Lists a remote directory: (name, is_dir, size).
    /// Returns an empty list if the directory does not exist.
    pub fn list_dir(&mut self, remote_dir: &str) -> Vec<RemoteEntry> {
        if self.cwd(remote_dir).is_err() {
            return Vec::new();
        }
        self.list_cwd()
    }

    /// Recursive size of a remote directory, with bounded depth.
    pub fn dir_size(&mut self, remote_dir: &str, max_depth: u32) -> u64 {
        let mut total = 0;
        for entry in self.list_dir(remote_dir) {
            if entry.is_dir {
                if max_depth > 0 {
                    total += self.dir_size(&format!("{remote_dir}/{}", entry.name), max_depth - 1);
                }
            } else {
                total += entry.size;
            }
        }
        total
    }

    /// Downloads a remote file into memory.
    pub fn download_file(&mut self, remote_path: &str) -> Result<Vec<u8>> {
        let (parent, name) = parent_and_name(remote_path);
        self.cwd(&parent)?;
        Ok(self
            .stream
            .retr_as_buffer(&name)
            .with_context(|| format!("downloading {remote_path}"))?
            .into_inner())
    }

    /// Counts files in a remote directory, recursively.
    pub fn count_files(&mut self, remote_dir: &str) -> u64 {
        let mut count = 0;
        for entry in self.list_dir(remote_dir) {
            if entry.is_dir {
                count += self.count_files(&format!("{remote_dir}/{}", entry.name));
            } else {
                count += 1;
            }
        }
        count
    }

    /// Recursively removes a remote directory.
    /// `progress(deleted_files, total_files)` is called after each file.
    pub fn remove_dir_recursive(
        &mut self,
        remote_dir: &str,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()> {
        let total = self.count_files(remote_dir);
        let mut done: u64 = 0;
        progress(0, total);
        self.remove_dir_inner(remote_dir, &mut done, total, progress)
    }

    fn remove_dir_inner(
        &mut self,
        remote_dir: &str,
        done: &mut u64,
        total: u64,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()> {
        for entry in self.list_dir(remote_dir) {
            if entry.is_dir {
                self.remove_dir_inner(
                    &format!("{remote_dir}/{}", entry.name),
                    done,
                    total,
                    progress,
                )?;
            } else {
                self.cwd(remote_dir)?;
                self.stream
                    .rm(&entry.name)
                    .with_context(|| format!("removing {remote_dir}/{}", entry.name))?;
                *done += 1;
                progress(*done, total);
            }
        }

        let (parent, name) = parent_and_name(remote_dir);
        self.cwd(&parent)?;
        self.stream
            .rmdir(&name)
            .with_context(|| format!("removing {remote_dir}"))?;
        Ok(())
    }

    /// Recursively sends `local_dir` to `remote_dir` (created if needed).
    /// `progress(sent_bytes, total_bytes)` is called after each file.
    pub fn upload_dir(
        &mut self,
        local_dir: &Path,
        remote_dir: &str,
        progress: &mut dyn FnMut(u64, u64, Option<f64>),
    ) -> Result<()> {
        let total = dir_size(local_dir);
        let mut sent: u64 = 0;
        progress(0, total, None);
        self.upload_dir_inner(local_dir, remote_dir, &mut sent, total, progress)
    }

    fn upload_dir_inner(
        &mut self,
        local_dir: &Path,
        remote_dir: &str,
        sent: &mut u64,
        total: u64,
        progress: &mut dyn FnMut(u64, u64, Option<f64>),
    ) -> Result<()> {
        self.cwd_create(remote_dir)
            .with_context(|| format!("creating {remote_dir}"))?;

        let entries = std::fs::read_dir(local_dir)
            .with_context(|| format!("reading {}", local_dir.display()))?;
        for entry in entries.flatten() {
            let local_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if local_path.is_dir() {
                let remote_path = format!("{}/{}", remote_dir.trim_end_matches('/'), name);
                self.upload_dir_inner(&local_path, &remote_path, sent, total, progress)?;
                // Recursive uploads changed the current directory.
                self.cwd(remote_dir)?;
            } else {
                let file = File::open(&local_path)
                    .with_context(|| format!("opening {}", local_path.display()))?;
                let size = file.metadata().map(|m| m.len()).unwrap_or(0);

                // Average speed for THIS file: bytes streamed so far divided by
                // the time elapsed since this file started uploading.
                let base = *sent;
                let file_start = Instant::now();
                let mut last_notify = file_start;
                {
                    let mut reader = ProgressReader {
                        inner: BufReader::new(file),
                        sent: 0,
                        on_read: |file_sent: u64| {
                            let now = Instant::now();
                            if now.duration_since(last_notify) >= PROGRESS_DEBOUNCE {
                                last_notify = now;
                                let secs = file_start.elapsed().as_secs_f64();
                                let speed = (secs > 0.0).then(|| file_sent as f64 / 1e6 / secs);
                                progress(base + file_sent, total, speed);
                            }
                        },
                    };
                    self.stream
                        .put_file(&name, &mut reader)
                        .with_context(|| format!("sending {remote_dir}/{name}"))?;
                }

                *sent = base + size;
                let secs = file_start.elapsed().as_secs_f64();
                let speed = (secs > 0.0).then(|| size as f64 / 1e6 / secs);
                progress(*sent, total, speed);
            }
        }
        Ok(())
    }

    /// Uploads an in-memory buffer as a single file, creating `remote_dir`
    /// if needed. Unlike `upload_dir`, this needs no local directory tree.
    pub fn put_bytes(&mut self, remote_dir: &str, file_name: &str, bytes: &[u8]) -> Result<()> {
        self.cwd_create(remote_dir)
            .with_context(|| format!("creating {remote_dir}"))?;
        let mut reader = std::io::Cursor::new(bytes);
        self.stream
            .put_file(file_name, &mut reader)
            .with_context(|| format!("sending {remote_dir}/{file_name}"))?;
        Ok(())
    }

    /// Removes a single file from a remote directory.
    pub fn remove_file(&mut self, remote_dir: &str, file_name: &str) -> Result<()> {
        self.cwd(remote_dir)
            .with_context(|| format!("entering {remote_dir}"))?;
        self.stream
            .rm(file_name)
            .with_context(|| format!("removing {remote_dir}/{file_name}"))
    }

    /// Restarts the Aurora dashboard on the console (FTP `SITE RESTART`).
    ///
    /// Aurora replies `211 Restarting Aurora. Please reconnect.` (status
    /// `System`, not the `CommandOk` that `FtpStream::site` expects), then
    /// resets the connection — confirmed against a real console. Use
    /// `custom_command` accepting both statuses instead.
    pub fn restart_aurora(&mut self) -> Result<()> {
        use suppaftp::Status;

        self.stream
            .custom_command("SITE RESTART", &[Status::CommandOk, Status::System])
            .context("sending SITE RESTART")?;
        Ok(())
    }

    /// Closes the connection. Just a documented drop point: the actual
    /// `QUIT` is sent by the `Drop` impl, which also covers error paths.
    pub fn quit(self) {}
}
