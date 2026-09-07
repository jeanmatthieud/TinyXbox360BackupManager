// SPDX-License-Identifier: GPL-3.0-only

//! Where a game's `<TitleID>` folder lives inside the GOD storage directory.
//!
//! The console's own layout is flat (`Content/0000000000000000/<TitleID>`), but
//! third-party managers (x360tm's "Game Tidy", among others) can nest that
//! folder under a human-readable parent — Aurora accepts it, since it reads the
//! STFS header rather than the path. Two rules follow from that:
//!
//!  * **reading** — the scanners walk one extra level (see
//!    [`crate::target::DEFAULT_SCAN_DEPTH`]), so both shapes are found;
//!  * **writing** — an existing `<TitleID>` folder is reused wherever it
//!    already sits, and only a brand-new game follows the configured
//!    [`GodLayout`]. Re-adding a game therefore overwrites it in place instead
//!    of creating a flat duplicate next to the nested original.

use crate::config::GodLayout;
use crate::remote_fs::RemoteFs;
use crate::game::is_title_id;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Display name used to build a parent folder, when the caller has none: the
/// bundled game list, falling back to the TitleID itself.
pub fn display_name(title_id: &str, name: Option<&str>) -> String {
    name.map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or_else(|| {
            u32::from_str_radix(title_id, 16)
                .ok()
                .and_then(iso2god::game_list::find_title_by_id)
        })
        .unwrap_or_else(|| title_id.to_string())
}

/// Local directory that must contain the `<TitleID>` folder of this game:
/// the one already holding it, otherwise the one `layout` asks for.
pub fn local_title_parent(
    god_dir: &Path,
    title_id: &str,
    name: Option<&str>,
    layout: GodLayout,
) -> PathBuf {
    if let Some(parent) = find_local_title_parent(god_dir, title_id) {
        return parent;
    }
    match layout.parent_folder(title_id, &display_name(title_id, name)) {
        Some(folder) => god_dir.join(folder),
        None => god_dir.to_path_buf(),
    }
}

/// Local directory holding the `<TitleID>` folder of an already-installed
/// game, looked up directly under `god_dir` then one level below it.
///
/// TitleIDs are matched case-insensitively, like the FTP counterpart: FATX (and
/// Windows) ignore case, so `4d5307d5` and `4D5307D5` are the same game, and a
/// library staged on a case-sensitive filesystem must not end up with both.
fn find_local_title_parent(god_dir: &Path, title_id: &str) -> Option<PathBuf> {
    // A single listing serves both lookups: a flat hit wins over a nested one,
    // so the named parents are only descended into once `god_dir` itself is
    // known not to hold the TitleID folder.
    let mut named_parents = Vec::new();
    for entry in std::fs::read_dir(god_dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.eq_ignore_ascii_case(title_id) {
            return Some(god_dir.to_path_buf());
        }
        // Nested layouts: only folders that are not TitleIDs themselves can be
        // a named parent, which keeps a flat library from being walked at all.
        if !is_title_id(&name) {
            named_parents.push(path);
        }
    }
    named_parents
        .into_iter()
        .find(|parent| contains_title_dir(parent, title_id))
}

/// True when `dir` holds a `<TitleID>` sub-directory, whatever its case.
fn contains_title_dir(dir: &Path, title_id: &str) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            e.file_name().to_string_lossy().eq_ignore_ascii_case(title_id)
                && e.path().is_dir()
        })
    })
}

/// Snapshot of every `<TitleID>` folder already present under a console's GOD
/// directory (flat or nested under a named parent), keyed by TitleID.
///
/// Built with one listing per already-existing named parent, same as the
/// per-title lookup it replaces — but built **once per upload batch** and
/// reused for every title in it, instead of walking the console again for
/// each one. A batch adding N new games to a library of M named parents costs
/// `M` extra listings total instead of up to `N * M`.
pub struct GodDirIndex {
    god_dir: String,
    /// TitleID (uppercase) -> the directory holding its folder.
    by_title: HashMap<String, String>,
}

impl GodDirIndex {
    /// Builds the index by listing `god_dir` and every named (non-TitleID)
    /// parent directly under it.
    pub fn build_remote(session: &mut dyn RemoteFs, god_dir: &str) -> Self {
        let god_dir = god_dir.trim_end_matches('/').to_string();
        let mut by_title = HashMap::new();
        let children = session.list_dir(&god_dir);
        for entry in &children {
            if !entry.is_dir {
                continue;
            }
            if is_title_id(&entry.name) {
                by_title.insert(entry.name.to_uppercase(), god_dir.clone());
                continue;
            }
            let path = format!("{god_dir}/{}", entry.name);
            for sub in session.list_dir(&path) {
                if sub.is_dir && is_title_id(&sub.name) {
                    by_title.insert(sub.name.to_uppercase(), path.clone());
                }
            }
        }
        Self { god_dir, by_title }
    }

    fn parent_of(&self, title_id: &str) -> Option<&str> {
        self.by_title.get(&title_id.to_uppercase()).map(String::as_str)
    }
}

/// Remote counterpart of [`local_title_parent`]. `god_dir` and the result are
/// absolute console paths (`/Hdd1/Content/0000000000000000`). `index` must be
/// built from the same `god_dir`.
pub fn remote_title_parent(
    index: &GodDirIndex,
    title_id: &str,
    name: Option<&str>,
    layout: GodLayout,
) -> String {
    if let Some(parent) = index.parent_of(title_id) {
        return parent.to_string();
    }
    match layout.parent_folder(title_id, &display_name(title_id, name)) {
        Some(folder) => format!("{}/{folder}", index.god_dir),
        None => index.god_dir.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuses_a_nested_title_folder_whatever_the_layout() {
        let dir = std::env::temp_dir().join("txbm-god-dirs");
        let _ = std::fs::remove_dir_all(&dir);
        let god = dir.join("Content/0000000000000000");
        std::fs::create_dir_all(god.join("Gears of War/4D5307D5/00007000")).unwrap();

        // Configured flat, but the game already sits under a named parent.
        assert_eq!(
            local_title_parent(&god, "4D5307D5", Some("Gears of War"), GodLayout::TitleId),
            god.join("Gears of War")
        );
        // A new game follows the configured layout.
        assert_eq!(
            local_title_parent(&god, "4D5308AB", Some("Halo 3"), GodLayout::NameSlashTitleId),
            god.join("Halo 3")
        );
        assert_eq!(
            local_title_parent(&god, "4D5308AB", Some("Halo 3"), GodLayout::TitleId),
            god
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_flat_title_folder_wins_over_a_nested_one() {
        let dir = std::env::temp_dir().join("txbm-god-dirs-flat");
        let _ = std::fs::remove_dir_all(&dir);
        let god = dir.join("Content/0000000000000000");
        std::fs::create_dir_all(god.join("4D5307D5")).unwrap();
        std::fs::create_dir_all(god.join("Gears of War/4D5307D5")).unwrap();

        assert_eq!(
            local_title_parent(&god, "4D5307D5", None, GodLayout::NameSlashTitleId),
            god
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_existing_title_folder_is_matched_whatever_its_case() {
        let dir = std::env::temp_dir().join("txbm-god-dirs-case");
        let _ = std::fs::remove_dir_all(&dir);
        let god = dir.join("Content/0000000000000000");
        // Lower-cased on disk, upper-cased by the scanner: same game.
        std::fs::create_dir_all(god.join("4d5307d5/00007000")).unwrap();
        assert_eq!(
            local_title_parent(&god, "4D5307D5", None, GodLayout::NameSlashTitleId),
            god
        );

        // Same, one level below a named parent.
        let god = dir.join("Content2/0000000000000000");
        std::fs::create_dir_all(god.join("Gears of War/4d5307d5")).unwrap();
        assert_eq!(
            local_title_parent(&god, "4D5307D5", None, GodLayout::TitleId),
            god.join("Gears of War")
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parent_folder_shapes() {
        assert_eq!(GodLayout::TitleId.parent_folder("4D5307D5", "Gears of War"), None);
        assert_eq!(
            GodLayout::NameSlashTitleId.parent_folder("4D5307D5", "Gears of War"),
            Some("Gears of War".to_string())
        );
        assert_eq!(
            GodLayout::NameDashTitleId.parent_folder("4D5307D5", "Gears of War"),
            Some("Gears of War - 4D5307D5".to_string())
        );
        assert_eq!(
            GodLayout::TitleIdDashName.parent_folder("4D5307D5", "Gears of War"),
            Some("4D5307D5 - Gears of War".to_string())
        );
        // No name to work with: the TitleID stands in, no dangling separator.
        assert_eq!(
            GodLayout::NameSlashTitleId.parent_folder("4D5307D5", "  "),
            Some("4D5307D5".to_string())
        );
        // Slashes would fork the path; the FATX name limit is enforced.
        let long = GodLayout::NameSlashTitleId
            .parent_folder("4D5307D5", "Tom Clancy's Rainbow Six: Vegas 2 / Special Edition")
            .unwrap();
        assert!(!long.contains('/'));
        assert!(long.chars().count() <= crate::game::FATX_MAX_NAME);
    }
}
