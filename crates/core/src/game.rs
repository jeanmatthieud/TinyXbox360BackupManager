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
    /// `Content/0000000000000000/<TitleID>` folder holding this game's DLC and
    /// title updates, when it isn't [`Self::path`] itself.
    ///
    /// A GOD or Arcade game *is* that folder, so this stays `None`. An
    /// extracted game lives elsewhere (`Games`, `Games Xbox`…) while its
    /// installed content stays under `Content`, so [`merge_extracted_content`]
    /// points it here once the scan has found both halves.
    pub content_dir: Option<PathBuf>,
}

impl Game {
    /// Folder to look into for installed DLC / title updates.
    pub fn content_dir(&self) -> &Path {
        self.content_dir.as_deref().unwrap_or(&self.path)
    }
}

/// Folds the DLC-only `<TitleID>` entries into the extracted games they
/// belong to, and drops them from the list.
///
/// DLC and title updates always install under
/// `Content/0000000000000000/<TitleID>`, whatever the format of the game they
/// patch. For a GOD game that folder *is* the game, but an extracted game
/// lives in its own folder, so the scan sees the two halves as two unrelated
/// entries — the extracted game, and a stray `incomplete` one holding only
/// the content. Matching them by TitleID gives the user one entry per game,
/// with its content listed in its info modal.
///
/// A leftover `incomplete` entry with no extracted counterpart is kept: there
/// really is orphaned content installed, and nothing else surfaces it.
pub fn merge_extracted_content(games: &mut Vec<Game>) {
    // TitleIDs claimed by an extracted game. Games whose ID couldn't be
    // resolved (no folder suffix, unreadable executable) can't be matched,
    // and are skipped.
    let claimed: Vec<String> = games
        .iter()
        .filter(|g| {
            matches!(
                g.format,
                GameFormat::ExtractedXex | GameFormat::ExtractedXbe
            ) && !g.id.is_empty()
        })
        .map(|g| g.id.clone())
        .collect();
    if claimed.is_empty() {
        return;
    }

    // Keyed by TitleID rather than by index: the scan walks the `Content`
    // location first, so the entries being removed sit *before* their merge
    // targets and would shift every index.
    let mut merged: Vec<(String, PathBuf, u64)> = Vec::new();
    games.retain(|game| {
        if !game.incomplete || !claimed.contains(&game.id) {
            return true;
        }
        merged.push((game.id.clone(), game.path.clone(), game.size));
        false
    });

    // TitleIDs are unique per game; two non-incomplete entries sharing one
    // would mean the same title was installed twice under different
    // folders, a manual-setup anomaly this scan doesn't try to detect or
    // fix. `.find()` picks whichever comes first in scan order.
    for (id, content_dir, size) in merged {
        let Some(game) = games.iter_mut().find(|g| g.id == id && !g.incomplete) else {
            continue;
        };
        game.content_dir = Some(content_dir);
        // The content lives outside the game folder, so its bytes weren't
        // counted in the extracted game's own size.
        game.size += size;
    }
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
    let mut scanner = LocalScanner { games: Vec::new() };
    for location in &layout.scan_locations {
        // Local listing failures are non-fatal (an unreadable folder is just
        // skipped), so this walk never returns an error.
        let _ = crate::scan::walk(&mut scanner, &PathBuf::from(&location.path), location.depth);
    }
    merge_extracted_content(&mut scanner.games);
    scanner.games
}

/// Local-filesystem [`DirScanner`], detecting the format of every game found
/// (GOD/Arcade TitleID folders and extracted-game folders) via the shared walk.
struct LocalScanner {
    games: Vec<Game>,
}

impl crate::scan::DirScanner for LocalScanner {
    type Path = PathBuf;

    fn child_dirs(&mut self, dir: &PathBuf) -> anyhow::Result<Vec<(PathBuf, String)>> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Ok(Vec::new());
        };
        Ok(entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| (e.path(), e.file_name().to_string_lossy().to_string()))
            .collect())
    }

    fn classify(&mut self, path: &PathBuf, name: &str) -> anyhow::Result<crate::scan::ChildAction> {
        use crate::scan::ChildAction;

        // GOD / Arcade: an 8-hex TitleID folder.
        if is_title_id(name) && push_god_games_local(path, name, &mut self.games) {
            return Ok(ChildAction::Handled);
        }

        // Extracted game: a folder directly holding default.xex / default.xbe.
        if let Some(format) = detect_extracted_local(path) {
            push_extracted_local(path, name, format, &mut self.games);
            return Ok(ChildAction::Handled);
        }

        // Neither: let the walk descend (handles nested
        // Content/0000000000000000 folders and arbitrary scan roots).
        Ok(ChildAction::Recurse)
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
            content_dir: None,
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
        content_dir: None,
    });
    true
}

/// Detects an extracted-game folder from its default executable.
fn detect_extracted_local(game_dir: &Path) -> Option<GameFormat> {
    if crate::util::find_file_ci(game_dir, "default.xex").is_some() {
        Some(GameFormat::ExtractedXex)
    } else if crate::util::find_file_ci(game_dir, "default.xbe").is_some() {
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
    // Game added by hand (no TitleID suffix): read it from the executable,
    // which is cheap on a local target (only its header is read).
    if id.is_none() {
        id = match format {
            GameFormat::ExtractedXbe => crate::util::find_file_ci(game_dir, "default.xbe")
                .and_then(|xbe| crate::xbe::title_id_from_file(&xbe).ok()),
            GameFormat::ExtractedXex => crate::util::find_file_ci(game_dir, "default.xex")
                .and_then(|xex| crate::xex::title_id_from_file(&xex).ok()),
            _ => None,
        };
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
        content_dir: None,
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
