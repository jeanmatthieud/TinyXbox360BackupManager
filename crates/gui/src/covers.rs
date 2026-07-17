// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{AppWindow, Dispatcher, Message};
use anyhow::Result;
use slint::{ComponentHandle, SharedString, Weak};
use std::fs;
use txbm_core::{covers::download_cover, data_dir::DATA_DIR};

pub fn download_covers(ids: Vec<String>, weak: &Weak<AppWindow>) -> Result<()> {
    let covers_dir = DATA_DIR.join("covers");
    fs::create_dir_all(&covers_dir)?;

    for title_id in ids {
        if title_id.is_empty() {
            continue;
        }

        if download_cover(&covers_dir, &title_id).unwrap_or(false) {
            let _ = weak.upgrade_in_event_loop(move |app| {
                app.global::<Dispatcher<'_>>()
                    .invoke_dispatch(Message::RefreshDisplayedGames, SharedString::new());
            });
        }
    }

    Ok(())
}
