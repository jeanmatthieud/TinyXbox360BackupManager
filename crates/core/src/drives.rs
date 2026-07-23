// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

//! Enumeration of drives the user may pick as a local library target.
//!
//! A drive qualifies when the OS flags it as removable, or when it is simply
//! not the disk holding the operating system. External USB hard drives often
//! report as non-removable (notably on Linux, where `/sys/block/.../removable`
//! is 0 for them), so relying on the removable flag alone would hide the most
//! common case. Pseudo/system filesystems are filtered out to keep the list
//! clean. The debug "browse any folder" path (long-press) bypasses all of this.

use std::path::{Path, PathBuf};
use sysinfo::{Disk, Disks};
use which_fs::FsKind;

/// A drive offered in the target-selection modal.
#[derive(Debug, Clone)]
pub struct RemovableDrive {
    /// Volume label / mount name, for display.
    pub name: String,
    /// Mount root, used as the library location.
    pub mount_point: PathBuf,
    pub total_bytes: u64,
    pub available_bytes: u64,
    /// Whether the OS flags this drive as removable (drives the shown icon).
    pub is_removable: bool,
    /// Human-readable filesystem name ("FAT32", "exFAT", "NTFS", …).
    pub fs_label: String,
    /// Whether the filesystem is FAT32, the only one the console reads. Non-FAT32
    /// drives are still listed, but disabled in the UI.
    pub is_fat32: bool,
}

/// Filesystems that are never a game library and should never be offered.
const PSEUDO_FS: &[&str] = &[
    "tmpfs", "devtmpfs", "squashfs", "overlay", "overlayfs", "proc", "sysfs", "cgroup", "cgroup2",
    "efivarfs", "ramfs", "autofs", "mqueue", "debugfs", "tracefs", "fusectl", "configfs",
];

/// Linux mount points that hold system data or transient mounts rather than
/// user storage. `/tmp` catches AppImage/gvfs FUSE mounts; `/recovery` the
/// distro recovery partition.
const SYSTEM_PREFIXES: &[&str] = &[
    "/boot", "/snap", "/var", "/proc", "/sys", "/run", "/dev", "/tmp", "/recovery",
];

/// Lists drives the user may pick as a local target: removable media plus any
/// non-system disk. The result is de-duplicated and sorted by mount point for a
/// stable display order.
pub fn list_removable_drives() -> Vec<RemovableDrive> {
    let disks = Disks::new_with_refreshed_list();
    let system_mount = system_disk_mount(&disks);

    let mut drives: Vec<RemovableDrive> = disks
        .list()
        .iter()
        .filter(|d| is_selectable(d, system_mount.as_deref()))
        .map(|d| {
            let mount_point = d.mount_point().to_path_buf();
            // Read the actual filesystem from the mount rather than trusting the
            // OS-reported name (e.g. Linux reports "vfat" for any FAT variant).
            let fs_kind = FsKind::try_from_path(&mount_point).unwrap_or(FsKind::Unknown);
            // Fall back to the OS-reported name (e.g. "ntfs3") when the magic
            // isn't recognised, so the label still tells the user what it is.
            let fs_label = if matches!(fs_kind, FsKind::Unknown) {
                let raw = d.file_system().to_string_lossy();
                if raw.trim().is_empty() {
                    "Unknown".to_string()
                } else {
                    raw.to_string()
                }
            } else {
                fs_kind.to_string()
            };
            RemovableDrive {
                name: drive_name(d),
                mount_point,
                total_bytes: d.total_space(),
                available_bytes: d.available_space(),
                is_removable: d.is_removable(),
                fs_label,
                is_fat32: matches!(fs_kind, FsKind::Fat32),
            }
        })
        .collect();

    // FAT32 (usable) drives first, incompatible ones at the bottom; stable and
    // human-friendly order by mount point within each group.
    drives.sort_by(|a, b| (!a.is_fat32, &a.mount_point).cmp(&(!b.is_fat32, &b.mount_point)));
    drives.dedup_by(|a, b| a.mount_point == b.mount_point);
    drives
}

/// The mount point of the disk holding the OS/user profile, found by anchoring
/// on the home directory: the disk whose mount point is the longest prefix of
/// it. Used to keep the internal system disk out of the list.
fn system_disk_mount(disks: &Disks) -> Option<PathBuf> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    disks
        .list()
        .iter()
        .map(|d| d.mount_point().to_path_buf())
        .filter(|m| home.starts_with(m))
        .max_by_key(|m| m.as_os_str().len())
}

fn is_selectable(disk: &Disk, system_mount: Option<&Path>) -> bool {
    let fs = disk.file_system().to_string_lossy();
    if PSEUDO_FS.iter().any(|p| fs.eq_ignore_ascii_case(p)) {
        return false;
    }
    // Named FUSE mounts ("fuse.AppImage", "fuse.gvfsd", "fuse.sshfs", …) are
    // never local storage. Real disks over FUSE report "fuseblk", kept here.
    if fs.starts_with("fuse.") {
        return false;
    }

    let mount = disk.mount_point();
    let mount_str = mount.to_string_lossy();

    // A removable device is always offered, wherever it is mounted. This must
    // come before the system-prefix filter below: udisks2 mounts removable
    // media under `/run/media/<user>/LABEL`, so filtering `/run` first would
    // hide the most common case. We'd rather list a spurious drive than miss a
    // real USB stick.
    if disk.is_removable() {
        return true;
    }

    // The Unix root filesystem always holds the OS.
    if mount == Path::new("/") {
        return false;
    }
    if SYSTEM_PREFIXES.iter().any(|p| mount_str.starts_with(p)) {
        return false;
    }

    // Non-removable: accept only when it isn't the system disk.
    Some(mount) != system_mount
}

/// Best-effort human-friendly name: on Linux/macOS the mount's last path
/// component is usually the volume label (`/media/user/LABEL`, `/Volumes/LABEL`);
/// Windows drive roots (`E:\`) have none, so we fall back to the device name.
fn drive_name(disk: &Disk) -> String {
    let mount = disk.mount_point();
    if let Some(last) = mount.file_name() {
        let last = last.to_string_lossy();
        if !last.trim().is_empty() {
            return last.to_string();
        }
    }
    let name = disk.name().to_string_lossy();
    if !name.trim().is_empty() {
        return name.trim().to_string();
    }
    mount.to_string_lossy().to_string()
}
