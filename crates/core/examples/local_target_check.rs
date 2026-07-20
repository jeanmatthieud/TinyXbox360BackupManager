// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification of disc/DLC/title-update inspection against a local
//! mount (USB drive or any folder using the GOD layout), read-only:
//! `cargo run -p txbm-core --example local_target_check -- <mount path>`

use txbm_core::config::{Config, TargetKind};
use txbm_core::target::Target;

fn main() -> anyhow::Result<()> {
    let mount = std::env::args()
        .nth(1)
        .expect("usage: local_target_check <mount path>");

    let mut config = Config::load();
    config.contents.target_kind = TargetKind::Local;
    config.contents.mount_point = mount.into();

    let target = Target::from_config(&config.contents).unwrap();
    let games = txbm_core::game::scan_drive(&config.contents.mount_point);

    for game in games.iter().filter(|g| g.is_x360 && !g.id.is_empty()) {
        let details = target.game_details(game)?;
        println!("[{}] {}", game.id, game.title);
        for disc in &details.discs {
            println!(
                "  disc {} — {} — {} bytes",
                disc.media_id, disc.description, disc.size
            );
        }
        for dlc in &details.dlc {
            println!("  dlc — {} bytes", dlc.size);
        }

        let installed = target.installed_title_updates(game)?;
        println!("  installed title updates: {installed:?}");

        let cached = target.cached_title_update_hashes(&game.id)?;
        println!("  Aurora-cached hashes: {cached:?}");
    }

    Ok(())
}
