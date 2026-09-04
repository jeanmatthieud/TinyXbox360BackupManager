// SPDX-License-Identifier: GPL-3.0-only

//! Finding the Xbox 360 hard drives plugged into this computer.
//!
//! An Xbox 360 disk carries no partition table, so the operating system never
//! mounts it and it appears nowhere among the drives the USB target lists: it
//! is only ever a raw block device. Candidates are therefore enumerated at the
//! device level and then *probed* — a disk is only offered once the FATX
//! signature has actually been read at the offset the console's user-content
//! partition sits at, which is what tells an Xbox 360 drive apart from a blank
//! disk or from someone's unrelated hardware.

use crate::fatx::{DEFAULT_PARTITION, FatxConfig};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The four signature bytes at the start of a FATX partition, as the console
/// writes them: the Xbox 360 stores every multi-byte value big-endian, so what
/// the original Xbox spells `FATX` reads `XTAF` on a 360 disk. Both are
/// recognized — a 360 drive is the point here, but reporting "this is an
/// original Xbox disk" beats reporting "not an Xbox disk at all".
const SIGNATURE_X360: &[u8; 4] = b"XTAF";
const SIGNATURE_XBOX: &[u8; 4] = b"FATX";

/// What a probe found on a candidate disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatxProbe {
    /// An Xbox 360 filesystem: usable as a target.
    Xbox360,
    /// A FATX filesystem, but the original Xbox's flavour.
    OriginalXbox,
    /// Readable, but nothing FATX at the expected offset.
    NoFilesystem,
    /// The device exists but this user may not open it.
    AccessDenied,
    /// Any other read failure (device disappeared, I/O error…).
    Unreadable,
}

impl FatxProbe {
    /// Whether this disk can be selected as a target.
    pub fn is_usable(&self) -> bool {
        matches!(self, FatxProbe::Xbox360)
    }

    /// Short explanation shown under the device's name in the picker.
    pub fn label(&self) -> &'static str {
        match self {
            FatxProbe::Xbox360 => "Xbox 360 hard drive",
            FatxProbe::OriginalXbox => "Original Xbox disk — not supported",
            FatxProbe::NoFilesystem => "no Xbox filesystem on this disk",
            FatxProbe::AccessDenied => "access denied — raw disk access needs privileges",
            FatxProbe::Unreadable => "could not be read",
        }
    }
}

/// A disk offered in the FATX target picker.
#[derive(Debug, Clone)]
pub struct FatxDrive {
    /// Raw device path (`/dev/sdb`, `\\.\PhysicalDrive1`, `/dev/rdisk3`).
    pub path: PathBuf,
    /// Model / device name, for display.
    pub name: String,
    /// Total size of the disk, 0 when it could not be read.
    pub size_bytes: u64,
    pub is_removable: bool,
    pub probe: FatxProbe,
}

/// Lists the disks that could hold an Xbox 360 filesystem, each with the
/// result of probing it. Disks that are clearly something else (too small to
/// even reach the partition, or holding a mounted filesystem — an Xbox 360
/// disk never does) are left out entirely rather than listed as unusable.
pub fn list_fatx_drives() -> Vec<FatxDrive> {
    let mut drives: Vec<FatxDrive> = candidates()
        .into_iter()
        .map(|mut drive| {
            drive.probe = probe_device(&drive.path);
            drive
        })
        .collect();
    // Usable disks first, then the ones the user may need to act on
    // (permissions), then the rest; alphabetical within each group.
    drives.sort_by_key(|d| {
        let rank = match d.probe {
            FatxProbe::Xbox360 => 0,
            FatxProbe::AccessDenied => 1,
            _ => 2,
        };
        (rank, d.path.to_string_lossy().to_string())
    });
    drives
}

/// Offset of the partition a target opens, used by the probe. Reading the
/// signature there is exactly the check the `fatx` crate performs when
/// opening, so a disk that probes as [`FatxProbe::Xbox360`] opens.
fn partition_offset() -> u64 {
    fatx::PartitionMapEntry::from_x360_name(DEFAULT_PARTITION)
        .map(|p| p.offset_bytes)
        .unwrap_or(0)
}

/// Reads the four signature bytes at the partition offset.
fn probe_device(path: &Path) -> FatxProbe {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) => {
            return match e.kind() {
                std::io::ErrorKind::PermissionDenied => FatxProbe::AccessDenied,
                _ => FatxProbe::Unreadable,
            };
        }
    };

    if file.seek(SeekFrom::Start(partition_offset())).is_err() {
        return FatxProbe::Unreadable;
    }
    let mut signature = [0u8; 4];
    match file.read_exact(&mut signature) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return FatxProbe::AccessDenied;
        }
        // A short read means the disk stops before the partition would start:
        // whatever it is, it is not an Xbox 360 drive.
        Err(_) => return FatxProbe::NoFilesystem,
    }

    if &signature == SIGNATURE_X360 {
        FatxProbe::Xbox360
    } else if &signature == SIGNATURE_XBOX {
        FatxProbe::OriginalXbox
    } else {
        FatxProbe::NoFilesystem
    }
}

/// A disk picked by hand (a raw image, or a device the enumeration missed),
/// probed the same way so the caller can report the same diagnostics.
pub fn probe_path(path: &Path) -> FatxProbe {
    probe_device(path)
}

/// Builds a target configuration for a device path.
pub fn config_for(path: &Path) -> FatxConfig {
    FatxConfig::new(path.to_path_buf())
}

#[cfg(target_os = "linux")]
fn candidates() -> Vec<FatxDrive> {
    let mounted = mounted_devices();
    let Ok(entries) = std::fs::read_dir("/sys/block") else {
        return Vec::new();
    };

    let mut drives = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Virtual and non-disk devices: loopback files, RAM disks, device
        // mapper, software RAID, optical drives, NBD.
        if ["loop", "zram", "dm-", "md", "sr", "ram", "nbd", "fd"]
            .iter()
            .any(|p| name.starts_with(p))
        {
            continue;
        }

        let sys = entry.path();
        // Size is in 512-byte sectors whatever the drive's real sector size.
        let size_bytes = read_sys(&sys.join("size"))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|sectors| sectors * 512)
            .unwrap_or(0);
        // A disk too small to even hold the start of the partition cannot be
        // an Xbox 360 drive, and opening it would only spin it up for nothing.
        if size_bytes != 0 && size_bytes <= partition_offset() {
            continue;
        }

        let path = PathBuf::from("/dev").join(&name);
        // An Xbox 360 disk has no partition table, so the OS has nothing to
        // mount off it. Anything currently mounted is the user's own system or
        // data disk, and must never be offered for raw writing.
        if mounted.iter().any(|m| m.starts_with(&name)) {
            continue;
        }

        let model = read_sys(&sys.join("device/model")).unwrap_or_default();
        let vendor = read_sys(&sys.join("device/vendor")).unwrap_or_default();
        let label = format!("{vendor} {model}").trim().to_string();

        drives.push(FatxDrive {
            path,
            name: if label.is_empty() { name.clone() } else { label },
            size_bytes,
            is_removable: read_sys(&sys.join("removable")).as_deref() == Some("1"),
            probe: FatxProbe::Unreadable,
        });
    }
    drives
}

#[cfg(target_os = "linux")]
fn read_sys(path: &Path) -> Option<String> {
    let value = std::fs::read_to_string(path).ok()?;
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// Kernel names (`sda`, `nvme0n1p2`…) of every device with something mounted
/// off it, so their whole disk can be kept out of the list.
#[cfg(target_os = "linux")]
fn mounted_devices() -> Vec<String> {
    let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") else {
        return Vec::new();
    };
    mounts
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(|dev| dev.strip_prefix("/dev/"))
        .map(|dev| dev.to_string())
        .collect()
}

/// Windows exposes whole disks as `\\.\PhysicalDriveN` with no directory to
/// enumerate, so the numbers are simply tried in turn; probing decides which
/// of them are Xbox 360 drives.
#[cfg(target_os = "windows")]
fn candidates() -> Vec<FatxDrive> {
    (0..16)
        .map(|n| {
            let path = PathBuf::from(format!("\\\\.\\PhysicalDrive{n}"));
            FatxDrive {
                name: format!("Physical drive {n}"),
                // The size of a raw disk needs an IOCTL Windows only answers on
                // an already-open handle; the picker shows nothing rather than
                // a wrong number.
                size_bytes: 0,
                is_removable: false,
                probe: FatxProbe::Unreadable,
                path,
            }
        })
        .filter(|d| !matches!(probe_device(&d.path), FatxProbe::Unreadable))
        .collect()
}

/// macOS names whole disks `/dev/diskN`, with `/dev/rdiskN` as the unbuffered
/// character device — far faster for the large sequential reads and writes a
/// game transfer is made of.
#[cfg(target_os = "macos")]
fn candidates() -> Vec<FatxDrive> {
    let Ok(entries) = std::fs::read_dir("/dev") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            // Whole disks only: `rdisk3` yes, its `rdisk3s1` slices no.
            let number = name.strip_prefix("rdisk")?;
            if !number.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            Some(FatxDrive {
                path: entry.path(),
                name: format!("Disk {number}"),
                size_bytes: 0,
                is_removable: false,
                probe: FatxProbe::Unreadable,
            })
        })
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn candidates() -> Vec<FatxDrive> {
    Vec::new()
}
