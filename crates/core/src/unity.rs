// SPDX-License-Identifier: GPL-3.0-only

//! Client for the XboxUnity API (https://www.xboxunity.net).
//! Endpoints observed in doc/www.xboxunity.net.har, documented in doc/assets-url.md.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::sync::LazyLock;
use std::time::Duration;

const BASE: &str = "https://www.xboxunity.net/Resources/Lib";
const USER_AGENT: &str = concat!(
    "TinyXbox360BackupManager/",
    env!("CARGO_PKG_VERSION")
);

/// Download limit (some title updates exceed 100 MiB).
const DOWNLOAD_LIMIT: u64 = 1024 * 1024 * 1024;

/// Shared HTTP agent, with timeouts (the server sometimes leaves
/// a reused connection without response indefinitely).
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(300)))
        .user_agent(USER_AGENT)
        .build()
        .into()
});

#[derive(Debug, Clone, Deserialize)]
pub struct TitleListItem {
    #[serde(rename = "TitleID")]
    pub title_id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "TitleType")]
    pub title_type: String,
    #[serde(rename = "Covers")]
    pub covers: String,
    #[serde(rename = "Updates")]
    pub updates: String,
}

#[derive(Debug, Deserialize)]
struct TitleListResponse {
    #[serde(rename = "Items")]
    items: Vec<TitleListItem>,
}

/// Search for titles (by name or hexadecimal TitleID).
pub fn search_titles(query: &str) -> Result<Vec<TitleListItem>> {
    let response: TitleListResponse = AGENT.get(format!("{BASE}/TitleList.php"))
        .query("page", "0")
        .query("count", "25")
        .query("search", query)
        .query("sort", "3")
        .query("direction", "1")
        .query("category", "0")
        .query("filter", "0")
        .call()
        .context("TitleList request")?
        .body_mut()
        .read_json()
        .context("invalid TitleList response")?;
    Ok(response.items)
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoverEntry {
    #[serde(rename = "CoverID")]
    pub cover_id: String,
    /// Some API fields are sometimes `null`.
    #[serde(rename = "Rating")]
    pub rating: Option<String>,
    #[serde(rename = "Official")]
    pub official: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CoverInfoResponse {
    #[serde(rename = "Covers")]
    covers: Vec<CoverEntry>,
}

/// List of available covers for a TitleID.
pub fn cover_info(title_id: &str) -> Result<Vec<CoverEntry>> {
    let response: CoverInfoResponse = AGENT.get(format!("{BASE}/CoverInfo.php"))
        .query("titleid", title_id)
        .call()
        .context("CoverInfo request")?
        .body_mut()
        .read_json()
        .context("invalid CoverInfo response")?;
    Ok(response.covers)
}

/// URL of a cover (without the `dl` flag for direct display).
pub fn cover_url(cover_id: &str, large: bool) -> String {
    let size = if large { "large" } else { "small" };
    format!("{BASE}/Cover.php?cid={cover_id}&size={size}")
}

/// Downloads the best cover of a title
/// (official first, then best rating).
pub fn download_best_cover(title_id: &str) -> Result<Vec<u8>> {
    let mut covers = cover_info(title_id)?;
    if covers.is_empty() {
        bail!("no cover for title {title_id}");
    }
    covers.sort_by_key(|c| {
        let official = c.official.as_deref() == Some("1");
        let rating: i32 = c
            .rating
            .as_deref()
            .and_then(|r| r.parse().ok())
            .unwrap_or(0);
        (std::cmp::Reverse(official as i32), std::cmp::Reverse(rating))
    });

    let bytes = AGENT.get(cover_url(&covers[0].cover_id, true))
        .call()
        .context("downloading cover")?
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .read_to_vec()
        .context("reading cover")?;
    Ok(bytes)
}

#[derive(Debug, Clone, Deserialize)]
pub struct TitleUpdateEntry {
    #[serde(rename = "TitleUpdateID")]
    pub title_update_id: String,
    #[serde(rename = "Version")]
    pub version: Option<String>,
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "Size")]
    pub size: Option<String>,
    /// MediaID this update applies to.
    #[serde(skip)]
    pub media_id: String,
}

#[derive(Debug, Deserialize)]
struct MediaIdUpdates {
    #[serde(rename = "MediaID")]
    media_id: String,
    #[serde(rename = "Updates")]
    updates: Vec<TitleUpdateEntry>,
}

#[derive(Debug, Deserialize)]
struct TitleUpdateInfoResponse {
    #[serde(rename = "MediaIDS")]
    media_ids: Vec<MediaIdUpdates>,
}

/// List of title updates for a title, all MediaIDs combined.
pub fn title_updates(title_id: &str) -> Result<Vec<TitleUpdateEntry>> {
    let response: TitleUpdateInfoResponse = AGENT.get(format!("{BASE}/TitleUpdateInfo.php"))
        .query("titleid", title_id)
        .call()
        .context("TitleUpdateInfo request")?
        .body_mut()
        .read_json()
        .context("invalid TitleUpdateInfo response")?;

    let mut updates = Vec::new();
    for group in response.media_ids {
        for mut update in group.updates {
            update.media_id = group.media_id.clone();
            updates.push(update);
        }
    }
    Ok(updates)
}

/// Downloads a title update. Returns (filename, data).
/// The filename (from Content-Disposition) must be kept as is.
pub fn download_title_update(title_update_id: &str) -> Result<(String, Vec<u8>)> {
    let mut response = AGENT.get(format!("{BASE}/TitleUpdate.php"))
        .query("tuid", title_update_id)
        .call()
        .context("downloading title update")?;

    let filename = response
        .headers()
        .get("content-disposition")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_disposition_filename)
        .unwrap_or_else(|| format!("TU_{title_update_id}"));

    let bytes = response
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .read_to_vec()
        .context("reading title update")?;
    Ok((filename, bytes))
}

fn parse_content_disposition_filename(value: &str) -> Option<String> {
    let start = value.to_lowercase().find("filename=")?;
    let name = value[start + "filename=".len()..]
        .trim()
        .trim_matches('"')
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}
