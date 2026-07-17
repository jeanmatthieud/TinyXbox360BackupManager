// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::config::SortBy;
use crate::util::dir_size;
use crate::{CONTENT_DIR, GAMES_DIR};
use std::cmp::Ordering;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameFormat {
    /// GOD container in Content/0000000000000000/<TitleID>/00007000 (360)
    /// or 00005000 (Xbox Original).
    God,
    /// Extracted folder with default.xex (Xbox 360).
    ExtractedXex,
    /// Extracted folder with default.xbe (Original Xbox).
    ExtractedXbe,
}

impl GameFormat {
    pub fn label(&self) -> &'static str {
        match self {
            GameFormat::God => "GOD",
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

/// STFS content types considered as installed games.
const GOD_CONTENT_TYPES: [(&str, bool); 2] = [("00007000", true), ("00005000", false)];

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
            for (content_type, is_x360) in GOD_CONTENT_TYPES {
                let type_dir = title_dir.join(content_type);
                if !type_dir.is_dir() {
                    continue;
                }
                let title = con_header_title(&type_dir)
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
                    format: GameFormat::God,
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
            let title = entry.file_name().to_string_lossy().to_string();
            let search_term = title.to_lowercase();
            games.push(Game {
                id: String::new(),
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

/// Read the game name from the CON header of a GOD folder
/// (UTF-16 big-endian at offset 0x411).
fn con_header_title(type_dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(type_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        // The CON header is the file named after the MediaID (8 hex chars, no ".data").
        let name = entry.file_name().to_string_lossy().to_string();
        if !path.is_file() || name.len() != 8 {
            continue;
        }
        let mut file = File::open(&path).ok()?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic).ok()?;
        if &magic != b"CON " && &magic != b"LIVE" && &magic != b"PIRS" {
            continue;
        }
        file.seek(SeekFrom::Start(0x411)).ok()?;
        let mut buf = [0u8; 0x100];
        file.read_exact(&mut buf).ok()?;
        let utf16: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .collect();
        let title = String::from_utf16_lossy(&utf16).trim().to_string();
        if !title.is_empty() {
            return Some(title);
        }
    }
    None
}
