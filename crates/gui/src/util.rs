// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use std::path::PathBuf;

pub const GIB: f32 = 1024. * 1024. * 1024.;

/// Quickly checks that a picked file looks like a usable ISO.
/// GOD games already installed for the same TitleID are still accepted:
/// re-adding overwrites the existing data, which is the common intent.
pub fn should_add_game(path: PathBuf) -> Option<PathBuf> {
    let _ = path.file_name()?;
    let ext = path.extension()?;

    if !ext.eq_ignore_ascii_case("iso") {
        return None;
    }

    // Cheap validity check: XDVDFS magic must be found by the ISO reader.
    txbm_core::iso_info::inspect(&path).ok()?;

    Some(path)
}
