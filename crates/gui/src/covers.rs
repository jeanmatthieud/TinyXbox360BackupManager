// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{AppWindow, Dispatcher, Message};
use anyhow::Result;
use slint::{ComponentHandle, SharedString, Weak};
use std::fs;
use txbm_core::{
    covers::{self, XbeIdCache},
    data_dir::DATA_DIR,
    ftp::FtpSession,
    game::{Game, GameFormat},
    mobcat,
    target::Target,
};

pub fn download_covers(
    mut games: Vec<Game>,
    target: Option<Target>,
    weak: &Weak<AppWindow>,
) -> Result<()> {
    let covers_dir = DATA_DIR.join("covers");
    fs::create_dir_all(&covers_dir)?;

    // Original Xbox games added by hand over FTP have no TitleID
    // suffix: read it from their default.xbe, once ever per game
    // thanks to a local cache.
    if let Some(Target::Ftp(ftp)) = &target {
        resolve_ftp_title_ids(&mut games, ftp, weak);
    }

    // Refresh the MobCat database (conditional request, silent on
    // failure) before looking Original Xbox covers up in it.
    if games
        .iter()
        .any(|g| !g.is_x360 && !g.id.is_empty() && covers::cached_cover(&covers_dir, &g.id).is_none())
    {
        mobcat::ensure_db();
    }

    for game in &games {
        if game.id.is_empty() {
            continue;
        }

        let downloaded = covers::download_cover(&covers_dir, &game.id, game.is_x360).unwrap_or(false);
        // Build the downscaled thumbnail off the UI thread. Also catches
        // covers cached by a previous run that have no thumbnail yet.
        let thumbnailed = covers::ensure_thumbnail(&covers_dir, &game.id).unwrap_or(false);

        if downloaded || thumbnailed {
            let _ = weak.upgrade_in_event_loop(move |app| {
                app.global::<Dispatcher<'_>>()
                    .invoke_dispatch(Message::RefreshDisplayedGames, SharedString::new());
            });
        }
    }

    Ok(())
}

/// Fills the missing TitleIDs of extracted Original Xbox games by
/// downloading their `default.xbe` over FTP (one shared session), with
/// a persistent path→TitleID cache so each game is only read once.
/// Every resolved ID is pushed back to the UI state via `SetGameId`.
fn resolve_ftp_title_ids(games: &mut [Game], ftp: &txbm_core::ftp::FtpConfig, weak: &Weak<AppWindow>) {
    let mut unresolved: Vec<&mut Game> = games
        .iter_mut()
        .filter(|g| g.format == GameFormat::ExtractedXbe && g.id.is_empty())
        .collect();
    if unresolved.is_empty() {
        return;
    }

    let mut cache = XbeIdCache::load();
    let mut cache_dirty = false;
    let mut session: Option<FtpSession> = None;

    for game in &mut unresolved {
        let remote_path = game.path.to_string_lossy().replace('\\', "/");

        let id = match cache.get(&ftp.host, &remote_path) {
            Some(id) => Some(id.clone()),
            None => {
                if session.is_none() {
                    session = FtpSession::connect(ftp).ok();
                }
                let Some(session) = session.as_mut() else {
                    // Console unreachable: retry at the next covers pass.
                    break;
                };
                session
                    .download_file(&format!("{remote_path}/default.xbe"))
                    .ok()
                    .and_then(|bytes| txbm_core::xbe::title_id_from_bytes(&bytes).ok())
                    .inspect(|id| {
                        cache.insert(&ftp.host, &remote_path, id.clone());
                        cache_dirty = true;
                    })
            }
        };

        if let Some(id) = id {
            game.id = id.clone();
            game.search_term = format!("{}\0{id}", game.title).to_lowercase();
            let payload = slint::format!("{remote_path}\n{id}");
            let _ = weak.upgrade_in_event_loop(move |app| {
                app.global::<Dispatcher<'_>>()
                    .invoke_dispatch(Message::SetGameId, payload);
            });
        }
    }

    if let Some(session) = session {
        session.quit();
    }
    if cache_dirty {
        cache.save();
    }
}
