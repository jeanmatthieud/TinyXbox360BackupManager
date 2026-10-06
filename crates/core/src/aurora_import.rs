// SPDX-License-Identifier: GPL-3.0-only

//! Game info and artwork handed to Aurora through its import folder, so the
//! console never has to fetch them itself.
//!
//! Aurora reads `<Aurora>/User/Import/<TitleID>/` when the user runs
//! *Settings > Assets > Import*: pictures (`cover`, `icon`, `banner`,
//! `background`, as PNG or JPEG) and one text file per field (`titlename`,
//! `description`, `publisher`, `developer`, `releasedate`, `genre`). What was
//! observed on a console, and what this module relies on
//! (`docs/aurora-import-notes.md`):
//!
//! - the folder is keyed by TitleID alone and waits for its game: one written
//!   before Aurora has scanned the game is silently skipped, and picked up by
//!   the first import run after the scan;
//! - Aurora never removes the files, and whatever is in the folder overwrites
//!   what Aurora holds for that item on every run. So a folder that exists is
//!   never touched again here — it may be the user's own;
//! - text files are read as UTF-8, with or without a BOM;
//! - a cover is only shown when it measures exactly 900x600.
//!
//! Screenshots are left out on purpose: Aurora appends them on every run.
//!
//! A title gets its folder even when no source has anything for it: that is
//! what marks it as dealt with, and Aurora ignores an empty folder.

use crate::config::CoverSource;
use crate::covers;
use crate::data_dir::DATA_DIR;
use crate::marketplace::{self, Artwork, GameInfo};
use crate::remote_fs::RemoteFs;
use crate::target::{DELETION_CANCELLED, Target, find_aurora_data_dir, find_child_ci, local_aurora_dir};
use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Error message used when the preparation is cancelled by the user (mirrors
/// [`crate::target::DELETION_CANCELLED`]). The folders already written stay.
pub const IMPORT_CANCELLED: &str = "Aurora import preparation cancelled";

/// Error message used when the target carries no Aurora to prepare for.
pub const AURORA_NOT_FOUND: &str = "Aurora was not found on this target";

/// Aurora's import folder, below its install folder.
const IMPORT_DIR: [&str; 2] = ["User", "Import"];

/// A game of the library to prepare the import folder of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportTitle {
    /// TitleID, 8 hex chars.
    pub title_id: String,
    /// Name shown while it is being prepared.
    pub title: String,
    /// Original Xbox games have no marketplace entry, and their own cover
    /// source.
    pub is_x360: bool,
}

/// Outcome of a preparation, one count per title that had no folder yet.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportReport {
    /// Titles whose folder was written with something in it.
    pub prepared: usize,
    /// Titles no source has anything for (their folder is created empty).
    pub unavailable: usize,
    /// Titles left for a later run: a source could not be reached.
    pub failed: usize,
}

/// Where the import folders are written to: a console-shaped session or a
/// mounted drive.
trait ImportStore {
    /// TitleIDs that already have a folder, upper-cased. Fails when the folder
    /// could not be listed: an empty answer would have every folder, the
    /// user's own included, overwritten.
    fn existing(&mut self) -> Result<HashSet<String>>;
    /// Removes the folder of a title, after a write into it failed.
    fn remove(&mut self, title_id: &str);
    /// Creates the folder of a title and writes `files` into it.
    fn write(&mut self, title_id: &str, files: &[(String, Vec<u8>)]) -> Result<()>;
}

struct RemoteStore<'a> {
    session: &'a mut dyn RemoteFs,
    /// `/Device/.../Aurora/User/Import`.
    root: String,
    /// False until Aurora, or a first preparation, has created `root`.
    root_exists: bool,
}

impl ImportStore for RemoteStore<'_> {
    fn existing(&mut self) -> Result<HashSet<String>> {
        // Aurora only creates the folder on its first import, and both
        // backends fail to list a missing one.
        if !self.root_exists {
            return Ok(HashSet::new());
        }
        let entries = self
            .session
            .try_list_dir(&self.root)
            .with_context(|| format!("listing {}", self.root))?;
        Ok(entries
            .into_iter()
            .filter(|e| e.is_dir)
            .map(|e| e.name.to_uppercase())
            .collect())
    }

    fn remove(&mut self, title_id: &str) {
        let dir = format!("{}/{title_id}", self.root);
        let _ = self
            .session
            .remove_dir_recursive(&dir, &AtomicBool::new(false), &mut |_, _| {});
    }

    fn write(&mut self, title_id: &str, files: &[(String, Vec<u8>)]) -> Result<()> {
        let dir = format!("{}/{title_id}", self.root);
        self.session.ensure_dir(&dir)?;
        for (name, bytes) in files {
            self.session.put_bytes(&dir, name, bytes)?;
        }
        Ok(())
    }
}

struct LocalStore {
    root: PathBuf,
}

impl ImportStore for LocalStore {
    fn existing(&mut self) -> Result<HashSet<String>> {
        match std::fs::read_dir(&self.root) {
            Ok(entries) => Ok(entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.file_name().to_string_lossy().to_uppercase())
                .collect()),
            // Not created yet: nothing in it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashSet::new()),
            Err(e) => Err(e).with_context(|| format!("listing {}", self.root.display())),
        }
    }

    fn remove(&mut self, title_id: &str) {
        let _ = std::fs::remove_dir_all(self.root.join(title_id));
    }

    fn write(&mut self, title_id: &str, files: &[(String, Vec<u8>)]) -> Result<()> {
        let dir = self.root.join(title_id);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        for (name, bytes) in files {
            let path = dir.join(name);
            std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        }
        Ok(())
    }
}

fn remote_dir_names(session: &mut dyn RemoteFs, dir: &str) -> HashSet<String> {
    session
        .list_dir(dir)
        .into_iter()
        .filter(|e| e.is_dir)
        .map(|e| e.name.to_uppercase())
        .collect()
}

fn local_dir_names(dir: &Path) -> HashSet<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.file_name().to_string_lossy().to_uppercase())
                .collect()
        })
        .unwrap_or_default()
}

/// Aurora's import folder on a console and whether it exists yet, `None` when
/// Aurora is not installed. Each level is matched whatever its case: the
/// console's filesystems ignore it, the FTP listing does not.
fn remote_import_dir(session: &mut dyn RemoteFs) -> Option<(String, bool)> {
    let data = find_aurora_data_dir(session)?;
    let mut dir = data.rsplit_once('/')?.0.to_string();
    let mut exists = true;
    for part in IMPORT_DIR {
        let found = exists
            .then(|| {
                session
                    .list_dir(&dir)
                    .into_iter()
                    .find(|e| e.is_dir && e.name.eq_ignore_ascii_case(part))
                    .map(|e| e.name)
            })
            .flatten();
        exists = found.is_some();
        dir = format!("{dir}/{}", found.unwrap_or_else(|| part.to_string()));
    }
    Some((dir, exists))
}

/// Aurora's import folder on a mounted drive (see [`remote_import_dir`]).
fn local_import_dir(mount: &Path) -> Option<PathBuf> {
    let mut dir = local_aurora_dir(mount)?;
    for part in IMPORT_DIR {
        dir = find_child_ci(&dir, part).unwrap_or_else(|| dir.join(part));
    }
    Some(dir)
}

/// TitleIDs (upper-cased) that already have an import folder on a console;
/// empty without Aurora. A read: it needs no place in the job queue.
pub(crate) fn remote_import_titles(session: &mut dyn RemoteFs) -> Vec<String> {
    remote_import_dir(session)
        .map(|(dir, _)| remote_dir_names(session, &dir).into_iter().collect())
        .unwrap_or_default()
}

/// Local pendant of [`remote_import_titles`].
pub(crate) fn local_import_titles(mount: &Path) -> Vec<String> {
    local_import_dir(mount)
        .map(|dir| local_dir_names(&dir).into_iter().collect())
        .unwrap_or_default()
}

impl Target {
    /// False when this target is known to carry no Aurora, without opening a
    /// console session: only a mounted drive can tell that cheaply.
    pub fn may_have_aurora(&self) -> bool {
        match self {
            Target::Local(mount) => local_aurora_dir(mount).is_some(),
            _ => true,
        }
    }

    /// Writes the import folder of every game of `titles` that has none yet
    /// (see the module docs). `progress(done, total, title)` is called before
    /// each of them. Writes to the target: run it as a queued job.
    ///
    /// Fails with [`AURORA_NOT_FOUND`] when the target has no Aurora. Raising
    /// `cancel` stops at the next title, failing with [`IMPORT_CANCELLED`];
    /// what was written so far stays.
    pub fn prepare_aurora_import(
        &self,
        titles: &[ImportTitle],
        source: CoverSource,
        locale: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize, &str),
    ) -> Result<ImportReport> {
        match self {
            Target::Local(mount) => {
                let root = local_import_dir(mount).context(AURORA_NOT_FOUND)?;
                prepare(&mut LocalStore { root }, titles, source, locale, cancel, progress)
            }
            _ => {
                let mut session = self.open_remote(true)?;
                let result = match remote_import_dir(&mut session) {
                    Some((root, root_exists)) => {
                        let mut store = RemoteStore {
                            session: &mut session,
                            root,
                            root_exists,
                        };
                        prepare(&mut store, titles, source, locale, cancel, progress)
                    }
                    None => Err(anyhow::anyhow!(AURORA_NOT_FOUND)),
                };
                let closed = session.quit();
                result.and_then(|report| closed.map(|()| report))
            }
        }
    }
}

impl Target {
    /// Removes every folder of Aurora's import folder — the app's and the
    /// user's alike — and returns how many there were. What Aurora has already
    /// imported is its own copy, and stays. `progress(done, total)` is called
    /// before each folder. Writes to the target: run it as a queued job.
    ///
    /// Raising `cancel` fails with [`DELETION_CANCELLED`]; the folders already
    /// removed are gone.
    pub fn clear_aurora_import(
        &self,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<usize> {
        match self {
            Target::Local(mount) => {
                let root = local_import_dir(mount).context(AURORA_NOT_FOUND)?;
                let dirs: Vec<PathBuf> = std::fs::read_dir(&root)
                    .map(|entries| {
                        entries
                            .flatten()
                            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                            .map(|e| e.path())
                            .collect()
                    })
                    .unwrap_or_default();
                for (done, dir) in dirs.iter().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        bail!(DELETION_CANCELLED);
                    }
                    progress(done, dirs.len());
                    std::fs::remove_dir_all(dir)
                        .with_context(|| format!("removing {}", dir.display()))?;
                }
                Ok(dirs.len())
            }
            _ => {
                let mut session = self.open_remote(true)?;
                let result = (|| {
                    let (root, _) = remote_import_dir(&mut session).context(AURORA_NOT_FOUND)?;
                    let dirs: Vec<String> = session
                        .list_dir(&root)
                        .into_iter()
                        .filter(|e| e.is_dir)
                        .map(|e| e.name)
                        .collect();
                    for (done, name) in dirs.iter().enumerate() {
                        if cancel.load(Ordering::Relaxed) {
                            bail!(DELETION_CANCELLED);
                        }
                        progress(done, dirs.len());
                        session.remove_dir_recursive(&format!("{root}/{name}"), cancel, &mut |_, _| {})?;
                    }
                    Ok(dirs.len())
                })();
                let closed = session.quit();
                result.and_then(|count| closed.map(|()| count))
            }
        }
    }
}

/// The preparation proper, shared by both kinds of target.
fn prepare(
    store: &mut dyn ImportStore,
    titles: &[ImportTitle],
    source: CoverSource,
    locale: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize, &str),
) -> Result<ImportReport> {
    let existing = store.existing()?;
    // The discs of a game share its TitleID, hence its folder.
    let mut seen = HashSet::new();
    let todo: Vec<&ImportTitle> = titles
        .iter()
        .filter(|t| {
            let id = t.title_id.to_uppercase();
            !id.is_empty() && !existing.contains(&id) && seen.insert(id)
        })
        .collect();

    // Original Xbox covers come from MobCat's database: refresh it once, and
    // only when one of them may be needed.
    if todo.iter().any(|t| !t.is_x360) {
        crate::mobcat::ensure_db();
    }

    let mut report = ImportReport::default();
    for (done, title) in todo.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            bail!(IMPORT_CANCELLED);
        }
        progress(done, todo.len(), &title.title);

        let title_id = title.title_id.to_uppercase();
        match import_files(&title_id, title.is_x360, source, locale) {
            Ok(files) => {
                // A folder that exists counts as done: one left half written
                // would never get the rest.
                if let Err(e) = store.write(&title_id, &files) {
                    store.remove(&title_id);
                    return Err(e);
                }
                if files.is_empty() {
                    report.unavailable += 1;
                } else {
                    report.prepared += 1;
                }
            }
            // No folder: the next run tries this title again.
            Err(e) => {
                eprintln!("Aurora import of {title_id} skipped: {e:#}");
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// Everything the sources have for a title, as `(file name, content)`. Fails
/// when the marketplace sources could not be reached, so that an outage is
/// not mistaken for a title that has nothing.
fn import_files(
    title_id: &str,
    is_x360: bool,
    source: CoverSource,
    locale: &str,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();

    if let Some(cover) = cover(title_id, is_x360, source) {
        files.push(cover);
    }
    if !is_x360 {
        return Ok(files);
    }

    for (kind, stem) in [
        (Artwork::Icon, "icon"),
        (Artwork::Banner, "banner"),
        (Artwork::Background, "background"),
    ] {
        if let Some(bytes) = marketplace::artwork(title_id, kind)?
            && let Some(ext) = marketplace::image_extension(&bytes)
        {
            files.push((format!("{stem}.{ext}"), bytes));
        }
    }
    if let Some(info) = marketplace::game_info(title_id, locale)? {
        files.extend(info_files(&info));
    }
    Ok(files)
}

/// Size Aurora wants a cover in; it shows nothing for any other.
const COVER_SIZE: (u32, u32) = (900, 600);

/// The cover of a title, from the cache the library's covers already live in
/// (downloaded into it first when missing), brought to [`COVER_SIZE`] when
/// it is not already. A picture of about that shape is stretched to it —
/// XboxUnity's odd 897x600 — while any other (the portrait box fronts of
/// original Xbox games, 300x420) is fitted inside and centred on black.
fn cover(title_id: &str, is_x360: bool, source: CoverSource) -> Option<(String, Vec<u8>)> {
    let covers_dir = DATA_DIR.join("covers");
    // A failed download just means no cover: the cache check below decides.
    let _ = covers::download_cover(&covers_dir, title_id, is_x360, source);
    let bytes = std::fs::read(covers::cached_cover(&covers_dir, title_id)?).ok()?;

    let img = match image::load_from_memory(&bytes) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("Unusable cover for {title_id}: {e}");
            return None;
        }
    };
    if (img.width(), img.height()) == COVER_SIZE {
        // Named after what the file really is, like the cache does.
        let ext = marketplace::image_extension(&bytes)?;
        return Some((format!("cover.{ext}"), bytes));
    }

    let (w, h) = COVER_SIZE;
    let filter = image::imageops::FilterType::Lanczos3;
    let ratio = f64::from(img.width()) / f64::from(img.height());
    let resized = if (ratio - f64::from(w) / f64::from(h)).abs() < 0.03 {
        img.resize_exact(w, h, filter)
    } else {
        let fitted = img.resize(w, h, filter);
        let mut canvas = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
        let x = i64::from((w - fitted.width()) / 2);
        let y = i64::from((h - fitted.height()) / 2);
        image::imageops::overlay(&mut canvas, &fitted.to_rgba8(), x, y);
        image::DynamicImage::ImageRgba8(canvas)
    };
    // Lossless, so the resize is the only change made to the picture.
    let mut png = std::io::Cursor::new(Vec::new());
    match resized.write_to(&mut png, image::ImageFormat::Png) {
        Ok(()) => Some(("cover.png".to_string(), png.into_inner())),
        Err(e) => {
            eprintln!("Unusable cover for {title_id}: {e}");
            None
        }
    }
}

/// One text file per non-empty field, in UTF-8.
fn info_files(info: &GameInfo) -> Vec<(String, Vec<u8>)> {
    // No space after the comma: nothing says Aurora trims the names it
    // matches against its own list.
    let genres = info.genres.join(",");
    [
        ("titlename.txt", info.title.as_str()),
        ("description.txt", info.description.as_str()),
        ("publisher.txt", info.publisher.as_str()),
        ("developer.txt", info.developer.as_str()),
        ("releasedate.txt", info.release_date.as_str()),
        ("genre.txt", genres.as_str()),
    ]
    .into_iter()
    .filter(|(_, text)| !text.is_empty())
    .map(|(name, text)| (name.to_string(), text.as_bytes().to_vec()))
    .collect()
}
