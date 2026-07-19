// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification of the BundledContent ISO path (e.g. a "bonus disc"
//! bundling DLC under an installer's own placeholder TitleID):
//! `cargo run -p txbm-core --example bundled_content_check -- <iso path>`

use txbm_core::config::{Config, TargetKind};

fn main() -> anyhow::Result<()> {
    let iso = std::env::args()
        .nth(1)
        .expect("usage: bundled_content_check <iso path>");

    let info = txbm_core::iso_info::inspect(iso.as_ref())?;
    println!("detected kind: {:?} ({})", info.kind, info.kind.label());

    let root = std::env::temp_dir().join("txbm-bundled-content-check");
    let _ = std::fs::remove_dir_all(&root);

    let mut config = Config::load();
    config.contents.target_kind = TargetKind::Local;
    config.contents.mount_point = root.clone();
    config.contents.remove_sources_games = false;

    let cancel = std::sync::atomic::AtomicBool::new(false);
    txbm_core::convert::perform(iso.into(), &config, &cancel, &|p, _| println!("  {p}%"))?;

    let games = txbm_core::game::scan_drive(&root);
    println!("games found after install:");
    for game in &games {
        println!(
            "  [{}] {} ({}, {} bytes) @ {}",
            game.id,
            game.title,
            game.format.label(),
            game.size,
            game.path.display()
        );
    }

    let content_dir = root.join("Content/0000000000000000");
    println!("content dir tree:");
    print_tree(&content_dir, 1);

    Ok(())
}

fn print_tree(dir: &std::path::Path, depth: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let indent = "  ".repeat(depth);
        if path.is_dir() {
            println!("{indent}{}/", entry.file_name().to_string_lossy());
            print_tree(&path, depth + 1);
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            println!("{indent}{} ({size} bytes)", entry.file_name().to_string_lossy());
        }
    }
}
