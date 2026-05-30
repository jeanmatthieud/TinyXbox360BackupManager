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
    let ptr = ALIGNED_BYTES.0.as_ptr().cast::<u32>();
    unsafe { std::slice::from_raw_parts(ptr, COUNT) }
}

#[inline(always)]
fn ghids() -> &'static [u32] {
    let ptr = ALIGNED_BYTES.0.as_ptr().cast::<u32>();
    unsafe { std::slice::from_raw_parts(ptr.add(COUNT), COUNT) }
}

#[inline(always)]
fn title_offsets() -> &'static [u32] {
    let ptr = ALIGNED_BYTES.0.as_ptr().cast::<u32>();
    unsafe { std::slice::from_raw_parts(ptr.add(COUNT * 2), COUNT) }
}

#[inline(always)]
fn title_lengths() -> &'static [u8] {
    &ALIGNED_BYTES.0[COUNT * 12..COUNT * 13]
}

#[inline(always)]
fn titles() -> &'static str {
    let slice = &ALIGNED_BYTES.0[COUNT * 13..];
    unsafe { std::str::from_utf8_unchecked(slice) }
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

    let title_offset = unsafe { *title_offsets().get_unchecked(idx) } as usize;
    let title_len = unsafe { *title_lengths().get_unchecked(idx) } as usize;
    let title = unsafe { titles().get_unchecked(title_offset..title_offset + title_len) };

    Some(title)
}
