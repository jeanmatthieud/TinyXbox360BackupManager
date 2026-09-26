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
    ogxbox_compat::{self, OgXboxCompatConfig},
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

/// Checks the picked console can take the files, and reports whether it already
/// has some. Read-only throughout.
pub fn inspect(target: &CompatTarget) -> Result<bool> {
    let mut session = open_session(target, false)?;
    let present = ogxbox_compat::inspect_remote(&mut session);
    // Report the inspection's own failure in preference to the close's: the
    // former is what the user needs to read.
    let closed = session.quit();
    let present = present?;
    closed?;
    Ok(present)
}

/// The warning the confirmation modal shows, empty when there is nothing to
/// warn about.
pub fn summarize(present: bool) -> &'static str {
    if present {
        "The existing compatibility files will be permanently deleted."
    } else {
        ""
    }
}

/// What a successful install did beyond succeeding, for the toasts at the end.
#[derive(Default)]
pub struct InstallOutcome {
    /// A backup was asked for, but the partition held no file to save — so
    /// nothing was written at the path the user picked. Worth saying out loud:
    /// they chose a destination and would otherwise only find out by looking.
    pub backup_was_empty: bool,
    /// The per-title configs were asked for and did not make it, with the
    /// reason. The emulator itself is in place — these are an extra, so their
    /// failure is a warning rather than a failed install.
    pub configs_failed: Option<String>,
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
    let staged_configs = match cfg.update_configs && restore_zip.is_none() {
        true => match ogxbox_compat::stage_configs(cfg, cancel, status) {
            Ok(dir) => Some(dir),
            // The option is on by default, and the configs are an extra on top
            // of the emulator: GitHub being unreachable must not stop an
            // install whose pack already downloaded.
            Err(e) if !e.to_string().contains(ogxbox_compat::COMPAT_CANCELLED) => {
                outcome.configs_failed =
                    Some(crate::download_errors::compat(&e).unwrap_or_else(|| format!("{e:#}")));
                None
            }
            // A cancellation is the user stopping the whole thing, and nothing
            // has been touched yet — so it stays what it is.
            Err(e) => return Err(e),
        },
        false => None,
    };

    // The backup reads, so it gets a read-only session, closed before the
    // writable one opens — two writable FATX sessions on one device would each
    // rearrange a FAT the other holds a stale copy of.
    if let Some(dest) = backup_zip {
        status("Backing up the current compatibility files…");
        let mut session = open_session(target, false)?;
        let result = ogxbox_compat::backup_remote(&mut session, dest, cancel, &mut |done, total| {
            if total > 0 {
                status(&format!("Backing up…  {done}/{total} files"));
            }
        });
        let closed = session.quit();
        // Same rule as in `ogxbox_compat`: the cancellation marker has to stay
        // the top-level message for the GUI to recognise it.
        ogxbox_compat::check_cancel(cancel)?;
        let saved = result.context("backing up the current compatibility files")?;
        closed?;
        outcome.backup_was_empty = saved == 0;
    }

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
        &mut |sent, total, _speed| {
            if let Some(percent) = (sent * 100).checked_div(total) {
                status(&format!("Writing the compatibility files…  {percent}%"));
            }
        },
    );

    // Written into the folder the install has just laid down, on the same
    // session: a second writable one on a FATX device would rearrange a FAT
    // this one still holds in memory.
    //
    // Reported aside from `result` rather than into it: by this point the
    // emulator is written and the console can launch a game again, so neither
    // a broken link nor a cancellation here may be announced as a partition
    // left incomplete. What did land is fine — each config is a file of its
    // own, read only for the title whose ID names it.
    if result.is_ok()
        && let Some(configs) = staged_configs.as_deref()
    {
        status("Writing the compatibility configs…");
        if let Err(e) = ogxbox_compat::install_configs_remote(
            &mut session,
            configs,
            cancel,
            &mut |sent, total, _speed| {
                if let Some(percent) = (sent * 100).checked_div(total) {
                    status(&format!("Writing the compatibility configs…  {percent}%"));
                }
            },
        ) {
            outcome.configs_failed = Some(format!("{e:#}"));
        }
    }

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
