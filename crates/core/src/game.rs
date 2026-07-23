// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::config::SortBy;
use crate::util::dir_size;
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
            GameFormat::ExtractedXex => "XEX",
            GameFormat::ExtractedXbe => "XBE",
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
    /// True when the title folder only has DLC and/or a title update, with
    /// no actual game package installed (e.g. the base install was removed
    /// or never completed).
    pub incomplete: bool,
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

/// True when `name` is an 8-hex-character TitleID folder (e.g. `58410889`),
/// as found directly under a `Content/0000000000000000` directory.
pub(crate) fn is_title_id(name: &str) -> bool {
    name.len() == 8 && name.chars().all(|c| c.is_ascii_hexdigit())
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

/// Scan the drive (mount point) for installed games, honoring the target's
/// resolved layout (its `.txbm.json` manifest, otherwise the defaults).
pub fn scan_drive(drive_dir: &Path) -> Vec<Game> {
    let layout = crate::target::local_layout(drive_dir);
    let mut games = Vec::new();
    for location in &layout.scan_locations {
        scan_location_local(Path::new(&location.path), location.depth, &mut games);
    }
    games
}

/// Scans one local location, detecting the format of every game found and
/// descending up to `depth` levels for anything that is neither a GOD/Arcade
/// TitleID folder nor an extracted-game folder.
fn scan_location_local(dir: &Path, depth: u32, games: &mut Vec<Game>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if !child.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();

        // GOD / Arcade: an 8-hex TitleID folder.
        if is_title_id(&name) && push_god_games_local(&child, &name, games) {
            continue;
        }

        // Extracted game: a folder directly holding default.xex / default.xbe.
        if let Some(format) = detect_extracted_local(&child) {
            push_extracted_local(&child, &name, format, games);
            continue;
        }

        // Neither: descend if the scan depth allows (handles nested
        // Content/0000000000000000 folders and arbitrary scan roots).
        if depth > 1 {
            scan_location_local(&child, depth - 1, games);
        }
    }
}

/// Handles a GOD/Arcade `<TitleID>` folder locally. Returns true when a game
/// (or an incomplete DLC/title-update-only entry) was pushed.
fn push_god_games_local(title_dir: &Path, title_id_raw: &str, games: &mut Vec<Game>) -> bool {
    let title_id = title_id_raw.to_uppercase();
    let mut found_package = false;

    for (content_type, format, is_x360) in INSTALLED_CONTENT_TYPES {
        let type_dir = title_dir.join(content_type);
        if !type_dir.is_dir() {
            continue;
        }
        found_package = true;
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
            path: title_dir.to_path_buf(),
            size: dir_size(&type_dir) + dir_size(&title_dir.join(crate::stfs::dlc_dir_name())),
            is_x360,
            search_term,
            incomplete: false,
        });
    }

    if found_package {
        return true;
    }

    // No game package: only DLC and/or a title update sit here, orphaned from
    // a base install that was removed or never completed. Still surface it,
    // flagged incomplete.
    let dlc_dir = title_dir.join(crate::stfs::dlc_dir_name());
    let title_update_dir = title_dir.join(crate::stfs::title_update_dir_name());
    if !dlc_dir.is_dir() && !title_update_dir.is_dir() {
        return false;
    }
    let title = u32::from_str_radix(&title_id, 16)
        .ok()
        .and_then(iso2god::game_list::find_title_by_id)
        .unwrap_or_else(|| title_id.clone());
    let search_term = format!("{title}\0{title_id}").to_lowercase();
    games.push(Game {
        id: title_id.clone(),
        title,
        format: GameFormat::God,
        path: title_dir.to_path_buf(),
        size: dir_size(&dlc_dir) + dir_size(&title_update_dir),
        is_x360: true,
        search_term,
        incomplete: true,
    });
    true
}

/// Detects an extracted-game folder from its default executable.
fn detect_extracted_local(game_dir: &Path) -> Option<GameFormat> {
    if game_dir.join("default.xex").is_file() {
        Some(GameFormat::ExtractedXex)
    } else if game_dir.join("default.xbe").is_file() {
        Some(GameFormat::ExtractedXbe)
    } else {
        None
    }
}

/// Pushes an extracted game found locally.
fn push_extracted_local(
    game_dir: &Path,
    folder_name: &str,
    format: GameFormat,
    games: &mut Vec<Game>,
) {
    let (title, mut id) = split_title_id_suffix(folder_name);
    // Game added by hand (no TitleID suffix): read it from the XBE, which is
    // cheap on a local target.
    if id.is_none() && format == GameFormat::ExtractedXbe {
        id = crate::xbe::title_id_from_file(&game_dir.join("default.xbe")).ok();
    }
    let id = id.unwrap_or_default();
    let search_term = format!("{title}\0{id}").to_lowercase();
    games.push(Game {
        id,
        title,
        format,
        path: game_dir.to_path_buf(),
        size: dir_size(game_dir),
        is_x360: format == GameFormat::ExtractedXex,
        search_term,
        incomplete: false,
    });
}

pub fn get_compare_fn(sort_by: SortBy) -> impl FnMut(&Game, &Game) -> Ordering {
    move |a, b| match sort_by {
        SortBy::NameDescending => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
        SortBy::NameAscending => b.title.to_lowercase().cmp(&a.title.to_lowercase()),
        SortBy::SizeDescending => a.size.cmp(&b.size),
        SortBy::SizeAscending => b.size.cmp(&a.size),
    }
}
