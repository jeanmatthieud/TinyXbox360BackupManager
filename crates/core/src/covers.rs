// SPDX-License-Identifier: GPL-3.0-only

use crate::unity;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Path of the cached cover for a TitleID inside `covers_dir`, if any.
pub fn cached_cover(covers_dir: &Path, title_id: &str) -> Option<PathBuf> {
    for ext in ["png", "jpg"] {
        let path = covers_dir.join(format!("{title_id}.{ext}"));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Download the best XboxUnity cover for `title_id` into `covers_dir`.
/// Returns true if a new cover was downloaded.
pub fn download_cover(covers_dir: &Path, title_id: &str) -> Result<bool> {
    if cached_cover(covers_dir, title_id).is_some() {
        return Ok(false);
    }

    let bytes = unity::download_best_cover(title_id)?;
    let ext = if bytes.starts_with(&[0xFF, 0xD8]) {
        "jpg"
    } else {
        "png"
    };
    std::fs::create_dir_all(covers_dir)?;
    std::fs::write(covers_dir.join(format!("{title_id}.{ext}")), bytes)?;
    Ok(true)
}
