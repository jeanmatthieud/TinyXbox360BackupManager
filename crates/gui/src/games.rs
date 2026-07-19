// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{DisplayedGame, util::GIB};
use slint::{Image, ToSharedString};
use txbm_core::{
    covers,
    data_dir::DATA_DIR,
    game::{Game, GameFormat},
};

impl From<&Game> for DisplayedGame {
    fn from(game: &Game) -> Self {
        let covers_dir = DATA_DIR.join("covers");
        let cover = covers::cached_cover(&covers_dir, &game.id)
            .and_then(|path| Image::load_from_path(&path).ok())
            .unwrap_or_default();

        Self {
            id: game.id.to_shared_string(),
            title: game.title.to_shared_string(),
            format: game.format.label().to_shared_string(),
            path: game.path.to_string_lossy().to_shared_string(),
            size_gib: game.size as f32 / GIB,
            is_x360: game.is_x360,
            is_arcade: game.format == GameFormat::Arcade,
            cover,
            incomplete: game.incomplete,
        }
    }
}
