// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification that game size includes DLC but not title updates:
//! `cargo run -p txbm-core --example size_check -- <host:port>`

use std::sync::atomic::AtomicBool;
use txbm_core::config::{Config, TargetKind};
use txbm_core::target::Target;

static NO_CANCEL: AtomicBool = AtomicBool::new(false);

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let hostport = args.next().expect("usage: size_check <host:port>");
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
        let disc_size: u64 = details.discs.iter().map(|d| d.size).sum();
        let dlc_size: u64 = details.dlc.iter().map(|d| d.size).sum();
        println!(
            "[{}] {} — game.size={} bytes (discs={} + dlc={} = {})",
            game.id,
            game.title,
            game.size,
            disc_size,
            dlc_size,
            disc_size + dlc_size
        );
    }

    Ok(())
}
