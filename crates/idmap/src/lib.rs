// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
// SPDX-License-Identifier: GPL-3.0-only

#![warn(clippy::all, rust_2018_idioms)]

include!(concat!(env!("OUT_DIR"), "/id_map_meta.rs"));

use std::num::NonZeroU32;

#[repr(align(4))]
struct Data([u8; DATA_LEN]);

impl Data {
    #[inline]
    fn game_ids(&self) -> &[u32] {
        let ptr = self.0.as_ptr().cast::<u32>();
        unsafe { std::slice::from_raw_parts(ptr, COUNT) }
    }

    #[inline]
    fn ghids(&self) -> &[u32] {
        let ptr = self.0.as_ptr().cast::<u32>();
        unsafe { std::slice::from_raw_parts(ptr.add(COUNT), COUNT) }
    }

    #[inline]
    fn title_offsets(&self) -> &[u32] {
        let ptr = self.0.as_ptr().cast::<u32>();
        unsafe { std::slice::from_raw_parts(ptr.add(COUNT * 2), COUNT + 1) }
    }

    #[inline]
    fn titles(&self) -> &str {
        let slice = unsafe { self.0.get_unchecked(COUNT * 12 + 4..DATA_LEN) };
        unsafe { std::str::from_utf8_unchecked(slice) }
    }
}

static DATA: Data = Data(*include_bytes!(concat!(env!("OUT_DIR"), "/id_map.bin")));

fn find(id: u32) -> Option<usize> {
    DATA.game_ids().binary_search(&id).ok()
}

pub fn get_ghid(id: u32) -> Option<NonZeroU32> {
    let idx = find(id)?;

    let ghid = unsafe { *DATA.ghids().get_unchecked(idx) };

    NonZeroU32::new(ghid)
}

pub fn get_title(id: u32) -> Option<&'static str> {
    let idx = find(id)?;

    let start = unsafe { *DATA.title_offsets().get_unchecked(idx) } as usize;
    let end = unsafe { *DATA.title_offsets().get_unchecked(idx + 1) } as usize;
    let title = unsafe { DATA.titles().get_unchecked(start..end) };

    Some(title)
}
