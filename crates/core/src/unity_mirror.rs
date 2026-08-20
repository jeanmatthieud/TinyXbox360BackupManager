// SPDX-License-Identifier: GPL-3.0-only

//! Cover source backed by the XboxUnity-Scraper archive
//! (https://github.com/UncreativeXenon/XboxUnity-Scraper), a static mirror of
//! XboxUnity's assets served from raw.githubusercontent.com.
//!
//! Layout: `Covers/<TitleID>/metadata.json` lists the covers of a title with
//! the exact same schema as XboxUnity's `CoverInfo.php`, so the entries and
//! the "best cover" ranking are shared with [`crate::unity`]. The images
//! themselves live in `Covers/<TitleID>/{Small,Medium,Large}/<CoverID>.png`
//! (180x120, 300x200 and 900x600); `Large` holds the very files XboxUnity
//! serves for `size=large`, which is what the cache expects.
//!
//! Note that files are named `.png` whatever their real format — many are in
//! fact JPEG. Callers sniff the magic bytes, so this is harmless.

use crate::unity::{CoverEntry, sort_covers_best_first};
use anyhow::{Context, Result, bail};
use std::sync::LazyLock;
use std::time::Duration;

const RAW_BASE: &str =
    "https://raw.githubusercontent.com/UncreativeXenon/XboxUnity-Scraper/master";
const USER_AGENT: &str = concat!(
    "TinyXbox360BackupManager/",
    env!("CARGO_PKG_VERSION")
);

/// A `Large` cover tops out at a couple of MiB.
const DOWNLOAD_LIMIT: u64 = 64 * 1024 * 1024;

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(USER_AGENT)
        .http_status_as_error(false)
        .build()
        .into()
});

#[derive(Debug, serde::Deserialize)]
struct MetadataResponse {
    #[serde(rename = "Covers")]
    covers: Vec<CoverEntry>,
}

/// List of covers archived for a TitleID.
pub fn cover_info(title_id: &str) -> Result<Vec<CoverEntry>> {
    let title_id = title_id.to_uppercase();
    let mut response = AGENT
        .get(format!("{RAW_BASE}/Covers/{title_id}/metadata.json"))
        .call()
        .context("metadata request")?;
    if response.status().as_u16() != 200 {
        bail!("title {title_id} is not in the XboxUnity mirror");
    }
    let metadata: MetadataResponse = response
        .body_mut()
        .read_json()
        .context("invalid metadata.json")?;
    Ok(metadata.covers)
}

/// URL of an archived cover. `Large` is the full-resolution image (900x600),
/// the same file XboxUnity serves as `size=large`.
pub fn cover_url(title_id: &str, cover_id: &str) -> String {
    format!(
        "{RAW_BASE}/Covers/{}/Large/{cover_id}.png",
        title_id.to_uppercase()
    )
}

/// Downloads the best cover of a title (official first, then best rating),
/// falling back to the next candidate whenever an image is missing from the
/// archive.
pub fn download_best_cover(title_id: &str) -> Result<Vec<u8>> {
    let mut covers = cover_info(title_id)?;
    if covers.is_empty() {
        bail!("no cover for title {title_id}");
    }
    sort_covers_best_first(&mut covers);

    for cover in &covers {
        let Ok(mut response) = AGENT.get(cover_url(title_id, &cover.cover_id)).call() else {
            continue;
        };
        if response.status().as_u16() != 200 {
            continue;
        }
        if let Ok(bytes) = response
            .body_mut()
            .with_config()
            .limit(DOWNLOAD_LIMIT)
            .read_to_vec()
        {
            return Ok(bytes);
        }
    }
    bail!("no downloadable cover for title {title_id}")
}
