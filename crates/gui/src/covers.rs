// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{AppWindow, Dispatcher, Message};
use anyhow::Result;
use slint::{ComponentHandle, SharedString, Weak};
use std::fs;
use txbm_core::{
    config::CoverSource,
    covers::{self, TitleIdCache},
    data_dir::DATA_DIR,
    game::{Game, GameFormat},
    mobcat,
    remote_fs::{RemoteFs, RemoteSession},
    target::Target,
};

pub fn download_covers(
    mut games: Vec<Game>,
    target: Option<Target>,
    source: CoverSource,
    weak: &Weak<AppWindow>,
) -> Result<()> {
    let covers_dir = DATA_DIR.join("covers");
    fs::create_dir_all(&covers_dir)?;

    // Extracted games added by hand on a console have no TitleID suffix: read
    // it from their default.xbe / default.xex, once ever per game thanks to a
    // local cache. A local drive needs none of this — its scanner already
    // reads the executable as it goes.
    if let Some(target) = target.as_ref().filter(|t| !matches!(t, Target::Local(_))) {
        resolve_remote_title_ids(&mut games, target, weak);
    }

    // Refresh the MobCat database (conditional request, silent on
    // failure) before looking Original Xbox covers up in it.
    if games
        .iter()
        .any(|g| !g.is_x360 && !g.id.is_empty() && covers::cached_cover(&covers_dir, &g.id).is_none())
    {
        mobcat::ensure_db();
    }

    // Every refresh rebuilds the whole library model, and each row of it costs
    // two filesystem calls (the cached cover, then its thumbnail). One per
    // downloaded cover meant O(N²) of them — on a 600-game library with an
    // empty cache, hundreds of thousands of stat calls on the UI thread, which
    // is what made the grid stutter through the first cover pass. New covers
    // are shown in batches instead; `FinishedDownloadingCovers` does the last
    // refresh, so nothing is left waiting for the next tick.
    const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_millis(400);
    let mut pending = false;
    let mut last_refresh = std::time::Instant::now();

    for game in &games {
        if game.id.is_empty() {
            continue;
        }

        let downloaded =
            covers::download_cover(&covers_dir, &game.id, game.is_x360, source).unwrap_or(false);
        // Build the downscaled thumbnail off the UI thread. Also catches
        // covers cached by a previous run that have no thumbnail yet.
        let thumbnailed = covers::ensure_thumbnail(&covers_dir, &game.id).unwrap_or(false);

        pending |= downloaded || thumbnailed;
        if pending && last_refresh.elapsed() >= REFRESH_EVERY {
            pending = false;
            last_refresh = std::time::Instant::now();
            let _ = weak.upgrade_in_event_loop(move |app| {
                app.global::<Dispatcher<'_>>()
                    .invoke_dispatch(Message::RefreshDisplayedGames, SharedString::new());
            });
        }
    }

    Ok(())
}

/// Fills the missing TitleIDs of extracted games by reading their
/// `default.xbe` / `default.xex` from the console (one shared session), with a
/// persistent path→TitleID cache so each game is only read once.
/// Every resolved ID is pushed back to the UI state via `SetGameId`.
fn resolve_remote_title_ids(games: &mut [Game], target: &Target, weak: &Weak<AppWindow>) {
    let mut unresolved: Vec<&mut Game> = games
        .iter_mut()
        .filter(|g| {
            matches!(
                g.format,
                GameFormat::ExtractedXbe | GameFormat::ExtractedXex
            ) && g.id.is_empty()
        })
        .collect();
    if unresolved.is_empty() {
        return;
    }

    let mut cache = TitleIdCache::load();
    let mut cache_dirty = false;
    let mut resolved: Vec<String> = Vec::new();
    let mut session: Option<RemoteSession> = None;
    let cache_key = target.remote_key();

    for game in &mut unresolved {
        let remote_path = game.path.to_string_lossy().replace('\\', "/");

        let id = match cache.get(&cache_key, &remote_path) {
            Some(id) => Some(id.clone()),
            None => {
                if session.is_none() {
                    session = target.open_remote(false).ok();
                }
                let Some(session) = session.as_mut() else {
                    // Console unreachable: retry at the next covers pass.
                    break;
                };
                let read = if game.format == GameFormat::ExtractedXex {
                    // A default.xex is multi-MiB but its TitleID lives in the
                    // header, so only the prefix is pulled.
                    session
                        .download_prefix(
                            &format!("{remote_path}/default.xex"),
                            txbm_core::xex::HEADER_PREFIX_SIZE,
                        )
                        .ok()
                        .and_then(|bytes| txbm_core::xex::title_id_from_bytes(&bytes).ok())
                } else {
                    session
                        .download_file(&format!("{remote_path}/default.xbe"))
                        .ok()
                        .and_then(|bytes| txbm_core::xbe::title_id_from_bytes(&bytes).ok())
                };
                read.inspect(|id| {
                    cache.insert(&cache_key, &remote_path, id.clone());
                    cache_dirty = true;
                })
            }
        };

        if let Some(id) = id {
            game.id = id.clone();
            game.search_term = format!("{}\0{id}", game.title).to_lowercase();
            resolved.push(format!("{remote_path}\n{id}"));
        }
    }

    if let Some(session) = session {
        let _ = session.quit();
    }
    if cache_dirty {
        cache.save();
    }

    // One message for the whole pass: each `SetGameId` re-runs
    // `merge_extracted_content` over the library and rebuilds the whole
    // displayed model, so sending one per resolved game made the pass
    // quadratic for no visible gain — the covers download follows immediately
    // and refreshes the grid anyway.
    if !resolved.is_empty() {
        let payload = resolved.join("\n").into();
        let _ = weak.upgrade_in_event_loop(move |app| {
            app.global::<Dispatcher<'_>>()
                .invoke_dispatch(Message::SetGameId, payload);
        });
    }
}
