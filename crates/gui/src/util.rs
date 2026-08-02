// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use std::path::PathBuf;

pub const GIB: f32 = 1024. * 1024. * 1024.;

/// A picked file accepted for conversion.
pub struct PickedGame {
    pub path: PathBuf,
    /// TitleID of the game this input would install, when it is known without
    /// any extra work — the inspection this pick already did read it. Used to
    /// warn, before the queue is confirmed, that an installed game would be
    /// overwritten (see `State::set_games_to_add`).
    ///
    /// Which inputs carry one, and why:
    ///
    /// - Xbox 360 / Original Xbox ISO → yes, from the disc's executable.
    /// - Arcade STFS package → yes: it is a game in its own right.
    /// - DLC / title update / content disc / bundled-content disc → `None`.
    ///   They install *beside* a game (under its TitleID, or under the one of
    ///   each package they carry, the disc's own being a placeholder) and
    ///   merge rather than replace: there is no overwrite to announce.
    /// - **Archive (.zip/.7z) → `None`, deliberately and permanently.** The
    ///   TitleID sits in the `default.xex`, hundreds of megabytes into an
    ///   image that is itself a multi-gigabyte deflate stream — not seekable,
    ///   so reading it means unpacking a large part of every archive before
    ///   the confirmation can even be shown (minutes, on a recursive add of a
    ///   whole Redump folder). Guessing from the file name instead was
    ///   rejected too: it mistakes "Bad Company" for "Bad Company 2". An
    ///   archive therefore gets no annotation at all — silence was preferred
    ///   over unreliable information. The overwrite itself still happens
    ///   correctly, just unannounced.
    pub installs_title_id: Option<String>,
}

/// Quickly checks that a picked file looks like a usable input: an ISO,
/// an XBLA archive (.7z/.zip) or a bare STFS package (Arcade/DLC/TU).
/// Games already installed for the same TitleID are still accepted:
/// re-adding overwrites the existing data, which is the common intent.
pub fn should_add_game(path: PathBuf) -> Option<PickedGame> {
    use txbm_core::iso_info::IsoKind;

    let _ = path.file_name()?;

    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("iso"))
    {
        // Cheap validity check: XDVDFS magic must be found by the ISO reader.
        let info = txbm_core::iso_info::inspect(&path).ok()?;
        // A content/bundled disc installs under the TitleID of each package it
        // carries, not under the disc's own (often a placeholder), and merges
        // rather than replaces: no overwrite to announce.
        let installs_title_id = matches!(
            info.kind,
            IsoKind::Xbox360Game | IsoKind::XboxOriginal
        )
        .then_some(info.title_id)
        .flatten();
        return Some(PickedGame {
            path,
            installs_title_id,
        });
    }

    if txbm_core::archive::is_supported_archive(&path) {
        // The archive content (Arcade package present?) is validated
        // during the conversion itself — and so is the game it installs:
        // see `installs_title_id` for why it stays unknown until then.
        return txbm_core::archive::looks_valid(&path).then_some(PickedGame {
            path,
            installs_title_id: None,
        });
    }

    // Anything else: accept installable STFS packages.
    let info = txbm_core::stfs::inspect(&path).ok()??;
    matches!(
        info.content_type,
        txbm_core::stfs::CONTENT_TYPE_ARCADE
            | txbm_core::stfs::CONTENT_TYPE_DLC
            | txbm_core::stfs::CONTENT_TYPE_TITLE_UPDATE
    )
    .then(|| PickedGame {
        // Only an Arcade package is a game in its own right; DLC and title
        // updates land beside an existing install without replacing it.
        installs_title_id: (info.content_type == txbm_core::stfs::CONTENT_TYPE_ARCADE)
            .then(|| info.title_id.clone()),
        path,
    })
}
