// SPDX-License-Identifier: GPL-3.0-only

use crate::xdvd::XdvdImage;
use anyhow::{Context, Result};
use iso2god::game_list;
use iso2god::god::ContentType;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoKind {
    /// Xbox 360 game (default.xex): to convert to GOD.
    Xbox360Game,
    /// Original Xbox game (default.xbe): to extract.
    XboxOriginal,
    /// Installation disc / DLC (no executable): extract its Content folder
    /// as-is, trusting its TitleID folder name.
    ContentDisc,
    /// Disc with its own executable (typically a bonus/"expansion
    /// installer" app) that also bundles DLC / title-update packages for a
    /// *different* game under a placeholder TitleID folder. Each bundled
    /// package must be installed under its own real TitleID, read from its
    /// STFS header, not from the installer's folder name.
    BundledContent,
    /// Disc that is a playable game *and* carries DLC / title updates for
    /// itself — Iso2God's "Mix" method: it needs both a GOD (or an extracted
    /// XEX) and its bundled packages installed.
    ///
    /// It cannot be told apart from an ordinary bonus disc by inspection; see
    /// [`MIX_DISCS`].
    GameWithBundledContent,
}

impl IsoKind {
    pub fn label(&self) -> &'static str {
        match self {
            IsoKind::Xbox360Game => "Xbox 360 Game",
            IsoKind::XboxOriginal => "Original Xbox Game",
            IsoKind::ContentDisc => "Installation disc / DLC",
            IsoKind::BundledContent => "Bonus disc (bundled DLC / updates)",
            IsoKind::GameWithBundledContent => "Xbox 360 Game + bundled content",
        }
    }
}

/// Discs that must be installed **both** as a game and as bundled content,
/// keyed by (TitleID, disc number).
///
/// There is no way to recognise them from the image. A survey of 33 Redump
/// dumps (`examples/mix_disc_survey.rs`, results in
/// `docs/iso2god-compatibility-notes.md`) tested five candidate criteria — the
/// XEX disc number/count, its module flags, its original PE name, the share of
/// the volume sitting under `/Content`, and whether the XEX claims the same
/// TitleID as the packages it carries — and every one of them puts Splinter
/// Cell: Blacklist D2 on the same side as discs that must *not* get a GOD.
/// Forza Motorsport 3 D2 settles it: same `2/2`, same `default.exe`, same
/// TitleID as its packages, and it wants no GOD.
///
/// So the list is held by hand, and it is short on purpose: "Mix" has exactly
/// one row in the whole Iso2God compatibility table. Further entries are
/// expected to come from user reports, not from a heuristic.
const MIX_DISCS: &[(u32, u8)] = &[
    // Tom Clancy's Splinter Cell: Blacklist, disc 2 — the second half of the
    // campaign (bootable) plus a 2.8 GiB HD texture pack sitting in
    // `Content/0000000000000000/555308B6/00000002`.
    (0x5553_08B6, 2),
];

fn is_mix_disc(title_id: u32, disc_number: u8) -> bool {
    MIX_DISCS.contains(&(title_id, disc_number))
}

#[derive(Debug, Clone)]
pub struct IsoInfo {
    pub path: PathBuf,
    pub kind: IsoKind,
    pub title_id: Option<String>,
    pub media_id: Option<String>,
    pub name: Option<String>,
    pub disc_number: Option<u8>,
    pub disc_count: Option<u8>,
}

/// Analyzes an ISO image and determines its type and metadata.
pub fn inspect(path: &Path) -> Result<IsoInfo> {
    let mut image = XdvdImage::open(path)?;

    let has_xex = image.find("/default.xex")?.is_some();
    let has_xbe = image.find("/default.xbe")?.is_some();
    // Bonus discs bundling DLC/title updates for a different game (e.g. an
    // "ExpansionInstaller" app) carry their own executable *and* a
    // Content/0000000000000000 tree; real games essentially never embed
    // that folder, so its presence takes priority.
    let has_bundled_content = image.find("/Content/0000000000000000")?.is_some();

    if !has_xex && !has_xbe {
        return Ok(IsoInfo {
            path: path.to_owned(),
            kind: IsoKind::ContentDisc,
            title_id: None,
            media_id: None,
            name: None,
            disc_number: None,
            disc_count: None,
        });
    }

    if has_bundled_content {
        // Reading the executable is best-effort here: a bonus disc's own XEX is
        // never converted, so one that fails to parse must not fail the whole
        // install — that used to work and has to keep working. It is read only
        // to look the disc up in `MIX_DISCS`, so only a successful read can
        // promote the disc to `GameWithBundledContent`.
        //
        // The identity it yields is reported for information. On a carrier disc
        // it is the `FFED2000` placeholder the installer declares, which is why
        // no caller uses it to decide where anything goes: each bundled package
        // is filed under the TitleID of its own STFS header.
        let title_info = image.title_info().ok();
        let exe = title_info.as_ref().map(|info| &info.execution_info);
        let kind = match exe {
            Some(exe) if is_mix_disc(exe.title_id, exe.disc_number) => {
                IsoKind::GameWithBundledContent
            }
            _ => IsoKind::BundledContent,
        };
        return Ok(IsoInfo {
            path: path.to_owned(),
            kind,
            title_id: exe.map(|exe| format!("{:08X}", exe.title_id)),
            media_id: exe.map(|exe| format!("{:08X}", exe.media_id)),
            name: exe.and_then(|exe| game_list::find_title_by_id(exe.title_id)),
            disc_number: exe.map(|exe| exe.disc_number),
            disc_count: exe.map(|exe| exe.disc_count),
        });
    }

    let title_info = image.title_info().context("reading game executable")?;
    let exe = &title_info.execution_info;

    let kind = match title_info.content_type {
        ContentType::GamesOnDemand => IsoKind::Xbox360Game,
        ContentType::XboxOriginal => IsoKind::XboxOriginal,
    };

    Ok(IsoInfo {
        path: path.to_owned(),
        kind,
        title_id: Some(format!("{:08X}", exe.title_id)),
        media_id: Some(format!("{:08X}", exe.media_id)),
        name: game_list::find_title_by_id(exe.title_id),
        disc_number: Some(exe.disc_number),
        disc_count: Some(exe.disc_count),
    })
}
