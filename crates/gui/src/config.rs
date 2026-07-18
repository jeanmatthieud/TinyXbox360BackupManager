// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{DisplayedConfig, TargetKind};
use slint::ToSharedString;
use txbm_core::{config::Config, target::Target};

impl From<txbm_core::config::TargetKind> for TargetKind {
    fn from(kind: txbm_core::config::TargetKind) -> Self {
        match kind {
            txbm_core::config::TargetKind::Local => TargetKind::Local,
            txbm_core::config::TargetKind::Ftp => TargetKind::Ftp,
        }
    }
}

impl From<&Config> for DisplayedConfig {
    fn from(config: &Config) -> Self {
        let target = Target::from_config(&config.contents)
            .map(|t| t.display())
            .unwrap_or_default();

        Self {
            path: config.path.to_string_lossy().to_shared_string(),
            target: target.to_shared_string(),
            target_kind: config.contents.target_kind.into(),
            mount_point: config
                .contents
                .mount_point
                .to_string_lossy()
                .to_shared_string(),
            remove_sources_games: config.contents.remove_sources_games.to_shared_string(),
            sort_by: config.contents.sort_by.to_shared_string(),
            view_as: config.contents.view_as.to_shared_string(),
            theme_preference: config.contents.theme_preference.to_shared_string(),
            show_x360: config.contents.show_x360,
            show_arcade: config.contents.show_arcade,
            show_og: config.contents.show_og,
            console_ip: config.contents.console_ip.to_shared_string(),
            ftp_port: config.contents.ftp_port.to_shared_string(),
            ftp_user: config.contents.ftp_user.to_shared_string(),
            ftp_password: config.contents.ftp_password.to_shared_string(),
        }
    }
}
