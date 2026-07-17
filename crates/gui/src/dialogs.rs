// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use rfd::FileDialog;
use slint::WindowHandle;
use std::path::PathBuf;
use walkdir::WalkDir;

const INPUT_DIALOG_FILTER: &[&str] = &["iso"];

pub fn pick_mount_point(window_handle: &WindowHandle) -> Option<PathBuf> {
    FileDialog::new()
        .set_parent(window_handle)
        .set_title("Select Drive/Mount Point")
        .pick_folder()
}

pub fn pick_games(window_handle: &WindowHandle) -> Vec<PathBuf> {
    FileDialog::new()
        .set_parent(window_handle)
        .set_title("Select Games")
        .add_filter("Xbox Optical Disc Image", INPUT_DIALOG_FILTER)
        .pick_files()
        .unwrap_or_default()
}

pub fn pick_games_r(window_handle: &WindowHandle) -> Vec<PathBuf> {
    let res = FileDialog::new()
        .set_parent(window_handle)
        .set_title("Select folder (games will be searched recursively)")
        .pick_folder();

    let mut paths = Vec::new();

    let Some(res) = res else {
        return paths;
    };

    for entry in WalkDir::new(res).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file()
            && let Some(ext) = entry.path().extension()
            && INPUT_DIALOG_FILTER
                .iter()
                .any(|e| ext.eq_ignore_ascii_case(e))
        {
            paths.push(entry.into_path());
        }
    }

    paths
}
