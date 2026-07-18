// SPDX-License-Identifier: GPL-3.0-only

//! Minimal STFS package header parsing (CON / LIVE / PIRS containers):
//! just enough metadata to identify and install a package, without
//! reading the internal STFS file system.

use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Marketplace content (DLC, unlocks).
pub const CONTENT_TYPE_DLC: u32 = 0x0000_0002;
/// Title update.
pub const CONTENT_TYPE_TITLE_UPDATE: u32 = 0x000B_0000;
/// Arcade (XBLA) title.
pub const CONTENT_TYPE_ARCADE: u32 = 0x000D_0000;

pub fn is_stfs_magic(magic: &[u8; 4]) -> bool {
    magic == b"CON " || magic == b"LIVE" || magic == b"PIRS"
}

#[derive(Debug, Clone)]
pub struct StfsInfo {
    pub path: PathBuf,
    pub content_type: u32,
    /// 8 uppercase hex chars.
    pub title_id: String,
    /// 8 uppercase hex chars.
    pub media_id: String,
    /// Package display name (first locale), often the content name.
    pub display_name: Option<String>,
    /// Game title (first locale); not always filled in.
    pub title_name: Option<String>,
}

impl StfsInfo {
    /// Content sub-folder on the console: Content/0000000000000000/
    /// <TitleID>/<this>/<package file>.
    pub fn content_type_dir(&self) -> String {
        format!("{:08X}", self.content_type)
    }

    /// Best available human-readable name.
    pub fn name(&self) -> Option<&str> {
        self.title_name.as_deref().or(self.display_name.as_deref())
    }
}

/// Reads the STFS header of `path`. Returns Ok(None) if the file does not
/// carry an STFS magic (not a package); Err only on I/O failures.
pub fn inspect(path: &Path) -> Result<Option<StfsInfo>> {
    let mut file =
        File::open(path).with_context(|| format!("opening {}", path.display()))?;

    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() || !is_stfs_magic(&magic) {
        return Ok(None);
    }

    let read_u32 = |file: &mut File, offset: u64| -> Result<u32> {
        let mut buf = [0u8; 4];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut buf)?;
        Ok(u32::from_be_bytes(buf))
    };

    let content_type = read_u32(&mut file, 0x344)?;
    let media_id = read_u32(&mut file, 0x354)?;
    let title_id = read_u32(&mut file, 0x360)?;
    let display_name = read_utf16_be(&mut file, 0x411);
    let title_name = read_utf16_be(&mut file, 0x1691);

    Ok(Some(StfsInfo {
        path: path.to_owned(),
        content_type,
        title_id: format!("{title_id:08X}"),
        media_id: format!("{media_id:08X}"),
        display_name,
        title_name,
    }))
}

/// Reads a 0x100-byte UTF-16 big-endian string field.
fn read_utf16_be(file: &mut File, offset: u64) -> Option<String> {
    let mut buf = [0u8; 0x100];
    file.seek(SeekFrom::Start(offset)).ok()?;
    file.read_exact(&mut buf).ok()?;
    let utf16: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    let s = String::from_utf16_lossy(&utf16).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Reads the game name from the header of the first STFS package found in a
/// content-type folder (e.g. .../<TitleID>/00007000 or .../000D0000).
pub fn title_from_dir(type_dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(type_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // GOD data folders (".data") are skipped by the magic check.
        if let Ok(Some(info)) = inspect(&path)
            && let Some(name) = info.name()
        {
            return Some(name.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_utf16_be(buf: &mut [u8], offset: usize, s: &str) {
        for (i, c) in s.encode_utf16().enumerate() {
            let [hi, lo] = c.to_be_bytes();
            buf[offset + i * 2] = hi;
            buf[offset + i * 2 + 1] = lo;
        }
    }

    #[test]
    fn parses_synthetic_header() {
        let mut buf = vec![0u8; 0x1800];
        buf[..4].copy_from_slice(b"LIVE");
        buf[0x344..0x348].copy_from_slice(&CONTENT_TYPE_ARCADE.to_be_bytes());
        buf[0x354..0x358].copy_from_slice(&0x11223344u32.to_be_bytes());
        buf[0x360..0x364].copy_from_slice(&0x58410889u32.to_be_bytes());
        write_utf16_be(&mut buf, 0x411, "Full Game");
        write_utf16_be(&mut buf, 0x1691, "Castle Crashers");

        let dir = std::env::temp_dir().join("txbm-stfs-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("package");
        std::fs::write(&path, &buf).unwrap();

        let info = inspect(&path).unwrap().unwrap();
        assert_eq!(info.content_type, CONTENT_TYPE_ARCADE);
        assert_eq!(info.title_id, "58410889");
        assert_eq!(info.media_id, "11223344");
        assert_eq!(info.content_type_dir(), "000D0000");
        assert_eq!(info.name(), Some("Castle Crashers"));

        assert_eq!(title_from_dir(&dir), Some("Castle Crashers".to_string()));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_non_stfs() {
        let dir = std::env::temp_dir().join("txbm-stfs-test-neg");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("not-a-package");
        std::fs::write(&path, b"hello world").unwrap();
        assert!(inspect(&path).unwrap().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
