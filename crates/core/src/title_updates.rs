// SPDX-License-Identifier: GPL-3.0-only

//! Title Update installation state and activation on the console / local
//! drive: which updates (from the XboxUnity API) sit in `<TitleID>/000B0000`
//! (the folder the dashboard reads at boot), which ones Aurora has already
//! downloaded into its own cache, and moving a cached one in or out of
//! `000B0000` to activate or deactivate it.
//!
//! This app never downloads a title update itself: activating one only
//! works if Aurora's own cache already has it (the user fetched it from
//! within Aurora), so we never redistribute copyrighted update content
//! ourselves.
//!
//! The console only tolerates one FTP connection at a time, so every
//! operation here opens its own short-lived session.

use crate::hash_cache::HashCache;
use crate::remote_fs::RemoteFs;
use crate::game::{FATX_MAX_NAME, Game};
use crate::target::{self, Target};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Folder read by the dashboard for a title's active updates.
const TITLE_UPDATE_DIR: &str = "000B0000";
/// Aurora's own cache of downloaded title updates, under its `Data` folder.
const AURORA_CACHE_SUBDIR: &str = "TitleUpdates";

fn truncate_fatx(name: &str) -> String {
    name.chars().take(FATX_MAX_NAME).collect()
}

/// A title update file found in `<TitleID>/000B0000`, identified by the
/// SHA1 of its content (matches the `hash` field of the XboxUnity API).
#[derive(Debug, Clone)]
pub struct InstalledTitleUpdate {
    pub file_name: String,
    pub hash: String,
}

/// What the Title Updates window needs to know about one game: the updates
/// sitting in its `000B0000` folder, and the hashes Aurora already holds in
/// its own cache (so an update can be activated without downloading it).
pub struct TitleUpdateState {
    pub installed: Vec<InstalledTitleUpdate>,
    pub cached_hashes: Vec<String>,
}

impl Target {
    /// Both halves of [`TitleUpdateState`], read in **one** session.
    ///
    /// They used to be two calls, hence two FTP logins — and, worse, two full
    /// runs of [`target::find_aurora_data_dir`], whose memoization lives on
    /// the session and died with the first one.
    pub fn title_update_state(&self, game: &Game) -> Result<TitleUpdateState> {
        match self {
            Target::Local(path) => Ok(TitleUpdateState {
                installed: installed_local(game.content_dir()),
                cached_hashes: cached_hashes_local(path, &game.id),
            }),
            _ => {
                let mut session = self.open_remote(false)?;
                let key = self.remote_key();
                let result = (|| {
                    let mut cache = HashCache::load();
                    let installed =
                        installed_remote(&mut session, game.content_dir(), &key, &mut cache)?;
                    cache.save();
                    let cached_hashes = cached_hashes_remote(&mut session, &game.id);
                    Ok(TitleUpdateState {
                        installed,
                        cached_hashes,
                    })
                })();
                session.quit()?;
                result
            }
        }
    }

    /// Installs the title update identified by `hash` into
    /// `<TitleID>/000B0000`. Fails if Aurora's cache does not already have
    /// it: this app never downloads update content itself.
    pub fn activate_title_update(&self, game: &Game, hash: &str) -> Result<()> {
        match self {
            Target::Local(path) => {
                let Some((file_name, bytes)) = find_cached_local(path, &game.id, hash)? else {
                    bail!(
                        "this title update is not in Aurora's cache; \
                         download it from within Aurora first"
                    );
                };
                let dest_dir = game.content_dir().join(TITLE_UPDATE_DIR);
                std::fs::create_dir_all(&dest_dir)
                    .with_context(|| format!("creating {}", dest_dir.display()))?;
                std::fs::write(dest_dir.join(truncate_fatx(&file_name)), bytes)
                    .with_context(|| format!("writing {file_name}"))
            }
            _ => {
                let mut session = self.open_remote(true)?;
                let result = (|| {
                    let Some((file_name, bytes)) = find_cached_remote(&mut session, &game.id, hash)?
                    else {
                        bail!(
                            "this title update is not in Aurora's cache; \
                             download it from within Aurora first"
                        );
                    };
                    let remote_dir = format!(
                        "{}/{TITLE_UPDATE_DIR}",
                        game.content_dir().to_string_lossy().replace('\\', "/")
                    );
                    session.put_bytes(&remote_dir, &truncate_fatx(&file_name), &bytes)
                })();
                let closed = session.quit();
                result.and(closed)
            }
        }
    }

    /// Removes an installed title update from `<TitleID>/000B0000`.
    pub fn deactivate_title_update(&self, game: &Game, file_name: &str) -> Result<()> {
        match self {
            Target::Local(_) => {
                let path = game.content_dir().join(TITLE_UPDATE_DIR).join(file_name);
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))
            }
            _ => {
                let mut session = self.open_remote(true)?;
                let remote_dir = format!(
                    "{}/{TITLE_UPDATE_DIR}",
                    game.content_dir().to_string_lossy().replace('\\', "/")
                );
                let result = session.remove_file(&remote_dir, file_name);
                let closed = session.quit();
                result.and(closed)
            }
        }
    }
}

fn installed_local(title_dir: &Path) -> Vec<InstalledTitleUpdate> {
    let dir = title_dir.join(TITLE_UPDATE_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            // Streamed: a title update is routinely 100 MB and only its
            // digest is ever wanted.
            let hash = crate::util::sha1_hex_file(&path).ok()?;
            Some(InstalledTitleUpdate {
                file_name: entry.file_name().to_string_lossy().to_string(),
                hash,
            })
        })
        .collect()
}

fn installed_remote(
    session: &mut dyn RemoteFs,
    title_dir: &Path,
    cache_key: &str,
    cache: &mut HashCache,
) -> Result<Vec<InstalledTitleUpdate>> {
    let remote = format!(
        "{}/{TITLE_UPDATE_DIR}",
        title_dir.to_string_lossy().replace('\\', "/")
    );
    let mut updates = Vec::new();
    for entry in session.list_dir(&remote) {
        if entry.is_dir {
            continue;
        }
        let path = format!("{remote}/{}", entry.name);
        // Hashing a file on the console means pulling all of it across, so the
        // digest is remembered between runs: opening this window again, or
        // re-reading the list after an activation, costs no transfer at all.
        let hash = match cache.get(cache_key, &path, entry.size) {
            Some(hash) => hash.clone(),
            None => {
                let hash = session.sha1_file(&path)?;
                cache.insert(cache_key, &path, entry.size, hash.clone());
                hash
            }
        };
        updates.push(InstalledTitleUpdate {
            file_name: entry.name.clone(),
            hash,
        });
    }
    Ok(updates)
}

/// Aurora's title-update cache on a mounted drive:
/// `<aurora>/Data/TitleUpdates`, with `<aurora>` resolved the way every other
/// local lookup resolves it (a `launch.ini` override, `Dashboard/Aurora`, any
/// casing) rather than assumed to be `<mount>/Aurora`.
fn local_cache_root(mount: &Path) -> Option<PathBuf> {
    let data = target::local_aurora_data_dir(mount)?;
    crate::util::find_dir_ci(&data, AURORA_CACHE_SUBDIR)
}

fn cached_hashes_local(mount: &Path, title_id: &str) -> Vec<String> {
    let Some(root) = local_cache_root(mount) else {
        return Vec::new();
    };
    let Ok(profiles) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut hashes = Vec::new();
    for profile in profiles.flatten() {
        let Some(title_dir) = crate::util::find_dir_ci(&profile.path(), title_id) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(&title_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                hashes.push(entry.file_name().to_string_lossy().to_uppercase());
            }
        }
    }
    hashes
}

/// Looks for `hash`'s cached download under Aurora's own
/// `<aurora>/Data/TitleUpdates/<profile>/<TitleID>/<hash>/` cache.
fn find_cached_local(
    mount: &Path,
    title_id: &str,
    hash: &str,
) -> Result<Option<(String, Vec<u8>)>> {
    let Some(root) = local_cache_root(mount) else {
        return Ok(None);
    };
    let Ok(profiles) = std::fs::read_dir(&root) else {
        return Ok(None);
    };
    for profile in profiles.flatten() {
        let Some(title_dir) = crate::util::find_dir_ci(&profile.path(), title_id) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(&title_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || !entry.file_name().to_string_lossy().eq_ignore_ascii_case(hash) {
                continue;
            }
            let Some(file) = std::fs::read_dir(&path)?
                .flatten()
                .find(|e| e.path().is_file())
            else {
                continue;
            };
            let bytes = std::fs::read(file.path())
                .with_context(|| format!("reading {}", file.path().display()))?;
            return Ok(Some((file.file_name().to_string_lossy().to_string(), bytes)));
        }
    }
    Ok(None)
}

fn cached_hashes_remote(session: &mut dyn RemoteFs, title_id: &str) -> Vec<String> {
    let Some(data_dir) = target::find_aurora_data_dir(session) else {
        return Vec::new();
    };
    let root = format!("{data_dir}/TitleUpdates");
    let mut hashes = Vec::new();
    for profile in session.list_dir(&root) {
        if !profile.is_dir {
            continue;
        }
        let title_dir = format!("{root}/{}/{title_id}", profile.name);
        for entry in session.list_dir(&title_dir) {
            if entry.is_dir {
                hashes.push(entry.name.to_uppercase());
            }
        }
    }
    hashes
}

/// Looks for `hash`'s cached download under Aurora's
/// `Data/TitleUpdates/<profile>/<TitleID>/<hash>/` cache.
fn find_cached_remote(
    session: &mut dyn RemoteFs,
    title_id: &str,
    hash: &str,
) -> Result<Option<(String, Vec<u8>)>> {
    let Some(data_dir) = target::find_aurora_data_dir(session) else {
        return Ok(None);
    };
    let root = format!("{data_dir}/TitleUpdates");
    for profile in session.list_dir(&root) {
        if !profile.is_dir {
            continue;
        }
        let title_dir = format!("{root}/{}/{title_id}", profile.name);
        let Some(hash_entry) = session
            .list_dir(&title_dir)
            .into_iter()
            .find(|e| e.is_dir && e.name.eq_ignore_ascii_case(hash))
        else {
            continue;
        };
        let hash_dir = format!("{title_dir}/{}", hash_entry.name);
        if let Some(file) = session.list_dir(&hash_dir).into_iter().find(|f| !f.is_dir) {
            let bytes = session.download_file(&format!("{hash_dir}/{}", file.name))?;
            return Ok(Some((file.name, bytes)));
        }
    }
    Ok(None)
}
