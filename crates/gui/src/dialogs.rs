// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use rfd::FileDialog;
use slint::WindowHandle;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const INPUT_DIALOG_FILTER: &[&str] = &["iso", "7z", "zip"];

pub fn pick_mount_point(window_handle: &WindowHandle) -> Option<PathBuf> {
    FileDialog::new()
        .set_parent(window_handle)
        .set_title("Select Drive/Mount Point")
        .pick_folder()
}

/// Folder picker for a storage location, opening inside `start_dir` (the
/// selected drive's root) so the user stays on the right drive.
pub fn pick_storage_folder(window_handle: &WindowHandle, start_dir: &Path) -> Option<PathBuf> {
    FileDialog::new()
        .set_parent(window_handle)
        .set_title("Select storage folder")
        .set_directory(start_dir)
        .pick_folder()
}

/// Where to write the zip holding a copy of the console's current compatibility
/// files. A save dialog rather than a folder picker: the user is choosing the
/// name of one archive, not a place to scatter files into.
pub fn save_compat_backup(window_handle: &WindowHandle) -> Option<PathBuf> {
    // No `set_directory`: the app's own data folder would be a poor place to
    // keep a backup, and the desktop already reopens where the user last saved.
    let path = FileDialog::new()
        .set_parent(window_handle)
        .set_title("Save the current compatibility files")
        .set_file_name("compatibility-backup.zip")
        .add_filter("Zip archive", &["zip"])
        .save_file()?;

    // A filter is a hint, not a rule: some desktops hand back whatever was
    // typed. The extension matters here, because restoring reads the archive
    // back by it.
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        return Some(path);
    }

    // Appended, never substituted: `with_extension` would turn a name the user
    // typed as `backup.old` into `backup.zip`, and the dialog's overwrite
    // prompt answered for `backup.old` says nothing about a `backup.zip` that
    // may already be sitting there from an earlier run.
    let mut name = path.into_os_string();
    name.push(".zip");
    Some(PathBuf::from(name))
}

/// Picker for a backup this tool wrote earlier, to put back on a console.
pub fn pick_compat_backup(window_handle: &WindowHandle) -> Option<PathBuf> {
    FileDialog::new()
        .set_parent(window_handle)
        .set_title("Select a compatibility backup")
        .add_filter("Zip archive", &["zip"])
        .pick_file()
}

/// Picker for a raw disk image holding an Xbox 360 filesystem, the hidden
/// alternative to selecting a physical drive. Handy to work on a dump of a
/// console drive without touching the drive itself.
pub fn pick_fatx_image(window_handle: &WindowHandle) -> Option<PathBuf> {
    FileDialog::new()
        .set_parent(window_handle)
        .set_title("Select an Xbox 360 disk image")
        .add_filter("Disk images", &["img", "bin", "raw", "hdd"])
        .add_filter("All files", &["*"])
        .pick_file()
}

pub fn pick_games(window_handle: &WindowHandle) -> Vec<PathBuf> {
    FileDialog::new()
        .set_parent(window_handle)
        .set_title("Select Games")
        .add_filter("Xbox games (ISO / XBLA archive)", INPUT_DIALOG_FILTER)
        // STFS packages (XBLA/DLC) usually have no extension.
        .add_filter("All files", &["*"])
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
        if !entry.file_type().is_file() {
            continue;
        }
        // Known extensions, plus extension-less files: STFS packages
        // (XBLA) usually have none — `should_add_game` sniffs their magic.
        let accepted = match entry.path().extension() {
            Some(ext) => INPUT_DIALOG_FILTER
                .iter()
                .any(|e| ext.eq_ignore_ascii_case(e)),
            None => true,
        };
        if accepted {
            paths.push(entry.into_path());
        }
    }

    paths
}
