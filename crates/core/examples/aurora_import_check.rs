// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification of Aurora's import folder (real downloads, synthetic
//! Aurora on a local folder):
//! `cargo run -p txbm-core --example aurora_import_check [-- <dest dir>]`
//!
//! Prepares four titles — a retail game, an Arcade one, an original Xbox one
//! and a TitleID nothing knows — then runs again to check that folders already
//! there are left alone.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use txbm_core::aurora_import::{ImportReport, ImportTitle};
use txbm_core::config::CoverSource;
use txbm_core::target::Target;

fn title(title_id: &str, title: &str, is_x360: bool) -> ImportTitle {
    ImportTitle {
        title_id: title_id.to_string(),
        title: title.to_string(),
        is_x360,
    }
}

fn run(target: &Target, titles: &[ImportTitle]) -> anyhow::Result<ImportReport> {
    let cancel = AtomicBool::new(false);
    target.prepare_aurora_import(titles, CoverSource::default(), "fr-fr", &cancel, &mut |done, total, name| {
        println!("  [{}/{total}] {name}", done + 1);
    })
}

fn print_tree(import_dir: &Path) -> anyhow::Result<()> {
    let mut dirs: Vec<_> = std::fs::read_dir(import_dir)?.flatten().collect();
    dirs.sort_by_key(|e| e.file_name());
    for dir in dirs {
        println!("{}/", dir.file_name().to_string_lossy());
        let mut files: Vec<_> = std::fs::read_dir(dir.path())?.flatten().collect();
        files.sort_by_key(|e| e.file_name());
        for file in files {
            let name = file.file_name().to_string_lossy().to_string();
            let size = file.metadata()?.len();
            if name.ends_with(".txt") {
                let text = std::fs::read_to_string(file.path())?;
                let short: String = text.chars().take(70).collect();
                println!("  {name:<16} {short}");
            } else {
                println!("  {name:<16} {size} bytes");
            }
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let dest = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("txbm-aurora-import-check"));
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }

    let target = Target::Local(dest.clone());
    let titles = [
        title("54540861", "BioShock 2", true),
        // XboxUnity's cover is 897x600: it must come out resized.
        title("545407E7", "Borderlands", true),
        title("58410A5A", "Super Meat Boy", true),
        title("4D530004", "Halo: Combat Evolved", false),
        title("FFFF0861", "Nothing knows this one", true),
        // A second disc of the first game: same folder, prepared once.
        title("54540861", "BioShock 2 (disc 2)", true),
    ];

    println!("without Aurora:");
    std::fs::create_dir_all(&dest)?;
    assert!(!target.may_have_aurora());
    let err = run(&target, &titles).expect_err("no Aurora on the drive yet");
    println!("  refused: {err:#}");

    // What `local_aurora_dir` looks for.
    std::fs::create_dir_all(dest.join("Aurora"))?;
    std::fs::write(dest.join("Aurora").join("Aurora.xex"), b"")?;
    assert!(target.may_have_aurora());

    println!("first run:");
    let report = run(&target, &titles)?;
    println!("  {report:?}");
    print_tree(&dest.join("Aurora").join("User").join("Import"))?;
    assert_eq!(report.prepared + report.unavailable + report.failed, 5);

    println!("second run:");
    let again = run(&target, &titles)?;
    println!("  {again:?}");
    assert_eq!(again.prepared + again.unavailable, 0, "existing folders are left alone");

    println!("ok — left in {}", dest.display());
    Ok(())
}
