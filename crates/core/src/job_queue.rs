// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{game::Game, game_details::ContentKind};
use std::path::{Path, PathBuf};

/// Family a queued job belongs to. Mirrors `JobKind` in `ui/types.slint`
/// (same order), which picks the row icon and the wording of the
/// cancellation modals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Add,
    Delete,
    DeleteContent,
}

/// One unit of work in the queue. Everything that writes to the target goes
/// through it, so only one such operation is ever in flight — which is also
/// what the console's FTP server requires (see `crates/core/src/ftp.rs`).
#[derive(Debug, Clone)]
pub enum QueuedJob {
    /// ISO added to target: GOD conversion or extraction,
    /// depending on the detected image type.
    Add(PathBuf),
    /// Installed game removed from the target, along with its separate
    /// DLC/title-update folder when it has one.
    Delete(Box<Game>),
    /// One stored component of an installed game removed (a disc package or
    /// a DLC file), named by its on-disk `file_name`.
    DeleteContent {
        game: Box<Game>,
        kind: ContentKind,
        file_name: String,
        /// Human-readable description of the component, as shown in the game
        /// info modal ("Disc 1 · 545408A7", the DLC name…). Carried along so
        /// the queue row can name it: `file_name` alone is opaque.
        description: String,
    },
}

impl QueuedJob {
    pub fn kind(&self) -> JobKind {
        match self {
            Self::Add(_) => JobKind::Add,
            Self::Delete(_) => JobKind::Delete,
            Self::DeleteContent { .. } => JobKind::DeleteContent,
        }
    }

    /// File this entry works on: the input image for an addition, the
    /// installed game folder for a deletion. Used to tell whether a freshly
    /// picked file — or a game the user just asked to delete — is already
    /// waiting in the queue.
    pub fn path(&self) -> &Path {
        match self {
            Self::Add(path) => path,
            Self::Delete(game) => &game.path,
            Self::DeleteContent { game, .. } => &game.path,
        }
    }

    /// Text shown for this entry in the queue (row and progress card). Rows
    /// carry an icon of their own, so no marker is prepended here.
    pub fn label(&self) -> String {
        match self {
            // File name only: the full path is too long for the row and its
            // directory is the same for a whole batch anyway.
            Self::Add(path) => path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            Self::Delete(game) => game.title.clone(),
            Self::DeleteContent {
                game, description, ..
            } => format!("{}  ·  {description}", game.title),
        }
    }

    /// Whether two entries denote the same work, so the queue never holds a
    /// duplicate of it. Two deletions of the same component match on the
    /// component, not just on the game.
    pub fn is_same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Add(a), Self::Add(b)) => a == b,
            (Self::Delete(a), Self::Delete(b)) => a.path == b.path,
            (
                Self::DeleteContent {
                    game: a,
                    file_name: fa,
                    ..
                },
                Self::DeleteContent {
                    game: b,
                    file_name: fb,
                    ..
                },
            ) => a.path == b.path && fa == fb,
            _ => false,
        }
    }
}
