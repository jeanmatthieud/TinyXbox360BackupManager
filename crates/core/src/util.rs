// SPDX-License-Identifier: GPL-3.0-only

use sha1::{Digest, Sha1};
use std::collections::VecDeque;
use std::path::Path;
use std::time::{Duration, Instant};

/// File name of `path` for display (queue rows, confirmations, notices),
/// falling back to the whole path when it has none. Shared so that the same
/// input reads the same everywhere it is named.
pub fn display_file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// Whether the app runs inside a Flatpak sandbox: the runtime puts this file at
/// the root of every sandbox, and nothing else creates it.
pub fn in_flatpak() -> bool {
    cfg!(target_os = "linux") && Path::new("/.flatpak-info").exists()
}

/// `path` as the user knows it, for display only.
///
/// Inside a Flatpak, a file picked outside the sandbox's own folders is handed
/// over by the document portal as `/run/user/<uid>/doc/<id>/<name>`: that is
/// the path to open, but it means nothing to the user. The portal records
/// where the file really lives in an extended attribute, which is what gets
/// shown instead. Never open or store the result: the sandbox cannot reach it.
pub fn display_path(path: &Path) -> String {
    #[cfg(target_os = "linux")]
    if is_portal_document(path)
        && let Ok(Some(host)) = xattr::get(path, "user.document-portal.host-path")
    {
        // The portal NUL-terminates the value.
        let host = String::from_utf8_lossy(&host);
        let host = host.trim_end_matches('\0');
        if !host.is_empty() {
            return host.to_owned();
        }
    }

    path.to_string_lossy().into_owned()
}

/// Whether `path` sits under the document portal's mount,
/// `/run/user/<uid>/doc/`.
#[cfg(target_os = "linux")]
fn is_portal_document(path: &Path) -> bool {
    let mut parts = path.components().skip(1).map(|c| c.as_os_str());
    parts.next() == Some("run".as_ref())
        && parts.next() == Some("user".as_ref())
        && parts.next().is_some()
        && parts.next() == Some("doc".as_ref())
}

/// SHA1 hex digest of `bytes` (lowercase).
pub fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// SHA1 hex digest of everything `reader` yields (lowercase), streamed so the
/// content never needs to sit in memory at once. Reading a title update — a
/// routinely 100 MB file, on a console drive or over FTP — into a `Vec` just
/// to hash it is what this exists to avoid.
pub fn sha1_hex_reader(reader: &mut dyn std::io::Read) -> std::io::Result<String> {
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// SHA1 hex digest of a file's content (lowercase), streamed so the whole
/// file never needs to sit in memory at once.
pub fn sha1_hex_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    sha1_hex_reader(&mut file)
}

/// Path of the sub-directory named `name` (case-insensitively) directly
/// inside `dir`, if any. The console's own filesystem is case-insensitive, so
/// a folder Aurora wrote can be reached with any casing there and must be
/// looked up the same way once the drive is read from a case-sensitive one.
pub fn find_dir_ci(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| {
            e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_dir()
        })
        .map(|e| e.path())
}

/// Path of the file named `name` (case-insensitively) directly inside `dir`,
/// if any. Xbox images store their executable as `Default.xex`/`default.xbe`
/// with inconsistent casing, so extracted folders must never be probed with a
/// case-sensitive `join()` on a case-sensitive filesystem.
pub fn find_file_ci(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| {
            e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_file()
        })
        .map(|e| e.path())
}

/// Number of files in a directory (recursive).
pub fn file_count(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                total += file_count(&path);
            } else {
                total += 1;
            }
        }
    }
    total
}

/// Total size of a directory (recursive), in bytes.
pub fn dir_size(path: &Path) -> u64 {
    dir_size_on(path, 1)
}

/// Room a directory takes on a filesystem that allocates in whole units of
/// `cluster_size` bytes — every file rounded up, as FATX stores it.
///
/// An extracted game is tens of thousands of small files, and on a 16 KiB
/// cluster the difference against [`dir_size`] runs into hundreds of
/// megabytes: enough for a "there is room" answer to be wrong and for the
/// copy to die with a half-installed game on the console.
pub fn dir_size_on(path: &Path, cluster_size: u64) -> u64 {
    let cluster_size = cluster_size.max(1);
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // A directory's own entry table costs a cluster too — no part
                // of a plain byte count, hence only when one is being applied.
                if cluster_size > 1 {
                    total += cluster_size;
                }
                total += dir_size_on(&path, cluster_size);
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len().div_ceil(cluster_size) * cluster_size;
            }
        }
    }
    total
}

/// Readable formatting of a size in bytes.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["o", "Kio", "Mio", "Gio", "Tio"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} o")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Transfer rate averaged over the last few seconds, for display.
///
/// Fed with the running byte count of a whole transfer, so the figure spans
/// file boundaries: averaging per file instead makes it swing wildly on a tree
/// of small files, and spike at the start of each large one while the writes
/// are still landing in a cache. The displayed value only moves once per
/// [`Self::REFRESH`] so that it can actually be read.
pub struct RateMeter {
    /// `(when, bytes transferred so far)`, oldest first, spaced at least
    /// [`Self::SAMPLE_GAP`] apart.
    samples: VecDeque<(Instant, u64)>,
    /// Last value handed out, and when it was computed.
    shown: Option<(Instant, f64)>,
}

impl RateMeter {
    /// How far back the average looks.
    const WINDOW: Duration = Duration::from_secs(5);
    /// History needed before a first figure is given.
    const MIN_SPAN: Duration = Duration::from_secs(1);
    /// How often the displayed value changes.
    const REFRESH: Duration = Duration::from_secs(1);
    /// Callers may report thousands of times per second (one call per small
    /// file); closer samples than this add nothing to a 5-second average.
    const SAMPLE_GAP: Duration = Duration::from_millis(100);

    pub fn new() -> Self {
        Self { samples: VecDeque::new(), shown: None }
    }

    /// Records that `total_bytes` have been transferred so far, and returns
    /// the average rate in megabytes per second — `None` until there is
    /// enough history for a meaningful figure.
    pub fn record(&mut self, total_bytes: u64) -> Option<f64> {
        let now = Instant::now();
        if self
            .samples
            .back()
            .is_none_or(|&(at, _)| now.duration_since(at) >= Self::SAMPLE_GAP)
        {
            self.samples.push_back((now, total_bytes));
        }
        // Keeps one sample at or beyond the window's edge, so the average
        // covers the whole window rather than slightly less of it.
        while self.samples.len() > 1
            && now.duration_since(self.samples[1].0) >= Self::WINDOW
        {
            self.samples.pop_front();
        }

        if let Some((at, rate)) = self.shown
            && now.duration_since(at) < Self::REFRESH
        {
            return Some(rate);
        }

        let &(since, bytes_then) = self.samples.front()?;
        let span = now.duration_since(since);
        if span < Self::MIN_SPAN {
            return None;
        }
        let rate = total_bytes.saturating_sub(bytes_then) as f64 / 1e6 / span.as_secs_f64();
        self.shown = Some((now, rate));
        Some(rate)
    }
}

impl Default for RateMeter {
    fn default() -> Self {
        Self::new()
    }
}

/// Sanitizes a name to make it a valid FAT32 directory name.
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_string();
    if cleaned.is_empty() {
        "Unnamed".to_string()
    } else {
        cleaned
    }
}
