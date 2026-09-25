// SPDX-License-Identifier: GPL-3.0-only

//! Minimal STFS package header parsing (CON / LIVE / PIRS containers):
//! just enough metadata to identify and install a package, without
//! reading the internal STFS file system.

use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Discs sharing a TitleID are told apart by these two bytes.
const DISC_NUMBER_OFFSET: u64 = 0x366;

/// License table: 16 entries of `{ u64 license ID, u32 license bits, u32
/// flags }`. The top 16 bits of the ID give its type.
const LICENSE_TABLE_OFFSET: u64 = 0x22C;
const LICENSE_ENTRIES: usize = 16;
const LICENSE_UNRESTRICTED: u16 = 0xFFFF;
const LICENSE_CONSOLE: u16 = 0xF000;
const LICENSE_PROFILE_CONSOLE: u16 = 0x0009;
const LICENSE_PROFILE_WINDOWS: u16 = 0x0003;

/// Bytes needed to cover every field `inspect_reader` reads. The last one is
/// `title_name` at 0x1691 spanning 0x100 bytes (ends at 0x1791); rounded up.
/// Used to fetch just the header prefix of a large package (e.g. over FTP)
/// instead of downloading the whole file.
pub const HEADER_SIZE: usize = 0x1800;

/// Marketplace content (DLC, unlocks).
pub const CONTENT_TYPE_DLC: u32 = 0x0000_0002;
/// Title update.
pub const CONTENT_TYPE_TITLE_UPDATE: u32 = 0x000B_0000;
/// Arcade (XBLA) title.
pub const CONTENT_TYPE_ARCADE: u32 = 0x000D_0000;

pub fn is_stfs_magic(magic: &[u8; 4]) -> bool {
    magic == b"CON " || magic == b"LIVE" || magic == b"PIRS"
}

/// Folder name for installed DLC / marketplace content
/// (`Content/0000000000000000/<TitleID>/00000002`).
pub fn dlc_dir_name() -> String {
    format!("{CONTENT_TYPE_DLC:08X}")
}

/// Folder name for installed title updates, read by the dashboard at boot
/// (`Content/0000000000000000/<TitleID>/000B0000`).
pub fn title_update_dir_name() -> String {
    format!("{CONTENT_TYPE_TITLE_UPDATE:08X}")
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
    /// 1-based disc number, telling apart the packages of a multi-disc
    /// game sharing the same TitleID (0 if unset).
    pub disc_number: u8,
    /// Total number of discs in the set (0 or 1 for single-disc games).
    pub disc_in_set: u8,
    /// Who the license table restricts the full content to; `None` when it
    /// plays anywhere.
    pub license_lock: Option<LicenseLock>,
}

/// Consoles and profiles a package is licensed to, when its license table
/// restricts it. The package runs as a trial (or not at all, for DLC) unless
/// one of them matches, or Dashlaunch's license patches are on. Whether they
/// match the target console can't be told from here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LicenseLock {
    /// 40-bit console IDs.
    pub console_ids: Vec<u64>,
    /// Profile XUIDs.
    pub xuids: Vec<u64>,
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
    inspect_reader(&mut file, path.to_owned())
}

/// Like `inspect`, but reads from an arbitrary seekable reader (e.g. an
/// in-memory buffer downloaded over FTP). `path` is only carried through
/// for display purposes.
pub fn inspect_reader<R: Read + Seek>(
    reader: &mut R,
    path: PathBuf,
) -> Result<Option<StfsInfo>> {
    let mut magic = [0u8; 4];
    if reader.read_exact(&mut magic).is_err() || !is_stfs_magic(&magic) {
        return Ok(None);
    }

    let read_u32 = |reader: &mut R, offset: u64| -> Result<u32> {
        let mut buf = [0u8; 4];
        reader.seek(SeekFrom::Start(offset))?;
        reader.read_exact(&mut buf)?;
        Ok(u32::from_be_bytes(buf))
    };
    let read_u8 = |reader: &mut R, offset: u64| -> Result<u8> {
        let mut buf = [0u8; 1];
        reader.seek(SeekFrom::Start(offset))?;
        reader.read_exact(&mut buf)?;
        Ok(buf[0])
    };

    let content_type = read_u32(reader, 0x344)?;
    let media_id = read_u32(reader, 0x354)?;
    let title_id = read_u32(reader, 0x360)?;
    let disc_number = read_u8(reader, DISC_NUMBER_OFFSET)?;
    let disc_in_set = read_u8(reader, DISC_NUMBER_OFFSET + 1)?;
    let display_name = read_utf16_be(reader, 0x411);
    let title_name = read_utf16_be(reader, 0x1691);
    let license_lock = read_license_lock(reader)?;

    Ok(Some(StfsInfo {
        path,
        content_type,
        title_id: format!("{title_id:08X}"),
        media_id: format!("{media_id:08X}"),
        display_name,
        title_name,
        disc_number,
        disc_in_set,
        license_lock,
    }))
}

/// Reads the license table. An entry grants something only when its license
/// bits are set; a single granted unrestricted entry unlocks the package for
/// everyone, whatever the other entries say. Other types (media flags,
/// privileges…) bind the package to no console or profile, so they are
/// ignored.
fn read_license_lock<R: Read + Seek>(reader: &mut R) -> Result<Option<LicenseLock>> {
    let mut table = [0u8; LICENSE_ENTRIES * 0x10];
    reader.seek(SeekFrom::Start(LICENSE_TABLE_OFFSET))?;
    reader.read_exact(&mut table)?;

    let mut lock = LicenseLock::default();
    for entry in table.chunks_exact(0x10) {
        let id = u64::from_be_bytes(entry[..8].try_into().unwrap());
        let bits = u32::from_be_bytes(entry[8..12].try_into().unwrap());
        if bits == 0 {
            continue;
        }
        match (id >> 48) as u16 {
            LICENSE_UNRESTRICTED => return Ok(None),
            LICENSE_CONSOLE => lock.console_ids.push(id & 0xFF_FFFF_FFFF),
            LICENSE_PROFILE_CONSOLE | LICENSE_PROFILE_WINDOWS => lock.xuids.push(id),
            _ => {}
        }
    }

    let locked = !lock.console_ids.is_empty() || !lock.xuids.is_empty();
    Ok(locked.then_some(lock))
}

/// Reads a 0x100-byte UTF-16 big-endian string field.
fn read_utf16_be<R: Read + Seek>(reader: &mut R, offset: u64) -> Option<String> {
    let mut buf = [0u8; 0x100];
    reader.seek(SeekFrom::Start(offset)).ok()?;
    reader.read_exact(&mut buf).ok()?;
    let utf16: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    let s = String::from_utf16_lossy(&utf16).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Reads the header of the first STFS package found in a content-type folder
/// (e.g. .../<TitleID>/00007000 or .../000D0000).
pub fn package_in_dir(type_dir: &Path) -> Option<StfsInfo> {
    std::fs::read_dir(type_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        // GOD data folders (".data") are skipped here, anything else that is
        // not a package by the magic check.
        .filter(|path| path.is_file())
        .find_map(|path| inspect(&path).ok().flatten())
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
        assert_eq!(info.license_lock, None);

        assert_eq!(title_from_dir(&dir), Some("Castle Crashers".to_string()));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn license_entry(buf: &mut [u8], index: usize, id: u64, bits: u32) {
        let at = LICENSE_TABLE_OFFSET as usize + index * 0x10;
        buf[at..at + 8].copy_from_slice(&id.to_be_bytes());
        buf[at + 8..at + 12].copy_from_slice(&bits.to_be_bytes());
    }

    fn license_of(buf: &[u8]) -> Option<LicenseLock> {
        let mut cursor = std::io::Cursor::new(buf);
        inspect_reader(&mut cursor, PathBuf::new())
            .unwrap()
            .unwrap()
            .license_lock
    }

    #[test]
    fn parses_license_table() {
        let mut buf = vec![0u8; HEADER_SIZE];
        buf[..4].copy_from_slice(b"LIVE");

        // Empty table: nothing restricts the package.
        assert_eq!(license_of(&buf), None);

        license_entry(&mut buf, 0, 0x0009_0000_1234_5678, 1);
        license_entry(&mut buf, 1, 0xF000_00AB_CDEF_0123, 1);
        // Granting no bits, so ignored.
        license_entry(&mut buf, 2, 0xF000_0011_1111_1111, 0);
        assert_eq!(
            license_of(&buf),
            Some(LicenseLock {
                console_ids: vec![0xAB_CDEF_0123],
                xuids: vec![0x0009_0000_1234_5678],
            })
        );

        // One unrestricted entry unlocks it for everyone.
        license_entry(&mut buf, 3, u64::MAX, 1);
        assert_eq!(license_of(&buf), None);
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
