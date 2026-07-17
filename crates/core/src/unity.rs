// SPDX-License-Identifier: GPL-3.0-only

//! Client pour l'API XboxUnity (https://www.xboxunity.net).
//! Endpoints observés dans doc/www.xboxunity.net.har, documentés dans doc/assets-url.md.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::sync::LazyLock;
use std::time::Duration;

const BASE: &str = "https://www.xboxunity.net/Resources/Lib";
const USER_AGENT: &str = concat!(
    "TinyXbox360BackupManager/",
    env!("CARGO_PKG_VERSION")
);

/// Limite de téléchargement (certains title updates dépassent 100 Mio).
const DOWNLOAD_LIMIT: u64 = 1024 * 1024 * 1024;

/// Agent HTTP partagé, avec timeouts (le serveur laisse parfois
/// une connexion réutilisée sans réponse indéfiniment).
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

/// Recherche de titres (par nom ou par TitleID hexadécimal).
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
        .context("requête TitleList")?
        .body_mut()
        .read_json()
        .context("réponse TitleList invalide")?;
    Ok(response.items)
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoverEntry {
    #[serde(rename = "CoverID")]
    pub cover_id: String,
    /// Certains champs de l'API valent parfois `null`.
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

/// Liste des jaquettes disponibles pour un TitleID.
pub fn cover_info(title_id: &str) -> Result<Vec<CoverEntry>> {
    let response: CoverInfoResponse = AGENT.get(format!("{BASE}/CoverInfo.php"))
        .query("titleid", title_id)
        .call()
        .context("requête CoverInfo")?
        .body_mut()
        .read_json()
        .context("réponse CoverInfo invalide")?;
    Ok(response.covers)
}

/// URL d'une jaquette (sans le flag `dl` pour un affichage direct).
pub fn cover_url(cover_id: &str, large: bool) -> String {
    let size = if large { "large" } else { "small" };
    format!("{BASE}/Cover.php?cid={cover_id}&size={size}")
}

/// Télécharge la meilleure jaquette d'un titre
/// (officielle d'abord, puis meilleure note).
pub fn download_best_cover(title_id: &str) -> Result<Vec<u8>> {
    let mut covers = cover_info(title_id)?;
    if covers.is_empty() {
        bail!("aucune jaquette pour le titre {title_id}");
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
        .context("téléchargement de la jaquette")?
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .read_to_vec()
        .context("lecture de la jaquette")?;
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
    /// MediaID auquel cette mise à jour s'applique.
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

/// Liste des title updates d'un titre, tous MediaID confondus.
pub fn title_updates(title_id: &str) -> Result<Vec<TitleUpdateEntry>> {
    let response: TitleUpdateInfoResponse = AGENT.get(format!("{BASE}/TitleUpdateInfo.php"))
        .query("titleid", title_id)
        .call()
        .context("requête TitleUpdateInfo")?
        .body_mut()
        .read_json()
        .context("réponse TitleUpdateInfo invalide")?;

    let mut updates = Vec::new();
    for group in response.media_ids {
        for mut update in group.updates {
            update.media_id = group.media_id.clone();
            updates.push(update);
        }
    }
    Ok(updates)
}

/// Télécharge un title update. Retourne (nom de fichier, données).
/// Le nom de fichier (issu de Content-Disposition) doit être conservé tel quel.
pub fn download_title_update(title_update_id: &str) -> Result<(String, Vec<u8>)> {
    let mut response = AGENT.get(format!("{BASE}/TitleUpdate.php"))
        .query("tuid", title_update_id)
        .call()
        .context("téléchargement du title update")?;

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
        .context("lecture du title update")?;
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
