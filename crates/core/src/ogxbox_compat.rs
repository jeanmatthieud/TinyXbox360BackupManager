// SPDX-License-Identifier: GPL-3.0-only

//! Writing the original-Xbox compatibility partition of an Xbox 360.
//!
//! The Xbox 360 plays original Xbox games through an emulator Microsoft called
//! Xenon Fusion — `xefu` for short. It does not live with the games: it sits on
//! a partition of its own, which the console exposes as `HddX` and which holds
//! a single `Compatibility` folder. That partition is only ever created when a
//! drive is formatted at the Microsoft factory, so a third-party drive, a
//! reformatted one, or one whose partition was lost simply cannot launch an
//! original Xbox title — whatever the dashboard says.
//!
//! This module downloads one of the public emulator sets ("XeFu packs") and
//! writes it there. Everything it does goes through [`RemoteFs`], so the same
//! code serves a console reached over FTP and a console hard drive plugged
//! into this computer over SATA.
//!
//! # Why only the ConsoleMods packs
//!
//! Two "retail" sets circulate and look contradictory. FATXplorer distributes a
//! *restoration kit*: the partition files as Microsoft first shipped them in
//! 2005 (`xefu.xex` SHA-1 `A97D65C4…`), plus the April 2018 title update that
//! brings the console up to date. ConsoleMods distributes a *dump of a factory
//! partition*: the same `xbox.xex` (`1C577B74…`) but with all eight official
//! xefu revisions already in place and `xefu.xex` already patched
//! (`87CD5F28…`).
//!
//! Reading the strings of that title update settles it — it carries
//! `xbox.xexp`, `xefu.xexp` and `xefu1_1/2/3/5/6/7/7b` plus the `xefutitle*`
//! files. In other words it *is* the delivery vehicle for what ConsoleMods
//! lays down directly. Taking the ConsoleMods packs therefore reaches the same
//! result in one write on one partition, which is why this tool offers only
//! those and never touches the content partition. The one thing it does not
//! carry is the `.xexp` patch that takes `xbox.xex` to build 5832 — an older
//! whitelist, irrelevant on a modded console (the hacked packs drop the
//! whitelist entirely) and re-delivered over Live on a stock one.
//!
//! # Why web.archive.org
//!
//! consolemods.org sits behind a Cloudflare managed challenge: every
//! non-browser client is answered `403 cf-mitigated: challenge`, whatever its
//! User-Agent. The URLs below therefore point at pinned web.archive.org
//! snapshots, which also freezes a known version of each pack — the wiki
//! replaces them in place as they are updated.

use crate::archive;
use crate::data_dir::TMP_DIR;
use crate::download::{self, archive_extension, find_entry};
use crate::remote_fs::RemoteFs;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Device the emulator partition is exposed under, on either transport.
pub const COMPAT_VOLUME: &str = "HddX";

/// The one folder the partition holds, and the one this tool replaces.
pub const COMPAT_DIR: &str = "Compatibility";

/// Full path of that folder, as both backends spell it.
pub const COMPAT_PATH: &str = "/HddX/Compatibility";

/// Sub-directory of the app's temp area used to download and unpack a pack.
/// Recreated from scratch on every run, so a half-finished previous attempt is
/// never mistaken for this one's payload.
pub const WORK_SUBDIR: &str = "ogxbox-compat";

/// Error message signalling that the user cancelled, matched by the GUI to
/// show a friendly notice instead of an error (mirrors
/// `convert::CONVERSION_CANCELLED`).
pub const COMPAT_CANCELLED: &str = "compatibility install cancelled";

/// One emulator set offered in the drop-down.
pub struct CompatPack {
    /// Stable key stored in the config; survives a reordering of the list.
    pub key: &'static str,
    pub label: &'static str,
    /// One line shown under the picker: what the pack is, and what it needs.
    pub description: &'static str,
    pub url: &'static str,
    /// A retail pack is signed by Microsoft and runs on a stock console; a
    /// hacked one needs a JTAG, RGH or XDK.
    pub needs_exploit: bool,
}

/// The public XeFu packs, in the order the drop-down shows them. The first is
/// the default. URLs are pinned on purpose — see the module docs on both the
/// choice of packs and the use of web.archive.org.
pub const COMPAT_PACKS: &[CompatPack] = &[
    CompatPack {
        key: "retail",
        label: "Retail — unmodified",
        description: "The eight official emulator revisions, as a factory drive carried them. \
                      Signed by Microsoft, so it runs on a stock console.",
        url: "https://web.archive.org/web/20260805050419id_/https://consolemods.org/wiki/images/2/28/Unmodified_Retail_Xefu_Pack.zip",
        needs_exploit: false,
    },
    CompatPack {
        key: "hacked",
        label: "Hacked — no whitelist",
        description: "The official revisions with every restriction and the game whitelist \
                      removed, plus the four emulators taken from Xbox One/Series releases \
                      and the per-game config loader.",
        url: "https://web.archive.org/web/20260805050419id_/https://consolemods.org/wiki/images/9/9d/Hacked_Xefu_Pack.zip",
        needs_exploit: true,
    },
    CompatPack {
        key: "hacked_hud",
        label: "Hacked — no whitelist, with HUD",
        description: "Same as the hacked pack, but the Xbox 360 guide stays available while \
                      an original Xbox game runs. The extra memory it uses can make some \
                      games behave worse.",
        url: "https://web.archive.org/web/20260805050419id_/https://consolemods.org/wiki/images/c/c4/Hacked_Xefu_Pack_with_HUD.zip",
        needs_exploit: true,
    },
];

/// The pack a config names, falling back to the first for an empty or unknown
/// key — a config written by a later version must never stop the tool working.
pub fn pack_by_key(key: &str) -> &'static CompatPack {
    COMPAT_PACKS
        .iter()
        .find(|p| p.key == key)
        .unwrap_or(&COMPAT_PACKS[0])
}

/// Persisted settings for the compatibility tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
// A config written before a field existed still loads, with that field at its
// default — the app must never refuse to start over its own settings file.
#[serde(default)]
pub struct OgXboxCompatConfig {
    /// [`CompatPack::key`] of the chosen pack; empty means the first one.
    pub pack: String,
    /// Copy the existing `Compatibility` folder to a local directory first.
    pub backup_first: bool,
}

impl OgXboxCompatConfig {
    pub fn pack(&self) -> &'static CompatPack {
        pack_by_key(&self.pack)
    }
}

/// Bails with [`COMPAT_CANCELLED`] once the user has asked to stop. Callers
/// use it right after a fallible step and *before* adding any context, so the
/// marker stays the error's top-level message.
pub fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    download::check_cancel(cancel, COMPAT_CANCELLED)
}

/// Fails unless the target actually exposes the emulator partition.
///
/// Over FTP this is a real question — the console lists `HddX` only when the
/// partition exists. On a FATX session the partition was named when the device
/// was opened, so `list_root` answers `HddX` by construction and this check
/// costs nothing.
fn ensure_compat_volume(fs: &mut dyn RemoteFs) -> Result<()> {
    let devices = fs.list_root().context("listing the target's volumes")?;
    if devices
        .iter()
        .any(|d| d.eq_ignore_ascii_case(COMPAT_VOLUME))
    {
        return Ok(());
    }
    bail!(
        "this console has no original-Xbox compatibility partition ({COMPAT_VOLUME}).\n\n\
         It is only created when a drive is formatted at the Microsoft factory. Create it \
         first — with the `HDD Compatibility Partition Fixer` homebrew run on the console, \
         or with FATXplorer on Windows — then come back here."
    );
}

/// Downloads the configured pack and unpacks it, returning the local
/// `Compatibility` folder ready to be copied. Touches no console storage.
/// Whether a zip carries a `Compatibility` folder **at its root**, which is
/// what this tool's own backups look like.
///
/// Deliberately stricter than [`stage_pack`], which hunts the folder down
/// wherever a publisher chose to bury it: a file the user points at by hand is
/// worth refusing early and clearly, rather than extracting fifty megabytes to
/// discover it was the wrong archive. Reads the central directory only —
/// nothing is unpacked.
pub fn zip_holds_compatibility(path: &Path) -> Result<bool> {
    let file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("reading {} as a zip archive", path.display()))?;

    for i in 0..zip.len() {
        let entry = zip.by_index(i).context("reading the archive's index")?;
        // `enclosed_name` refuses the traversal tricks a hand-made archive
        // could carry, and is what the extractor will honour later.
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let mut parts = name.components();
        let Some(first) = parts.next() else {
            continue;
        };
        if first.as_os_str().eq_ignore_ascii_case(COMPAT_DIR) {
            // A bare `Compatibility/` entry with nothing under it is an empty
            // folder, not a backup.
            if entry.is_dir() && parts.next().is_none() {
                continue;
            }
            return Ok(true);
        }
    }
    Ok(false)
}

/// Unpacks a backup this tool wrote earlier, returning its `Compatibility`
/// folder ready to install. The archive is the user's own file, so it is only
/// ever read.
pub fn stage_backup(
    archive_path: &Path,
    cancel: &AtomicBool,
    status: &dyn Fn(&str),
) -> Result<PathBuf> {
    let work = fresh_work_dir()?;
    let label = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the backup".to_string());
    extract_and_locate(archive_path, &work, &label, cancel, status)
}

/// Empties and recreates the working directory, so a half-finished previous run
/// cannot be mistaken for this one's payload.
fn fresh_work_dir() -> Result<PathBuf> {
    let work = TMP_DIR.join(WORK_SUBDIR);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;
    Ok(work)
}

/// Extracts an archive into `work` and returns the `Compatibility` folder
/// inside it, wherever the archive chose to put it.
fn extract_and_locate(
    archive_path: &Path,
    work: &Path,
    label: &str,
    cancel: &AtomicBool,
    status: &dyn Fn(&str),
) -> Result<PathBuf> {
    check_cancel(cancel)?;
    status(&format!("Extracting {label}…"));
    let extracted = work.join("pack");
    let res = archive::extract_to(archive_path, &extracted, cancel, &mut |_done, _total| {});
    // A cancelled extraction is reported with this module's own marker, as the
    // top-level message (the GUI matches on `to_string()`, not on the chain).
    check_cancel(cancel)?;
    res.with_context(|| format!("extracting {label}"))?;

    // The packs wrap their payload differently: some put the folder under a
    // directory named after the release, others have it at the root. So it is
    // found by name rather than at a fixed path.
    find_entry(&extracted, COMPAT_DIR, true).with_context(|| {
        format!("{label}: no `{COMPAT_DIR}` folder inside the archive — is this really a XeFu pack?")
    })
}

pub fn stage_pack(
    cfg: &OgXboxCompatConfig,
    cancel: &AtomicBool,
    status: &dyn Fn(&str),
) -> Result<PathBuf> {
    let pack = cfg.pack();
    let url = pack.url;
    // The URLs are pinned constants, so this only ever fires on a bad edit to
    // `COMPAT_PACKS` — but the archive reader supports nothing else, and a
    // wrong extension is far clearer said here than as a failed extraction.
    let ext = archive_extension(url).with_context(|| {
        format!(
            "{}: only .zip and .7z archives are supported (got {url}). Use a .zip/.7z mirror.",
            pack.label
        )
    })?;

    let work = fresh_work_dir()?;

    check_cancel(cancel)?;
    let archive_path = work.join(format!("pack.{ext}"));
    status(&format!("Downloading {}…", pack.label));
    let res = download::download_to_file(
        url,
        &archive_path,
        pack.label,
        cancel,
        status,
        COMPAT_CANCELLED,
    );
    // A cancelled download is reported with this module's own marker as the
    // top-level message: the GUI matches on `to_string()`, which returns the
    // outermost context and not the chain, so no `.context()` may wrap it.
    check_cancel(cancel)?;
    res.with_context(|| format!("downloading {}", pack.label))?;

    extract_and_locate(&archive_path, &work, pack.label, cancel, status)
}

/// Checks the target really has a compatibility partition, and reports whether
/// it already holds files. The session may (and should) be read-only.
///
/// Deliberately shallow: what is there gets deleted either way, so the user
/// only needs to be told that it will be. Naming the emulator in place would
/// mean carrying a table of hashes that no pack variant, and no hand-made
/// arrangement of a user's own, is obliged to match.
pub fn inspect_remote(fs: &mut dyn RemoteFs) -> Result<bool> {
    ensure_compat_volume(fs)?;
    Ok(!list_dir(fs, COMPAT_PATH)?.is_empty())
}

/// Lists a directory of the partition, reporting one that cannot be read.
///
/// Everything here either copies files that are about to be deleted or decides
/// whether to delete them, so the scanner's forgiving [`RemoteFs::list_dir`] —
/// which answers an unreadable directory with an empty list — must never be
/// used: a transient `LIST` failure would look exactly like an empty folder and
/// quietly leave files out of the backup. An absent folder is the one case that
/// legitimately reads as empty, and it is reported as such.
fn list_dir(fs: &mut dyn RemoteFs, dir: &str) -> Result<Vec<crate::ftp::RemoteEntry>> {
    match fs.try_list_dir(dir) {
        Ok(entries) => Ok(entries),
        // The one directory that may legitimately not be there is the
        // `Compatibility` folder itself, on a partition that has never held
        // the emulator. Anything else missing is a tree that moved under the
        // walk, which is not something to paper over.
        Err(_) if dir == COMPAT_PATH && dir_missing(fs, dir) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Whether `dir`'s parent could be read *and* does not list it.
///
/// Only ever asked about a directory whose own listing just failed, to tell
/// "not there" from "unreadable". A parent that cannot be read either answers
/// `false`, so the original failure is reported rather than taken for an empty
/// folder — the whole point of this module listing the strict way.
fn dir_missing(fs: &mut dyn RemoteFs, dir: &str) -> bool {
    let Some((parent, name)) = dir.rsplit_once('/') else {
        return false;
    };
    match fs.try_list_dir(parent) {
        Ok(entries) => !entries.iter().any(|e| e.name.eq_ignore_ascii_case(name)),
        Err(_) => false,
    }
}

/// Recursive file count, used to scale the backup's progress *and* to check
/// afterwards that every file it saw was copied.
fn count_files(fs: &mut dyn RemoteFs, dir: &str) -> Result<u64> {
    let mut count = 0;
    for entry in list_dir(fs, dir)? {
        if entry.is_dir {
            count += count_files(fs, &format!("{dir}/{}", entry.name))?;
        } else {
            count += 1;
        }
    }
    Ok(count)
}

/// Saves the target's `Compatibility` folder as a zip archive at `dest`.
///
/// The entries keep the `Compatibility/…` prefix the published packs use, so a
/// backup *is* a pack: restoring one later is [`stage_pack`] pointed at a local
/// file, not a code path of its own.
///
/// The archive is assembled in the working directory and only moved into place
/// once it is complete, so a run that fails or is cancelled leaves nothing
/// half-written where the user asked for their copy.
///
/// A partition with nothing on it writes no archive at all — there is nothing
/// to save, and the install that follows is a first installation.
///
/// Returns how many files were saved, so that case can be told apart from a
/// real backup: the user picked a destination and no file appeared there, which
/// they have to be told about rather than left to discover.
pub fn backup_remote(
    fs: &mut dyn RemoteFs,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<u64> {
    ensure_compat_volume(fs)?;

    let total = count_files(fs, COMPAT_PATH).context("listing the compatibility files")?;
    progress(0, total);
    if total == 0 {
        return Ok(0);
    }

    let work = TMP_DIR.join(WORK_SUBDIR);
    fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;
    let staged = work.join("backup.zip");
    let _ = fs::remove_file(&staged);

    let done = {
        let file =
            fs::File::create(&staged).with_context(|| format!("creating {}", staged.display()))?;
        let mut backup = Backup {
            zip: ZipWriter::new(file),
            cancel,
            done: 0,
            total,
            progress,
        };
        backup.walk(fs, COMPAT_PATH, COMPAT_DIR)?;
        backup.zip.finish().context("closing the backup archive")?;
        backup.done
    };

    // The caller deletes these files right after, on the strength of this
    // `Ok(())`. A count that has drifted means the walk saw a different tree
    // than the one it was scaled against, so the copy cannot be called
    // complete — and only a copy that is provably complete may license a
    // delete.
    if done != total {
        // The half-built archive never reaches the user: an incomplete backup
        // that looks like a complete one is worse than none.
        let _ = fs::remove_file(&staged);
        bail!(
            "backed up {done} of {total} compatibility files — the console's answers did \
             not add up, so the backup cannot be trusted. Nothing was changed on the console."
        );
    }

    move_file(&staged, dest)?;
    Ok(done)
}

/// Moves the finished archive to where the user asked for it, falling back to a
/// copy when that crosses a filesystem — the working directory sits beside the
/// app's own data, and the destination is wherever the save dialog pointed.
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to).with_context(|| format!("writing {}", to.display()))?;
    let _ = fs::remove_file(from);
    Ok(())
}

/// The archive being filled, and what the walk needs to keep as it descends.
struct Backup<'a> {
    zip: ZipWriter<fs::File>,
    cancel: &'a AtomicBool,
    done: u64,
    total: u64,
    progress: &'a mut dyn FnMut(u64, u64),
}

impl Backup<'_> {
    /// Copies one remote directory into the archive under `zip_dir`, and
    /// everything below it.
    fn walk(&mut self, fs: &mut dyn RemoteFs, remote_dir: &str, zip_dir: &str) -> Result<()> {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        self.zip
            .add_directory(format!("{zip_dir}/"), options)
            .with_context(|| format!("adding {zip_dir}/ to the archive"))?;

        for entry in list_dir(fs, remote_dir)? {
            check_cancel(self.cancel)?;
            let remote = format!("{remote_dir}/{}", entry.name);
            let inside = format!("{zip_dir}/{}", entry.name);
            if entry.is_dir {
                self.walk(fs, &remote, &inside)?;
            } else {
                let bytes = fs
                    .download_file(&remote)
                    .with_context(|| format!("reading {remote}"))?;
                self.zip
                    .start_file(&inside, options)
                    .with_context(|| format!("adding {inside} to the archive"))?;
                self.zip
                    .write_all(&bytes)
                    .with_context(|| format!("writing {inside} to the archive"))?;
                self.done += 1;
                (self.progress)(self.done, self.total);
            }
        }
        Ok(())
    }
}

/// Replaces the target's `Compatibility` folder with the staged one.
///
/// The old folder is removed first rather than written over: the packs do not
/// all carry the same set of files, and leaving a stray emulator from a
/// previous set behind is exactly how a console ends up loading the wrong one.
///
/// `started_writing` is raised the moment the partition stops being what it
/// was — that is, just before the old folder is removed. From then on an
/// interrupted run, cancellation included, leaves the console unable to launch
/// an original Xbox game until the install is run again, and the GUI has to say
/// so rather than report a plain "cancelled".
///
/// `fs` must be open for writing, and must be the only session doing so.
pub fn install_remote(
    fs: &mut dyn RemoteFs,
    staged: &Path,
    cancel: &AtomicBool,
    started_writing: &AtomicBool,
    delete_progress: &mut dyn FnMut(u64, u64),
    progress: &mut dyn FnMut(u64, u64, Option<f64>),
) -> Result<()> {
    ensure_compat_volume(fs)?;
    check_cancel(cancel)?;

    // Listed the strict way: an unreadable folder must not be taken for one
    // that has never held the emulator, which would leave a previous set's
    // stray files under the one about to be written.
    let existing = list_dir(fs, COMPAT_PATH).context("listing the compatibility files")?;

    check_cancel(cancel)?;
    started_writing.store(true, std::sync::atomic::Ordering::Relaxed);

    // A partition that has never held the emulator has nothing to remove.
    if !existing.is_empty() {
        let res = fs.remove_dir_recursive(COMPAT_PATH, cancel, delete_progress);
        // Checked before any context is added: the shared helpers raise their
        // own markers (a conversion's, a deletion's), and the GUI matches on
        // `to_string()`, which returns the outermost context rather than the
        // chain. Re-raising here makes this module's marker the top-level one.
        check_cancel(cancel)?;
        res.context("removing the previous compatibility files")?;
    }

    check_cancel(cancel)?;
    let res = fs.upload_dir(staged, COMPAT_PATH, cancel, progress);
    check_cancel(cancel)?;
    res.context("writing the compatibility files")
}
