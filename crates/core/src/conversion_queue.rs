// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use derive_more::Display;
use std::path::PathBuf;

#[derive(Debug, Clone, Display)]
pub enum QueuedConversion {
    /// ISO ajoutée à la cible : conversion GOD ou extraction,
    /// selon le type d'image détecté.
    #[display("↑ Add: {}", _0.display())]
    Standard(PathBuf),
}
