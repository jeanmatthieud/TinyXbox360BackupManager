// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
// SPDX-License-Identifier: GPL-3.0-only

#![warn(clippy::all, rust_2018_idioms)]

use rkyv::{Archive, Deserialize, primitive::ArchivedU32, vec::ArchivedVec};
use std::num::NonZeroU32;

#[repr(align(4))]
struct AlignedBytes<const N: usize>([u8; N]);

static ALIGNED_BYTES: &AlignedBytes<
    { include_bytes!(concat!(env!("OUT_DIR"), "/id_map.bin")).len() },
> = &AlignedBytes(*include_bytes!(concat!(env!("OUT_DIR"), "/id_map.bin")));

#[derive(Archive, Deserialize)]
struct GameEntry {
    id: u32,
    pub ghid: Option<NonZeroU32>,
    pub title: String,
}

fn get(id: ArchivedU32) -> Option<&'static ArchivedGameEntry> {
    let archived =
        unsafe { rkyv::access_unchecked::<ArchivedVec<ArchivedGameEntry>>(&ALIGNED_BYTES.0) };

    match archived.binary_search_by_key(&id, |e| e.id) {
        Ok(i) => Some(&archived[i]),
        Err(_) => None,
    }
}

pub fn get_title(id: impl Into<u32>) -> Option<&'static str> {
    get(id.into().into()).map(|e| e.title.as_str())
}

pub fn get_ghid(id: impl Into<u32>) -> Option<u32> {
    get(id.into().into())
        .and_then(|e| e.ghid.as_ref())
        .map(|ghid| ghid.get())
}
