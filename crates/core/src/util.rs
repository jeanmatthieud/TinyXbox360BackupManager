// SPDX-License-Identifier: GPL-3.0-only

use sha1::{Digest, Sha1};
use std::path::Path;

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
