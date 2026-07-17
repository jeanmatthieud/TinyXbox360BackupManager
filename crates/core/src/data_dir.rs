// SPDX-License-Identifier: GPL-3.0-only

use directories::ProjectDirs;
use std::path::PathBuf;
use std::sync::LazyLock;

/// Application data folder (config, covers cache).
pub static DATA_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    ProjectDirs::from("net", "jeanm", "TinyXbox360BackupManager")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_default()
});

pub fn ensure_data_dir() -> std::io::Result<()> {
    std::fs::create_dir_all(&*DATA_DIR)
}
