// SPDX-License-Identifier: GPL-3.0-only

//! Minimal reader for Original Xbox executables (`default.xbe`):
//! only extracts the TitleID from the certificate.

use anyhow::{Context, Result, bail};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Reads the TitleID (4 bytes at certificate offset 0x8) from an XBE stream.
pub fn read_title_id(reader: &mut (impl Read + Seek)) -> Result<u32> {
    let mut header = [0u8; 0x11C];
    reader
        .read_exact(&mut header)
        .context("reading XBE header")?;
    if &header[0..4] != b"XBEH" {
        bail!("not an XBE file (bad magic)");
    }

    let base_addr = u32::from_le_bytes(header[0x104..0x108].try_into().unwrap());
    let cert_addr = u32::from_le_bytes(header[0x118..0x11C].try_into().unwrap());
    let cert_offset = cert_addr
        .checked_sub(base_addr)
        .context("invalid XBE certificate address")?;

    reader.seek(SeekFrom::Start(cert_offset as u64 + 0x8))?;
    let mut title_id = [0u8; 4];
    reader
        .read_exact(&mut title_id)
        .context("reading XBE TitleID")?;
    Ok(u32::from_le_bytes(title_id))
}

/// Reads the TitleID of a `default.xbe` file, as 8 hex chars.
pub fn title_id_from_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    Ok(format!("{:08X}", read_title_id(&mut file)?))
}

/// Same, from a buffer already in memory (XBE downloaded over FTP).
pub fn title_id_from_bytes(bytes: &[u8]) -> Result<String> {
    let mut cursor = std::io::Cursor::new(bytes);
    Ok(format!("{:08X}", read_title_id(&mut cursor)?))
}
