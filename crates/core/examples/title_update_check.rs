// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification of disc/DLC/title-update inspection against a real
//! console (read-only, no install/uninstall):
//! `cargo run -p txbm-core --example title_update_check -- <host:port>`

use std::sync::atomic::AtomicBool;
use txbm_core::config::{Config, TargetKind};
use txbm_core::target::Target;

static NO_CANCEL: AtomicBool = AtomicBool::new(false);

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let hostport = args.next().expect("usage: title_update_check <host:port>");
    let (host, port) = hostport.split_once(':').unwrap_or((hostport.as_str(), "21"));

    let mut config = Config::load();
    config.contents.target_kind = TargetKind::Ftp;
    config.contents.console_ip = host.to_string();
    config.contents.ftp_port = port.to_string();
    config.contents.ftp_user = std::env::var("TXBM_FTP_USER").unwrap_or_else(|_| "xboxftp".into());
    config.contents.ftp_password =
        std::env::var("TXBM_FTP_PASSWORD").unwrap_or_else(|_| "xboxftp".into());

    let target = Target::from_config(&config.contents).unwrap();
    let (games, _) = target.scan(&NO_CANCEL)?;

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

        match txbm_core::unity::title_updates(&game.id) {
            Ok(entries) => {
                for entry in &entries {
                    let is_active = entry
                        .hash
                        .as_deref()
                        .map(|h| installed.iter().any(|i| i.hash.eq_ignore_ascii_case(h)))
                        .unwrap_or(false);
                    let is_cached = entry
                        .hash
                        .as_deref()
                        .map(|h| cached.iter().any(|c| c.eq_ignore_ascii_case(h)))
                        .unwrap_or(false);
                    println!(
                        "  TU {} v{:?} hash={:?} size={:?}KiB active={is_active} cached={is_cached}",
                        entry.title_update_id, entry.version, entry.hash, entry.size
                    );
                }
            }
            Err(e) => println!("  XboxUnity title_updates ERROR: {e:#}"),
        }
    }

    Ok(())
}
