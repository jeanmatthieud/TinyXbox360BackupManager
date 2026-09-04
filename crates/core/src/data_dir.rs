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

/// Where a conversion is staged before being copied to a console.
pub static STAGING_DIR: LazyLock<PathBuf> = LazyLock::new(|| DATA_DIR.join("staging"));

/// Scratch space for a running job: archive extraction, the BadAvatar key
/// builder, the copies of Aurora's databases.
pub static TMP_DIR: LazyLock<PathBuf> = LazyLock::new(|| DATA_DIR.join("tmp"));

/// Empties the scratch folders and reports how many bytes that freed.
///
/// Nothing under them is meant to outlive the job that wrote it — every job
/// deletes its own working directory on the way out, on failure as much as on
/// success. But a process that is killed, crashes, or loses power never gets
/// there, and what it was extracting stays behind for good: an interrupted
/// import of a single DVD9 game strands 8 GB that nothing ever comes back for.
///
/// So anything found here at startup is by definition debris. Call this **once,
/// before any job can start** — a job of this run would be writing in there.
pub fn sweep_work_dirs() -> u64 {
    sweep_dirs(&[&STAGING_DIR, &TMP_DIR])
}

fn sweep_dirs(dirs: &[&PathBuf]) -> u64 {
    let mut reclaimed = 0;
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        // Measured before the removal, and only counted if it actually went:
        // a folder we failed to delete has freed nothing.
        let size = crate::util::dir_size(dir);
        if std::fs::remove_dir_all(dir).is_ok() {
            reclaimed += size;
        }
    }
    reclaimed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweeping_removes_the_debris_and_counts_what_it_freed() {
        let root = std::env::temp_dir().join(format!("txbm-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let staging = root.join("staging");
        let tmp = root.join("tmp");
        std::fs::create_dir_all(staging.join("Halo 3/00007000")).unwrap();
        std::fs::create_dir_all(tmp.join("archive/Fallout")).unwrap();
        std::fs::write(staging.join("Halo 3/00007000/data"), vec![0u8; 2048]).unwrap();
        std::fs::write(tmp.join("archive/Fallout/game.iso"), vec![0u8; 1024]).unwrap();

        assert_eq!(sweep_dirs(&[&staging, &tmp]), 3072);
        assert!(!staging.exists());
        assert!(!tmp.exists());

        // Nothing left to sweep, and no complaint about the folders being gone:
        // a first run has no scratch folders at all.
        assert_eq!(sweep_dirs(&[&staging, &tmp]), 0);

        let _ = std::fs::remove_dir_all(&root);
    }
}
