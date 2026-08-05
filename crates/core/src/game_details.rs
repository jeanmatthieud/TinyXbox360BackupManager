// SPDX-License-Identifier: GPL-3.0-only

//! Disc and DLC listing for an installed GOD game: one entry per STFS
//! package found in its content folder (one per disc, sharing the same
//! TitleID) and one per installed DLC / marketplace package.

use crate::ftp::FtpSession;
use crate::game::{Game, GameFormat};
use crate::stfs::{self, dlc_dir_name};
use crate::target::{Target, remove_dir_all_with_progress};
use crate::util::dir_size;
use anyhow::{Context, Result};
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
    /// On-disk header file name (the MediaID in its original case); its GOD
    /// payload lives in the sibling `<file_name>.data` folder. Used to delete
    /// the disc.
    pub file_name: String,
    /// e.g. "Disc 1 of 2", or plain "Disc" for single-disc games.
    pub description: String,
    pub size: u64,
    /// False when the STFS header couldn't be parsed (no magic / read
    /// error) — a disc header always should, so this flags a corrupted or
    /// interrupted install.
    pub readable: bool,
}

#[derive(Debug, Clone)]
pub struct DlcInfo {
    /// On-disk file name under the DLC folder. Used to delete the package.
    pub file_name: String,
    /// The DLC's own display name from its STFS header, when readable
    /// (e.g. "Multiplayer Map Pack"). `None` if the file isn't a parseable
    /// STFS package or carries no name.
    pub name: Option<String>,
    pub size: u64,
    /// False when the STFS header couldn't be parsed (no magic / read
    /// error): the package is corrupted or was only partially uploaded.
    pub readable: bool,
}

/// Which kind of stored component a delete targets.
#[derive(Debug, Clone, Copy)]
pub enum ContentKind {
    /// A game disc package: its header file plus its `.data` payload folder.
    Disc,
    /// A DLC / marketplace package: a single file under the DLC folder.
    Dlc,
}

/// Best human-readable name for a DLC package: its own `display_name` first
/// (the content name), falling back to `title_name` (the parent game).
fn dlc_name(info: &stfs::StfsInfo) -> Option<String> {
    info.display_name
        .clone()
        .or_else(|| info.title_name.clone())
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

    /// Deletes a single stored component of `game`: one disc package (its
    /// header file plus its `.data` payload folder) or one DLC file, named by
    /// its on-disk `file_name`. `update_progress` receives a percentage
    /// (0-100). Over FTP this is a WRITE, so it must not run concurrently with
    /// any other connection.
    pub fn delete_content(
        &self,
        game: &Game,
        kind: ContentKind,
        file_name: &str,
        update_progress: &dyn Fn(u32),
    ) -> Result<()> {
        match self {
            Target::Local(_) => delete_content_local(game, kind, file_name, update_progress),
            Target::Ftp(ftp) => {
                let mut session = FtpSession::connect(ftp)?;
                let result = delete_content_ftp(&mut session, game, kind, file_name, update_progress);
                session.quit();
                result
            }
        }
    }
}

fn delete_content_local(
    game: &Game,
    kind: ContentKind,
    file_name: &str,
    update_progress: &dyn Fn(u32),
) -> Result<()> {
    match kind {
        ContentKind::Disc => {
            let content_type =
                god_content_type(game).context("deleting a disc from a non-GOD game")?;
            let type_dir = game.path.join(content_type);
            let data_dir = type_dir.join(format!("{file_name}.data"));
            // Remove the (potentially large) payload with progress, then the
            // small header. Both are best-effort about existence so a corrupted
            // install (e.g. header without payload) can still be cleaned up.
            if data_dir.is_dir() {
                let total = crate::util::file_count(&data_dir).max(1);
                let mut done: u64 = 0;
                remove_dir_all_with_progress(&data_dir, &mut done, total, update_progress)?;
            }
            let header = type_dir.join(file_name);
            if header.is_file() {
                std::fs::remove_file(&header)
                    .with_context(|| format!("removing {}", header.display()))?;
            }
            update_progress(100);
            Ok(())
        }
        ContentKind::Dlc => {
            let file = game.content_dir().join(dlc_dir_name()).join(file_name);
            std::fs::remove_file(&file)
                .with_context(|| format!("removing {}", file.display()))?;
            update_progress(100);
            Ok(())
        }
    }
}

fn delete_content_ftp(
    session: &mut FtpSession,
    game: &Game,
    kind: ContentKind,
    file_name: &str,
    update_progress: &dyn Fn(u32),
) -> Result<()> {
    let remote = game.path.to_string_lossy().replace('\\', "/");
    match kind {
        ContentKind::Disc => {
            let content_type =
                god_content_type(game).context("deleting a disc from a non-GOD game")?;
            let type_dir = format!("{remote}/{content_type}");
            // Payload removal is best-effort (a corrupted disc may have a
            // header but no `.data` folder); the header removal is what makes
            // the disc disappear from the listing.
            let data_dir = format!("{type_dir}/{file_name}.data");
            let _ = session.remove_dir_recursive(&data_dir, &mut |done, total| {
                update_progress((done * 100 / total.max(1)) as u32);
            });
            session.remove_file(&type_dir, file_name)?;
            update_progress(100);
            Ok(())
        }
        ContentKind::Dlc => {
            let content = game.content_dir().to_string_lossy().replace('\\', "/");
            let dlc_dir = format!("{content}/{}", dlc_dir_name());
            session.remove_file(&dlc_dir, file_name)?;
            update_progress(100);
            Ok(())
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
                let info = stfs::inspect(&path).ok().flatten();
                let readable = info.is_some();
                details.discs.push(DiscInfo {
                    media_id: name.to_uppercase(),
                    description: disc_description(info),
                    size: header_size + dir_size(&data_dir),
                    readable,
                    file_name: name,
                });
            }
        }
    }

    let dlc_dir = game.content_dir().join(dlc_dir_name());
    if let Ok(entries) = std::fs::read_dir(&dlc_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let info = stfs::inspect(&path).ok().flatten();
            let readable = info.is_some();
            let name = info.as_ref().and_then(dlc_name);
            details.dlc.push(DlcInfo {
                file_name,
                name,
                size,
                readable,
            });
        }
    }

    details
}

fn inspect_ftp(session: &mut FtpSession, game: &Game) -> GameDetails {
    let mut details = GameDetails::default();
    let remote = game.path.to_string_lossy().replace('\\', "/");
    let content = game.content_dir().to_string_lossy().replace('\\', "/");

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
            let readable = info.is_some();
            let data_size = session.dir_size(&format!("{type_dir}/{}.data", entry.name), 1);
            details.discs.push(DiscInfo {
                media_id: entry.name.to_uppercase(),
                description: disc_description(info),
                size: entry.size + data_size,
                readable,
                file_name: entry.name,
            });
        }
    }

    let dlc_dir = format!("{content}/{}", dlc_dir_name());
    for entry in session.list_dir(&dlc_dir) {
        if entry.is_dir {
            continue;
        }
        // Read just the STFS header prefix (Aurora has no REST, so this is a
        // prefix read from offset 0) rather than downloading the whole DLC.
        let header_path = format!("{dlc_dir}/{}", entry.name);
        let info = session
            .download_prefix(&header_path, stfs::HEADER_SIZE)
            .ok()
            .and_then(|bytes| {
                let mut cursor = std::io::Cursor::new(bytes);
                stfs::inspect_reader(&mut cursor, PathBuf::from(&header_path))
                    .ok()
                    .flatten()
            });
        let readable = info.is_some();
        let name = info.as_ref().and_then(dlc_name);
        details.dlc.push(DlcInfo {
            name,
            size: entry.size,
            readable,
            file_name: entry.name,
        });
    }

    details
}
