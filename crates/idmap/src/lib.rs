// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
// SPDX-License-Identifier: GPL-3.0-only

#![warn(clippy::all, rust_2018_idioms)]

include!(concat!(env!("OUT_DIR"), "/id_map_meta.rs"));

use std::num::NonZeroU32;

#[repr(align(4))]
struct AlignedBytes<const N: usize>([u8; N]);

static ALIGNED_BYTES: AlignedBytes<BIN_LEN> =
    AlignedBytes(*include_bytes!(concat!(env!("OUT_DIR"), "/id_map.bin")));

fn find(id: u32) -> Option<usize> {
    let ptr = ALIGNED_BYTES.0.as_ptr().cast::<u32>();
    let game_ids = unsafe { std::slice::from_raw_parts(ptr, COUNT) };
    game_ids.binary_search(&id).ok()
}

pub fn get_ghid(id: u32) -> Option<NonZeroU32> {
    let idx = find(id)?;

    let offset = COUNT * 4 + idx * 4;
    let ghid = u32::from_ne_bytes(ALIGNED_BYTES.0[offset..offset + 4].try_into().unwrap());

    NonZeroU32::new(ghid)
}

pub fn get_title(id: u32) -> Option<&'static str> {
    let idx = find(id)?;

    let relative_title_offset = {
        let offset = COUNT * 8 + idx * 4;
        u32::from_ne_bytes(ALIGNED_BYTES.0[offset..offset + 4].try_into().unwrap())
    };

    let title_len = {
        let offset = COUNT * 12 + idx;
        ALIGNED_BYTES.0[offset] as usize
    };

    let title_offset = relative_title_offset as usize + COUNT * 13;
    let title_slice = &ALIGNED_BYTES.0[title_offset..title_offset + title_len];
    let title = unsafe { std::str::from_utf8_unchecked(title_slice) };

    Some(title)
}
