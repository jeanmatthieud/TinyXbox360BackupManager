// SPDX-License-Identifier: GPL-3.0-only

//! What the Xbox 360 marketplace knew about a title: its artwork and its
//! description, for Aurora's import folder (see [`crate::aurora_import`]).
//!
//! Both sources are keyed by the marketplace product of the game, a GUID
//! derived from the TitleID alone (`66acd000-77fe-1000-9115-d802<TitleID>`):
//!
//! - the artwork still sits on the marketplace's own CDN,
//!   `download.xbox.com/content/images/<product>/1033/` — plain HTTP only, and
//!   Microsoft may switch it off without notice;
//! - the catalogue itself is gone, but <https://dbox.tools> archived it and
//!   serves each product as JSON.
//!
//! Neither is guaranteed, hence the three-way answer of the functions below:
//! `Ok(Some(_))` when the source has it, `Ok(None)` when it answered that it
//! does not (original Xbox games, homebrew), and `Err(_)` when it could not be
//! asked — which is no reason to conclude a title has nothing. Whatever was
//! found is cached under [`DATA_DIR`], so a second console costs no download.

use crate::data_dir::DATA_DIR;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

const CDN_BASE: &str = "http://download.xbox.com/content/images";
const DBOX_BASE: &str = "https://dbox.tools/api/marketplace/products";
const USER_AGENT: &str = concat!("TinyXbox360BackupManager/", env!("CARGO_PKG_VERSION"));

/// The largest file, a 1280x720 background, stays well under a MiB; a product
/// record is a few dozen KiB.
const DOWNLOAD_LIMIT: u64 = 16 * 1024 * 1024;

/// Locale read when the asked-for one is missing from a product: the one
/// every product carries.
const FALLBACK_LOCALE: &str = "en-us";

/// Locales the catalogue was published in and the app offers, as
/// `(locale, name)`. The marketplace had more (regional variants of these);
/// the texts are the same.
pub const LOCALES: [(&str, &str); 10] = [
    ("en-us", "English"),
    ("fr-fr", "Français"),
    ("de-de", "Deutsch"),
    ("es-es", "Español"),
    ("it-it", "Italiano"),
    ("pt-br", "Português"),
    ("nl-nl", "Nederlands"),
    ("pl-pl", "Polski"),
    ("ru-ru", "Русский"),
    ("ja-jp", "日本語"),
];

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent(USER_AGENT)
        .http_status_as_error(false)
        .build()
        .into()
});

/// One picture of a title, as the marketplace published it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Artwork {
    /// 64x64 tile.
    Icon,
    /// 420x95 banner.
    Banner,
    /// 1280x720 background.
    Background,
}

impl Artwork {
    pub const ALL: [Artwork; 3] = [Artwork::Icon, Artwork::Banner, Artwork::Background];

    /// File name on the CDN, also used for the cache.
    fn cdn_name(self) -> &'static str {
        match self {
            Artwork::Icon => "tile.png",
            Artwork::Banner => "banner.png",
            Artwork::Background => "background.jpg",
        }
    }
}

/// The catalogue entry of a title, reduced to what Aurora can import. Any
/// field may be empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameInfo {
    pub title: String,
    pub description: String,
    pub publisher: String,
    pub developer: String,
    /// `YYYY-MM-DD`. For a game sold as Games on Demand this is the day it
    /// reached the marketplace, which can be years after its disc release.
    pub release_date: String,
    /// Named the way Aurora expects them (see [`aurora_genre`]).
    pub genres: Vec<String>,
}

/// Marketplace product of a game.
fn product_id(title_id: &str) -> String {
    format!("66acd000-77fe-1000-9115-d802{}", title_id.to_lowercase())
}

fn cache_dir(title_id: &str) -> PathBuf {
    DATA_DIR.join("marketplace").join(title_id.to_uppercase())
}

/// GETs `url`: the body on a 200, `None` on a 404, an error on anything else.
fn fetch(url: &str) -> Result<Option<Vec<u8>>> {
    let mut response = AGENT.get(url).call().with_context(|| format!("requesting {url}"))?;
    match response.status().as_u16() {
        200 => {}
        404 => return Ok(None),
        status => bail!("{url} answered {status}"),
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .read_to_vec()
        .with_context(|| format!("reading {url}"))?;
    Ok(Some(bytes))
}

/// File extension matching what the bytes really are, or `None` when they are
/// neither a PNG nor a JPEG (an error page served with a 200, say).
pub fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        Some("jpg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else {
        None
    }
}

/// One picture of a title (see the module docs for the three outcomes).
pub fn artwork(title_id: &str, kind: Artwork) -> Result<Option<Vec<u8>>> {
    let cached = cache_dir(title_id).join(kind.cdn_name());
    if let Ok(bytes) = std::fs::read(&cached) {
        return Ok(Some(bytes));
    }

    let url = format!("{CDN_BASE}/{}/1033/{}", product_id(title_id), kind.cdn_name());
    let mut bytes = fetch(&url)?;
    // The XboxUnity mirror archived the very same tiles, and will outlive the
    // CDN: worth a second request for the one picture every game list shows.
    if bytes.is_none() && kind == Artwork::Icon {
        bytes = fetch(&crate::unity_mirror::icon_url(title_id))?;
    }

    let Some(bytes) = bytes.filter(|b| image_extension(b).is_some()) else {
        return Ok(None);
    };
    if let Some(dir) = cached.parent()
        && std::fs::create_dir_all(dir).is_ok()
    {
        let _ = std::fs::write(&cached, &bytes);
    }
    Ok(Some(bytes))
}

/// The catalogue entry of a title, its texts in `locale` when the product
/// has them, in English otherwise (see the module docs for the three
/// outcomes).
pub fn game_info(title_id: &str, locale: &str) -> Result<Option<GameInfo>> {
    // The record is cached as served, every language in it: a change of
    // language, or of what is made of the record, costs no download.
    let cached = cache_dir(title_id).join("product.json");
    let bytes = match std::fs::read(&cached) {
        Ok(bytes) => bytes,
        Err(_) => {
            let url = format!("{DBOX_BASE}/{}", product_id(title_id));
            let Some(bytes) = fetch(&url)? else {
                return Ok(None);
            };
            if let Some(dir) = cached.parent()
                && std::fs::create_dir_all(dir).is_ok()
            {
                let _ = std::fs::write(&cached, &bytes);
            }
            bytes
        }
    };
    let product: Product = serde_json::from_slice(&bytes).context("invalid product record")?;
    Ok(Some(product.localized(&locale.to_lowercase())))
}

/// A product as dbox.tools serves it — only the fields read here.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Product {
    /// dbox.tools' product type: 1 for a retail game, 14 for an Arcade one.
    product_type: Option<u32>,
    default_title: Option<String>,
    developer_name: Option<String>,
    publisher_name: Option<String>,
    global_original_release_date: Option<String>,
    categories: Vec<u32>,
    localizations: Vec<Localization>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Localization {
    locale: String,
    full_title: Option<String>,
    reduced_title: Option<String>,
    reduced_description: Option<String>,
    full_description: Option<String>,
}

impl Product {
    fn localized(self, locale: &str) -> GameInfo {
        let product = self;
        let find = |locale: &str| {
            product
                .localizations
                .iter()
                .find(|l| l.locale.eq_ignore_ascii_case(locale))
        };
        let localization = find(locale)
            .or_else(|| find(FALLBACK_LOCALE))
            .or(product.localizations.first());
        // The short text first: most Aurora skins show eight lines at most.
        let description = localization
            .and_then(|l| {
                [&l.reduced_description, &l.full_description]
                    .into_iter()
                    .flatten()
                    .map(|text| single_spaced(text))
                    .find(|text| !text.is_empty())
            })
            .unwrap_or_default();

        // A localized title when the product has one ("Les Sims 3").
        let title = localization
            .and_then(|l| l.full_title.as_deref())
            .filter(|title| !title.trim().is_empty())
            .or(product.default_title.as_deref())
            .map(single_spaced)
            .unwrap_or_default();
        let reduced = localization
            .and_then(|l| l.reduced_title.as_deref())
            .map(single_spaced)
            .unwrap_or_default();
        let title = if product.product_type == Some(ARCADE_FULL_GAME) {
            strip_full_game_prefix(&title, &reduced)
        } else {
            title
        };

        GameInfo {
            title,
            description,
            publisher: single_spaced(product.publisher_name.as_deref().unwrap_or_default()),
            developer: single_spaced(product.developer_name.as_deref().unwrap_or_default()),
            // "2010-02-09T00:00:00Z": the date alone.
            release_date: product
                .global_original_release_date
                .as_deref()
                .and_then(|date| date.get(..10))
                .filter(|date| date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-')
                .unwrap_or_default()
                .to_string(),
            genres: product
                .categories
                .iter()
                .filter_map(|id| aurora_genre(*id))
                .map(str::to_string)
                .collect(),
        }
    }
}

/// dbox.tools' product type of an Arcade game sold as a whole.
const ARCADE_FULL_GAME: u32 = 14;

/// The title of an Arcade game without the "Full Game - " the marketplace put
/// in front of it — in the catalogue's every language ("Version complète - ",
/// "Vollversion - ", "完全版 - "), and now and then twice over ("Full Game -
/// Full Game - Sonic The Hedgehog"). `reduced` is the product's short title,
/// which never carries the prefix but is sometimes abbreviated ("PAC-MAN CE
/// DX+"): when the full title ends with it, what precedes is the prefix;
/// otherwise everything up to the first " - " goes, as many times as the same
/// words repeat.
fn strip_full_game_prefix(title: &str, reduced: &str) -> String {
    if !reduced.is_empty()
        && title.len() > reduced.len()
        && title.to_lowercase().ends_with(&reduced.to_lowercase())
    {
        return title[title.len() - reduced.len()..].to_string();
    }
    let Some((prefix, mut rest)) = title.split_once(" - ") else {
        return title.to_string();
    };
    while let Some(again) = rest.strip_prefix(prefix).and_then(|r| r.strip_prefix(" - ")) {
        rest = again;
    }
    rest.to_string()
}

/// Trims `text` and folds every run of whitespace into one space: the
/// catalogue's texts are single paragraphs padded with double spaces.
fn single_spaced(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Genre Aurora knows for a dbox.tools category, by the id the product
/// records carry (dbox.tools' own numbering of the marketplace's "Xbox LIVE
/// Games" genres). The names are Aurora's: it shows anything else as "Not
/// Available". Categories it has no genre for (Kinect, Avatar, Educational,
/// and everything that is not a genre) yield `None`.
fn aurora_genre(category: u32) -> Option<&'static str> {
    Some(match category {
        8 => "Shooter",
        9 => "Action & Adventure",
        14 => "Sports & Recreation",
        15 => "Other",
        19 => "Family",
        20 => "Strategy & Simulation",
        22 => "Puzzle & Trivia",
        25 => "Role Playing",
        26 => "Fighting",
        29 => "Music",
        30 => "Racing & Flying",
        37 => "Platformer",
        46 => "Board & Card Games",
        49 => "Classics",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::strip_full_game_prefix;

    #[test]
    fn full_game_prefix() {
        assert_eq!(strip_full_game_prefix("Full Game - Super Meat Boy", "Super Meat Boy"), "Super Meat Boy");
        assert_eq!(strip_full_game_prefix("Version complète - Super Meat Boy", "Super Meat Boy"), "Super Meat Boy");
        // Abbreviated short title: the generic cut.
        assert_eq!(
            strip_full_game_prefix("Full Game - PAC-MAN Championship Edition DX+", "PAC-MAN CE DX+"),
            "PAC-MAN Championship Edition DX+"
        );
        // Doubled prefix, both ways.
        assert_eq!(strip_full_game_prefix("Full Game - Full Game - Sonic", "Sonic"), "Sonic");
        assert_eq!(strip_full_game_prefix("完全版 - 完全版 - ソニック", ""), "ソニック");
        // A dash inside the title itself survives.
        assert_eq!(strip_full_game_prefix("Full Game - Castle - Remastered", ""), "Castle - Remastered");
        assert_eq!(strip_full_game_prefix("Community Game", "Community Game"), "Community Game");
    }
}
