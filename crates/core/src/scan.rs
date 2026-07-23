// SPDX-License-Identifier: GPL-3.0-only
// Based on the architecture of TinyWiiBackupManager by Manuel Quarneti.

//! Backend-agnostic directory walk shared by the local and FTP scanners.
//!
//! Both targets discover games the same way — for each sub-directory, decide
//! whether it *is* a game (GOD/Arcade TitleID folder or an extracted-game
//! folder) or a container to descend into up to the location's scan depth. Only
//! the I/O and the game-construction differ between backends, so those are the
//! two methods a backend implements ([`DirScanner::child_dirs`] and
//! [`DirScanner::classify`]); the recursion and depth handling live here once.

use anyhow::Result;

/// What to do with a child directory after the backend has looked at it.
pub(crate) enum ChildAction {
    /// A game (or incomplete entry) was built from it; nothing more to do.
    Handled,
    /// Not a game itself: recurse into it if the remaining depth allows.
    Recurse,
}

/// A scanning backend (local filesystem or FTP session). Implementors keep the
/// discovered games as internal state; [`walk`] only drives the traversal.
pub(crate) trait DirScanner {
    /// Path identifying a directory on this backend (`PathBuf` locally,
    /// `String` over FTP).
    type Path;

    /// Direct sub-directories of `dir`, as `(path, file_name)` pairs. Returns
    /// an error only for conditions that must abort the whole scan (e.g. a
    /// user cancellation); a plain I/O failure to list a folder yields an
    /// empty list.
    fn child_dirs(&mut self, dir: &Self::Path) -> Result<Vec<(Self::Path, String)>>;

    /// Inspects a child directory and either builds a game from it
    /// ([`ChildAction::Handled`]) or asks to recurse ([`ChildAction::Recurse`]).
    fn classify(&mut self, path: &Self::Path, name: &str) -> Result<ChildAction>;
}

/// Walks `dir` and its sub-directories up to `depth` levels, dispatching each
/// child through the scanner. `depth` counts the levels below `dir` that are
/// explored: `1` scans only the direct children, `2` one nested level, etc.
pub(crate) fn walk<S: DirScanner>(scanner: &mut S, dir: &S::Path, depth: u32) -> Result<()> {
    for (child, name) in scanner.child_dirs(dir)? {
        match scanner.classify(&child, &name)? {
            ChildAction::Handled => {}
            ChildAction::Recurse if depth > 1 => walk(scanner, &child, depth - 1)?,
            ChildAction::Recurse => {}
        }
    }
    Ok(())
}
