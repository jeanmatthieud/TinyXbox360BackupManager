// SPDX-License-Identifier: GPL-3.0-only

//! Fetches the Title Update list+state for the game shown in the info
//! modal's Title Updates window. Needs an XboxUnity HTTP call plus an FTP
//! round-trip, so this runs off the UI thread (see
//! `Message::FetchTitleUpdates`).

use crate::DisplayedTitleUpdate;
use anyhow::Result;
use slint::ToSharedString;
use txbm_core::{
    game::Game,
    target::Target,
    title_updates::InstalledTitleUpdate,
    unity::{self, TitleUpdateEntry},
};

/// Empty, with no error, for non-360 games or games with no resolved
/// TitleID.
pub fn fetch(target: &Target, game: &Game) -> Result<Vec<DisplayedTitleUpdate>> {
    if !game.is_x360 || game.id.is_empty() {
        return Ok(Vec::new());
    }

    let entries = unity::title_updates(&game.id)?;
    let installed = target.installed_title_updates(game)?;
    let cached_hashes = target.cached_title_update_hashes(&game.id)?;
    Ok(merge(entries, &installed, &cached_hashes))
}

fn merge(
    entries: Vec<TitleUpdateEntry>,
    installed: &[InstalledTitleUpdate],
    cached_hashes: &[String],
) -> Vec<DisplayedTitleUpdate> {
    entries
        .into_iter()
        .map(|entry| {
            let installed_match = entry.hash.as_deref().and_then(|hash| {
                installed.iter().find(|i| i.hash.eq_ignore_ascii_case(hash))
            });
            let is_cached = entry.hash.as_deref().is_some_and(|hash| {
                cached_hashes.iter().any(|h| h.eq_ignore_ascii_case(hash))
            });
            let size_kib: f64 = entry.size.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0.0);

            DisplayedTitleUpdate {
                id: entry.title_update_id.to_shared_string(),
                hash: entry.hash.clone().unwrap_or_default().to_shared_string(),
                name: entry.name.clone().unwrap_or_default().to_shared_string(),
                version: entry.version.clone().unwrap_or_default().to_shared_string(),
                size_text: slint::format!("{:.1} MiB", size_kib / 1024.0),
                installed_file_name: installed_match
                    .map(|i| i.file_name.clone())
                    .unwrap_or_default()
                    .to_shared_string(),
                is_active: installed_match.is_some(),
                is_cached,
            }
        })
        .collect()
}
