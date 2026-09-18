// SPDX-License-Identifier: GPL-3.0-only

use crate::quirks::{self, DiscQuirk};
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
}

impl IsoKind {
    pub fn label(&self) -> &'static str {
        match self {
            IsoKind::Xbox360Game => "Xbox 360 Game",
            IsoKind::XboxOriginal => "Original Xbox Game",
            IsoKind::ContentDisc => "Installation disc / DLC",
            IsoKind::BundledContent => "Bonus disc (bundled DLC / updates)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct IsoInfo {
    pub path: PathBuf,
    pub kind: IsoKind,
    /// Set when this exact disc needs a treatment its shape does not imply.
    /// It takes precedence over `kind`; see [`crate::quirks`].
    pub quirk: Option<DiscQuirk>,
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
            // No executable, so no TitleID to look a quirk up by.
            quirk: None,
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
        // to look the disc up in the quirks table, so an unreadable one simply
        // gets no quirk.
        //
        // The identity it yields is reported for information. On a carrier disc
        // it is the `FFED2000` placeholder the installer declares, which is why
        // no caller uses it to decide where anything goes: each bundled package
        // is filed under the TitleID of its own STFS header.
        let title_info = image.title_info().ok();
        let exe = title_info.as_ref().map(|info| &info.execution_info);
        return Ok(IsoInfo {
            path: path.to_owned(),
            kind: IsoKind::BundledContent,
            quirk: exe.and_then(|exe| quirks::lookup(exe.title_id, exe.disc_number)),
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
        quirk: quirks::lookup(exe.title_id, exe.disc_number),
        title_id: Some(format!("{:08X}", exe.title_id)),
        media_id: Some(format!("{:08X}", exe.media_id)),
        name: game_list::find_title_by_id(exe.title_id),
        disc_number: Some(exe.disc_number),
        disc_count: Some(exe.disc_count),
    })
}
