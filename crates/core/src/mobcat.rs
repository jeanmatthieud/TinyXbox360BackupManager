// SPDX-License-Identifier: GPL-3.0-only

//! Covers for Original Xbox games, from MobCat's community database
//! (https://github.com/MobCat/MobCats-original-xbox-game-list).
//!
//! The SQLite database maps a TitleID (8 hex chars) to XMIDs; box art
//! is served on raw.githubusercontent.com keyed by XMID:
//! `thumbnail/<XMID[0..2]>/<XMID>.jpg` (and `cover/` for full scans).

use crate::data_dir::DATA_DIR;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

const RAW_BASE: &str =
    "https://raw.githubusercontent.com/MobCat/MobCats-original-xbox-game-list/main";
const DB_URL: &str =
    "https://raw.githubusercontent.com/MobCat/MobCats-original-xbox-game-list/main/MobCatsOGXboxTitleIDs.db";
const USER_AGENT: &str = concat!(
    "TinyXbox360BackupManager/",
    env!("CARGO_PKG_VERSION")
);

/// The database is ~2 MiB; anything bigger than this is suspicious.
const DB_DOWNLOAD_LIMIT: u64 = 64 * 1024 * 1024;

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(USER_AGENT)
        .http_status_as_error(false)
        .build()
        .into()
});

pub fn db_path() -> PathBuf {
    DATA_DIR.join("mobcat.db")
}

fn etag_path() -> PathBuf {
    DATA_DIR.join("mobcat.db.etag")
}

/// Deletes the local database (covers cache clearing).
pub fn clear_db() {
    let _ = std::fs::remove_file(db_path());
    let _ = std::fs::remove_file(etag_path());
}

/// Downloads or refreshes the local database, silently: any network
/// error keeps the existing file (covers will simply stay missing or
/// slightly outdated until a later attempt succeeds).
pub fn ensure_db() {
    if let Err(e) = try_update_db() {
        eprintln!("MobCat database update failed: {e:#}");
    }
}

fn try_update_db() -> Result<()> {
    let db = db_path();
    let mut request = AGENT.get(DB_URL);
    if db.is_file()
        && let Ok(etag) = std::fs::read_to_string(etag_path())
    {
        request = request.header("If-None-Match", etag.trim());
    }

    let mut response = request.call().context("database request")?;
    match response.status().as_u16() {
        304 => return Ok(()),
        200 => {}
        status => bail!("unexpected HTTP status {status}"),
    }

    let etag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .body_mut()
        .with_config()
        .limit(DB_DOWNLOAD_LIMIT)
        .read_to_vec()
        .context("reading database")?;

    // Atomic replacement: a truncated download must not clobber
    // a working database.
    crate::data_dir::ensure_data_dir()?;
    let tmp = db.with_extension("db.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &db)?;
    if let Some(etag) = etag {
        let _ = std::fs::write(etag_path(), etag);
    }
    Ok(())
}

/// XMIDs known for a TitleID (one per region/release), or an empty
/// list if the database is absent or does not know the title.
fn xmids_for_title_id(title_id: &str) -> Vec<String> {
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        db_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT XMID FROM TitleIDs WHERE Title_ID = ?1 ORDER BY XMID",
    ) else {
        return Vec::new();
    };
    stmt.query_map([title_id], |row| row.get::<_, String>(0))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

/// Downloads the box art of an Original Xbox title (thumbnail first,
/// then full cover scan), trying every known XMID.
pub fn download_best_cover(title_id: &str) -> Result<Vec<u8>> {
    let xmids = xmids_for_title_id(title_id);
    if xmids.is_empty() {
        bail!("title {title_id} not in the MobCat database");
    }

    for xmid in &xmids {
        let publisher = &xmid[..2.min(xmid.len())];
        for kind in ["thumbnail", "cover"] {
            let url = format!("{RAW_BASE}/{kind}/{publisher}/{xmid}.jpg");
            let Ok(mut response) = AGENT.get(&url).call() else {
                continue;
            };
            if response.status().as_u16() != 200 {
                continue;
            }
            if let Ok(bytes) = response.body_mut().read_to_vec() {
                return Ok(bytes);
            }
        }
    }
    bail!("no cover found for title {title_id}")
}
