// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
// SPDX-License-Identifier: GPL-3.0-only

pub mod meta;

use crate::{config::SortBy, homebrew::meta::HomebrewAppMeta};
use std::{
    cmp::Ordering,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct HomebrewApp {
    pub path: PathBuf,
    pub meta: HomebrewAppMeta,
    pub size: u64,
    pub icon_bytes: Option<Vec<u8>>,
    pub search_term: String,
}

impl HomebrewApp {
    pub fn from_path(path: impl Into<PathBuf>) -> Option<Self> {
        let path = path.into();

        let meta = HomebrewAppMeta::parse(&path).ok()?;
        let size = fs_extra::dir::get_size(&path).ok()?;
        let icon_bytes = fs::read(path.join("icon.png")).ok();
        let search_term = format!("{}\0{}", &meta.name, &meta.coder).to_lowercase();

        Some(Self {
            path,
            meta,
            size,
            icon_bytes,
            search_term,
        })
    }
}

pub fn scan_dir(path: impl AsRef<Path>) -> impl Iterator<Item = HomebrewApp> {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(HomebrewApp::from_path)
}

pub fn get_compare_fn(sort_by: SortBy) -> impl FnMut(&HomebrewApp, &HomebrewApp) -> Ordering {
    move |a, b| match sort_by {
        SortBy::NameDescending => a.meta.name.cmp(&b.meta.name),
        SortBy::NameAscending => b.meta.name.cmp(&a.meta.name),
        SortBy::SizeDescending => a.size.cmp(&b.size),
        SortBy::SizeAscending => b.size.cmp(&a.size),
    }
}
