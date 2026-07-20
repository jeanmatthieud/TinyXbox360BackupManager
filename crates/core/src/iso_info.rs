// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result};
use iso2god::executable::TitleInfo;
use iso2god::god::ContentType;
use iso2god::{game_list, iso};
use std::fs::File;
use std::io::BufReader;
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
    pub title_id: Option<String>,
    pub media_id: Option<String>,
    pub name: Option<String>,
    pub disc_number: Option<u8>,
    pub disc_count: Option<u8>,
}

/// Analyzes an ISO image and determines its type and metadata.
pub fn inspect(path: &Path) -> Result<IsoInfo> {
    let file =
        File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut reader = iso::IsoReader::read(BufReader::new(file))
        .context("reading image (invalid XDVDFS format?)")?;

    let has_xex = reader.get_entry(&"\\default.xex".into())?.is_some();
    let has_xbe = reader.get_entry(&"\\default.xbe".into())?.is_some();
    // Bonus discs bundling DLC/title updates for a different game (e.g. an
    // "ExpansionInstaller" app) carry their own executable *and* a
    // Content/0000000000000000 tree; real games essentially never embed
    // that folder, so its presence takes priority.
    let has_bundled_content = reader
        .get_entry(&"\\Content\\0000000000000000".into())?
        .is_some();

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
        return Ok(IsoInfo {
            path: path.to_owned(),
            kind: IsoKind::BundledContent,
            title_id: None,
            media_id: None,
            name: None,
            disc_number: None,
            disc_count: None,
        });
    }

    let title_info =
        TitleInfo::from_image(&mut reader).context("reading game executable")?;
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
