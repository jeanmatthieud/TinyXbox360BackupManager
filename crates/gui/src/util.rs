// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use std::path::PathBuf;

pub const GIB: f32 = 1024. * 1024. * 1024.;

/// Quickly checks that a picked file looks like a usable input: an ISO,
/// an XBLA archive (.7z/.zip) or a bare STFS package (Arcade/DLC/TU).
/// Games already installed for the same TitleID are still accepted:
/// re-adding overwrites the existing data, which is the common intent.
pub fn should_add_game(path: PathBuf) -> Option<PathBuf> {
    let _ = path.file_name()?;

    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("iso"))
    {
        // Cheap validity check: XDVDFS magic must be found by the ISO reader.
        txbm_core::iso_info::inspect(&path).ok()?;
        return Some(path);
    }

    if txbm_core::archive::is_supported_archive(&path) {
        // The archive content (Arcade package present?) is validated
        // during the conversion itself.
        return txbm_core::archive::looks_valid(&path).then_some(path);
    }

    // Anything else: accept installable STFS packages.
    let info = txbm_core::stfs::inspect(&path).ok()??;
    matches!(
        info.content_type,
        txbm_core::stfs::CONTENT_TYPE_ARCADE
            | txbm_core::stfs::CONTENT_TYPE_DLC
            | txbm_core::stfs::CONTENT_TYPE_TITLE_UPDATE
    )
    .then_some(path)
}
