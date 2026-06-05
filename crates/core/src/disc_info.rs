// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};
use wii_disc_info::Meta;

#[derive(Debug, Clone)]
pub struct DiscInfo {
    pub path: PathBuf,
    pub meta: Meta,
    pub is_worth_scrubbing: bool,
    pub crc32: u32,
}

impl DiscInfo {
    pub fn from_path(path: PathBuf) -> Option<Self> {
        let mut f = File::open(&path).ok()?;
        let meta = wii_disc_info::Meta::read(&mut f).ok()?;

        let is_worth_scrubbing = meta.format() == wii_disc_info::Format::Wbfs
            && is_worth_scrubbing(&mut f).unwrap_or(false);

        let crc32_path = path.with_file_name(format!("{}.crc32", meta.game_id()));
        let crc32 = fs::read_to_string(&crc32_path).unwrap_or_default();
        let crc32 = u32::from_str_radix(&crc32, 16).unwrap_or_default();

        Some(Self {
            path,
            meta,
            is_worth_scrubbing,
            crc32,
        })
    }
}

// use this only on wbfs files
pub fn is_worth_scrubbing<R: Read + Seek>(disc_reader: &mut R) -> Result<bool> {
    fn true_offset(offset: [u8; 4]) -> u64 {
        let offset = u32::from_be_bytes(offset);
        0x200000 + (u64::from(offset) << 2)
    }

    let mut buf = [0u8; 8];

    // partitions info
    disc_reader.seek(SeekFrom::Start(0x240000))?;
    disc_reader.read_exact(&mut buf)?;

    let part_count = u32::from_be_bytes(buf[0..4].try_into().unwrap());
    #[cfg(debug_assertions)]
    eprintln!("DEBUG: part_count: {part_count}");
    if part_count == 0 {
        return Ok(false);
    }

    let part_info_table_offset = true_offset(buf[4..8].try_into().unwrap());
    #[cfg(debug_assertions)]
    eprintln!("DEBUG: part_info_table_offset: {part_info_table_offset}");
    if part_info_table_offset == 0 {
        return Ok(false);
    }

    // read the first partition meta
    disc_reader.seek(SeekFrom::Start(part_info_table_offset))?;
    disc_reader.read_exact(&mut buf)?;

    let part_offset = true_offset(buf[0..4].try_into().unwrap());
    #[cfg(debug_assertions)]
    eprintln!("DEBUG: part_offset: {part_offset}");
    if part_offset == 0 {
        return Ok(false);
    }

    // check if the partition type is 0x0001 (update partition)
    #[cfg(debug_assertions)]
    eprintln!("DEBUG: part_type: {:?}", &buf[4..8]);
    if buf[4..8] != [0, 0, 0, 1] {
        return Ok(false);
    }

    disc_reader.seek(SeekFrom::Start(part_offset + 0x2b8))?;
    disc_reader.read_exact(&mut buf)?;

    let data_offset = true_offset(buf[0..4].try_into().unwrap());
    #[cfg(debug_assertions)]
    eprintln!("DEBUG: data_offset: {data_offset}");
    if data_offset == 0 {
        return Ok(false);
    }

    let data_size_raw = u32::from_be_bytes(buf[4..8].try_into().unwrap());
    let data_size = u64::from(data_size_raw) << 2;
    #[cfg(debug_assertions)]
    eprintln!("DEBUG: data_size: {data_size}");
    if data_size == 0 {
        return Ok(false);
    }

    // too small to bother
    if data_size < 1024 * 1024 * 8 {
        return Ok(false);
    }

    // check if the update data is unmapped
    disc_reader.seek(SeekFrom::Start(0x300))?;
    disc_reader.read_exact(&mut buf)?;
    #[cfg(debug_assertions)]
    eprintln!("DEBUG: buf: {buf:?}");
    let unmapped = buf == [0, 1, 0, 0, 0, 0, 0, 0];

    Ok(!unmapped)
}
