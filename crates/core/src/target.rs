// SPDX-License-Identifier: GPL-3.0-only

//! Cible de la bibliothèque : disque local (USB) ou console distante (FTP).
//! Toutes les opérations (scan, suppression, installation) s'appliquent
//! directement à la cible.
//!
//! En FTP, les emplacements scannés par Aurora sont lus depuis ses bases
//! SQLite (`Aurora/Data/Databases/settings.db` + `content.db`), avec repli
//! sur les chemins par défaut si Aurora est introuvable.

use crate::config::{ConfigContents, TargetKind};
use crate::data_dir::DATA_DIR;
use crate::drive_info::DriveInfo;
use crate::ftp::{FtpConfig, FtpSession};
use crate::game::{Game, GameFormat};
use crate::{CONTENT_DIR, GAMES_DIR};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Message d'erreur utilisé quand un scan est annulé par l'utilisateur.
pub const SCAN_CANCELLED: &str = "scan cancelled";

#[derive(Debug, Clone)]
pub enum Target {
    Local(PathBuf),
    Ftp(FtpConfig),
}

impl Target {
    pub fn from_config(contents: &ConfigContents) -> Option<Target> {
        match contents.target_kind {
            TargetKind::Local => {
                if contents.mount_point.as_os_str().is_empty() {
                    None
                } else {
                    Some(Target::Local(contents.mount_point.clone()))
                }
            }
            TargetKind::Ftp => {
                let ftp = contents.ftp_config();
                if ftp.host.is_empty() {
                    None
                } else {
                    Some(Target::Ftp(ftp))
                }
            }
        }
    }

    /// Libellé affiché (chemin local ou ftp://ip).
    pub fn display(&self) -> String {
        match self {
            Target::Local(path) => path.to_string_lossy().to_string(),
            Target::Ftp(ftp) => format!("ftp://{}", ftp.host),
        }
    }

    /// Liste les jeux et les informations du disque de la cible.
    /// `cancel` peut être levé depuis un autre thread pour interrompre
    /// un scan FTP entre deux commandes.
    pub fn scan(&self, cancel: &AtomicBool) -> Result<(Vec<Game>, DriveInfo)> {
        match self {
            Target::Local(path) => {
                let games = crate::game::scan_drive(path);
                let drive_info = DriveInfo::from_path(path).unwrap_or_default();
                Ok((games, drive_info))
            }
            Target::Ftp(ftp) => scan_ftp(ftp, cancel),
        }
    }

    /// Supprime un jeu de la cible.
    pub fn delete_game(&self, game: &Game) -> Result<()> {
        match self {
            Target::Local(_) => {
                std::fs::remove_dir_all(&game.path)?;
                Ok(())
            }
            Target::Ftp(ftp) => {
                let mut session = FtpSession::connect(ftp)?;
                let remote = game.path.to_string_lossy().replace('\\', "/");
                let result = session.remove_dir_recursive(&remote);
                session.quit();
                result
            }
        }
    }
}

/// Types de contenu STFS considérés comme des jeux installés.
const GOD_CONTENT_TYPES: [(&str, bool); 2] = [("00007000", true), ("00005000", false)];

/// Trouve la racine du disque interne de la console (Hdd1).
pub fn ftp_hdd_root(session: &mut FtpSession) -> String {
    session
        .list_root()
        .unwrap_or_default()
        .iter()
        .find(|r| r.eq_ignore_ascii_case("Hdd1"))
        .cloned()
        .unwrap_or_else(|| "Hdd1".to_string())
}

/// Emplacements scannés par Aurora, résolus en chemins FTP absolus.
#[derive(Debug, Clone)]
pub struct AuroraPaths {
    /// Dossiers de type Content/0000000000000000 (jeux GOD).
    pub content_dirs: Vec<String>,
    /// Autres dossiers (jeux extraits), avec la profondeur de scan d'Aurora.
    pub extracted_dirs: Vec<(String, u32)>,
}

impl AuroraPaths {
    /// Chemins par défaut si les bases d'Aurora sont introuvables.
    pub fn defaults(hdd: &str) -> Self {
        Self {
            content_dirs: vec![format!("/{hdd}/{CONTENT_DIR}")],
            extracted_dirs: vec![(format!("/{hdd}/{GAMES_DIR}"), 2)],
        }
    }

    /// Destination des installations GOD.
    pub fn install_content_dir(&self, hdd: &str) -> String {
        self.content_dirs
            .first()
            .cloned()
            .unwrap_or_else(|| format!("/{hdd}/{CONTENT_DIR}"))
    }

    /// Destination des jeux extraits.
    pub fn install_extracted_dir(&self, hdd: &str) -> String {
        self.extracted_dirs
            .first()
            .map(|(p, _)| p.clone())
            .unwrap_or_else(|| format!("/{hdd}/{GAMES_DIR}"))
    }
}

/// Cherche l'installation d'Aurora sur les lecteurs de la console
/// et retourne le dossier de ses bases de données.
fn find_aurora_databases(session: &mut FtpSession) -> Option<String> {
    let roots = session.list_root().ok()?;
    for root in roots {
        for entry in session.list_dir(&format!("/{root}")) {
            if !entry.is_dir || !entry.name.eq_ignore_ascii_case("Aurora") {
                continue;
            }
            let db_dir = format!("/{root}/{}/Data/Databases", entry.name);
            let files = session.list_dir(&db_dir);
            let has_settings = files
                .iter()
                .any(|f| f.name.eq_ignore_ascii_case("settings.db"));
            let has_content = files
                .iter()
                .any(|f| f.name.eq_ignore_ascii_case("content.db"));
            if has_settings && has_content {
                return Some(db_dir);
            }
        }
    }
    None
}

/// Lit les ScanPaths d'Aurora (settings.db) et les résout en chemins FTP
/// via la table MountedDevices (content.db).
pub fn aurora_paths(session: &mut FtpSession) -> Result<AuroraPaths> {
    let db_dir = find_aurora_databases(session)
        .context("installation Aurora introuvable sur la console")?;

    let tmp_dir = DATA_DIR.join("tmp");
    std::fs::create_dir_all(&tmp_dir)?;

    let settings_bytes = session.download_file(&format!("{db_dir}/settings.db"))?;
    let content_bytes = session.download_file(&format!("{db_dir}/content.db"))?;
    let settings_path = tmp_dir.join("aurora-settings.db");
    let content_path = tmp_dir.join("aurora-content.db");
    std::fs::write(&settings_path, settings_bytes)?;
    std::fs::write(&content_path, content_bytes)?;

    let result = read_aurora_databases(&settings_path, &content_path);

    let _ = std::fs::remove_file(&settings_path);
    let _ = std::fs::remove_file(&content_path);

    result
}

fn read_aurora_databases(
    settings_path: &std::path::Path,
    content_path: &std::path::Path,
) -> Result<AuroraPaths> {
    let content_db =
        rusqlite::Connection::open(content_path).context("ouverture de content.db")?;
    let mut devices = std::collections::HashMap::new();
    {
        let mut stmt = content_db
            .prepare("SELECT DeviceId, DeviceName FROM MountedDevices")
            .context("lecture de MountedDevices")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows.flatten() {
            devices.insert(row.0, row.1);
        }
    }

    let settings_db =
        rusqlite::Connection::open(settings_path).context("ouverture de settings.db")?;
    let mut stmt = settings_db
        .prepare("SELECT Path, DeviceId, Depth FROM ScanPaths")
        .context("lecture de ScanPaths")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let mut paths = AuroraPaths {
        content_dirs: Vec::new(),
        extracted_dirs: Vec::new(),
    };

    for (path, device_id, depth) in rows.flatten() {
        let Some(device) = devices.get(&device_id) else {
            continue;
        };
        let relative = path.replace('\\', "/");
        let relative = relative.trim_matches('/');
        let remote = format!("/{device}/{relative}");

        // Un chemin Content est identifié par son suffixe 0000000000000000.
        if relative
            .to_lowercase()
            .ends_with("content/0000000000000000")
        {
            paths.content_dirs.push(remote);
        } else {
            paths.extracted_dirs.push((remote, depth.max(1) as u32));
        }
    }

    if paths.content_dirs.is_empty() && paths.extracted_dirs.is_empty() {
        bail!("aucun ScanPath dans les bases d'Aurora");
    }

    Ok(paths)
}

fn scan_ftp(ftp: &FtpConfig, cancel: &AtomicBool) -> Result<(Vec<Game>, DriveInfo)> {
    let check_cancel = || -> Result<()> {
        if cancel.load(Ordering::Relaxed) {
            bail!(SCAN_CANCELLED);
        }
        Ok(())
    };

    check_cancel()?;
    let mut session = FtpSession::connect(ftp)?;
    let hdd = ftp_hdd_root(&mut session);

    let paths = aurora_paths(&mut session).unwrap_or_else(|_| AuroraPaths::defaults(&hdd));

    let mut games = Vec::new();
    let mut games_bytes: u64 = 0;

    // GOD : <content>/<TitleID>/0000[57]000/…
    for content_dir in &paths.content_dirs {
        for entry in session.list_dir(content_dir) {
            check_cancel()?;
            let title_id = entry.name.to_uppercase();
            if !entry.is_dir || title_id.len() != 8 {
                continue;
            }
            let title_dir = format!("{content_dir}/{}", entry.name);
            for sub in session.list_dir(&title_dir) {
                let Some((_, is_x360)) = GOD_CONTENT_TYPES
                    .iter()
                    .find(|(t, _)| sub.is_dir && sub.name.eq_ignore_ascii_case(t))
                else {
                    continue;
                };
                let size = session.dir_size(&format!("{title_dir}/{}", sub.name), 3);
                let title = u32::from_str_radix(&title_id, 16)
                    .ok()
                    .and_then(iso2god::game_list::find_title_by_id)
                    .unwrap_or_else(|| title_id.clone());
                let search_term = format!("{title}\0{title_id}").to_lowercase();
                games_bytes += size;
                games.push(Game {
                    id: title_id.clone(),
                    title,
                    format: GameFormat::God,
                    path: PathBuf::from(&title_dir),
                    size,
                    is_x360: *is_x360,
                    search_term,
                });
            }
        }
    }

    // Jeux extraits : <dossier>/…/default.xex ou default.xbe,
    // en respectant la profondeur de scan d'Aurora.
    for (dir, depth) in &paths.extracted_dirs {
        scan_extracted_dir(
            &mut session,
            dir,
            *depth,
            &check_cancel,
            &mut games,
            &mut games_bytes,
        )?;
    }

    session.quit();

    let drive_info = DriveInfo {
        label: format!("{} (FTP)", ftp.host),
        used_bytes: 0,
        total_bytes: 0,
        games_bytes,
        fs_kind: Default::default(),
        allocation_granularity: 0,
    };

    Ok((games, drive_info))
}

fn scan_extracted_dir(
    session: &mut FtpSession,
    dir: &str,
    depth: u32,
    check_cancel: &dyn Fn() -> Result<()>,
    games: &mut Vec<Game>,
    games_bytes: &mut u64,
) -> Result<()> {
    for entry in session.list_dir(dir) {
        check_cancel()?;
        if !entry.is_dir {
            continue;
        }
        let game_dir = format!("{dir}/{}", entry.name);
        let children = session.list_dir(&game_dir);

        let format = if children
            .iter()
            .any(|c| !c.is_dir && c.name.eq_ignore_ascii_case("default.xex"))
        {
            Some(GameFormat::ExtractedXex)
        } else if children
            .iter()
            .any(|c| !c.is_dir && c.name.eq_ignore_ascii_case("default.xbe"))
        {
            Some(GameFormat::ExtractedXbe)
        } else {
            None
        };

        match format {
            Some(format) => {
                let size = session.dir_size(&game_dir, 3);
                let title = entry.name.clone();
                let search_term = title.to_lowercase();
                *games_bytes += size;
                games.push(Game {
                    id: String::new(),
                    title,
                    format,
                    path: PathBuf::from(&game_dir),
                    size,
                    is_x360: format == GameFormat::ExtractedXex,
                    search_term,
                });
            }
            // Pas d'exécutable ici : on descend d'un niveau si Aurora
            // le ferait aussi.
            None if depth > 1 => {
                scan_extracted_dir(
                    session,
                    &game_dir,
                    depth - 1,
                    check_cancel,
                    games,
                    games_bytes,
                )?;
            }
            None => {}
        }
    }
    Ok(())
}
