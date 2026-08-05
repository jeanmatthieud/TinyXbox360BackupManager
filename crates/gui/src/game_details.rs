// SPDX-License-Identifier: GPL-3.0-only

//! Builds the stored-components table shown on the game info modal (discs,
//! DLC, and a "Missing" placeholder for an absent base disc).

use crate::util::GIB;
use crate::{ComponentStatus, DisplayedGame, DisplayedGameComponent};
use slint::ToSharedString;
use txbm_core::game_details::GameDetails;

/// One table row per disc and DLC, plus a synthetic "Missing" disc row when
/// the game is incomplete (only DLC/updates installed, base disc gone).
///
/// Only GOD games are made of discrete disc packages; an XBLA title or an
/// extracted folder has no per-disc listing, so a synthetic "Game" row stands
/// in for the install itself. That keeps the table an inventory of what is
/// stored, whatever the format — otherwise these games showed an empty table
/// while a plain GOD game showed a row.
pub fn components(details: &GameDetails, game: &DisplayedGame) -> Vec<DisplayedGameComponent> {
    let mut rows = Vec::new();

    for disc in &details.discs {
        rows.push(DisplayedGameComponent {
            kind: "Disc".into(),
            id: disc.file_name.as_str().into(),
            description: slint::format!("{} · {}", disc.description, disc.media_id),
            size_gib: disc.size as f32 / GIB,
            status: status(disc.readable),
        });
    }

    if game.incomplete {
        rows.push(DisplayedGameComponent {
            kind: "Disc".into(),
            id: "".into(),
            description: "Game disc".into(),
            size_gib: 0.0,
            status: ComponentStatus::Missing,
        });
    } else if details.discs.is_empty() {
        // Not a disc-based install: the game itself is the single component.
        // Left with an empty `id`, so no per-component delete button shows —
        // removing it means deleting the game.
        rows.push(DisplayedGameComponent {
            kind: "Game".into(),
            id: "".into(),
            description: base_description(game),
            size_gib: game.size_gib,
            status: ComponentStatus::Installed,
        });
    }

    for (i, dlc) in details.dlc.iter().enumerate() {
        let description = dlc
            .name
            .clone()
            .map(|n| n.to_shared_string())
            .unwrap_or_else(|| slint::format!("DLC {}", i + 1));
        rows.push(DisplayedGameComponent {
            kind: "DLC".into(),
            id: dlc.file_name.as_str().into(),
            description,
            size_gib: dlc.size as f32 / GIB,
            status: status(dlc.readable),
        });
    }

    rows
}

/// Label for the synthetic base-install row, from the format string built in
/// `GameFormat::label` ("XBLA", "XEX", "XBE").
fn base_description(game: &DisplayedGame) -> slint::SharedString {
    match game.format.as_str() {
        "XBLA" => "Arcade package".into(),
        "XEX" | "XBE" => "Extracted files".into(),
        _ => "Game files".into(),
    }
}

fn status(readable: bool) -> ComponentStatus {
    if readable {
        ComponentStatus::Installed
    } else {
        ComponentStatus::Corrupted
    }
}
