// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::badavatar::BadAvatarConfig;
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
    pub auto_reconnect: AutoReconnect,
    pub show_x360: bool,
    pub show_arcade: bool,
    pub show_og: bool,
    pub known_drives: Vec<PathBuf>,

    /// Most-recently-used library locations (most recent first, max 5).
    pub recent_locations: Vec<RecentLocation>,

    /// Console (Aurora FTP server)
    pub console_ip: String,
    pub ftp_port: String,
    pub ftp_user: String,
    pub ftp_password: String,

    /// BadAvatar USB-key creation settings (Toolbox).
    pub badavatar: BadAvatarConfig,
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
            auto_reconnect: AutoReconnect::Never,
            show_x360: true,
            show_arcade: true,
            show_og: true,
            known_drives: Vec::new(),
            recent_locations: Vec::new(),
            console_ip: String::new(),
            ftp_port: "21".to_string(),
            ftp_user: "xboxftp".to_string(),
            ftp_password: "xboxftp".to_string(),
            badavatar: BadAvatarConfig::default(),
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

    /// Applies the startup `auto_reconnect` policy to the loaded config:
    /// clears the active target (as if disconnected) when the policy forbids
    /// reconnecting to the last-used target kind. FTP credentials and the
    /// recent-locations history are kept, so the target-selection modal can
    /// still offer a manual reconnect.
    pub fn apply_auto_reconnect_policy(&mut self) {
        let keep = match self.auto_reconnect {
            AutoReconnect::Always => true,
            AutoReconnect::Never => false,
            AutoReconnect::FtpOnly => self.target_kind == TargetKind::Ftp,
            AutoReconnect::UsbOnly => self.target_kind == TargetKind::Local,
        };

        if !keep {
            self.target_kind = TargetKind::Local;
            self.mount_point = PathBuf::new();
        }
    }

    /// Records the current target at the top of the recent-locations list
    /// (most recent first), de-duplicating and keeping at most 5 entries.
    /// No-op if no usable target is configured.
    pub fn record_recent_location(&mut self) {
        let entry = match self.target_kind {
            TargetKind::Local => {
                if self.mount_point.as_os_str().is_empty() {
                    return;
                }
                RecentLocation {
                    kind: TargetKind::Local,
                    mount_point: self.mount_point.clone(),
                    console_ip: String::new(),
                    ftp_port: String::new(),
                    ftp_user: String::new(),
                    ftp_password: String::new(),
                    last_used: now_secs(),
                }
            }
            TargetKind::Ftp => {
                if self.console_ip.trim().is_empty() {
                    return;
                }
                RecentLocation {
                    kind: TargetKind::Ftp,
                    mount_point: PathBuf::new(),
                    console_ip: self.console_ip.clone(),
                    ftp_port: self.ftp_port.clone(),
                    ftp_user: self.ftp_user.clone(),
                    ftp_password: self.ftp_password.clone(),
                    last_used: now_secs(),
                }
            }
        };

        self.recent_locations
            .retain(|l| !l.same_identity(&entry));
        self.recent_locations.insert(0, entry);
        self.recent_locations.truncate(5);
    }
}

/// One previously-used library location (local drive or console over FTP),
/// remembered so the user can reconnect from the target-selection modal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentLocation {
    pub kind: TargetKind,
    /// Local target only; empty for FTP.
    #[serde(default)]
    pub mount_point: PathBuf,
    /// FTP target only; empty for local.
    #[serde(default)]
    pub console_ip: String,
    #[serde(default)]
    pub ftp_port: String,
    #[serde(default)]
    pub ftp_user: String,
    #[serde(default)]
    pub ftp_password: String,
    /// Unix timestamp (seconds) of the last connection to this location.
    #[serde(default)]
    pub last_used: u64,
}

impl RecentLocation {
    /// Two entries designate the same location (used for de-duplication).
    /// The password is intentionally excluded so re-entering it just refreshes
    /// the stored credentials rather than creating a duplicate.
    fn same_identity(&self, other: &RecentLocation) -> bool {
        self.kind == other.kind
            && match self.kind {
                TargetKind::Local => self.mount_point == other.mount_point,
                TargetKind::Ftp => {
                    self.console_ip.trim() == other.console_ip.trim()
                        && self.ftp_port.trim() == other.ftp_port.trim()
                        && self.ftp_user.trim() == other.ftp_user.trim()
                }
            }
    }

    /// Short label for the UI: the folder name for a local drive (falling back
    /// to the full path for a filesystem root), or the IP for a console.
    pub fn display_name(&self) -> String {
        match self.kind {
            TargetKind::Local => self
                .mount_point
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| self.mount_point.to_string_lossy().to_string()),
            TargetKind::Ftp => self.console_ip.trim().to_string(),
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

/// Whether the app should reconnect to the last-used target on startup.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, Display, FromStr)]
#[serde(rename_all = "snake_case")]
pub enum AutoReconnect {
    Always,
    #[default]
    Never,
    /// Reconnect only when the last target was a console over FTP.
    FtpOnly,
    /// Reconnect only when the last target was a local (USB) drive.
    UsbOnly,
}
