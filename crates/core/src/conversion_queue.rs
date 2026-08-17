// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    fmt,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub enum QueuedConversion {
    /// ISO added to target: GOD conversion or extraction,
    /// depending on the detected image type.
    Standard(PathBuf),
}

impl QueuedConversion {
    /// Input file this entry will convert. Used to tell whether a freshly
    /// picked file is already waiting in the queue.
    pub fn path(&self) -> &Path {
        match self {
            Self::Standard(path) => path,
        }
    }
}

/// Queue rows show the file name only: the full path is too long for the
/// column and its directory is the same for a whole batch anyway.
impl fmt::Display for QueuedConversion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Standard(path) => {
                let name = path.file_name().unwrap_or(path.as_os_str());
                write!(f, "↑  {}", name.to_string_lossy())
            }
        }
    }
}
