// SPDX-License-Identifier: GPL-3.0-only

//! Background work for the original-Xbox compatibility Toolbox tool.
//!
//! The tool runs with no target connected — that is what makes it safe to
//! write outside the job queue: nothing else can be writing to a console at
//! the same time. It therefore opens its own session, over FTP or on a raw
//! FATX device, and closes it before handing back.

use crate::state::CompatTarget;
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use txbm_core::{
    fatx::{COMPAT_PARTITION, FatxConfig, FatxSession},
    ftp::FtpSession,
    ogxbox_compat::{self, CompatPartition, OgXboxCompatConfig},
    remote_fs::RemoteSession,
};

/// Opens a session on the picked console's compatibility partition.
///
/// `writable` is honoured only by the FATX backend, where a read-only session
/// opens the device without write access at all — a bug then cannot damage the
/// user's drive. FTP has no such notion.
pub fn open_session(target: &CompatTarget, writable: bool) -> Result<RemoteSession> {
    match target {
        CompatTarget::Fatx(device) => {
            let config = FatxConfig::for_partition(device.clone(), COMPAT_PARTITION);
            Ok(RemoteSession::Fatx(FatxSession::open(&config, writable)?))
        }
        CompatTarget::Ftp(config) => Ok(RemoteSession::Ftp(
            FtpSession::connect(config).with_context(|| {
                format!("connecting to {}:{}", config.host, config.port)
            })?,
        )),
    }
}

/// Reads the picked console's compatibility partition: whether it exists, holds
/// files, and — on a hard drive plugged into this computer — whether its
/// filesystem is sound. Read-only throughout.
pub fn inspect(target: &CompatTarget) -> Result<CompatPartition> {
    match target {
        CompatTarget::Fatx(device) => ogxbox_compat::inspect_fatx(device),
        CompatTarget::Ftp(_) => {
            let mut session = open_session(target, false)?;
            let found = ogxbox_compat::inspect_remote(&mut session);
            // Report the inspection's own failure in preference to the
            // close's: the former is what the user needs to read.
            let closed = session.quit();
            let found = found?;
            closed?;
            Ok(found)
        }
    }
}

/// Why an install cannot go ahead on a partition in this state, if it cannot.
/// Said before the backup location is asked for, so the user is never made to
/// pick a file for an install that was never going to run.
pub fn install_blocker(target: &CompatTarget, found: &CompatPartition) -> Option<String> {
    match found {
        CompatPartition::Missing => Some(match target {
            CompatTarget::Fatx(device) => format!(
                "{} has no original-Xbox compatibility partition. Create it first (expert mode).",
                txbm_core::util::display_path(device)
            ),
            CompatTarget::Ftp(_) => "This console has no original-Xbox compatibility \
                partition. Create it first, with its hard drive plugged into this computer \
                (expert mode)."
                .to_string(),
        }),
        CompatPartition::Present {
            damage: Some(why), ..
        } => Some(format!(
            "The compatibility partition is damaged ({why}). Format it first (expert mode)."
        )),
        CompatPartition::Present { .. } => None,
    }
}

/// The warning the install's confirmation modal shows, empty when there is
/// nothing to warn about.
pub fn install_summary(found: &CompatPartition) -> &'static str {
    match found {
        CompatPartition::Present {
            has_files: true, ..
        } => "The existing compatibility files will be permanently deleted.",
        _ => "",
    }
}

/// Whether the files of a partition can be backed up: it holds some, and they
/// can be read.
pub fn has_backable_files(found: &CompatPartition) -> bool {
    matches!(
        found,
        CompatPartition::Present {
            has_files: true,
            readable: true,
            ..
        }
    )
}

/// Whether a failed backup must stop the format. Not on a damaged partition:
/// the backup is then a best effort, since a partition that cannot be walked
/// to the end is exactly the one the format is for.
pub fn backup_is_best_effort(found: &CompatPartition) -> bool {
    matches!(
        found,
        CompatPartition::Present {
            damage: Some(_),
            ..
        }
    )
}

/// The warning the format's confirmation modal shows: empty for a partition
/// about to be created, where there is nothing to lose.
pub fn format_summary(found: &CompatPartition) -> String {
    match found {
        CompatPartition::Missing => String::new(),
        CompatPartition::Present {
            damage,
            has_files,
            readable,
        } => {
            let lost = match has_files {
                true => "Everything on it, the compatibility files included, will be erased.",
                false => "Everything on it will be erased.",
            };
            match damage {
                Some(why) if *readable && *has_files => format!(
                    "The partition is damaged ({why}). {lost} The backup is a best effort: if it \
                     fails, the format goes on."
                ),
                Some(why) => format!("The partition is damaged ({why}). {lost}"),
                None => lost.to_string(),
            }
        }
    }
}

/// Saves the target's compatibility files to `dest` over a read-only session,
/// and reports how many files were saved.
fn backup(
    session: &mut RemoteSession,
    dest: &Path,
    cancel: &AtomicBool,
    status: &dyn Fn(&str),
) -> Result<u64> {
    status("Backing up the current compatibility files…");
    let result = ogxbox_compat::backup_remote(session, dest, cancel, &mut |done, total| {
        if total > 0 {
            status(&format!("Backing up…  {done}/{total} files"));
        }
    });
    // Same rule as in `ogxbox_compat`: the cancellation marker has to stay the
    // top-level message for the GUI to recognise it.
    ogxbox_compat::check_cancel(cancel)?;
    result.context("backing up the current compatibility files")
}

/// What a successful install did beyond succeeding, for the toasts at the end.
#[derive(Default)]
pub struct InstallOutcome {
    /// A backup was asked for, but the partition held no file to save — so
    /// nothing was written at the path the user picked. Worth saying out loud:
    /// they chose a destination and would otherwise only find out by looking.
    pub backup_was_empty: bool,
}

/// Downloads the chosen pack and writes it to the picked console, backing the
/// existing files up to a zip first when `backup_zip` is set. `restore_zip`
/// puts a backup back instead of installing a published pack — and puts back
/// exactly that, nothing else: the community per-title configs are only
/// fetched when a published pack is being installed.
///
/// `started_writing` is raised once the partition has started changing; the
/// caller reads it to tell a harmless interruption from one that leaves the
/// console without a usable emulator.
pub fn install(
    target: &CompatTarget,
    cfg: &OgXboxCompatConfig,
    restore_zip: Option<&Path>,
    backup_zip: Option<&Path>,
    cancel: &AtomicBool,
    started_writing: &AtomicBool,
    status: &dyn Fn(&str),
) -> Result<InstallOutcome> {
    let mut outcome = InstallOutcome::default();
    // Prepared before anything is touched on the console: a download — or an
    // unreadable backup — must leave the existing files in place.
    let staged = match restore_zip {
        Some(zip) => ogxbox_compat::stage_backup(zip, cancel, status)?,
        None => ogxbox_compat::stage_pack(cfg, cancel, status)?,
    };
    // Same reason, and staged second: `stage_pack` empties the working
    // directory the two of them share.
    //
    // Never on a restore: putting a backup back means putting back what was
    // saved, and nothing else. Mixing today's configs into it would leave the
    // user with a partition that is not the one they saved.
    //
    // Asked for, the configs are part of the install, not an extra: titles the
    // emulator only starts with their config do not run without it. So they
    // must download before anything is touched, and their failure fails the
    // install like the pack's would.
    let staged_configs = match cfg.update_configs && restore_zip.is_none() {
        true => Some(ogxbox_compat::stage_configs(cfg, cancel, status)?),
        false => None,
    };

    // One read-only session for both reads, closed before the writable one
    // opens — two writable FATX sessions on one device would each rearrange a
    // FAT the other holds a stale copy of.
    //
    // The room is checked first, before the backup and the delete: a pack that
    // cannot fit must leave the current files exactly where they are. A FATX
    // session answers exactly; over FTP the check writes a scratch file, which
    // a session has no way to refuse anyway.
    status("Checking the room on the console…");
    let mut session = open_session(target, false)?;
    let mut staged_dirs = vec![staged.as_path()];
    staged_dirs.extend(staged_configs.as_deref());
    let checked = ogxbox_compat::check_room(&mut session, &staged_dirs, cancel, status);
    let saved = match (&checked, backup_zip) {
        (Ok(()), Some(dest)) => Some(backup(&mut session, dest, cancel, status)),
        _ => None,
    };
    let closed = session.quit();
    // A room check stopped mid-file may have left part of its scratch file,
    // and its session is unusable: cleaned up over a fresh one.
    if checked.as_ref().is_err_and(is_cancelled)
        && let CompatTarget::Ftp(_) = target
        && let Ok(mut session) = open_session(target, false)
    {
        ogxbox_compat::remove_room_probe(&mut session);
        let _ = session.quit();
    }
    checked?;
    if let Some(saved) = saved {
        outcome.backup_was_empty = saved? == 0;
    }
    closed?;

    status("Opening the compatibility partition…");
    let mut session = open_session(target, true)?;
    let result = ogxbox_compat::install_remote(
        &mut session,
        &staged,
        cancel,
        started_writing,
        &mut |done, total| {
            if total > 0 {
                status(&format!("Removing the previous files…  {done}/{total}"));
            }
        },
        &mut |sent, total| {
            if let Some(percent) = (sent * 100).checked_div(total) {
                status(&format!("Writing the compatibility files…  {percent}%"));
            }
        },
    );

    // Written into the folder the install has just laid down, on the same
    // session: a second writable one on a FATX device would rearrange a FAT
    // this one still holds in memory. A failure here is the install's own,
    // reported as a partition left incomplete: the configs were asked for.
    let result = match (result, staged_configs.as_deref()) {
        (Ok(()), Some(configs)) => {
            status("Writing the compatibility configs…");
            ogxbox_compat::install_configs_remote(&mut session, configs, cancel, &mut |sent, total| {
                if let Some(percent) = (sent * 100).checked_div(total) {
                    status(&format!("Writing the compatibility configs…  {percent}%"));
                }
            })
        }
        (result, _) => result,
    };

    // Whether the write finished or was interrupted, what did reach the disk
    // must be described correctly on it — so the session is always closed.
    let closed = session.quit();
    result?;
    closed?;

    // Best-effort cleanup. `stage_pack` recreates this directory from scratch
    // on every run, so leaving it behind on an error path costs nothing.
    let _ = std::fs::remove_dir_all(txbm_core::data_dir::TMP_DIR.join(ogxbox_compat::WORK_SUBDIR));

    status("");
    Ok(outcome)
}

/// What a successful format did, for the toasts at the end.
#[derive(Default)]
pub struct FormatOutcome {
    /// The partition did not exist and was created, rather than formatted.
    pub created: bool,
    /// Same as [`InstallOutcome::backup_was_empty`].
    pub backup_was_empty: bool,
    /// The backup was a best effort, failed, and the format went on without
    /// it — with the reason.
    pub backup_failed: Option<String>,
}

/// Creates or formats the compatibility partition of a hard drive plugged into
/// this computer, backing the existing files up to a zip first when
/// `backup_zip` is set. With `backup_best_effort`, a backup that fails does
/// not stop the format: see [`backup_is_best_effort`].
///
/// `started_writing` has the same meaning as for [`install`]: raised once the
/// partition has started changing, so an interruption from then on is
/// reported as one that left it altered.
pub fn format(
    device: &Path,
    backup_zip: Option<&Path>,
    backup_best_effort: bool,
    cancel: &AtomicBool,
    started_writing: &AtomicBool,
    status: &dyn Fn(&str),
) -> Result<FormatOutcome> {
    let mut outcome = FormatOutcome::default();
    let target = CompatTarget::Fatx(device.to_path_buf());

    if let Some(dest) = backup_zip {
        let mut session = open_session(&target, false)?;
        let saved = backup(&mut session, dest, cancel, status);
        let closed = session.quit();
        match saved {
            Ok(saved) => outcome.backup_was_empty = saved == 0,
            Err(e) if backup_best_effort && !is_cancelled(&e) => {
                outcome.backup_failed = Some(format!("{e:#}"));
            }
            Err(e) => return Err(e),
        }
        closed?;
    }

    ogxbox_compat::check_cancel(cancel)?;
    status("Formatting the compatibility partition…");
    started_writing.store(true, std::sync::atomic::Ordering::Relaxed);
    outcome.created = ogxbox_compat::format_fatx(device)?;

    let _ = std::fs::remove_dir_all(txbm_core::data_dir::TMP_DIR.join(ogxbox_compat::WORK_SUBDIR));
    status("");
    Ok(outcome)
}

/// Whether an error is the user's cancellation rather than a failure.
pub fn is_cancelled(error: &anyhow::Error) -> bool {
    error.to_string().contains(ogxbox_compat::COMPAT_CANCELLED)
}
