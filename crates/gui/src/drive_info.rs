// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{DisplayedDriveInfo, util::GIB};
use slint::ToSharedString;
use txbm_core::drive_info::DriveInfo;

impl From<&DriveInfo> for DisplayedDriveInfo {
    fn from(drive_info: &DriveInfo) -> Self {
        Self {
            label: drive_info.label.to_shared_string(),
            fs_kind: drive_info.fs_kind.to_shared_string(),
            used_gib: drive_info.used_bytes as f32 / GIB,
            total_gib: drive_info.total_bytes as f32 / GIB,
            games_gib: drive_info.games_bytes as f32 / GIB,
            allocation_granularity: drive_info.allocation_granularity as i32,
        }
    }
}
