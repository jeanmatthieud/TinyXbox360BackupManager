// SPDX-License-Identifier: GPL-3.0-only

//! Manual check: scans a drive folder and prints what the game list looks
//! like, including how DLC folders were merged into extracted games.
//!
//! Usage: `cargo run --example scan_merge_check -- /path/to/drive`

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: scan_merge_check <drive dir>");
        std::process::exit(2);
    };
    let target = txbm_core::target::Target::Local(std::path::PathBuf::from(&path));
    for game in txbm_core::game::scan_drive(std::path::Path::new(&path)) {
        println!(
            "{:<24} id={:<8} fmt={:<5} incomplete={:<5} size={:>10} content_dir={:?}",
            game.title,
            game.id,
            game.format.label(),
            game.incomplete,
            game.size,
            game.content_dir,
        );
        let details = target.game_details(&game).unwrap_or_default();
        for disc in &details.discs {
            println!("    disc {} ({} bytes)", disc.description, disc.size);
        }
        for dlc in &details.dlc {
            println!(
                "    dlc  {} ({} bytes, readable={})",
                dlc.file_name, dlc.size, dlc.readable
            );
        }
    }
}
