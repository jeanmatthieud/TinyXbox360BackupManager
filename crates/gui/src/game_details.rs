// SPDX-License-Identifier: GPL-3.0-only

//! Formats the disc/DLC listing shown on the game info modal, below the
//! Size line.

use crate::util::GIB;
use slint::SharedString;
use txbm_core::game_details::GameDetails;

pub fn disc_lines(details: &GameDetails) -> Vec<SharedString> {
    details
        .discs
        .iter()
        .map(|disc| {
            slint::format!(
                "{} {}: {:.2} GiB",
                disc.description,
                disc.media_id,
                disc.size as f32 / GIB
            )
        })
        .collect()
}

pub fn dlc_lines(details: &GameDetails) -> Vec<SharedString> {
    details
        .dlc
        .iter()
        .enumerate()
        .map(|(i, dlc)| slint::format!("DLC {}: {:.2} GiB", i + 1, dlc.size as f32 / GIB))
        .collect()
}
