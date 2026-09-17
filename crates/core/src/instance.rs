// SPDX-License-Identifier: GPL-3.0-only

//! One running copy of the app at a time.
//!
//! Two instances are not merely redundant, they destroy each other's work.
//! The scratch folders ([`crate::data_dir::STAGING_DIR`], [`crate::data_dir::TMP_DIR`])
//! carry no owner: a second instance starts by moving them aside as debris —
//! renaming the `staging` folder the first one is 20 minutes into extracting
//! a DVD9 game into — and then deletes them on a thread. And beyond the local
//! disk, both would happily write to the same console at the same time, which
//! is the one thing the whole job queue exists to prevent (two FTP uploads at
//! once leave games the console cannot read; two FATX writers each rearrange
//! a FAT the other holds a stale copy of).
//!
//! The guard is an advisory exclusive lock on a file in the data folder, held
//! for the life of the process and released by the OS however it ends —
//! including a kill or a power loss, which is what makes it safe to leave the
//! file behind.

use crate::data_dir::DATA_DIR;
use std::fs::File;

/// Held for as long as this process is the one running instance. Dropping it
/// (or exiting, whichever comes first) releases the lock.
pub struct InstanceLock(#[allow(dead_code)] File);

pub enum InstanceGuard {
    /// This process holds the lock: it is the only instance.
    Acquired(InstanceLock),
    /// Another instance is already running.
    AlreadyRunning,
    /// The lock file could not be created or locked — a read-only data folder,
    /// a filesystem with no lock support. The caller carries on unguarded:
    /// refusing to start over an unusable lock would be worse than the race it
    /// protects against.
    Unavailable,
}

/// Takes the single-instance lock. Call once, before anything touches the
/// scratch folders.
pub fn lock() -> InstanceGuard {
    if crate::data_dir::ensure_data_dir().is_err() {
        return InstanceGuard::Unavailable;
    }
    let Ok(file) = File::create(DATA_DIR.join("instance.lock")) else {
        return InstanceGuard::Unavailable;
    };
    match file.try_lock() {
        Ok(()) => InstanceGuard::Acquired(InstanceLock(file)),
        Err(std::fs::TryLockError::WouldBlock) => InstanceGuard::AlreadyRunning,
        Err(std::fs::TryLockError::Error(_)) => InstanceGuard::Unavailable,
    }
}
