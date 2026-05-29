// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
// SPDX-License-Identifier: GPL-3.0-only

#![warn(clippy::all, rust_2018_idioms)]

include!(concat!(env!("OUT_DIR"), "/id_map_meta.rs"));

use std::num::NonZeroU32;

#[repr(align(4))]
struct AlignedBytes<const N: usize>([u8; N]);

static ALIGNED_BYTES: AlignedBytes<BIN_LEN> =
    AlignedBytes(*include_bytes!(concat!(env!("OUT_DIR"), "/id_map.bin")));

#[inline(always)]
fn game_ids() -> &'static [u32] {
    unsafe { std::slice::from_raw_parts(ALIGNED_BYTES.0.as_ptr().cast::<u32>(), COUNT) }
}

#[inline(always)]
fn ghids() -> &'static [u32] {
    unsafe { std::slice::from_raw_parts(ALIGNED_BYTES.0.as_ptr().cast::<u32>().add(COUNT), COUNT) }
}

#[inline(always)]
fn title_offsets() -> &'static [u32] {
    unsafe {
        std::slice::from_raw_parts(ALIGNED_BYTES.0.as_ptr().cast::<u32>().add(COUNT * 2), COUNT)
    }
}

#[inline(always)]
fn title_lengths() -> &'static [u8] {
    unsafe { std::slice::from_raw_parts(ALIGNED_BYTES.0.as_ptr().add(COUNT * 12), COUNT) }
}

fn find(id: u32) -> Option<usize> {
    game_ids().binary_search(&id).ok()
}

pub fn get_ghid(id: u32) -> Option<NonZeroU32> {
    let idx = find(id)?;
    let ghid = unsafe { *ghids().get_unchecked(idx) };
    NonZeroU32::new(ghid)
}

pub fn get_title(id: u32) -> Option<&'static str> {
    let idx = find(id)?;

    let relative_title_offset = unsafe { *title_offsets().get_unchecked(idx) } as usize;
    let title_len = unsafe { *title_lengths().get_unchecked(idx) } as usize;

    let title_offset = COUNT * 13 + relative_title_offset;

    let title_slice = unsafe {
        ALIGNED_BYTES
            .0
            .get_unchecked(title_offset..title_offset + title_len)
    };

    let title = unsafe { std::str::from_utf8_unchecked(title_slice) };

    Some(title)
}
