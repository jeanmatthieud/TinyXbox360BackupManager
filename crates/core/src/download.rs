// SPDX-License-Identifier: GPL-3.0-only

//! Fetching a component archive off the web, for the Toolbox tools.
//!
//! Both Toolbox tools work the same way: pull a `.zip`/`.7z`/`.rar` from a URL,
//! unpack it somewhere temporary, and pick the one folder inside it that
//! matters. Only the assembly step differs, so everything up to it lives here
//! and is shared by [`crate::badavatar`] and [`crate::ogxbox_compat`].
//!
//! Each caller owns its own cancellation marker — the GUI matches on the error
//! message to tell "the user stopped it" from "it broke" — so the marker is
//! passed in rather than fixed here.

use anyhow::{Context, Result, bail};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Refuse to stream more than this from a single URL. A component archive is
/// tens of megabytes; anything near this ceiling means the URL is wrong.
const DOWNLOAD_LIMIT: u64 = 4 * 1024 * 1024 * 1024;

/// Shared HTTP agent with a browser-like User-Agent. Some hosts (e.g. Aurora's
/// download server behind Cloudflare) reject requests without one.
///
/// The timeouts are split by phase rather than capped globally: a server has to
/// answer promptly, but the body behind that answer is a component pack of
/// several hundred megabytes, which on a slow line legitimately takes longer
/// than any single global ceiling worth setting. A transfer the user no longer
/// wants is stopped by them — [`download_to_file`] polls the cancellation
/// marker between chunks — so the body ceiling is only there to keep a server
/// that stalls mid-stream from hanging the thread forever.
pub static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(Duration::from_secs(2 * 3600)))
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/124.0 Safari/537.36",
        )
        .build()
        .into()
});

/// Why a download failed, in terms the user can act on. Carried as a typed
/// error (rather than folded into the message) so the GUI can find it in the
/// chain with `downcast_ref` and word its own advice — "pick another source",
/// "try again later" — without matching on the HTTP client's wording.
#[derive(Debug)]
pub struct DownloadError {
    /// What was being downloaded, e.g. "Aurora".
    pub label: String,
    pub url: String,
    pub kind: DownloadFailure,
    /// The HTTP client's own error, kept for the log.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadFailure {
    /// The server answered, but with an error status (404, 500, …).
    HttpStatus(u16),
    /// The server could not be reached at all (DNS, connection refused, TLS).
    Unreachable,
    /// The server stopped answering.
    Timeout,
    /// The transfer started but broke off.
    Interrupted,
}

impl DownloadError {
    fn new(label: &str, url: &str, kind: DownloadFailure, detail: impl fmt::Display) -> Self {
        Self {
            label: label.to_string(),
            url: url.to_string(),
            kind,
            detail: detail.to_string(),
        }
    }

    fn from_ureq(label: &str, url: &str, err: ureq::Error) -> Self {
        let kind = match &err {
            ureq::Error::StatusCode(code) => DownloadFailure::HttpStatus(*code),
            ureq::Error::Timeout(_) => DownloadFailure::Timeout,
            ureq::Error::Io(e) if e.kind() == io::ErrorKind::TimedOut => DownloadFailure::Timeout,
            // Before any answer, an I/O error (DNS lookup, refused connection)
            // means the server was never reached.
            _ => DownloadFailure::Unreachable,
        };
        Self::new(label, url, kind, err)
    }

    /// For an error reading the body, once the server has answered.
    fn from_io(label: &str, url: &str, err: &io::Error) -> Self {
        let kind = match err.kind() {
            io::ErrorKind::TimedOut => DownloadFailure::Timeout,
            _ => DownloadFailure::Interrupted,
        };
        Self::new(label, url, kind, err)
    }

    /// Finds a download failure anywhere in an error's chain.
    pub fn find(err: &anyhow::Error) -> Option<&Self> {
        err.chain().find_map(|e| e.downcast_ref::<Self>())
    }
}

impl fmt::Display for DownloadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HttpStatus(code) => write!(f, "the server answered with error {code}"),
            Self::Unreachable => f.write_str("the server could not be reached"),
            Self::Timeout => f.write_str("the server stopped responding"),
            Self::Interrupted => f.write_str("the connection was interrupted"),
        }
    }
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Couldn't download {} ({})", self.label, self.kind)
    }
}

impl std::error::Error for DownloadError {}

/// Bails with `marker` when the user has asked to stop.
pub fn check_cancel(cancel: &AtomicBool, marker: &str) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("{marker}");
    }
    Ok(())
}

/// Streams `url` to `dest`, reporting a coarse percentage in the status line.
/// Polls `cancel` between chunks so a long transfer can be interrupted.
///
/// A network failure comes back as a [`DownloadError`].
pub fn download_to_file(
    url: &str,
    dest: &Path,
    label: &str,
    cancel: &AtomicBool,
    status: &dyn Fn(&str),
    cancelled_marker: &str,
) -> Result<()> {
    let mut response = AGENT
        .get(url)
        .header("Referer", origin_of(url).as_str())
        .call()
        .map_err(|e| DownloadError::from_ureq(label, url, e))?;

    let total = response.body().content_length();
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .reader();

    let mut file = File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    let mut done: u64 = 0;
    let mut last_report: u64 = 0;

    loop {
        check_cancel(cancel, cancelled_marker)?;
        let n = reader
            .read(&mut buf)
            .map_err(|e| DownloadError::from_io(label, url, &e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;

        // Debounce status updates to roughly every 2 MiB.
        if done - last_report >= 2 * 1024 * 1024 {
            last_report = done;
            match total {
                Some(t) if t > 0 => {
                    status(&format!("Downloading {label}…  {}%", done * 100 / t));
                }
                _ => {
                    let mib = done as f64 / (1024.0 * 1024.0);
                    status(&format!("Downloading {label}…  {mib:.1} MiB"));
                }
            }
        }
    }

    Ok(())
}

/// Returns the archive extension ("7z", "rar" or "zip") to save a download
/// under, inferred from the URL. `None` for an unsupported extension.
pub fn archive_extension(url: &str) -> Option<&'static str> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let lower = path.to_lowercase();
    if lower.ends_with(".7z") {
        Some("7z")
    } else if lower.ends_with(".rar") {
        Some("rar")
    } else if lower.ends_with(".zip") {
        Some("zip")
    } else {
        None
    }
}

/// `scheme://host/` for a URL, used as a plausible `Referer`. Falls back to the
/// URL itself if it can't be parsed.
pub fn origin_of(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let after = &url[scheme_end + 3..];
        let host_len = after.find('/').unwrap_or(after.len());
        return format!("{}{}/", &url[..scheme_end + 3], &after[..host_len]);
    }
    url.to_string()
}

/// Depth-first search under `root` for an entry named `name` (case-insensitive)
/// that is a directory (`want_dir = true`) or a file (`want_dir = false`).
///
/// Archives from different publishers wrap their payload differently — some
/// have a folder named after the release around it, some do not — so the
/// interesting folder is found by name rather than by a fixed path.
pub fn find_entry(root: &Path, name: &str, want_dir: bool) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let is_dir = path.is_dir();
            let matches = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(name));
            if matches && is_dir == want_dir {
                return Some(path);
            }
            if is_dir {
                stack.push(path);
            }
        }
    }
    None
}
