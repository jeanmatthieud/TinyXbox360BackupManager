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

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct GameEntry(usize);

impl GameEntry {
    pub fn lookup_str(id: impl AsRef<str>) -> Option<Self> {
        let id = u32::from_str_radix(id.as_ref(), 36).ok()?;
        Self::lookup(id)
    }

    pub fn lookup(id: u32) -> Option<Self> {
        let idx = DATA.game_ids().binary_search(&id).ok()?;
        Some(GameEntry(idx))
    }

    pub fn ghid(&self) -> Option<NonZeroU32> {
        let ghid = unsafe { *DATA.ghids().get_unchecked(self.0) };
        NonZeroU32::new(ghid)
    }

    pub fn title(&self) -> &'static str {
        let start = unsafe { *DATA.title_offsets().get_unchecked(self.0) } as usize;
        let end = unsafe { *DATA.title_offsets().get_unchecked(self.0 + 1) } as usize;
        unsafe { DATA.titles().get_unchecked(start..end) }
    }
}
