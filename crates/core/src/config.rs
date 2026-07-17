// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::data_dir::DATA_DIR;
use anyhow::Result;
use derive_more::{Display, FromStr};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub path: PathBuf,
    pub contents: ConfigContents,
}

impl Config {
    pub fn load() -> Self {
        let path = DATA_DIR.join("config.json");
        let s = fs::read_to_string(&path).unwrap_or_default();
        let contents = serde_json::from_str(&s).unwrap_or_default();

        Self { path, contents }
    }

    pub fn write(&self) -> Result<()> {
        fs::create_dir_all(&*DATA_DIR)?;
        let s = serde_json::to_string_pretty(&self.contents)?;
        fs::write(&self.path, s)?;
        Ok(())
    }

    /// Returns true if the notification should be shown
    pub fn check_mount_point(&mut self) -> bool {
        let drive = &self.contents.mount_point;

        if drive.as_os_str().is_empty() {
            return false;
        }

        let new = self.contents.known_drives.iter().all(|p| p != drive);

        if new {
            self.contents.known_drives.push(drive.clone());
            let _ = self.write();
        }

        new
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConfigContents {
    pub target_kind: TargetKind,
    pub mount_point: PathBuf,
    pub remove_sources_games: bool,
    pub sort_by: SortBy,
    pub view_as: ViewAs,
    pub theme_preference: ThemePreference,
    pub show_x360: bool,
    pub show_og: bool,
    pub known_drives: Vec<PathBuf>,

    /// Console (Aurora FTP server)
    pub console_ip: String,
    pub ftp_port: String,
    pub ftp_user: String,
    pub ftp_password: String,
}

impl Default for ConfigContents {
    fn default() -> Self {
        Self {
            target_kind: TargetKind::Local,
            mount_point: PathBuf::new(),
            remove_sources_games: false,
            sort_by: SortBy::NameDescending,
            view_as: ViewAs::Grid,
            theme_preference: ThemePreference::System,
            show_x360: true,
            show_og: true,
            known_drives: Vec::new(),
            console_ip: String::new(),
            ftp_port: "21".to_string(),
            ftp_user: "xboxftp".to_string(),
            ftp_password: "xboxftp".to_string(),
        }
    }
}

impl ConfigContents {
    pub fn ftp_config(&self) -> crate::ftp::FtpConfig {
        crate::ftp::FtpConfig {
            host: self.console_ip.trim().to_string(),
            port: self.ftp_port.trim().parse().unwrap_or(21),
            user: self.ftp_user.trim().to_string(),
            password: self.ftp_password.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Display, FromStr)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    #[default]
    Local,
    Ftp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Display, FromStr)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    NameDescending,
    NameAscending,
    SizeDescending,
    SizeAscending,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Display, FromStr)]
#[serde(rename_all = "lowercase")]
pub enum ViewAs {
    #[default]
    Grid,
    Table,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Display, FromStr)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}
