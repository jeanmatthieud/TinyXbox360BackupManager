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
        unsafe { std::slice::from_raw_parts(ptr.add(COUNT * 2), COUNT) }
    }

    #[inline]
    fn title_lengths(&self) -> &[u8] {
        &self.0[COUNT * 12..COUNT * 13]
    }

    #[inline]
    fn titles(&self) -> &str {
        let slice = &self.0[COUNT * 13..];
        unsafe { std::str::from_utf8_unchecked(slice) }
    }
}

static DATA: Data = Data(*include_bytes!(concat!(env!("OUT_DIR"), "/id_map.bin")));

fn find(id: u32) -> Option<usize> {
    DATA.game_ids().binary_search(&id).ok()
}

pub fn get_ghid(id: u32) -> Option<NonZeroU32> {
    let idx = find(id)?;
    let ghid = DATA.ghids()[idx];
    NonZeroU32::new(ghid)
}

pub fn get_title(id: u32) -> Option<&'static str> {
    let idx = find(id)?;

    let title_offset = DATA.title_offsets()[idx] as usize;
    let title_len = DATA.title_lengths()[idx] as usize;

    let title = &DATA.titles()[title_offset..title_offset + title_len];
    Some(title)
}
