// SPDX-License-Identifier: GPL-3.0-only

use crate::data_dir::DATA_DIR;
use crate::{mobcat, unity};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Max width (in pixels) of the cached cover thumbnails. Full XboxUnity
/// wraps can be several thousand pixels wide; decoding them on the UI
/// thread at every list refresh is what makes the grid lag. We downscale
/// them once, off the UI thread, into `covers/thumbs/`.
pub const THUMB_WIDTH: u32 = 1024;

/// Path of the cached cover for a TitleID inside `covers_dir`, if any.
pub fn cached_cover(covers_dir: &Path, title_id: &str) -> Option<PathBuf> {
    if title_id.is_empty() {
        return None;
    }
    for ext in ["png", "jpg"] {
        let path = covers_dir.join(format!("{title_id}.{ext}"));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Path of the downscaled cover thumbnail for a TitleID, if it exists.
pub fn cached_thumbnail(covers_dir: &Path, title_id: &str) -> Option<PathBuf> {
    if title_id.is_empty() {
        return None;
    }
    let path = covers_dir.join("thumbs").join(format!("{title_id}.png"));
    path.is_file().then_some(path)
}

/// Ensure a downscaled PNG thumbnail (at most `THUMB_WIDTH` wide, aspect
/// ratio preserved) exists for the cached cover of `title_id`. Returns
/// true if one was just created.
///
/// The heavy decode + resize runs on the caller's thread, so this MUST be
/// called off the UI thread. The result is written atomically (temp file +
/// rename) so a concurrent UI-thread load never sees a half-written PNG.
pub fn ensure_thumbnail(covers_dir: &Path, title_id: &str) -> Result<bool> {
    if title_id.is_empty() || cached_thumbnail(covers_dir, title_id).is_some() {
        return Ok(false);
    }
    let Some(cover) = cached_cover(covers_dir, title_id) else {
        return Ok(false);
    };

    let img = image::open(&cover)?;
    let img = if img.width() > THUMB_WIDTH {
        let height =
            (u64::from(img.height()) * u64::from(THUMB_WIDTH) / u64::from(img.width())).max(1) as u32;
        img.resize_exact(THUMB_WIDTH, height, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let dir = covers_dir.join("thumbs");
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{title_id}.png.tmp"));
    img.save_with_format(&tmp, image::ImageFormat::Png)?;
    std::fs::rename(&tmp, dir.join(format!("{title_id}.png")))?;
    Ok(true)
}

/// Download the best cover for `title_id` into `covers_dir`:
/// XboxUnity for Xbox 360 titles, MobCat's database for Original Xbox
/// (with XboxUnity as fallback — OG titles republished as GOD have
/// covers there too). Returns true if a new cover was downloaded.
pub fn download_cover(covers_dir: &Path, title_id: &str, is_x360: bool) -> Result<bool> {
    if cached_cover(covers_dir, title_id).is_some() {
        return Ok(false);
    }

    let bytes = if is_x360 {
        unity::download_best_cover(title_id)?
    } else {
        mobcat::download_best_cover(title_id)
            .or_else(|_| unity::download_best_cover(title_id))?
    };
    let ext = if bytes.starts_with(&[0xFF, 0xD8]) {
        "jpg"
    } else {
        "png"
    };
    std::fs::create_dir_all(covers_dir)?;
    std::fs::write(covers_dir.join(format!("{title_id}.{ext}")), bytes)?;
    Ok(true)
}

/// Persistent map `"host|remote path"` → TitleID, so an Original Xbox
/// game added by hand over FTP only costs one `default.xbe` download
/// ever, instead of one per scan.
pub struct XbeIdCache {
    path: PathBuf,
    map: HashMap<String, String>,
}

impl XbeIdCache {
    pub fn load() -> Self {
        let path = DATA_DIR.join("xbe-ids.json");
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, map }
    }

    pub fn get(&self, host: &str, remote_path: &str) -> Option<&String> {
        self.map.get(&Self::key(host, remote_path))
    }

    pub fn insert(&mut self, host: &str, remote_path: &str, title_id: String) {
        self.map.insert(Self::key(host, remote_path), title_id);
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.map) {
            let _ = crate::data_dir::ensure_data_dir();
            let _ = std::fs::write(&self.path, json);
        }
    }

    fn key(host: &str, remote_path: &str) -> String {
        format!("{host}|{remote_path}")
    }
}
