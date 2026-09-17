// SPDX-License-Identifier: GPL-3.0-only

use directories::ProjectDirs;
use std::path::{Path, PathBuf};
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

/// Name given to a scratch folder that has been moved aside and is waiting to
/// be deleted. A run that dies before finishing the deletion leaves one behind;
/// the next run picks it up by this prefix.
const TRASH_PREFIX: &str = "trash-";

/// Moves the scratch folders aside and returns what is now waiting to be
/// deleted (including anything a previous run left half-deleted).
///
/// Nothing under them is meant to outlive the job that wrote it — every job
/// deletes its own working directory on the way out, on failure as much as on
/// success. But a process that is killed, crashes, or loses power never gets
/// there, and what it was extracting stays behind for good: an interrupted
/// import of a single DVD9 game strands 8 GB that nothing ever comes back for.
///
/// So anything found here at startup is by definition debris. The deletion of
/// tens of thousands of extracted files takes far too long to hold the window
/// back, but it cannot run alongside this run's own writes either — the very
/// first scan already stages Aurora's databases under [`TMP_DIR`]. Hence the
/// two steps: this one is a rename, instant and safe to call **once, before
/// anything else**, and it leaves the paths for [`sweep_retired`] to delete on
/// a thread, far away from the folders this run uses.
pub fn retire_work_dirs() -> Vec<PathBuf> {
    retire_dirs(&DATA_DIR, &[&STAGING_DIR, &TMP_DIR])
}

fn retire_dirs(parent: &Path, dirs: &[&PathBuf]) -> Vec<PathBuf> {
    let mut retired = leftover_trash(parent);
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let name = format!("{TRASH_PREFIX}{}-{}", std::process::id(), retired.len());
        let dest = parent.join(name);
        // A rename inside the same folder is atomic and costs nothing whatever
        // the folder holds. Should it fail all the same (Windows refuses to
        // rename a directory something else has open), the path is handed over
        // as it is: deleting it in place is still better than keeping debris
        // for good, and this run has not written in it yet.
        retired.push(if std::fs::rename(dir, &dest).is_ok() {
            dest
        } else {
            (*dir).clone()
        });
    }
    retired
}

/// Folders a previous run moved aside but did not get to delete.
fn leftover_trash(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(TRASH_PREFIX))
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

/// Deletes what [`retire_work_dirs`] set aside and reports how many bytes that
/// freed. Slow by nature, and meant to run on a thread.
pub fn sweep_retired(dirs: Vec<PathBuf>) -> u64 {
    let mut reclaimed = 0;
    for dir in dirs {
        // Measured before the removal, and only counted if it actually went:
        // a folder we failed to delete has freed nothing.
        let size = crate::util::dir_size(&dir);
        if std::fs::remove_dir_all(&dir).is_ok() {
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

        // Retiring is what frees the names: both folders are gone from under
        // the app the moment it returns, before a byte has been deleted.
        let retired = retire_dirs(&root, &[&staging, &tmp]);
        assert_eq!(retired.len(), 2);
        assert!(!staging.exists());
        assert!(!tmp.exists());

        assert_eq!(sweep_retired(retired), 3072);

        // Nothing left to retire, and no complaint about the folders being
        // gone: a first run has no scratch folders at all.
        assert!(retire_dirs(&root, &[&staging, &tmp]).is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_a_previous_run_did_not_finish_deleting_is_picked_up() {
        let root = std::env::temp_dir().join(format!("txbm-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let leftover = root.join(format!("{TRASH_PREFIX}9999-0"));
        std::fs::create_dir_all(&leftover).unwrap();
        std::fs::write(leftover.join("game.iso"), vec![0u8; 512]).unwrap();

        let retired = retire_dirs(&root, &[]);
        assert_eq!(retired, vec![leftover]);
        assert_eq!(sweep_retired(retired), 512);

        let _ = std::fs::remove_dir_all(&root);
    }
}
