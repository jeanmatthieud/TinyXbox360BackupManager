// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
// SPDX-License-Identifier: GPL-3.0-only

#![warn(clippy::all, rust_2018_idioms)]

use rkyv::{Archive, Deserialize, vec::ArchivedVec};
use std::num::NonZeroU32;

const BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/id_map.bin"));

#[derive(Archive, Deserialize)]
struct GameEntry {
    id: u32,
    pub ghid: Option<NonZeroU32>,
    pub title: String,
}

pub fn get_title(id: impl Into<u32>) -> Option<&'static str> {
    let archived = unsafe { rkyv::access_unchecked::<ArchivedVec<ArchivedGameEntry>>(BYTES) };
    let id = id.into().into();
    let i = archived.binary_search_by_key(&id, |e| e.id).ok()?;
    Some(&archived[i].title)
}

pub fn get_ghid(id: impl Into<u32>) -> Option<u32> {
    let archived = unsafe { rkyv::access_unchecked::<ArchivedVec<ArchivedGameEntry>>(BYTES) };
    let id = id.into().into();
    let i = archived.binary_search_by_key(&id, |e| e.id).ok()?;
    archived[i].ghid.as_ref().map(|ghid| ghid.get())
}
