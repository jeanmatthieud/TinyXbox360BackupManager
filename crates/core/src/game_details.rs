// SPDX-License-Identifier: GPL-3.0-only

//! Disc and DLC listing for an installed GOD game: one entry per STFS
//! package found in its content folder (one per disc, sharing the same
//! TitleID) and one per installed DLC / marketplace package.

use crate::ftp::FtpSession;
use crate::game::{Game, GameFormat};
use crate::stfs::{self, dlc_dir_name};
use crate::target::Target;
use crate::util::dir_size;
use anyhow::Result;
use std::path::PathBuf;

fn god_content_type(game: &Game) -> Option<&'static str> {
    match (game.format, game.is_x360) {
        (GameFormat::God, true) => Some("00007000"),
        (GameFormat::God, false) => Some("00005000"),
        _ => None,
    }
}

/// A STFS header file is named after its 8-hex-char MediaID, with no
/// extension (the `.data` folders holding the actual GOD fragments are
/// named `<MediaID>.data`, so they're naturally excluded here).
fn is_media_id(name: &str) -> bool {
    name.len() == 8 && name.chars().all(|c| c.is_ascii_hexdigit())
}

fn disc_description(info: Option<stfs::StfsInfo>) -> String {
    match info {
        Some(info) if info.disc_in_set > 1 => {
            format!("Disc {}", info.disc_number)//, info.disc_in_set)
        }
        _ => "Disc".to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct DiscInfo {
    /// 8 uppercase hex chars.
    pub media_id: String,
    /// e.g. "Disc 1 of 2", or plain "Disc" for single-disc games.
    pub description: String,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct DlcInfo {
    pub size: u64,
}

#[derive(Debug, Clone, Default)]
pub struct GameDetails {
    pub discs: Vec<DiscInfo>,
    pub dlc: Vec<DlcInfo>,
}

impl Target {
    /// Lists `game`'s discs and installed DLC.
    pub fn game_details(&self, game: &Game) -> Result<GameDetails> {
        match self {
            Target::Local(_) => Ok(inspect_local(game)),
            Target::Ftp(ftp) => {
                let mut session = FtpSession::connect(ftp)?;
                let details = inspect_ftp(&mut session, game);
                session.quit();
                Ok(details)
            }
        }
    }
}

fn inspect_local(game: &Game) -> GameDetails {
    let mut details = GameDetails::default();

    if let Some(content_type) = god_content_type(game) {
        let type_dir = game.path.join(content_type);
        if let Ok(entries) = std::fs::read_dir(&type_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if !path.is_file() || !is_media_id(&name) {
                    continue;
                }
                let header_size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let data_dir = type_dir.join(format!("{name}.data"));
                let description = disc_description(stfs::inspect(&path).ok().flatten());
                details.discs.push(DiscInfo {
                    media_id: name.to_uppercase(),
                    description,
                    size: header_size + dir_size(&data_dir),
                });
            }
        }
    }

    let dlc_dir = game.path.join(dlc_dir_name());
    if let Ok(entries) = std::fs::read_dir(&dlc_dir) {
        for entry in entries.flatten() {
            if !entry.path().is_file() {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            details.dlc.push(DlcInfo { size });
        }
    }

    details
}

fn inspect_ftp(session: &mut FtpSession, game: &Game) -> GameDetails {
    let mut details = GameDetails::default();
    let remote = game.path.to_string_lossy().replace('\\', "/");

    if let Some(content_type) = god_content_type(game) {
        let type_dir = format!("{remote}/{content_type}");
        for entry in session.list_dir(&type_dir) {
            if entry.is_dir || !is_media_id(&entry.name) {
                continue;
            }
            let header_path = format!("{type_dir}/{}", entry.name);
            // The header file is small (a few tens of KiB): safe to
            // download in full, unlike the DLC packages below.
            let info = session.download_file(&header_path).ok().and_then(|bytes| {
                let mut cursor = std::io::Cursor::new(bytes);
                stfs::inspect_reader(&mut cursor, PathBuf::from(&header_path))
                    .ok()
                    .flatten()
            });
            let data_size = session.dir_size(&format!("{type_dir}/{}.data", entry.name), 1);
            details.discs.push(DiscInfo {
                media_id: entry.name.to_uppercase(),
                description: disc_description(info),
                size: entry.size + data_size,
            });
        }
    }

    for entry in session.list_dir(&format!("{remote}/{}", dlc_dir_name())) {
        if !entry.is_dir {
            details.dlc.push(DlcInfo { size: entry.size });
        }
    }

    details
}
