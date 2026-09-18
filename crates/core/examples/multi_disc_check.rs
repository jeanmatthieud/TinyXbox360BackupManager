// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification of a multi-disc set: converts several images **into the
//! same target, in the order given**, and reports what the library looks like
//! afterwards. Meant for the discs whose halves only work together, where the
//! result must not depend on which one is added first
//! (`crate::quirks::DiscQuirk`).
//!
//! ```text
//! cargo run --release -p txbm-core --example multi_disc_check -- [--keep] [--xex] <image>…
//! ```
//!
//! The target is wiped before the first image unless `--keep` is passed, and
//! `--xex` stores Xbox 360 games as extracted folders instead of GOD.

use txbm_core::config::{Config, TargetKind, Xbox360Format};

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let keep = args.iter().any(|a| a == "--keep");
    let xex = args.iter().any(|a| a == "--xex");
    args.retain(|a| a != "--keep" && a != "--xex");
    if args.is_empty() {
        anyhow::bail!("usage: multi_disc_check [--keep] <image>…");
    }

    let root = std::env::temp_dir().join("txbm-multi-disc-check");
    if !keep {
        let _ = std::fs::remove_dir_all(&root);
    }

    let mut config = Config::load();
    config.contents.target_kind = TargetKind::Local;
    config.contents.mount_point = root.clone();
    config.contents.remove_sources_games = false;
    config.contents.xbox360_format = if xex {
        Xbox360Format::Xex
    } else {
        Xbox360Format::God
    };

    let cancel = std::sync::atomic::AtomicBool::new(false);

    for (step, image) in args.iter().enumerate() {
        let path = std::path::PathBuf::from(image);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        println!("\n===== [{}/{}] {name}", step + 1, args.len());

        let info = txbm_core::iso_info::inspect(&path)?;
        println!("  kind:  {:?} ({})", info.kind, info.kind.label());
        println!("  quirk: {:?}", info.quirk);
        println!(
            "  id:    {} disc {:?}/{:?}",
            info.title_id.as_deref().unwrap_or("-"),
            info.disc_number,
            info.disc_count
        );

        txbm_core::convert::perform(
            path,
            &config,
            &cancel,
            &|_, _| {},
            &|_| {},
            &|s| println!("  [{s}]"),
        )?;

        println!("  --- library after this image ---");
        print_library(&root);
        println!("  --- tree ---");
        print_tree(&root, 2, 3);
    }

    Ok(())
}

fn print_library(root: &std::path::Path) {
    let games = txbm_core::game::scan_drive(root);
    if games.is_empty() {
        println!("    (no game listed)");
    }
    for game in &games {
        println!(
            "    [{}] {} ({}, {:.2} GiB){}",
            game.id,
            game.title,
            game.format.label(),
            game.size as f64 / 1073741824.0,
            if game.incomplete { " INCOMPLETE" } else { "" },
        );
    }
}

fn print_tree(dir: &std::path::Path, indent: usize, depth: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let pad = "  ".repeat(indent);
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            println!("{pad}{name}/");
            if depth > 1 {
                print_tree(&path, indent + 1, depth - 1);
            }
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            println!("{pad}{name} ({size} bytes)");
        }
    }
}
