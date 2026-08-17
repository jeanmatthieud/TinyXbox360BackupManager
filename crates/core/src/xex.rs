// SPDX-License-Identifier: GPL-3.0-only

//! Minimal reader for Xbox 360 executables (`default.xex`): only extracts the
//! TitleID from the XEX2 optional header.
//!
//! The parsing itself is `iso2god`'s (it already walks the XEX header to build
//! GOD containers); this wraps it in the same shape as [`crate::xbe`] so both
//! extracted formats resolve their TitleID the same way.

use anyhow::{Context, Result};
use iso2god::executable::xex::XexHeader;
use std::io::{Read, Seek};
use std::path::Path;

/// How much of a `default.xex` to fetch when reading it over FTP. Every
/// optional header and its payload sits before the PE image, and that region
/// is a few KiB in practice — 64 KiB leaves ample margin without pulling the
/// whole (multi-MiB) executable across the wire.
pub const HEADER_PREFIX_SIZE: usize = 64 * 1024;

/// Reads the TitleID from a XEX stream.
pub fn read_title_id(reader: &mut (impl Read + Seek)) -> Result<u32> {
    let header = XexHeader::read(reader).context("reading XEX header")?;
    let info = header
        .fields
        .execution_info
        .context("no execution info in XEX header")?;
    Ok(info.title_id)
}

/// Reads the TitleID of a `default.xex` file, as 8 hex chars.
pub fn title_id_from_file(path: &Path) -> Result<String> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    Ok(format!("{:08X}", read_title_id(&mut file)?))
}

/// Same, from a buffer already in memory (XEX header prefix downloaded over
/// FTP). Fails rather than guessing if the execution info sits past the
/// downloaded prefix.
pub fn title_id_from_bytes(bytes: &[u8]) -> Result<String> {
    let mut cursor = std::io::Cursor::new(bytes);
    Ok(format!("{:08X}", read_title_id(&mut cursor)?))
}
