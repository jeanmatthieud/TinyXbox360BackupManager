// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::config::SortBy;
use crate::util::dir_size;
use crate::{CONTENT_DIR, GAMES_DIR};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameFormat {
    /// GOD container in Content/0000000000000000/<TitleID>/00007000 (360)
    /// or 00005000 (Xbox Original).
    God,
    /// XBLA package in Content/0000000000000000/<TitleID>/000D0000.
    Arcade,
    /// Extracted folder with default.xex (Xbox 360).
    ExtractedXex,
    /// Extracted folder with default.xbe (Original Xbox).
    ExtractedXbe,
}

impl GameFormat {
    pub fn label(&self) -> &'static str {
        match self {
            GameFormat::God => "GOD",
            GameFormat::Arcade => "XBLA",
            GameFormat::ExtractedXex => "Extracted (360)",
            GameFormat::ExtractedXbe => "Xbox OG",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Game {
    /// TitleID (8 hex chars) for GOD games, empty for extracted games.
    pub id: String,
    pub title: String,
    pub format: GameFormat,
    pub path: PathBuf,
    pub size: u64,
    pub is_x360: bool,
    pub search_term: String,
}

/// STFS content types considered as installed games under
/// Content/0000000000000000/<TitleID>/: (folder, format, is Xbox 360).
pub const INSTALLED_CONTENT_TYPES: [(&str, GameFormat, bool); 3] = [
    ("00007000", GameFormat::God, true),
    ("00005000", GameFormat::God, false),
    ("000D0000", GameFormat::Arcade, true),
];

/// FATX limits file names to 42 characters.
pub const FATX_MAX_NAME: usize = 42;

/// Folder name for an extracted Original Xbox game: the TitleID is
/// embedded as a ` [XXXXXXXX]` suffix so scans (especially over FTP)
/// can identify the game without reading its `default.xbe`.
/// Aurora ignores the folder name (it displays the XBE title).
pub fn og_folder_name(title: &str, title_id: &str) -> String {
    let suffix = format!(" [{title_id}]");
    let max_title = FATX_MAX_NAME - suffix.chars().count();
    let title: String = title.trim().chars().take(max_title).collect();
    format!("{}{suffix}", title.trim_end())
}

/// Splits a folder name into (title, TitleID) if it carries
/// a ` [XXXXXXXX]` suffix.
pub fn split_title_id_suffix(name: &str) -> (String, Option<String>) {
    if let Some(start) = name.rfind(" [")
        && let Some(id) = name[start + 2..].strip_suffix(']')
        && id.len() == 8
        && id.chars().all(|c| c.is_ascii_hexdigit())
    {
        return (name[..start].trim().to_string(), Some(id.to_uppercase()));
    }
    (name.to_string(), None)
}

/// Scan the drive (mount point) for installed games.
pub fn scan_drive(drive_dir: &Path) -> Vec<Game> {
    let mut games = Vec::new();

    // GOD: <drive>/Content/0000000000000000/<TitleID>/0000[57]000/<MediaID>
    let content_dir = drive_dir.join(CONTENT_DIR);
    if let Ok(entries) = std::fs::read_dir(&content_dir) {
        for entry in entries.flatten() {
            let title_dir = entry.path();
            let title_id = entry.file_name().to_string_lossy().to_uppercase();
            if !title_dir.is_dir() || title_id.len() != 8 {
                continue;
            }
            for (content_type, format, is_x360) in INSTALLED_CONTENT_TYPES {
                let type_dir = title_dir.join(content_type);
                if !type_dir.is_dir() {
                    continue;
                }
                let title = crate::stfs::title_from_dir(&type_dir)
                    .or_else(|| {
                        u32::from_str_radix(&title_id, 16)
                            .ok()
                            .and_then(iso2god::game_list::find_title_by_id)
                    })
                    .unwrap_or_else(|| title_id.clone());
                let search_term = format!("{title}\0{title_id}").to_lowercase();
                games.push(Game {
                    id: title_id.clone(),
                    title,
                    format,
                    path: title_dir.clone(),
                    size: dir_size(&type_dir),
                    is_x360,
                    search_term,
                });
            }
        }
    }

    // Extracted games: <drive>/Games/<Name>/default.xex or default.xbe
    let games_dir = drive_dir.join(GAMES_DIR);
    if let Ok(entries) = std::fs::read_dir(&games_dir) {
        for entry in entries.flatten() {
            let game_dir = entry.path();
            if !game_dir.is_dir() {
                continue;
            }
            let format = if game_dir.join("default.xex").is_file() {
                GameFormat::ExtractedXex
            } else if game_dir.join("default.xbe").is_file() {
                GameFormat::ExtractedXbe
            } else {
                continue;
            };
            let folder_name = entry.file_name().to_string_lossy().to_string();
            let (title, mut id) = split_title_id_suffix(&folder_name);
            // Game added by hand (no TitleID suffix): read it from the
            // XBE, which is cheap on a local target.
            if id.is_none() && format == GameFormat::ExtractedXbe {
                id = crate::xbe::title_id_from_file(&game_dir.join("default.xbe")).ok();
            }
            let id = id.unwrap_or_default();
            let search_term = format!("{title}\0{id}").to_lowercase();
            games.push(Game {
                id,
                title,
                format,
                path: game_dir.clone(),
                size: dir_size(&game_dir),
                is_x360: format == GameFormat::ExtractedXex,
                search_term,
            });
        }
    }

    games
}

pub fn get_compare_fn(sort_by: SortBy) -> impl FnMut(&Game, &Game) -> Ordering {
    move |a, b| match sort_by {
        SortBy::NameDescending => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
        SortBy::NameAscending => b.title.to_lowercase().cmp(&a.title.to_lowercase()),
        SortBy::SizeDescending => a.size.cmp(&b.size),
        SortBy::SizeAscending => b.size.cmp(&a.size),
    }
}
