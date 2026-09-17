// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{DisplayedGame, util::GIB};
use slint::{Image, ToSharedString};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;
use txbm_core::{
    covers,
    data_dir::DATA_DIR,
    game::{Game, GameFormat},
};

thread_local! {
    // Decoded thumbnails, keyed by their file path, so a list refresh reuses
    // the already-decoded `Image` instead of re-reading and re-decoding every
    // cover from disk. The stored mtime lets us drop an entry whose file was
    // rewritten in place (cover refreshed at the same path), so an updated
    // thumbnail is not masked by the previously decoded one. Only ever
    // touched from the UI thread (`From` runs there).
    static THUMB_CACHE: RefCell<HashMap<PathBuf, (SystemTime, Image)>> =
        RefCell::new(HashMap::new());
}

/// Drop every cached thumbnail. Call this after the on-disk cover cache is
/// wiped (e.g. "redownload all covers") so stale images are not reused.
pub fn clear_thumb_cache() {
    THUMB_CACHE.with(|c| c.borrow_mut().clear());
}

/// Load a thumbnail through the in-memory cache, decoding from disk at most
/// once per (path, mtime). A cached entry is reused only while the file's
/// mtime is unchanged, so a thumbnail rewritten in place is picked up.
fn load_thumbnail(path: PathBuf) -> Option<Image> {
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    if let Some(img) = THUMB_CACHE.with(|c| {
        c.borrow()
            .get(&path)
            .filter(|(t, _)| *t == mtime)
            .map(|(_, img)| img.clone())
    }) {
        return Some(img);
    }
    let img = Image::load_from_path(&path).ok()?;
    THUMB_CACHE.with(|c| c.borrow_mut().insert(path, (mtime, img.clone())));
    Some(img)
}

impl From<&Game> for DisplayedGame {
    fn from(game: &Game) -> Self {
        let covers_dir = DATA_DIR.join("covers");
        let cover = covers::cached_thumbnail(&covers_dir, &game.id)
            .and_then(load_thumbnail)
            .unwrap_or_default();

        Self {
            id: game.id.to_shared_string(),
            title: game.title.to_shared_string(),
            format: game.format.label().to_shared_string(),
            path: game.path.to_string_lossy().to_shared_string(),
            size_gib: game.size as f32 / GIB,
            is_x360: game.is_x360,
            is_arcade: game.format == GameFormat::Arcade,
            cover,
            incomplete: game.incomplete,
            // Filled in by `RefreshDisplayedGames`, which is the only place
            // that knows what the job queue is doing.
            busy: false,
        }
    }
}
