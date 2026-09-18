// SPDX-License-Identifier: GPL-3.0-only

//! Per-disc deviations from what inspecting an image can decide on its own.
//!
//! Some discs cannot be handled correctly from what they contain: two images
//! with the same shape need opposite treatments, and only the published
//! compatibility lists — or a user report — say which is which. Those cases
//! live here, keyed by (TitleID, disc number), rather than as extra
//! [`crate::iso_info::IsoKind`] variants: `IsoKind` describes what an image
//! *is*, a quirk describes what we decided to do with one.
//!
//! The table is short on purpose, and it is expected to grow from reports
//! rather than from heuristics. See `docs/iso2god-compatibility-notes.md` for
//! the survey that established there is nothing to detect.

/// What to do with a disc, when inspection alone gets it wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscQuirk {
    /// Iso2God's "Mix": the disc is a playable game *and* carries DLC or title
    /// updates for itself, so it needs both treatments.
    GameAndBundledContent,

    /// Extract the disc into a game folder whatever format the target is
    /// configured for, because the game is only complete once another disc's
    /// folders are merged into it — and a GOD container cannot be completed
    /// after the fact.
    ///
    /// `entry_point`, when set, names the file that must end up as
    /// `default.xex` for the game to start (see
    /// `crate::convert::swap_entry_point`).
    ForceExtractedGame { entry_point: Option<&'static str> },

    /// The disc contributes nothing but what the named root folders *contain* to
    /// the extracted game of *another* disc of the same title.
    ///
    /// The folders are a wrapper the installer would have unpacked, not part of
    /// the layout the game expects: their contents land at the root of the game
    /// folder, the folder level itself is dropped. Everything else on the disc
    /// is redundant with the game disc, and copying it would overwrite that
    /// disc's own executable.
    MergesFolderContents(&'static [&'static str]),
}

/// (TitleID, disc number, what to do).
const QUIRKS: &[(u32, u8, DiscQuirk)] = &[
    // Tom Clancy's Splinter Cell: Blacklist, disc 2 — the second half of the
    // campaign (bootable) plus a 2.8 GiB HD texture pack in
    // `Content/0000000000000000/555308B6/00000002`.
    //
    // Nothing on the image distinguishes it from a bonus disc that must *not*
    // get a GOD: a survey of 33 Redump dumps tested the XEX disc number, its
    // module flags, its original PE name, the share of the volume under
    // `/Content`, and whether the XEX claims the same TitleID as its packages.
    // All five put this disc on the same side as Forza Motorsport 3 disc 2,
    // which is `2/2`, named `default.exe`, shares its packages' TitleID — and
    // wants no GOD.
    (0x5553_08B6, 2, DiscQuirk::GameAndBundledContent),
    // Watch_Dogs — a two-disc set whose halves only work together.
    //
    // Disc 1 is an installation disc: `installer.exe` (as `default.xex`) unpacks
    // `installation1` and `installation2` onto the hard drive, 6.5 GiB of the
    // 7.5 GiB it uses. Everything else on it duplicates disc 2, and its
    // `default.xex` would overwrite disc 2's.
    //
    // What they hold is ordinary game data — `common.dat`/`.fat`,
    // `shadersobj.dat`/`.fat`, `sound.dat`/`.fat`, `vidx/`, `worlds/` — which
    // belongs at the root of the game, beside disc 2's own `sound_*.dat`/`.fat`
    // pairs. Measured on a Redump dump: 281 files, and the two folders collide
    // neither with each other nor with anything on disc 2, so the merge cannot
    // overwrite anything and the order the discs are added in stays irrelevant.
    (
        0x5553_08B7,
        1,
        DiscQuirk::MergesFolderContents(&["installation1", "installation2"]),
    ),
    // Disc 2 holds the game. Iso2God's answer is to merge disc 1's folders in,
    // rebuild a ~10 GiB ISO and convert that; we cannot write an XDVDFS image,
    // but an extracted folder *is* what that ISO would contain, and it can be
    // completed by disc 1 whichever order the two are added in.
    //
    // Its `default.xex` is `starter.exe`, a 1.9 MiB launcher that looks for an
    // installation laid down by disc 1's installer and will not chain from a
    // hard drive; the real game sits beside it as `game.xex` — and that file
    // was built as `default.exe`, so the rename merely undoes the swap made at
    // mastering time.
    (
        0x5553_08B7,
        2,
        DiscQuirk::ForceExtractedGame {
            entry_point: Some("game.xex"),
        },
    ),
];

/// The quirk that applies to a disc, if any.
pub fn lookup(title_id: u32, disc_number: u8) -> Option<DiscQuirk> {
    QUIRKS
        .iter()
        .find(|(tid, disc, _)| *tid == title_id && *disc == disc_number)
        .map(|(_, _, quirk)| *quirk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_by_title_and_disc() {
        assert_eq!(
            lookup(0x5553_08B6, 2),
            Some(DiscQuirk::GameAndBundledContent)
        );
        // Disc 1 of the same game is an ordinary disc.
        assert_eq!(lookup(0x5553_08B6, 1), None);
        assert_eq!(lookup(0x1234_5678, 1), None);
    }

    #[test]
    fn watch_dogs_discs_have_opposite_roles() {
        assert_eq!(
            lookup(0x5553_08B7, 1),
            Some(DiscQuirk::MergesFolderContents(&[
                "installation1",
                "installation2"
            ]))
        );
        assert_eq!(
            lookup(0x5553_08B7, 2),
            Some(DiscQuirk::ForceExtractedGame {
                entry_point: Some("game.xex")
            })
        );
    }
}
