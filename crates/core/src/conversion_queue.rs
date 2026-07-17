// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use derive_more::Display;
use std::path::PathBuf;

#[derive(Debug, Clone, Display)]
pub enum QueuedConversion {
    /// ISO added to target: GOD conversion or extraction,
    /// depending on the detected image type.
    #[display("↑ Add: {}", _0.display())]
    Standard(PathBuf),
}
