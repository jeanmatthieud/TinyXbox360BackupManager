// SPDX-License-Identifier: GPL-3.0-only

//! Target of the library: local drive (USB) or remote console (FTP).
//! All operations (scan, deletion, installation) apply directly to the target.
//!
//! Storage locations are format-agnostic: a target exposes a set of
//! [`ScanLocation`]s (scanned for games of any format) and a [`StorageConfig`]
//! (where new GOD containers and extracted games are installed). Their
//! resolution order is:
//!   1. a `.txbm.json` manifest on the target (authoritative, user-confirmed);
//!   2. Aurora's own scan paths (read from its SQLite databases over FTP);
//!   3. built-in defaults (`Content/0000000000000000` + `Games`).

use crate::config::{ConfigContents, GodLayout, TargetKind};
use crate::data_dir::DATA_DIR;
use crate::drive_info::DriveInfo;
use crate::ftp::{FtpConfig, FtpSession};
use crate::game::{self, Game, GameFormat};
use crate::{DEFAULT_GOD_DIR, DEFAULT_XEX_DIR, DEFAULT_XBE_DIR};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Error message used when a scan is cancelled by the user.
pub const SCAN_CANCELLED: &str = "scan cancelled";

/// Error message used when a deletion is cancelled by the user (mirrors
/// [`SCAN_CANCELLED`] and [`crate::convert::CONVERSION_CANCELLED`]). What was
/// already removed stays removed: the game is left partially deleted on the
/// target, and deleting it again finishes the job.
pub const DELETION_CANCELLED: &str = "deletion cancelled";

/// Name of the per-disk configuration manifest, stored at the root of the disk
/// (mount root for a local drive; next to the `Aurora` folder over FTP). It
/// holds separate `usb` and `ftp` sections for the two connection kinds.
pub const MANIFEST_NAME: &str = ".txbm.json";

/// Default scan depth for a storage location: one nested level is allowed, both
/// for extracted games (`Games/<Publisher>/<Game>`) and for GOD containers
/// (`Content/0000000000000000/<Name>/<TitleID>`, the layout third-party
/// managers produce — see [`crate::god_dirs`]). Costs nothing on a flat
/// library, where every child is recognized as a game and never descended into.
pub const DEFAULT_SCAN_DEPTH: u32 = 2;

#[derive(Debug, Clone)]
pub enum Target {
    Local(PathBuf),
    Ftp(FtpConfig),
}

impl Target {
    pub fn from_config(contents: &ConfigContents) -> Option<Target> {
        match contents.target_kind {
            TargetKind::Local => {
                if contents.mount_point.as_os_str().is_empty() {
                    None
                } else {
                    Some(Target::Local(contents.mount_point.clone()))
                }
            }
            TargetKind::Ftp => {
                let ftp = contents.ftp_config();
                if ftp.host.is_empty() {
                    None
                } else {
                    Some(Target::Ftp(ftp))
                }
            }
        }
    }

    /// Displayed label (local path or ftp://ip).
    pub fn display(&self) -> String {
        match self {
            Target::Local(path) => path.to_string_lossy().to_string(),
            Target::Ftp(ftp) => format!("ftp://{}", ftp.host),
        }
    }

    /// Lists games and drive information of the target.
    /// `cancel` can be raised from another thread to interrupt
    /// an FTP scan between two commands.
    pub fn scan(&self, cancel: &AtomicBool) -> Result<(Vec<Game>, DriveInfo)> {
        match self {
            Target::Local(path) => {
                let games = crate::game::scan_drive(path);
                let mut drive_info = DriveInfo::from_path(path).unwrap_or_default();
                drive_info.games_bytes = games.iter().map(|g| g.size).sum();
                Ok((games, drive_info))
            }
            Target::Ftp(ftp) => scan_ftp(ftp, cancel),
        }
    }

    /// Deletes a game from the target, including its separate
    /// `Content/<TitleID>` folder (DLC/title updates) for a merged extracted
    /// game, when it has one. `update_progress` receives a percentage
    /// (0-100) — reset to 0 again if a second folder needs removing.
    ///
    /// Raising `cancel` stops the removal at its next file, failing with
    /// [`DELETION_CANCELLED`]; the files already removed are not restored.
    pub fn delete_game(
        &self,
        game: &Game,
        cancel: &AtomicBool,
        update_progress: &dyn Fn(u32),
    ) -> Result<()> {
        let mut paths = vec![game.path.clone()];
        if let Some(content_dir) = &game.content_dir {
            paths.push(content_dir.clone());
        }

        match self {
            Target::Local(root) => {
                for path in &paths {
                    let total = crate::util::file_count(path).max(1);
                    let mut done: u64 = 0;
                    remove_dir_all_with_progress(path, &mut done, total, cancel, update_progress)?;

                    // A nested layout (`<Name>/<TitleID>`, `Games/<Publisher>/<Game>`)
                    // leaves the named parent behind, empty. Drop it — but never a
                    // storage root, however empty the last deletion left it. The
                    // layout is only resolved once the folder is known to be empty,
                    // which a flat library never reaches.
                    if let Some(parent) = path.parent()
                        && dir_is_empty(parent)
                        && !local_layout(root).is_storage_root(&parent.to_string_lossy())
                    {
                        let _ = std::fs::remove_dir(parent);
                    }
                }
                Ok(())
            }
            Target::Ftp(ftp) => {
                let mut session = FtpSession::connect(ftp)?;
                let result = (|| {
                    for path in &paths {
                        let remote = path.to_string_lossy().replace('\\', "/");
                        session.remove_dir_recursive(&remote, cancel, &mut |done, total| {
                            update_progress((done * 100 / total.max(1)) as u32);
                        })?;
                        // Same parent pruning as locally. Resolving the layout
                        // (cached across calls, see `cached_ftp_layout`) only
                        // happens for a folder the deletion actually emptied.
                        let (parent, _) = crate::ftp::parent_and_name(&remote);
                        if session.list_dir(&parent).is_empty() {
                            let hdd = ftp_hdd_root(&mut session);
                            if !cached_ftp_layout(&mut session, &hdd, &ftp.host)
                                .is_storage_root(&parent)
                            {
                                let _ = session.remove_empty_dir(&parent);
                            }
                        }
                    }
                    Ok(())
                })();
                session.quit();
                result
            }
        }
    }
}

/// True when `dir` holds nothing (an unreadable directory counts as non-empty,
/// so it is left alone).
fn dir_is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut e| e.next().is_none())
}

/// Local equivalent of `fs::remove_dir_all` reporting per-file progress, and
/// bailing out with [`DELETION_CANCELLED`] once `cancel` is raised. The
/// directory is then left half-removed, which is what a cancelled deletion
/// means here — nothing is put back.
pub(crate) fn remove_dir_all_with_progress(
    dir: &std::path::Path,
    done: &mut u64,
    total: u64,
    cancel: &AtomicBool,
    update_progress: &dyn Fn(u32),
) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        if cancel.load(Ordering::Relaxed) {
            bail!(DELETION_CANCELLED);
        }

        let path = entry.path();
        if path.is_dir() {
            remove_dir_all_with_progress(&path, done, total, cancel, update_progress)?;
        } else {
            std::fs::remove_file(&path)
                .with_context(|| format!("removing {}", path.display()))?;
            *done += 1;
            update_progress((*done * 100 / total) as u32);
        }
    }
    std::fs::remove_dir(dir).with_context(|| format!("removing {}", dir.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Storage model (format-agnostic locations + install destinations)
// ---------------------------------------------------------------------------

/// A location scanned for games, without any notion of format: the scanner
/// detects the format of each game it finds. `depth` mirrors Aurora's scan
/// depth (how many folder levels below `path` are explored).
#[derive(Debug, Clone)]
pub struct ScanLocation {
    pub path: String,
    pub depth: u32,
}

/// Where new games are installed on the target, by storage format. Decoupled
/// from the platform: a converted Xbox 360 game goes to `god_dir`, an extracted
/// Original Xbox game (`default.xbe`) to `xbe_dir`, and an extracted
/// Xbox 360 game (`default.xex`) to `xex_dir`.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Directory holding the `<TitleID>` folders of GOD/Arcade containers.
    /// Includes the `Content/0000000000000000` segment.
    pub god_dir: String,
    /// Directory holding the `<Name>` folders of extracted Original Xbox
    /// games (each with a `default.xbe`).
    pub xbe_dir: String,
    /// Directory holding the `<Name>` folders of extracted Xbox 360 games
    /// (each with a `default.xex`).
    pub xex_dir: String,
    /// How `<TitleID>` folders are arranged inside `god_dir` on this target
    /// (see [`GodLayout`]). Per-target rather than a global preference: two
    /// targets can carry different pre-existing conventions.
    pub god_layout: GodLayout,
}

impl StorageConfig {
    /// Built-in defaults, relative to a target root (`/Hdd1` over FTP, the
    /// mount path locally). Both extracted kinds default to the same `Games`
    /// folder; the user can split them in the configuration modal.
    fn defaults(root: &str) -> Self {
        let root = root.trim_end_matches(['/', '\\']);
        Self {
            god_dir: format!("{root}/{DEFAULT_GOD_DIR}"),
            xbe_dir: format!("{root}/{DEFAULT_XBE_DIR}"),
            xex_dir: format!("{root}/{DEFAULT_XEX_DIR}"),
            god_layout: GodLayout::default(),
        }
    }

    /// Scan locations covering the install destinations, de-duplicated (the
    /// two extracted dirs often point to the same folder).
    fn scan_locations(&self) -> Vec<ScanLocation> {
        let mut locs = Vec::new();
        for dir in [&self.god_dir, &self.xbe_dir, &self.xex_dir] {
            push_scan_location(
                &mut locs,
                ScanLocation {
                    path: dir.clone(),
                    depth: DEFAULT_SCAN_DEPTH,
                },
            );
        }
        locs
    }
}

/// Adds `loc` to `locs`, folding it with any already-listed location that
/// walks the same tree — the same folder, an ancestor of it, or a descendant
/// of it — instead of listing both.
///
/// Nothing de-duplicates scan results, so two locations walking the same tree
/// would list every game they both reach twice (double-counted size, ghost
/// rows after a deletion). Whichever of the two locations is the ancestor is
/// kept, with its depth raised (if needed) to also reach everything the other
/// one wanted:
/// - if an already-listed location is `loc`'s ancestor (or the same folder),
///   that location's depth is raised instead of `loc` being pushed next to it;
/// - if `loc` is instead the ancestor of one or more already-listed locations
///   (e.g. the two extracted-game dirs both under one `Games` folder), those
///   narrower entries are folded into `loc`'s depth and dropped.
fn push_scan_location(locs: &mut Vec<ScanLocation>, loc: ScanLocation) {
    let key = normalize_path(&loc.path);

    // Closest already-listed ancestor (or `loc` itself), i.e. the one whose
    // depth has to grow the least to cover `loc`.
    let closest = locs
        .iter_mut()
        .filter_map(|l| {
            let base = normalize_path(&l.path);
            let below = if key == base {
                0
            } else {
                key.strip_prefix(&format!("{base}/"))?.split('/').count() as u32
            };
            Some((below, l))
        })
        .min_by_key(|(below, _)| *below);
    if let Some((below, l)) = closest {
        // Levels between the ancestor and `loc`, plus the levels `loc` itself
        // needs below its own root.
        l.depth = l.depth.max(below + loc.depth);
        return;
    }

    // The reverse: `loc` is an ancestor of one or more already-listed,
    // narrower locations. Fold them into `loc` rather than keeping both.
    let mut depth = loc.depth;
    locs.retain(|l| {
        let base = normalize_path(&l.path);
        match base.strip_prefix(&format!("{key}/")) {
            Some(rest) => {
                depth = depth.max(rest.split('/').count() as u32 + l.depth);
                false
            }
            None => true,
        }
    });
    locs.push(ScanLocation { depth, ..loc });
}

/// Lower-cased, forward-slashed, trailing-slash-trimmed form of a path, for
/// case-insensitive comparison across FTP/local separators.
fn normalize_path(p: &str) -> String {
    p.to_lowercase()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

/// Everything needed to scan a target and install into it.
#[derive(Debug, Clone)]
pub struct TargetLayout {
    pub scan_locations: Vec<ScanLocation>,
    pub storage: StorageConfig,
}

impl TargetLayout {
    /// True when `path` is one of the target's own storage/scan roots — a
    /// folder the app must never delete, however empty it looks.
    pub fn is_storage_root(&self, path: &str) -> bool {
        let key = normalize_path(path);
        self.scan_locations
            .iter()
            .map(|l| l.path.as_str())
            .chain([
                self.storage.god_dir.as_str(),
                self.storage.xbe_dir.as_str(),
                self.storage.xex_dir.as_str(),
            ])
            .any(|p| normalize_path(p) == key)
    }
}

/// One set of storage directories, as stored in a manifest section. Paths are
/// kept exactly as the corresponding connection expresses them (relative to the
/// mount for `usb`, `/Device/...` for `ftp`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxbmSection {
    pub god_dir: String,
    pub xbe_dir: String,
    pub xex_dir: String,
    /// Missing on a manifest written before this setting became per-target
    /// (or never touched since): defaults to [`GodLayout::TitleId`], the
    /// layout every earlier manifest was implicitly written with.
    #[serde(default)]
    pub god_layout: GodLayout,
}

impl TxbmSection {
    fn from_storage(storage: &StorageConfig) -> Self {
        Self {
            god_dir: storage.god_dir.clone(),
            xbe_dir: storage.xbe_dir.clone(),
            xex_dir: storage.xex_dir.clone(),
            god_layout: storage.god_layout,
        }
    }

    fn into_storage(self) -> StorageConfig {
        StorageConfig {
            god_dir: self.god_dir,
            xbe_dir: self.xbe_dir,
            xex_dir: self.xex_dir,
            god_layout: self.god_layout,
        }
    }
}

/// On-disk `.txbm.json` manifest. It keeps **separate** sections for a USB
/// (local) connection and an FTP (console) connection to the same physical
/// disk, since the two express paths differently (mount-relative vs
/// `/Device/...`). Each connection reads and updates only its own section, so a
/// disk configured both ways keeps both valid. Stored next to the `Aurora`
/// folder on the disk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TxbmManifest {
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb: Option<TxbmSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ftp: Option<TxbmSection>,
}

impl TxbmManifest {
    pub const CURRENT_VERSION: u32 = 1;
}

/// True when a path looks like a GOD container directory (ends with
/// `Content/0000000000000000`). Used only as a best-effort *suggestion* for
/// the default install destination — never to type a scan location, whose
/// format is always detected from its actual content. The GUI lets the user
/// override the suggestion.
fn looks_like_god_dir(path: &str) -> bool {
    normalize_path(path).ends_with("content/0000000000000000")
}

/// Finds the root of the console's internal hard drive (Hdd1).
pub fn ftp_hdd_root(session: &mut FtpSession) -> String {
    session
        .list_root()
        .unwrap_or_default()
        .iter()
        .find(|r| r.eq_ignore_ascii_case("Hdd1"))
        .cloned()
        .unwrap_or_else(|| "Hdd1".to_string())
}

// ---------------------------------------------------------------------------
// Aurora scan paths (read from its SQLite databases)
// ---------------------------------------------------------------------------

/// Aurora's configured scan paths, resolved as absolute FTP paths. These are
/// format-agnostic: the scanner detects each game's format from its content.
#[derive(Debug, Clone)]
pub struct AuroraScan {
    pub locations: Vec<ScanLocation>,
    /// Where the Aurora installation itself was found (e.g. `/Hdd1/Aurora` on
    /// a console, or an absolute local path on a mounted drive), for display.
    pub install_dir: Option<String>,
}

impl AuroraScan {
    /// True when Aurora scans no location at all.
    pub fn is_empty(&self) -> bool {
        self.locations.is_empty()
    }

    /// One line per scanned location, for display in the UI.
    pub fn display_lines(&self) -> Vec<String> {
        self.locations
            .iter()
            .map(|l| format!("{}  (depth {})", l.path, l.depth))
            .collect()
    }
}

/// Parses a `launch.ini` (dash-launch config) and returns the directory
/// holding `Aurora.xex`, relative to whichever drive letter names it (e.g.
/// `Default = Usb:\Aurora\Aurora.xex` yields `Aurora`), if any `[Paths]`
/// entry points at one. The drive letter itself is discarded: the caller is
/// expected to resolve the result against the volume `launch.ini` was read
/// from, since that's the drive dash-launch booted from and thus the one its
/// own drive letter (`Usb:`, `Hdd:`, ...) refers to.
fn aurora_dir_from_launch_ini(text: &str) -> Option<String> {
    let mut in_paths = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_paths = section.eq_ignore_ascii_case("Paths");
            continue;
        }
        if !in_paths {
            continue;
        }
        let Some((_, value)) = line.split_once('=') else {
            continue;
        };
        // Drop the drive letter: "Usb:\Aurora\Aurora.xex" -> "Aurora\Aurora.xex".
        let value = value.trim();
        let path = value.split_once(':').map_or(value, |(_, rest)| rest);
        let path = path.trim_start_matches(['\\', '/']);
        let Some((dir, file)) = path.rsplit_once('\\') else {
            continue;
        };
        if file.eq_ignore_ascii_case("Aurora.xex") {
            return Some(dir.replace('\\', "/"));
        }
    }
    None
}

/// Looks for the Aurora installation on the console's drives and returns
/// its `Aurora/Data` folder (holding `Databases/` and `TitleUpdates/`).
/// Memoized on the session: several callers (manifest lookup, Aurora scan
/// paths, title-update cache) may run against the same connection, and the
/// Aurora install cannot move while it stays open.
pub(crate) fn find_aurora_data_dir(session: &mut FtpSession) -> Option<String> {
    if let Some(cached) = &session.aurora_data_dir_cache {
        return cached.clone();
    }
    let result = (|| {
        for root in session.list_root().ok()? {
            if let Some(dir) = find_aurora_dir_on_root(session, &root) {
                return Some(format!("{dir}/Data"));
            }
        }
        None
    })();
    session.aurora_data_dir_cache = Some(result.clone());
    result
}

/// Resolves the Aurora install directory on one console volume: prefers a
/// `launch.ini`'s `[Paths]` entry when present (whatever the actual layout),
/// falling back to `Aurora` or `Dashboard/Aurora` at the volume's root.
fn find_aurora_dir_on_root(session: &mut FtpSession, root: &str) -> Option<String> {
    let root_path = format!("/{root}");

    let has_ini = session
        .list_dir(&root_path)
        .iter()
        .any(|e| !e.is_dir && e.name.eq_ignore_ascii_case("launch.ini"));
    if has_ini
        && let Ok(bytes) = session.download_file(&format!("{root_path}/launch.ini"))
        && let Ok(text) = String::from_utf8(bytes)
        && let Some(rel_dir) = aurora_dir_from_launch_ini(&text)
    {
        let dir = format!("{root_path}/{rel_dir}");
        let target_exists = session
            .list_dir(&dir)
            .iter()
            .any(|e| !e.is_dir && e.name.eq_ignore_ascii_case("Aurora.xex"));
        if target_exists {
            return Some(dir);
        }
    }

    for candidate in ["Aurora", "Dashboard/Aurora"] {
        let dir = format!("{root_path}/{candidate}");
        let target_exists = session
            .list_dir(&dir)
            .iter()
            .any(|e| !e.is_dir && e.name.eq_ignore_ascii_case("Aurora.xex"));
        if target_exists {
            return Some(dir);
        }
    }
    None
}

/// Looks for Aurora installation on the console's drives
/// and returns its database folder.
fn find_aurora_databases(session: &mut FtpSession) -> Option<String> {
    let db_dir = format!("{}/Databases", find_aurora_data_dir(session)?);
    let files = session.list_dir(&db_dir);
    let has_settings = files
        .iter()
        .any(|f| f.name.eq_ignore_ascii_case("settings.db"));
    let has_content = files
        .iter()
        .any(|f| f.name.eq_ignore_ascii_case("content.db"));
    (has_settings && has_content).then_some(db_dir)
}

/// Reads Aurora's ScanPaths (settings.db) and resolves them to FTP paths
/// via the MountedDevices table (content.db).
pub fn aurora_paths(session: &mut FtpSession) -> Result<AuroraScan> {
    let db_dir = find_aurora_databases(session)
        .context("Aurora installation not found on console")?;
    let install_dir = db_dir
        .strip_suffix("/Data/Databases")
        .map(str::to_string);

    let tmp_dir = DATA_DIR.join("tmp");
    std::fs::create_dir_all(&tmp_dir)?;

    let settings_bytes = session.download_file(&format!("{db_dir}/settings.db"))?;
    let content_bytes = session.download_file(&format!("{db_dir}/content.db"))?;
    let settings_path = tmp_dir.join("aurora-settings.db");
    let content_path = tmp_dir.join("aurora-content.db");
    std::fs::write(&settings_path, settings_bytes)?;
    std::fs::write(&content_path, content_bytes)?;

    let result = read_aurora_databases(&settings_path, &content_path, install_dir);

    let _ = std::fs::remove_file(&settings_path);
    let _ = std::fs::remove_file(&content_path);

    result
}

fn read_aurora_databases(
    settings_path: &std::path::Path,
    content_path: &std::path::Path,
    install_dir: Option<String>,
) -> Result<AuroraScan> {
    let content_db =
        rusqlite::Connection::open(content_path).context("opening content.db")?;
    let mut devices = std::collections::HashMap::new();
    {
        let mut stmt = content_db
            .prepare("SELECT DeviceId, DeviceName FROM MountedDevices")
            .context("reading MountedDevices")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows.flatten() {
            devices.insert(row.0, row.1);
        }
    }

    let settings_db =
        rusqlite::Connection::open(settings_path).context("opening settings.db")?;
    let mut stmt = settings_db
        .prepare("SELECT Path, DeviceId, Depth FROM ScanPaths")
        .context("reading ScanPaths")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let mut locations = Vec::new();
    for (path, device_id, depth) in rows.flatten() {
        let Some(device) = devices.get(&device_id) else {
            continue;
        };
        let relative = path.replace('\\', "/");
        let relative = relative.trim_matches('/');
        locations.push(ScanLocation {
            path: format!("/{device}/{relative}"),
            depth: depth.max(1) as u32,
        });
    }

    // An empty result is a valid state (Aurora scans nothing yet): callers
    // that need a fallback check `is_empty()`, while the UI reports it to
    // the user instead of treating it as an error.
    Ok(AuroraScan {
        locations,
        install_dir,
    })
}

// ---------------------------------------------------------------------------
// Layout resolution (manifest → Aurora → defaults)
// ---------------------------------------------------------------------------

/// Directory where the `.txbm.json` manifest lives on a console: **next to the
/// `Aurora` folder** (i.e. the root of whichever device Aurora is installed on,
/// e.g. `/Usb0` or `/Hdd1`), so the manifest travels with the Aurora install.
/// Falls back to the internal drive when no Aurora installation is found.
fn aurora_manifest_dir(session: &mut FtpSession) -> String {
    match find_aurora_data_dir(session) {
        // `/{device}/{Aurora}/Data` → keep `/{device}`.
        Some(data) => {
            let device = data.trim_start_matches('/').split('/').next().unwrap_or("Hdd1");
            format!("/{device}")
        }
        None => format!("/{}", ftp_hdd_root(session)),
    }
}

/// Reads the whole manifest from a console (both `usb` and `ftp` sections), so
/// a write can preserve the section it does not touch. Returns the default
/// (empty) manifest when absent or unreadable.
fn read_ftp_raw_manifest(session: &mut FtpSession, dir: &str) -> TxbmManifest {
    // Check for the file with a LIST first: the console's FTP server can hang
    // the data channel on a RETR of a non-existent file (no data-channel
    // timeout), so never RETR blindly. See the Aurora FTP quirks.
    let present = session
        .list_dir(dir)
        .iter()
        .any(|e| !e.is_dir && e.name.eq_ignore_ascii_case(MANIFEST_NAME));
    if !present {
        return TxbmManifest::default();
    }
    session
        .download_file(&format!("{dir}/{MANIFEST_NAME}"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Reads the console's storage configuration from the `ftp` section of the
/// manifest (absolute `/Device/...` paths), if present.
fn read_ftp_manifest(session: &mut FtpSession) -> Option<StorageConfig> {
    let dir = aurora_manifest_dir(session);
    read_ftp_raw_manifest(session, &dir)
        .ftp
        .map(TxbmSection::into_storage)
}

/// Writes the console's storage configuration into the `ftp` section of the
/// manifest (next to Aurora), preserving any existing `usb` section.
pub fn write_ftp_manifest(session: &mut FtpSession, storage: &StorageConfig) -> Result<()> {
    let dir = aurora_manifest_dir(session);
    let mut manifest = read_ftp_raw_manifest(session, &dir);
    manifest.version = TxbmManifest::CURRENT_VERSION;
    manifest.ftp = Some(TxbmSection::from_storage(storage));
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    session.put_bytes(&dir, MANIFEST_NAME, &bytes)?;
    if let Some(cache) = FTP_LAYOUT_CACHE.get() {
        *cache.lock().unwrap() = None;
    }
    Ok(())
}

/// Reads the whole manifest from the root of a local mount, so a write can
/// preserve the section it does not touch.
fn read_local_raw_manifest(mount: &Path) -> TxbmManifest {
    std::fs::read(mount.join(MANIFEST_NAME))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Reads the local storage configuration from the `usb` section of the manifest
/// (paths relative to the mount, resolved to absolute here), if present.
fn read_local_manifest(mount: &Path) -> Option<StorageConfig> {
    read_local_raw_manifest(mount).usb.map(|s| StorageConfig {
        god_dir: resolve_local(mount, &s.god_dir),
        xbe_dir: resolve_local(mount, &s.xbe_dir),
        xex_dir: resolve_local(mount, &s.xex_dir),
        god_layout: s.god_layout,
    })
}

/// Writes the local storage configuration into the `usb` section of the
/// manifest at the mount root, preserving any existing `ftp` section. Paths are
/// stored relative to the mount so the configuration stays valid if the drive
/// is later mounted elsewhere; a path outside the mount is kept absolute.
pub fn write_local_manifest(mount: &Path, storage: &StorageConfig) -> Result<()> {
    let mut manifest = read_local_raw_manifest(mount);
    manifest.version = TxbmManifest::CURRENT_VERSION;
    manifest.usb = Some(TxbmSection {
        god_dir: relativize_local(mount, &storage.god_dir),
        xbe_dir: relativize_local(mount, &storage.xbe_dir),
        xex_dir: relativize_local(mount, &storage.xex_dir),
        god_layout: storage.god_layout,
    });
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    std::fs::write(mount.join(MANIFEST_NAME), bytes)
        .with_context(|| format!("writing {MANIFEST_NAME} to {}", mount.display()))
}

/// Expresses an absolute local path relative to the mount root for storage in
/// the manifest (forward slashes). A path outside the mount is left absolute.
fn relativize_local(mount: &Path, path: &str) -> String {
    Path::new(path)
        .strip_prefix(mount)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string())
}

/// Resolves a manifest path (relative to the mount root, or absolute) back to
/// an absolute local path.
fn resolve_local(mount: &Path, path: &str) -> String {
    let p = Path::new(path);
    if p.is_absolute() {
        path.to_string()
    } else {
        mount.join(p).to_string_lossy().to_string()
    }
}

/// Best-effort suggestion of install destinations from a set of scan
/// locations, falling back to defaults under `root`.
fn suggest_storage(locations: &[ScanLocation], root: &str) -> StorageConfig {
    let defaults = StorageConfig::defaults(root);
    let god_dir = locations
        .iter()
        .find(|l| looks_like_god_dir(&l.path))
        .map(|l| l.path.clone())
        .unwrap_or(defaults.god_dir);
    // A path name can't tell XBE from XEX, so both extracted kinds are seeded
    // from the first non-GOD location (the user splits them in the modal).
    let extracted = locations
        .iter()
        .find(|l| !looks_like_god_dir(&l.path))
        .map(|l| l.path.clone());
    StorageConfig {
        god_dir,
        xbe_dir: extracted.clone().unwrap_or(defaults.xbe_dir),
        xex_dir: extracted.unwrap_or(defaults.xex_dir),
        god_layout: defaults.god_layout,
    }
}

/// Cache of the last [`TargetLayout`] resolved for a console, keyed by FTP
/// host so switching consoles never serves a stale layout. Only used by
/// [`Target::delete_game`]'s storage-root check: without a manifest,
/// `ftp_layout` downloads and parses Aurora's `settings.db`/`content.db`,
/// which would otherwise redo the same work for every single game removed in
/// a row from a nested-layout console. Cleared by
/// [`write_ftp_manifest`], since writing a manifest changes what `ftp_layout`
/// resolves to.
static FTP_LAYOUT_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<(String, TargetLayout)>>> =
    std::sync::OnceLock::new();

fn cached_ftp_layout(session: &mut FtpSession, hdd: &str, host: &str) -> TargetLayout {
    let cache = FTP_LAYOUT_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Some((cached_host, layout)) = cache.lock().unwrap().as_ref()
        && cached_host == host
    {
        return layout.clone();
    }
    let layout = ftp_layout(session, hdd);
    *cache.lock().unwrap() = Some((host.to_string(), layout.clone()));
    layout
}

/// Resolves the layout of a console over FTP.
pub fn ftp_layout(session: &mut FtpSession, hdd: &str) -> TargetLayout {
    // 1. A manifest is authoritative: scan exactly the two configured dirs.
    if let Some(storage) = read_ftp_manifest(session) {
        return TargetLayout {
            scan_locations: storage.scan_locations(),
            storage,
        };
    }

    // 2. Fall back to Aurora's own scan paths (format-agnostic locations),
    //    suggesting install destinations among them.
    if let Some(aurora) = aurora_paths(session).ok().filter(|a| !a.is_empty()) {
        let storage = suggest_storage(&aurora.locations, &format!("/{hdd}"));
        // Aurora indexes `Content/0000000000000000` implicitly on every device,
        // so that folder is almost never listed in its "Manage Paths" screen.
        // Scanning its paths alone would hide every GOD container — including
        // the ones we install ourselves — so the install destinations are
        // always scanned too, unless Aurora already covers them.
        let mut scan_locations = aurora.locations;
        for dir in [&storage.god_dir, &storage.xbe_dir, &storage.xex_dir] {
            push_scan_location(
                &mut scan_locations,
                ScanLocation {
                    path: dir.clone(),
                    depth: DEFAULT_SCAN_DEPTH,
                },
            );
        }
        return TargetLayout {
            scan_locations,
            storage,
        };
    }

    // 3. Built-in defaults.
    let storage = StorageConfig::defaults(&format!("/{hdd}"));
    TargetLayout {
        scan_locations: storage.scan_locations(),
        storage,
    }
}

/// Resolves the layout of a local drive.
///
/// Note: reading an Aurora installation carried on the drive itself (Aurora
/// can run from USB) is a follow-up; for now a local target relies on its
/// manifest, otherwise the built-in defaults.
pub fn local_layout(mount: &Path) -> TargetLayout {
    let root = mount.to_string_lossy();

    if let Some(storage) = read_local_manifest(mount) {
        return TargetLayout {
            scan_locations: storage.scan_locations(),
            storage,
        };
    }

    let storage = StorageConfig::defaults(&root);
    TargetLayout {
        scan_locations: storage.scan_locations(),
        storage,
    }
}

// ---------------------------------------------------------------------------
// Storage status vs Aurora (Toolbox card)
// ---------------------------------------------------------------------------

/// One folder this app installs/scans on the console, and whether Aurora is
/// configured to scan it (or an ancestor of it).
#[derive(Debug, Clone)]
pub struct StoragePathStatus {
    pub label: String,
    /// FTP path (`/Device/dir/…`).
    pub path: String,
    /// Same location in Aurora's "Manage Paths" format (`Device:\dir\…`).
    pub aurora_path: String,
    pub covered_by_aurora: bool,
}

/// Converts an FTP path (`/Hdd1/Content/0000000000000000`) to the format shown
/// in Aurora's "Manage Paths" screen (`Hdd1:\Content\0000000000000000`).
fn aurora_console_path(ftp_path: &str) -> String {
    let trimmed = ftp_path.trim_start_matches('/');
    match trimmed.split_once('/') {
        Some((device, rest)) => format!("{device}:\\{}", rest.replace('/', "\\")),
        None => format!("{trimmed}:\\"),
    }
}

/// Comparison between the app's storage locations and Aurora's scan paths.
#[derive(Debug, Clone)]
pub struct StorageStatus {
    /// The app's install/scan folders (GOD + extracted), with coverage.
    pub paths: Vec<StoragePathStatus>,
    /// Aurora's scan paths, formatted one per line (for the existing card).
    pub aurora_lines: Vec<String>,
    /// Where the Aurora installation itself was found, for display.
    pub aurora_install_dir: Option<String>,
    /// Set when Aurora's databases could not be read (message).
    pub aurora_error: Option<String>,
    /// True when at least one app path is not covered by Aurora (and Aurora
    /// was read successfully) — i.e. paths still need to be added on Aurora.
    pub has_uncovered: bool,
    /// True when the coverage badges are meaningful (Aurora was read and the
    /// comparison applies). False for a local drive, where the app paths are
    /// only listed without an Aurora comparison.
    pub aurora_compared: bool,
    /// The GOD layout actually configured for this target (from its
    /// manifest, or the suggested/default one when unconfigured yet).
    pub god_layout: GodLayout,
}

/// True when Aurora's scan paths reach the games this app writes under
/// `app_path`, either because it scans `app_path` itself or via an ancestor
/// scanned deeply enough.
///
/// `game_levels` is how deep below `app_path` a game folder is written — 1 for
/// a direct child (extracted games, flat GOD layout), 2 for a nested GOD layout
/// (`<Name>/<TitleID>`). The check is purely configuration-based: it compares
/// Aurora's configured depth against the depth this app's settings imply, and
/// never looks at the games actually present.
///
/// If `app_path` sits `k` folder levels below an Aurora location, its games are
/// at level `k + game_levels`, so that location's `depth` must be at least
/// that. Note that `depth` alone is no longer always sufficient for `k = 0`: a
/// nested layout needs depth ≥ 2 even on the storage folder itself.
fn path_covered(aurora: &[ScanLocation], app_path: &str, game_levels: u32) -> bool {
    let a = normalize_path(app_path);
    aurora.iter().any(|loc| {
        let l = normalize_path(&loc.path);
        if l.is_empty() {
            return false;
        }
        // How far `app_path` sits below this Aurora location, in folder levels.
        let below = if a == l {
            0
        } else {
            match a.strip_prefix(&format!("{l}/")) {
                Some(rest) => rest.split('/').count() as u32,
                None => return false,
            }
        };
        loc.depth >= below + game_levels
    })
}

/// Builds the storage-status rows for the Toolbox card: one row per storage
/// destination (GOD, extracted XBE, extracted XEX), always kept separate so the
/// user sees the three distinct choices even when two point to the same folder.
/// `display` formats a stored dir for the UI; `covered` tells whether Aurora
/// reaches games written `game_levels` folders below the dir — 1 for extracted
/// games (always direct children), `storage`'s own [`GodLayout`] for the GOD
/// directory.
fn storage_path_rows(
    storage: &StorageConfig,
    display: impl Fn(&str) -> String,
    covered: impl Fn(&str, u32) -> bool,
) -> Vec<StoragePathStatus> {
    let row = |label: &str, dir: &str, game_levels: u32| StoragePathStatus {
        label: label.to_string(),
        aurora_path: display(dir),
        path: dir.to_string(),
        covered_by_aurora: covered(dir, game_levels),
    };
    vec![
        row(
            "GOD Storage - Xbox360 games",
            &storage.god_dir,
            storage.god_layout.title_levels(),
        ),
        row("XEX Storage - Xbox360 extracted games", &storage.xex_dir, 1),
        row("XBE Storage - Xbox OG games", &storage.xbe_dir, 1),
    ]
}

/// Reads the app's storage layout and Aurora's scan paths from a console, and
/// reports which app folders Aurora is (not) configured to scan. The
/// resolved [`GodLayout`] (from the manifest, or suggested otherwise) decides
/// how deep Aurora must scan the GOD directory. Involves FTP I/O; run off the
/// UI thread.
pub fn ftp_storage_status(session: &mut FtpSession, hdd: &str) -> StorageStatus {
    let root = format!("/{hdd}");
    let manifest = read_ftp_manifest(session);

    let (aurora_locs, aurora_install_dir, aurora_error) = match aurora_paths(session) {
        Ok(a) => (a.locations, a.install_dir, None),
        Err(e) => (Vec::new(), None, Some(format!("{e:#}"))),
    };

    let storage = if let Some(s) = manifest {
        s
    } else if !aurora_locs.is_empty() {
        suggest_storage(&aurora_locs, &root)
    } else {
        StorageConfig::defaults(&root)
    };

    let aurora_lines = aurora_locs
        .iter()
        .map(|l| format!("{}  (depth {})", aurora_console_path(&l.path), l.depth))
        .collect();

    let paths = storage_path_rows(
        &storage,
        |p| aurora_console_path(p),
        |p, levels| path_covered(&aurora_locs, p, levels),
    );

    let aurora_compared = aurora_error.is_none();
    let has_uncovered = aurora_compared && paths.iter().any(|p| !p.covered_by_aurora);

    StorageStatus {
        paths,
        aurora_lines,
        aurora_install_dir,
        aurora_error,
        has_uncovered,
        aurora_compared,
        god_layout: storage.god_layout,
    }
}

/// Case-insensitive lookup of a direct child (file or dir) of `dir`.
fn find_child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

/// Resolves the Aurora install directory on a mounted drive: prefers a
/// `launch.ini`'s `[Paths]` entry when present (whatever the actual layout),
/// falling back to `Aurora` or `Dashboard/Aurora` at the drive's root.
fn local_aurora_dir(mount: &Path) -> Option<PathBuf> {
    if let Some(ini_path) = find_child_ci(mount, "launch.ini")
        && let Ok(text) = std::fs::read_to_string(&ini_path)
        && let Some(rel_dir) = aurora_dir_from_launch_ini(&text)
    {
        let mut dir = mount.to_path_buf();
        for part in rel_dir.split('/') {
            dir = find_child_ci(&dir, part).unwrap_or_else(|| dir.join(part));
        }
        if find_child_ci(&dir, "Aurora.xex").is_some() {
            return Some(dir);
        }
    }

    for candidate in [vec!["Aurora"], vec!["Dashboard", "Aurora"], vec!["Apps", "Aurora"]] {
        let mut dir = mount.to_path_buf();
        let mut ok = true;
        for part in candidate {
            match find_child_ci(&dir, part) {
                Some(child) => dir = child,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && find_child_ci(&dir, "Aurora.xex").is_some() {
            return Some(dir);
        }
    }
    None
}

/// Locates Aurora's databases on a mounted drive (Aurora on a bootable USB key
/// sits at the drive root: `<mount>/Aurora/Data/Databases`).
fn local_aurora_databases(mount: &Path) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let aurora = local_aurora_dir(mount)?;
    let db_dir = find_child_ci(&aurora, "Data").and_then(|d| find_child_ci(&d, "Databases"))?;
    let settings = find_child_ci(&db_dir, "settings.db")?;
    let content = find_child_ci(&db_dir, "content.db")?;
    Some((settings, content, aurora))
}

/// Reads Aurora's scan paths from an Aurora installation carried on a local
/// drive (e.g. a bootable USB key), if present.
pub fn local_aurora_paths(mount: &Path) -> Result<AuroraScan> {
    let (settings, content, aurora_dir) =
        local_aurora_databases(mount).context("no Aurora databases on this drive")?;
    read_aurora_databases(
        &settings,
        &content,
        Some(aurora_dir.to_string_lossy().to_string()),
    )
}

/// Local counterpart of [`ftp_storage_status`]: lists the app's storage folders
/// on a drive (relative to the mount). When the drive carries an Aurora install
/// (a bootable USB key), its scan paths are mapped onto the mount and the app's
/// folders are checked against them, just like for a console.
pub fn local_storage_status(mount: &Path) -> StorageStatus {
    let storage = local_layout(mount).storage;

    let aurora = local_aurora_paths(mount).ok().filter(|a| !a.is_empty());

    let aurora_lines = aurora
        .as_ref()
        .map(|a| {
            a.locations
                .iter()
                .map(|l| format!("{}  (depth {})", aurora_console_path(&l.path), l.depth))
                .collect()
        })
        .unwrap_or_default();

    // Map Aurora's `Device:\dir` scan paths onto the mount for the coverage
    // check. Only scan paths on a USB device count: when this disk is plugged
    // into the console it is a `UsbX` device, so a path on `Hdd1` (the console's
    // internal drive) does NOT scan this disk, even if a same-named folder
    // happens to exist on it. Comparison happens in absolute local paths.
    let aurora_locs: Vec<ScanLocation> = aurora
        .as_ref()
        .map(|a| {
            a.locations
                .iter()
                .filter_map(|l| {
                    let (device, rel) = l.path.trim_start_matches('/').split_once('/')?;
                    if !is_usb_device(device) {
                        return None;
                    }
                    let local = mount.join(rel);
                    local.is_dir().then(|| ScanLocation {
                        path: local.to_string_lossy().to_string(),
                        depth: l.depth,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Compare only when the drive carries an Aurora configuration.
    let aurora_compared = aurora.is_some();
    let paths = storage_path_rows(
        &storage,
        |p| relativize_local(mount, p),
        |p, levels| aurora_compared && path_covered(&aurora_locs, p, levels),
    );
    let has_uncovered = aurora_compared && paths.iter().any(|p| !p.covered_by_aurora);

    StorageStatus {
        paths,
        aurora_lines,
        aurora_install_dir: aurora.as_ref().and_then(|a| a.install_dir.clone()),
        // A drive without Aurora is not an error, just an empty list.
        aurora_error: None,
        has_uncovered,
        aurora_compared,
        god_layout: storage.god_layout,
    }
}

// ---------------------------------------------------------------------------
// Target analysis (drives the "storage configuration" modal)
// ---------------------------------------------------------------------------

/// Outcome of analyzing a freshly-connected target, feeding the storage
/// configuration modal.
#[derive(Debug, Clone)]
pub struct TargetAnalysis {
    /// True when a `.txbm.json` manifest is already present: the caller can
    /// skip the configuration modal and scan directly.
    pub already_configured: bool,
    /// Suggested install destinations (pre-selected in the modal).
    pub suggested: StorageConfig,
    /// Candidate directories offered in the modal's drop-downs. Always
    /// includes the suggested destinations and the built-in defaults.
    pub candidates: Vec<String>,
}

impl Target {
    /// Analyzes the target to propose install destinations. Involves I/O
    /// (FTP round-trips or a shallow local scan); run off the UI thread.
    pub fn analyze(&self) -> Result<TargetAnalysis> {
        match self {
            Target::Local(mount) => Ok(analyze_local(mount)),
            Target::Ftp(ftp) => {
                let mut session = FtpSession::connect(ftp)?;
                let hdd = ftp_hdd_root(&mut session);
                let analysis = analyze_ftp(&mut session, &hdd);
                session.quit();
                Ok(analysis)
            }
        }
    }

    /// Converts an absolute path to the form shown in the UI (and stored in the
    /// manifest): relative to the mount root for a local drive, unchanged for a
    /// console over FTP. A path outside the mount stays absolute.
    pub fn display_path(&self, absolute: &str) -> String {
        match self {
            Target::Local(mount) => relativize_local(mount, absolute),
            Target::Ftp(_) => absolute.to_string(),
        }
    }

    /// Like [`Target::display_path`], but returns `None` for a local path that
    /// lies outside the mount root, so the caller can reject a folder picked on
    /// another drive. For a console over FTP the path is returned unchanged.
    pub fn relative_within(&self, absolute: &Path) -> Option<String> {
        match self {
            Target::Local(mount) => absolute
                .strip_prefix(mount)
                .ok()
                .map(|rel| rel.to_string_lossy().replace('\\', "/")),
            Target::Ftp(_) => Some(absolute.to_string_lossy().to_string()),
        }
    }

    /// Inverse of [`Target::display_path`]: resolves a UI/manifest-form path
    /// back to the absolute path used at runtime.
    pub fn resolve_path(&self, form: &str) -> String {
        match self {
            Target::Local(mount) => resolve_local(mount, form),
            Target::Ftp(_) => form.to_string(),
        }
    }

    /// Creates the storage directories if needed and writes the `.txbm.json`
    /// manifest, persisting the user's confirmed choice. Involves I/O; run off
    /// the UI thread. `storage` holds absolute paths.
    pub fn apply_storage(&self, storage: &StorageConfig) -> Result<()> {
        let dirs = [
            &storage.god_dir,
            &storage.xbe_dir,
            &storage.xex_dir,
        ];
        match self {
            Target::Local(mount) => {
                for dir in dirs {
                    std::fs::create_dir_all(dir)
                        .with_context(|| format!("creating {dir}"))?;
                }
                write_local_manifest(mount, storage)
            }
            Target::Ftp(ftp) => {
                let mut session = FtpSession::connect(ftp)?;
                for dir in dirs {
                    session.ensure_dir(dir)?;
                }
                let result = write_ftp_manifest(&mut session, storage);
                session.quit();
                result
            }
        }
    }
}

/// Builds the candidate list for the modal from discovered locations, always
/// appending the suggested destinations and the built-in defaults.
fn build_candidates(discovered: &[String], suggested: &StorageConfig, root: &str) -> Vec<String> {
    let defaults = StorageConfig::defaults(root);
    let mut candidates: Vec<String> = Vec::new();
    let mut push = |p: &str| {
        if !p.is_empty() && !candidates.iter().any(|c| c == p) {
            candidates.push(p.to_string());
        }
    };
    for d in discovered {
        push(d);
    }
    push(&suggested.god_dir);
    push(&suggested.xbe_dir);
    push(&suggested.xex_dir);
    push(&defaults.god_dir);
    push(&defaults.xbe_dir);
    candidates
}

fn analyze_ftp(session: &mut FtpSession, hdd: &str) -> TargetAnalysis {
    if let Some(storage) = read_ftp_manifest(session) {
        let candidates = build_candidates(&[], &storage, &format!("/{hdd}"));
        return TargetAnalysis {
            already_configured: true,
            suggested: storage,
            candidates,
        };
    }

    let discovered: Vec<String> = aurora_paths(session)
        .ok()
        .filter(|a| !a.is_empty())
        .map(|a| a.locations.into_iter().map(|l| l.path).collect())
        .unwrap_or_default();

    let suggested = suggest_storage(
        &discovered
            .iter()
            .map(|p| ScanLocation {
                path: p.clone(),
                depth: 1,
            })
            .collect::<Vec<_>>(),
        &format!("/{hdd}"),
    );
    let candidates = build_candidates(&discovered, &suggested, &format!("/{hdd}"));

    TargetAnalysis {
        already_configured: false,
        suggested,
        candidates,
    }
}

fn analyze_local(mount: &Path) -> TargetAnalysis {
    let root = mount.to_string_lossy().to_string();

    let (already_configured, suggested_abs, discovered) =
        if let Some(storage) = read_local_manifest(mount) {
            (true, storage, Vec::new())
        } else {
            // No manifest. Candidate storage dirs come from the drive's own
            // Aurora scan paths (if it carries an Aurora install), mapped onto
            // the mount, then a shallow filesystem scan.
            let mut discovered = local_aurora_candidates(mount);
            for c in local_candidates(mount) {
                if !discovered.iter().any(|d| normalize_path(d) == normalize_path(&c)) {
                    discovered.push(c);
                }
            }
            let suggested = suggest_storage_local(&discovered, &root);
            (false, suggested, discovered)
        };

    let candidates_abs = build_candidates(&discovered, &suggested_abs, &root);

    // Present paths relative to the mount root (matching how the manifest
    // stores them); paths outside the mount stay absolute.
    let suggested = StorageConfig {
        god_dir: relativize_local(mount, &suggested_abs.god_dir),
        xbe_dir: relativize_local(mount, &suggested_abs.xbe_dir),
        xex_dir: relativize_local(mount, &suggested_abs.xex_dir),
        god_layout: suggested_abs.god_layout,
    };
    let mut candidates = Vec::new();
    for c in &candidates_abs {
        let rel = relativize_local(mount, c);
        // Skip an empty relative path (a candidate equal to the mount root):
        // it would show up as a blank drop-down entry.
        if !rel.is_empty() && !candidates.contains(&rel) {
            candidates.push(rel);
        }
    }

    TargetAnalysis {
        already_configured,
        suggested,
        candidates,
    }
}

/// Suggests install destinations for a local drive by *structural* detection:
/// GOD from a `Content/0000000000000000` folder, XBE from a folder that
/// actually holds a `default.xbe` game, XEX from one holding a `default.xex`
/// game. Falls back to the built-in defaults for anything not found.
fn suggest_storage_local(discovered: &[String], root: &str) -> StorageConfig {
    let defaults = StorageConfig::defaults(root);
    let god_dir = discovered
        .iter()
        .find(|p| looks_like_god_dir(p))
        .cloned()
        .unwrap_or(defaults.god_dir);
    let xbe_dir = discovered
        .iter()
        .find(|p| dir_contains_extracted(Path::new(p), "default.xbe"))
        .cloned()
        .unwrap_or(defaults.xbe_dir);
    let xex_dir = discovered
        .iter()
        .find(|p| dir_contains_extracted(Path::new(p), "default.xex"))
        .cloned()
        .unwrap_or(defaults.xex_dir);
    StorageConfig {
        god_dir,
        xbe_dir,
        xex_dir,
        god_layout: defaults.god_layout,
    }
}

/// True when a direct sub-folder of `dir` holds the given executable
/// (`default.xbe` / `default.xex`), i.e. `dir` is an extracted-games location.
fn dir_contains_extracted(dir: &Path, exe: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let p = e.path();
        p.is_dir() && crate::util::find_file_ci(&p, exe).is_some()
    })
}

/// True when an Aurora device name denotes a USB device (`Usb0`, `Usb1`, …).
/// When this disk is plugged into the console it is a `UsbX` device, so only
/// its own USB scan paths map onto the local mount; `Hdd1` etc. belong to the
/// console's internal drive.
fn is_usb_device(device: &str) -> bool {
    device.to_lowercase().starts_with("usb")
}

/// Maps the drive's own Aurora scan paths (`Device:\dir`) onto the mount,
/// keeping only USB-device paths that actually exist as folders on it. Empty
/// when the drive carries no Aurora install.
fn local_aurora_candidates(mount: &Path) -> Vec<String> {
    let Ok(aurora) = local_aurora_paths(mount) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for loc in aurora.locations {
        // Drop the leading `/Device/` segment, keep the path relative to it.
        let Some((device, rel)) = loc.path.trim_start_matches('/').split_once('/') else {
            continue;
        };
        if rel.is_empty() || !is_usb_device(device) {
            continue;
        }
        let local = mount.join(rel);
        if local.is_dir() {
            let s = local.to_string_lossy().to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// Shallow (depth-limited) scan of a local mount, collecting directories that
/// look like storage locations: a `Content/0000000000000000` GOD container,
/// or a folder directly holding extracted games.
fn local_candidates(mount: &Path) -> Vec<String> {
    let mut found = Vec::new();
    collect_local_candidates(mount, 2, &mut found);
    // Never offer the mount root itself as a storage location (games are kept
    // in sub-folders); it would relativize to an empty, blank entry.
    let mount_key = normalize_path(&mount.to_string_lossy());
    found.retain(|c| normalize_path(c) != mount_key);
    found
}

fn collect_local_candidates(dir: &Path, depth: u32, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut has_extracted_child = false;
    let mut children = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // A folder directly holding a default executable marks its PARENT as
        // an extracted-games location.
        if is_extracted_game_dir(&path) {
            has_extracted_child = true;
        }
        children.push(path);
    }

    if has_extracted_child {
        push_candidate(found, dir);
    }
    // A GOD container directly under `dir`.
    let god = dir.join(DEFAULT_GOD_DIR);
    if god.is_dir() {
        push_candidate(found, &god);
    }

    if depth > 1 {
        for child in children {
            // Don't descend into the GOD tree or extracted-game folders.
            let name = child.file_name().unwrap_or_default().to_string_lossy();
            if name.eq_ignore_ascii_case("Content") || is_extracted_game_dir(&child) {
                continue;
            }
            collect_local_candidates(&child, depth - 1, found);
        }
    }
}

/// True when `dir` is itself an extracted game (holds a default executable).
fn is_extracted_game_dir(dir: &Path) -> bool {
    crate::util::find_file_ci(dir, "default.xex").is_some()
        || crate::util::find_file_ci(dir, "default.xbe").is_some()
}

fn push_candidate(found: &mut Vec<String>, path: &Path) {
    let s = path.to_string_lossy().to_string();
    if !found.iter().any(|c| c == &s) {
        found.push(s);
    }
}

// ---------------------------------------------------------------------------
// FTP scan (generic, format-agnostic per location)
// ---------------------------------------------------------------------------

fn scan_ftp(ftp: &FtpConfig, cancel: &AtomicBool) -> Result<(Vec<Game>, DriveInfo)> {
    let check_cancel = || -> Result<()> {
        if cancel.load(Ordering::Relaxed) {
            bail!(SCAN_CANCELLED);
        }
        Ok(())
    };

    check_cancel()?;
    let mut session = FtpSession::connect(ftp)?;
    let hdd = ftp_hdd_root(&mut session);
    let layout = ftp_layout(&mut session, &hdd);

    let mut scanner = FtpScanner {
        session: &mut session,
        cancel: &check_cancel,
        games: Vec::new(),
        games_bytes: 0,
    };
    for location in &layout.scan_locations {
        crate::scan::walk(&mut scanner, &location.path, location.depth)?;
    }
    // Release the borrow on `session` before quitting it.
    let FtpScanner {
        mut games,
        games_bytes,
        ..
    } = scanner;
    // Leaves `games_bytes` correct: the merged entry's size moves to the game
    // it belongs to instead of being counted twice.
    crate::game::merge_extracted_content(&mut games);

    session.quit();

    let drive_info = DriveInfo {
        label: format!("{} (FTP)", ftp.host),
        used_bytes: 0,
        total_bytes: 0,
        games_bytes,
        fs_kind: Default::default(),
        allocation_granularity: 0,
    };

    Ok((games, drive_info))
}

/// FTP [`DirScanner`], detecting the format of every game found (GOD/Arcade
/// under `<TitleID>` folders, extracted under folders holding a
/// `default.xex`/`default.xbe`) via the shared walk. Games accumulate as
/// internal state; the scan depth and recursion are handled by [`crate::scan`].
struct FtpScanner<'a> {
    session: &'a mut FtpSession,
    cancel: &'a dyn Fn() -> Result<()>,
    games: Vec<Game>,
    games_bytes: u64,
}

impl crate::scan::DirScanner for FtpScanner<'_> {
    type Path = String;

    fn child_dirs(&mut self, dir: &String) -> Result<Vec<(String, String)>> {
        (self.cancel)()?;
        Ok(self
            .session
            .list_dir(dir)
            .into_iter()
            .filter(|e| e.is_dir)
            .map(|e| (format!("{dir}/{}", e.name), e.name))
            .collect())
    }

    fn classify(&mut self, path: &String, name: &str) -> Result<crate::scan::ChildAction> {
        use crate::scan::ChildAction;

        (self.cancel)()?;
        let children = self.session.list_dir(path);

        // GOD / Arcade: an 8-hex TitleID folder holding a content package
        // (and/or orphaned DLC / a title update).
        if game::is_title_id(name)
            && push_god_games_ftp(
                self.session,
                path,
                name,
                &children,
                &mut self.games,
                &mut self.games_bytes,
            )
        {
            return Ok(ChildAction::Handled);
        }

        // Extracted game: a folder directly holding default.xex / default.xbe.
        if let Some(format) = detect_extracted(&children) {
            push_extracted_ftp(
                self.session,
                path,
                name,
                format,
                &mut self.games,
                &mut self.games_bytes,
            );
            return Ok(ChildAction::Handled);
        }

        // Neither: let the walk descend (handles nested
        // Content/0000000000000000 folders and arbitrary scan roots).
        Ok(ChildAction::Recurse)
    }
}

/// Detects an extracted-game folder from its direct children.
fn detect_extracted(children: &[crate::ftp::RemoteEntry]) -> Option<GameFormat> {
    if children
        .iter()
        .any(|c| !c.is_dir && c.name.eq_ignore_ascii_case("default.xex"))
    {
        Some(GameFormat::ExtractedXex)
    } else if children
        .iter()
        .any(|c| !c.is_dir && c.name.eq_ignore_ascii_case("default.xbe"))
    {
        Some(GameFormat::ExtractedXbe)
    } else {
        None
    }
}

/// Handles a GOD/Arcade `<TitleID>` folder over FTP. Returns true when a game
/// (or an incomplete DLC/title-update-only entry) was pushed.
fn push_god_games_ftp(
    session: &mut FtpSession,
    title_dir: &str,
    title_id_raw: &str,
    sub_entries: &[crate::ftp::RemoteEntry],
    games: &mut Vec<Game>,
    games_bytes: &mut u64,
) -> bool {
    let title_id = title_id_raw.to_uppercase();
    let mut found_package = false;

    for sub in sub_entries {
        let Some((_, format, is_x360)) = game::INSTALLED_CONTENT_TYPES
            .iter()
            .find(|(t, _, _)| sub.is_dir && sub.name.eq_ignore_ascii_case(t))
        else {
            continue;
        };
        found_package = true;
        let dlc_size =
            session.dir_size(&format!("{title_dir}/{}", crate::stfs::dlc_dir_name()), 3);
        let size = session.dir_size(&format!("{title_dir}/{}", sub.name), 3) + dlc_size;
        let title = u32::from_str_radix(&title_id, 16)
            .ok()
            .and_then(iso2god::game_list::find_title_by_id)
            .unwrap_or_else(|| title_id.clone());
        let search_term = format!("{title}\0{title_id}").to_lowercase();
        *games_bytes += size;
        games.push(Game {
            id: title_id.clone(),
            title,
            format: *format,
            path: PathBuf::from(title_dir),
            size,
            is_x360: *is_x360,
            search_term,
            incomplete: false,
            content_dir: None,
        });
    }

    if found_package {
        return true;
    }

    // No game package: only DLC and/or a title update sit here, orphaned from
    // a base install that was removed or never completed. Still surface it,
    // flagged incomplete.
    let has_dlc = sub_entries
        .iter()
        .any(|s| s.is_dir && s.name.eq_ignore_ascii_case(&crate::stfs::dlc_dir_name()));
    let has_title_update = sub_entries.iter().any(|s| {
        s.is_dir && s.name.eq_ignore_ascii_case(&crate::stfs::title_update_dir_name())
    });
    if !has_dlc && !has_title_update {
        return false;
    }

    let dlc_size = session.dir_size(&format!("{title_dir}/{}", crate::stfs::dlc_dir_name()), 3);
    let title_update_size = session.dir_size(
        &format!("{title_dir}/{}", crate::stfs::title_update_dir_name()),
        3,
    );
    let size = dlc_size + title_update_size;
    let title = u32::from_str_radix(&title_id, 16)
        .ok()
        .and_then(iso2god::game_list::find_title_by_id)
        .unwrap_or_else(|| title_id.clone());
    let search_term = format!("{title}\0{title_id}").to_lowercase();
    *games_bytes += size;
    games.push(Game {
        id: title_id.clone(),
        title,
        format: GameFormat::God,
        path: PathBuf::from(title_dir),
        size,
        is_x360: true,
        search_term,
        incomplete: true,
        content_dir: None,
    });
    true
}

/// Pushes an extracted game found over FTP.
fn push_extracted_ftp(
    session: &mut FtpSession,
    game_dir: &str,
    folder_name: &str,
    format: GameFormat,
    games: &mut Vec<Game>,
    games_bytes: &mut u64,
) {
    let size = session.dir_size(game_dir, 3);
    // TitleID from the folder-name suffix if present; games added by hand are
    // resolved later (covers pass), not here — one RETR per game would slow
    // the scan down.
    let (title, id) = game::split_title_id_suffix(folder_name);
    let id = id.unwrap_or_default();
    let search_term = format!("{title}\0{id}").to_lowercase();
    *games_bytes += size;
    games.push(Game {
        id,
        title,
        format,
        path: PathBuf::from(game_dir),
        size,
        is_x360: format == GameFormat::ExtractedXex,
        search_term,
        incomplete: false,
        content_dir: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(path: &str, depth: u32) -> ScanLocation {
        ScanLocation {
            path: path.to_string(),
            depth,
        }
    }

    #[test]
    fn a_partially_covering_ancestor_is_deepened_not_duplicated() {
        // Aurora scans the whole device shallowly; the storage dirs must not be
        // added next to it, or every game under them would be listed twice.
        let mut locs = vec![loc("/Hdd1", 2)];
        push_scan_location(&mut locs, loc("/Hdd1/Games", DEFAULT_SCAN_DEPTH));
        assert_eq!(locs.len(), 1);
        // `/Hdd1/Games` sits 1 level below, its games up to 2 levels lower.
        assert_eq!(locs[0].depth, 3);

        // An ancestor already deep enough is left alone.
        let mut locs = vec![loc("/Hdd1", 4)];
        push_scan_location(&mut locs, loc("/Hdd1/Games", DEFAULT_SCAN_DEPTH));
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].depth, 4);

        // The same folder, scanned less deeply than we need: deepened in place.
        let mut locs = vec![loc("/Hdd1/GAMES/", 1)];
        push_scan_location(&mut locs, loc("/Hdd1/games", DEFAULT_SCAN_DEPTH));
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].depth, DEFAULT_SCAN_DEPTH);

        // An unrelated location is pushed as-is.
        let mut locs = vec![loc("/Hdd1/Games", 2)];
        push_scan_location(&mut locs, loc("/Usb0/Games", DEFAULT_SCAN_DEPTH));
        assert_eq!(locs.len(), 2);

        // Reverse direction: the new location is an ancestor of an
        // already-listed, narrower one (e.g. a nested GodLayout scan path
        // added inside a scan dir Aurora already listed). It must fold the
        // narrower entry in, not sit next to it.
        let mut locs = vec![loc("/Hdd1/Content/0000000000000000/Games", 1)];
        push_scan_location(&mut locs, loc("/Hdd1/Content/0000000000000000", 1));
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].path, "/Hdd1/Content/0000000000000000");
        // 1 level down to `Games`, plus its own depth of 1.
        assert_eq!(locs[0].depth, 2);
    }

    #[test]
    fn coverage_follows_the_configured_god_layout() {
        // Aurora scanning the GOD dir itself at depth 1 sees flat TitleID
        // folders, but not the ones a nested layout writes one level deeper.
        let aurora = vec![loc("/Hdd1/Content/0000000000000000", 1)];
        let god = "/Hdd1/Content/0000000000000000";
        assert!(path_covered(&aurora, god, GodLayout::TitleId.title_levels()));
        assert!(!path_covered(
            &aurora,
            god,
            GodLayout::NameSlashTitleId.title_levels()
        ));

        // Depth 2 on the same folder covers both layouts.
        let aurora = vec![loc("/Hdd1/Content/0000000000000000", 2)];
        assert!(path_covered(
            &aurora,
            god,
            GodLayout::NameDashTitleId.title_levels()
        ));

        // Ancestor scan path: the device root needs depth 3 for a flat layout
        // (Content + 0000000000000000 + <TitleID>), 4 for a nested one.
        let aurora = vec![loc("/Hdd1", 3)];
        assert!(path_covered(&aurora, god, GodLayout::TitleId.title_levels()));
        assert!(!path_covered(
            &aurora,
            god,
            GodLayout::TitleIdDashName.title_levels()
        ));

        // Extracted games are always direct children of their storage dir.
        assert!(path_covered(&vec![loc("/Hdd1/Games", 1)], "/Hdd1/Games", 1));
        assert!(!path_covered(&vec![loc("/Usb0/Games", 3)], "/Hdd1/Games", 1));
    }
}
