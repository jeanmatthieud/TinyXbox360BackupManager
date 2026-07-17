// SPDX-License-Identifier: GPL-3.0-only
//! Vérification manuelle de la cible FTP contre un serveur local :
//! `cargo run -p txbm-core --example ftp_target_check -- <host:port> <iso à ajouter>`

use txbm_core::config::{Config, TargetKind};
use txbm_core::target::Target;
use std::sync::atomic::AtomicBool;

static NO_CANCEL: AtomicBool = AtomicBool::new(false);

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let hostport = args.next().expect("usage: ftp_target_check <host:port> [iso]");
    let (host, port) = hostport.split_once(':').unwrap_or((hostport.as_str(), "21"));

    let mut config = Config::load();
    config.contents.target_kind = TargetKind::Ftp;
    config.contents.console_ip = host.to_string();
    config.contents.ftp_port = port.to_string();
    config.contents.ftp_user = std::env::var("TXBM_FTP_USER").unwrap_or_else(|_| "xboxftp".into());
    config.contents.ftp_password =
        std::env::var("TXBM_FTP_PASSWORD").unwrap_or_else(|_| "xboxftp".into());

    let target = Target::from_config(&config.contents).unwrap();
    println!("target: {}", target.display());

    {
        let mut session = txbm_core::ftp::FtpSession::connect(&config.contents.ftp_config())?;
        match txbm_core::target::aurora_paths(&mut session) {
            Ok(paths) => println!("aurora paths: {paths:?}"),
            Err(e) => println!("aurora paths ERREUR: {e:#}"),
        }
        session.quit();
    }

    let (games, info) = target.scan(&NO_CANCEL)?;
    println!("drive: {} — games: {} octets", info.label, info.games_bytes);
    for game in &games {
        println!(
            "- [{}] {} ({}, {} octets) @ {}",
            game.id,
            game.title,
            game.format.label(),
            game.size,
            game.path.display()
        );
    }

    if let Some(iso) = args.next() {
        println!("ajout de {iso}…");
        txbm_core::convert::perform(iso.into(), &config, &|p| println!("  {p}%"))?;

        // L'ISO de test est un disque d'installation : vérifie que son contenu
        // a bien été poussé dans Content/0000000000000000/AAAA0001/00000002.
        let mut session = txbm_core::ftp::FtpSession::connect(&config.contents.ftp_config())?;
        let entries =
            session.list_dir("/Hdd1/Content/0000000000000000/AAAA0001/00000002");
        println!("contenu distant AAAA0001/00000002 :");
        for entry in &entries {
            println!("  - {} ({} octets)", entry.name, entry.size);
        }
        session.quit();
        assert!(!entries.is_empty(), "le contenu n'a pas été envoyé !");

        // Test de suppression distante sur le premier jeu GOD trouvé.
        let (games, _) = target.scan(&NO_CANCEL)?;
        if let Some(game) = games.iter().find(|g| !g.id.is_empty()) {
            println!("suppression de {}…", game.title);
            target.delete_game(game)?;
            let (games, _) = target.scan(&NO_CANCEL)?;
            println!("après suppression : {} jeu(x)", games.len());
            for game in &games {
                println!("- {}", game.title);
            }
        }
    }

    Ok(())
}
