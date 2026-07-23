// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, bail};
use std::path::Path;
use which_fs::FsKind;

#[derive(Debug, Clone, Default)]
pub struct DriveInfo {
    pub label: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub games_bytes: u64,
    pub fs_kind: FsKind,
    pub allocation_granularity: u64,
}

impl DriveInfo {
    pub fn from_path(path: &Path) -> Result<Self> {
        if !path.is_dir() {
            bail!("Not a directory");
        }

        let label_osstr = path.file_name().unwrap_or(path.as_os_str());
        let label = label_osstr.to_string_lossy().to_string();

        let stat = fs4::statvfs(path)?;
        let total_bytes = stat.total_space();
        let avail_bytes = stat.available_space();
        let used_bytes = total_bytes.saturating_sub(avail_bytes);
        let allocation_granularity = stat.allocation_granularity();

        let fs_kind = FsKind::try_from_path(path).unwrap_or(FsKind::Unknown);

        // `games_bytes` is filled by the caller from the scanned games, so it
        // reflects the target's resolved storage locations rather than the
        // hard-coded default folders.
        Ok(Self {
            label,
            used_bytes,
            total_bytes,
            games_bytes: 0,
            fs_kind,
            allocation_granularity,
        })
    }
}
