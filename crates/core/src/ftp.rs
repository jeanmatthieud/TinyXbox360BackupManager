// SPDX-License-Identifier: GPL-3.0-only

//! Transfert FTP vers la console (serveur FTP d'Aurora).
//!
//! Particularités du serveur de la console :
//! - une seule connexion à la fois (pas de flux multiples) ;
//! - les arguments chemin des commandes (LIST, STOR, DELE…) sont mal gérés :
//!   il faut naviguer avec CWD puis n'utiliser que des noms relatifs ;
//! - NLST renvoie des lignes LIST complètes.

use crate::util::dir_size;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use suppaftp::FtpStream;
use suppaftp::types::FileType;

#[derive(Debug, Clone, Default)]
pub struct FtpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub struct FtpSession {
    stream: FtpStream,
}

fn parent_and_name(remote_dir: &str) -> (String, String) {
    let trimmed = remote_dir.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((parent, name)) if !parent.is_empty() => (parent.to_string(), name.to_string()),
        Some((_, name)) => ("/".to_string(), name.to_string()),
        None => ("/".to_string(), trimmed.to_string()),
    }
}

/// Délai maximal d'établissement de la connexion.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Délai maximal d'attente d'une réponse sur la connexion de contrôle.
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

impl FtpSession {
    pub fn connect(config: &FtpConfig) -> Result<Self> {
        use std::net::ToSocketAddrs;

        let addr = (config.host.as_str(), config.port)
            .to_socket_addrs()
            .with_context(|| format!("adresse invalide : {}:{}", config.host, config.port))?
            .next()
            .with_context(|| format!("adresse introuvable : {}", config.host))?;

        let mut stream = FtpStream::connect_timeout(addr, CONNECT_TIMEOUT)
            .with_context(|| {
                format!(
                    "connexion à {}:{} (délai de {}s dépassé ?)",
                    config.host,
                    config.port,
                    CONNECT_TIMEOUT.as_secs()
                )
            })?;
        let _ = stream.get_ref().set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.get_ref().set_write_timeout(Some(IO_TIMEOUT));

        stream
            .login(&config.user, &config.password)
            .context("authentification FTP refusée")?;
        stream
            .transfer_type(FileType::Binary)
            .context("passage en mode binaire")?;
        Ok(Self { stream })
    }

    /// Se place dans un dossier distant en descendant composant par composant
    /// (le serveur de la console gère mal les chemins complets).
    fn cwd(&mut self, remote_dir: &str) -> Result<()> {
        self.stream.cwd("/").context("CWD /")?;
        for part in remote_dir.split('/').filter(|p| !p.is_empty()) {
            self.stream
                .cwd(part)
                .with_context(|| format!("CWD {part} (dans {remote_dir})"))?;
        }
        Ok(())
    }

    /// Comme `cwd`, mais crée les dossiers manquants.
    fn cwd_create(&mut self, remote_dir: &str) -> Result<()> {
        self.stream.cwd("/").context("CWD /")?;
        for part in remote_dir.split('/').filter(|p| !p.is_empty()) {
            if self.stream.cwd(part).is_err() {
                self.stream
                    .mkdir(part)
                    .with_context(|| format!("MKD {part} (dans {remote_dir})"))?;
                self.stream
                    .cwd(part)
                    .with_context(|| format!("CWD {part} (dans {remote_dir})"))?;
            }
        }
        Ok(())
    }

    /// Liste le dossier courant.
    fn list_cwd(&mut self) -> Vec<RemoteEntry> {
        let Ok(lines) = self.stream.list(None) else {
            return Vec::new();
        };

        lines
            .iter()
            .filter_map(|line| {
                let file = suppaftp::list::ListParser::parse_posix(line)
                    .or_else(|_| suppaftp::list::ListParser::parse_dos(line))
                    .ok()?;
                let name = file.name().to_string();
                if name == "." || name == ".." {
                    return None;
                }
                Some(RemoteEntry {
                    name,
                    is_dir: file.is_directory(),
                    size: file.size() as u64,
                })
            })
            .collect()
    }

    /// Liste les entrées à la racine de la console (Hdd1, Usb0, ...).
    pub fn list_root(&mut self) -> Result<Vec<String>> {
        self.cwd("/")?;
        Ok(self.list_cwd().into_iter().map(|e| e.name).collect())
    }

    /// Liste un dossier distant : (nom, est_un_dossier, taille).
    /// Retourne une liste vide si le dossier n'existe pas.
    pub fn list_dir(&mut self, remote_dir: &str) -> Vec<RemoteEntry> {
        if self.cwd(remote_dir).is_err() {
            return Vec::new();
        }
        self.list_cwd()
    }

    /// Taille récursive d'un dossier distant, avec profondeur bornée.
    pub fn dir_size(&mut self, remote_dir: &str, max_depth: u32) -> u64 {
        let mut total = 0;
        for entry in self.list_dir(remote_dir) {
            if entry.is_dir {
                if max_depth > 0 {
                    total += self.dir_size(&format!("{remote_dir}/{}", entry.name), max_depth - 1);
                }
            } else {
                total += entry.size;
            }
        }
        total
    }

    /// Télécharge un fichier distant en mémoire.
    pub fn download_file(&mut self, remote_path: &str) -> Result<Vec<u8>> {
        let (parent, name) = parent_and_name(remote_path);
        self.cwd(&parent)?;
        Ok(self
            .stream
            .retr_as_buffer(&name)
            .with_context(|| format!("téléchargement de {remote_path}"))?
            .into_inner())
    }

    /// Supprime récursivement un dossier distant.
    pub fn remove_dir_recursive(&mut self, remote_dir: &str) -> Result<()> {
        for entry in self.list_dir(remote_dir) {
            if entry.is_dir {
                self.remove_dir_recursive(&format!("{remote_dir}/{}", entry.name))?;
            } else {
                self.cwd(remote_dir)?;
                self.stream
                    .rm(&entry.name)
                    .with_context(|| format!("suppression de {remote_dir}/{}", entry.name))?;
            }
        }

        let (parent, name) = parent_and_name(remote_dir);
        self.cwd(&parent)?;
        self.stream
            .rmdir(&name)
            .with_context(|| format!("suppression de {remote_dir}"))?;
        Ok(())
    }

    /// Envoie récursivement `local_dir` vers `remote_dir` (créé si besoin).
    /// `progress(octets_envoyés, octets_total)` est appelé après chaque fichier.
    pub fn upload_dir(
        &mut self,
        local_dir: &Path,
        remote_dir: &str,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()> {
        let total = dir_size(local_dir);
        let mut sent: u64 = 0;
        progress(0, total);
        self.upload_dir_inner(local_dir, remote_dir, &mut sent, total, progress)
    }

    fn upload_dir_inner(
        &mut self,
        local_dir: &Path,
        remote_dir: &str,
        sent: &mut u64,
        total: u64,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<()> {
        self.cwd_create(remote_dir)
            .with_context(|| format!("création de {remote_dir}"))?;

        let entries = std::fs::read_dir(local_dir)
            .with_context(|| format!("lecture de {}", local_dir.display()))?;
        for entry in entries.flatten() {
            let local_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if local_path.is_dir() {
                let remote_path = format!("{}/{}", remote_dir.trim_end_matches('/'), name);
                self.upload_dir_inner(&local_path, &remote_path, sent, total, progress)?;
                // Les envois récursifs ont changé de dossier courant.
                self.cwd(remote_dir)?;
            } else {
                let file = File::open(&local_path)
                    .with_context(|| format!("ouverture de {}", local_path.display()))?;
                let size = file.metadata().map(|m| m.len()).unwrap_or(0);
                self.stream
                    .put_file(&name, &mut BufReader::new(file))
                    .with_context(|| format!("envoi de {remote_dir}/{name}"))?;
                *sent += size;
                progress(*sent, total);
            }
        }
        Ok(())
    }

    pub fn quit(mut self) {
        let _ = self.stream.quit();
    }
}
