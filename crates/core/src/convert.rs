// SPDX-License-Identifier: GPL-3.0-only

//! High-level pipeline: from an ISO, does what is needed on the target
//! (local drive or FTP console).

use crate::config::Config;
use crate::data_dir::DATA_DIR;
use crate::ftp::FtpSession;
use crate::iso_info::{self, IsoKind};
use crate::target::{AuroraPaths, Target, aurora_paths, ftp_hdd_root};
use crate::util::sanitize_name;
use crate::{CONTENT_DIR, GAMES_DIR, extract, god, unity};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Converts/extracts `in_path` on the target, depending on the image type.
/// `update_progress` receives a percentage (0-100) and, during the FTP upload
/// phase, the running average upload speed in megabytes per second.
pub fn perform(
    in_path: PathBuf,
    config: &Config,
    update_progress: &dyn Fn(u32, Option<f64>),
) -> Result<()> {
    let target =
        Target::from_config(&config.contents).context("no target selected")?;

    match &target {
        Target::Local(root) => {
            convert_into(&in_path, root, &|p| update_progress(p, None))?;
        }
        Target::Ftp(ftp) => {
            let stem = sanitize_name(
                in_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("game"),
            );
            let staging = DATA_DIR.join("staging").join(&stem);
            if staging.exists() {
                std::fs::remove_dir_all(&staging)?;
            }
            std::fs::create_dir_all(&staging)?;

            let result = (|| -> Result<()> {
                // Local conversion in staging folder: 0-50%.
                convert_into(&in_path, &staging, &|p| update_progress(p * 50 / 100, None))?;

                // Direct upload to the console, to locations
                // scanned by Aurora: 50-100%.
                let mut session = FtpSession::connect(ftp)?;
                let hdd = ftp_hdd_root(&mut session);
                let paths =
                    aurora_paths(&mut session).unwrap_or_else(|_| AuroraPaths::defaults(&hdd));

                let upload = (|| -> Result<()> {
                    let total = crate::util::dir_size(&staging);
                    let mut sent_before: u64 = 0;

                    // Maps upload progress to the 50-100% band; the per-file
                    // average speed (megabytes/s) comes from the FTP layer.
                    let report = |base: u64, sent: u64, speed: Option<f64>| {
                        let done = base + sent;
                        let pct = 50 + (done * 50 / total.max(1)) as u32;
                        update_progress(pct, speed);
                    };

                    // GOD / content: staging/Content/0000000000000000/* →
                    // first Content path of Aurora.
                    let staging_content = staging.join(CONTENT_DIR);
                    if staging_content.is_dir() {
                        let remote = paths.install_content_dir(&hdd);
                        for entry in std::fs::read_dir(&staging_content)?.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            let base = sent_before;
                            session.upload_dir(
                                &entry.path(),
                                &format!("{remote}/{name}"),
                                &mut |sent, _, speed| report(base, sent, speed),
                            )?;
                            sent_before += crate::util::dir_size(&entry.path());
                        }
                    }

                    // Extracted games: staging/Games/* → first
                    // "extracted" path of Aurora (e.g. \XBox OG).
                    let staging_games = staging.join(GAMES_DIR);
                    if staging_games.is_dir() {
                        let remote = paths.install_extracted_dir(&hdd);
                        for entry in std::fs::read_dir(&staging_games)?.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            let base = sent_before;
                            session.upload_dir(
                                &entry.path(),
                                &format!("{remote}/{name}"),
                                &mut |sent, _, speed| report(base, sent, speed),
                            )?;
                            sent_before += crate::util::dir_size(&entry.path());
                        }
                    }

                    Ok(())
                })();
                session.quit();
                upload
            })();

            let _ = std::fs::remove_dir_all(DATA_DIR.join("staging"));
            result?;
        }
    }

    if config.contents.remove_sources_games {
        std::fs::remove_file(&in_path).ok();
    }

    Ok(())
}

/// Converts/extracts `in_path` to `root`, a local folder organized like
/// an Xbox drive (Content/0000000000000000 + Games).
fn convert_into(in_path: &Path, root: &Path, update_progress: &dyn Fn(u32)) -> Result<()> {
    let info = iso_info::inspect(in_path)?;

    match info.kind {
        IsoKind::Xbox360Game => {
            let title = info.name.clone().or_else(|| {
                let tid = info.title_id.as_deref()?;
                unity::search_titles(tid)
                    .ok()?
                    .into_iter()
                    .next()
                    .map(|t| t.name)
            });
            let content_dir = root.join(CONTENT_DIR);
            std::fs::create_dir_all(&content_dir)?;

            god::convert_to_god(in_path, &content_dir, title.as_deref(), &mut |done, total| {
                update_progress((done * 100 / total.max(1)) as u32);
            })?;
        }
        IsoKind::XboxOriginal => {
            let name = sanitize_name(
                info.name
                    .as_deref()
                    .or(in_path.file_stem().and_then(|s| s.to_str()))
                    .unwrap_or("Xbox game"),
            );
            // Embed the TitleID in the folder name so later scans can
            // identify the game without reading its XBE.
            let name = match info.title_id.as_deref() {
                Some(tid) => crate::game::og_folder_name(&name, tid),
                None => name,
            };
            let dest = root.join(GAMES_DIR).join(&name);
            if dest.exists() {
                bail!("the folder {} already exists", dest.display());
            }

            extract::extract_iso(in_path, &dest, &mut |done, total| {
                update_progress((done * 100 / total.max(1)) as u32);
            })?;
        }
        IsoKind::ContentDisc => {
            let stem = sanitize_name(
                in_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("disc"),
            );
            let tmp = root.join(".txbm-tmp").join(&stem);
            if tmp.exists() {
                std::fs::remove_dir_all(&tmp)?;
            }

            let result = (|| -> Result<()> {
                extract::extract_iso(in_path, &tmp, &mut |done, total| {
                    update_progress((done * 100 / total.max(1)) as u32);
                })?;

                // Expected structure: Content/0000000000000000/<TitleID>/...
                let extracted_content = find_dir_ci(&tmp, "Content")
                    .and_then(|c| find_dir_ci(&c, "0000000000000000"))
                    .context(
                        "unexpected structure: no Content/0000000000000000 folder \
                         in this image (is it really an install disc / DLC?)",
                    )?;

                let content_target = root.join(CONTENT_DIR);
                std::fs::create_dir_all(&content_target)?;
                for entry in std::fs::read_dir(&extracted_content)?.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    merge_move(&entry.path(), &content_target.join(&name))?;
                }
                Ok(())
            })();

            let _ = std::fs::remove_dir_all(root.join(".txbm-tmp"));
            result?;
        }
    }

    Ok(())
}

/// Moves `src` to `dst`, merging with existing content.
fn merge_move(src: &Path, dst: &Path) -> Result<()> {
    if !dst.exists() {
        if std::fs::rename(src, dst).is_ok() {
            return Ok(());
        }
        // Different file system: copy then delete.
        std::fs::create_dir_all(dst.parent().unwrap_or(dst))?;
        let options = fs_extra::dir::CopyOptions::new().copy_inside(true);
        fs_extra::dir::move_dir(src, dst, &options)
            .map_err(|e| anyhow::anyhow!("moving to {}: {e}", dst.display()))?;
        return Ok(());
    }
    if src.is_dir() && dst.is_dir() {
        for entry in std::fs::read_dir(src)?.flatten() {
            merge_move(&entry.path(), &dst.join(entry.file_name()))?;
        }
        let _ = std::fs::remove_dir(src);
        Ok(())
    } else {
        // File already present: replace.
        std::fs::remove_file(dst).ok();
        std::fs::rename(src, dst).or_else(|_| {
            std::fs::copy(src, dst)
                .map(|_| ())
                .and_then(|_| std::fs::remove_file(src))
        })?;
        Ok(())
    }
}

fn find_dir_ci(base: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(base).ok()?;
    for entry in entries.flatten() {
        if entry.path().is_dir()
            && entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
        {
            return Some(entry.path());
        }
    }
    None
}
