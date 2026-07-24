// SPDX-License-Identifier: GPL-3.0-only

//! Builds the stored-components table shown on the game info modal (discs,
//! DLC, and a "Missing" placeholder for an absent base disc).

use crate::util::GIB;
use crate::{ComponentStatus, DisplayedGameComponent};
use slint::ToSharedString;
use txbm_core::game_details::GameDetails;

/// One table row per disc and DLC, plus a synthetic "Missing" disc row when
/// the game is incomplete (only DLC/updates installed, base disc gone).
pub fn components(details: &GameDetails, incomplete: bool) -> Vec<DisplayedGameComponent> {
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

    if incomplete {
        rows.push(DisplayedGameComponent {
            kind: "Disc".into(),
            id: "".into(),
            description: "Game disc".into(),
            size_gib: 0.0,
            status: ComponentStatus::Missing,
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

fn status(readable: bool) -> ComponentStatus {
    if readable {
        ComponentStatus::Installed
    } else {
        ComponentStatus::Corrupted
    }
}
