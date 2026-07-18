// SPDX-License-Identifier: GPL-3.0-only

//! High-level pipeline: from an ISO, does what is needed on the target
//! (local drive or FTP console).

use crate::config::Config;
use crate::data_dir::DATA_DIR;
use crate::ftp::FtpSession;
use crate::iso_info::{self, IsoInfo, IsoKind};
use crate::stfs::{self, StfsInfo};
use crate::target::{AuroraPaths, Target, aurora_paths, ftp_hdd_root};
use crate::util::sanitize_name;
use crate::{CONTENT_DIR, GAMES_DIR, archive, extract, god, unity};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Kind of input file accepted by the conversion pipeline.
pub enum InputKind {
    /// Optical disc image (360 game, OG game or content disc).
    Iso(IsoInfo),
    /// .7z / .zip archive expected to contain XBLA (STFS) packages.
    Archive,
    /// Bare STFS package (CON / LIVE / PIRS).
    StfsPackage(StfsInfo),
}

/// Determines what `path` is, from its extension then its magic.
pub fn inspect_input(path: &Path) -> Result<InputKind> {
    let ext = path.extension().map(|e| e.to_ascii_lowercase());
    if ext.as_deref().is_some_and(|e| e == "iso") {
        return Ok(InputKind::Iso(iso_info::inspect(path)?));
    }
    if archive::is_supported_archive(path) {
        return Ok(InputKind::Archive);
    }
    if let Some(info) = stfs::inspect(path)? {
        return Ok(InputKind::StfsPackage(info));
    }
    // Last resort: maybe a renamed ISO.
    iso_info::inspect(path).map(InputKind::Iso).context(
        "unrecognized file: neither an ISO image, a .7z/.zip archive, \
         nor an STFS package (CON/LIVE/PIRS)",
    )
}

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
    let info = match inspect_input(in_path)? {
        InputKind::StfsPackage(package) => {
            install_stfs_package(&package, root, &mut |done, total| {
                update_progress((done * 100 / total.max(1)) as u32);
            })?;
            return Ok(());
        }
        InputKind::Archive => return install_archive(in_path, root, update_progress),
        InputKind::Iso(info) => info,
    };

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

/// Copies an STFS package as-is (original file name, truncated to the FATX
/// limit) to root/Content/0000000000000000/<TitleID>/<content type>/.
/// Only Arcade, DLC and title-update packages are accepted.
/// `update_progress` receives (copied bytes, total bytes).
fn install_stfs_package(
    info: &StfsInfo,
    root: &Path,
    update_progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    match info.content_type {
        stfs::CONTENT_TYPE_ARCADE
        | stfs::CONTENT_TYPE_DLC
        | stfs::CONTENT_TYPE_TITLE_UPDATE => {}
        other => bail!(
            "unsupported STFS content type {other:08X} in {} \
             (expected Arcade, DLC or title update)",
            info.path.display()
        ),
    }

    let file_name = info
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("package");
    let file_name: String = file_name.chars().take(crate::game::FATX_MAX_NAME).collect();

    let dest_dir = root
        .join(CONTENT_DIR)
        .join(&info.title_id)
        .join(info.content_type_dir());
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(&file_name);

    // Re-adding a package overwrites it, like GOD re-conversion does.
    let total = std::fs::metadata(&info.path)?.len();
    let mut src = std::fs::File::open(&info.path)
        .with_context(|| format!("opening {}", info.path.display()))?;
    let mut dst = std::fs::File::create(&dest)
        .with_context(|| format!("creating {}", dest.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    let mut done: u64 = 0;
    loop {
        let n = std::io::Read::read(&mut src, &mut buf)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut dst, &buf[..n])?;
        done += n as u64;
        update_progress(done, total);
    }
    Ok(())
}

/// Extracts a .7z/.zip archive and installs the XBLA packages it contains
/// (plus any DLC / title updates). Fails if no Arcade package is found.
fn install_archive(in_path: &Path, root: &Path, update_progress: &dyn Fn(u32)) -> Result<()> {
    let stem = sanitize_name(
        in_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("archive"),
    );
    let tmp = DATA_DIR.join("tmp").join("archive").join(&stem);
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }

    let result = (|| -> Result<()> {
        // Extraction: 0-80%.
        archive::extract_to(in_path, &tmp, &mut |done, total| {
            update_progress((done * 80 / total.max(1)) as u32);
        })?;

        let mut files = Vec::new();
        collect_files(&tmp, &mut files)?;
        let packages: Vec<StfsInfo> = files
            .iter()
            .filter_map(|f| stfs::inspect(f).ok().flatten())
            .filter(|p| {
                matches!(
                    p.content_type,
                    stfs::CONTENT_TYPE_ARCADE
                        | stfs::CONTENT_TYPE_DLC
                        | stfs::CONTENT_TYPE_TITLE_UPDATE
                )
            })
            .collect();

        if !packages
            .iter()
            .any(|p| p.content_type == stfs::CONTENT_TYPE_ARCADE)
        {
            bail!(
                "no Arcade package (content type 000D0000) found in this archive: \
                 is it really an XBLA game?"
            );
        }

        // Installation: 80-100%, weighted by package size.
        let total: u64 = packages
            .iter()
            .map(|p| std::fs::metadata(&p.path).map(|m| m.len()).unwrap_or(0))
            .sum();
        let mut done_before: u64 = 0;
        for package in &packages {
            install_stfs_package(package, root, &mut |done, _| {
                let pct = 80 + (done_before + done) * 20 / total.max(1);
                update_progress(pct as u32);
            })?;
            done_before += std::fs::metadata(&package.path).map(|m| m.len()).unwrap_or(0);
        }
        update_progress(100);
        Ok(())
    })();

    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Recursively collects the files under `dir`.
fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files)?;
        } else {
            files.push(path);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn stfs_package(content_type: u32, title_id: u32) -> Vec<u8> {
        let mut buf = vec![0u8; 0x2000];
        buf[..4].copy_from_slice(b"LIVE");
        buf[0x344..0x348].copy_from_slice(&content_type.to_be_bytes());
        buf[0x360..0x364].copy_from_slice(&title_id.to_be_bytes());
        buf
    }

    #[test]
    fn installs_zip_archive_with_arcade_and_dlc() {
        let dir = std::env::temp_dir().join("txbm-convert-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let zip_path = dir.join("game.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("Sub/ArcadeGamePackage", options).unwrap();
        zip.write_all(&stfs_package(stfs::CONTENT_TYPE_ARCADE, 0x58410889))
            .unwrap();
        zip.start_file("Sub/DlcPackage", options).unwrap();
        zip.write_all(&stfs_package(stfs::CONTENT_TYPE_DLC, 0x58410889))
            .unwrap();
        zip.start_file("readme.txt", options).unwrap();
        zip.write_all(b"hello").unwrap();
        zip.finish().unwrap();

        let root = dir.join("root");
        convert_into(&zip_path, &root, &|_| {}).unwrap();

        let title_dir = root.join(CONTENT_DIR).join("58410889");
        assert!(title_dir.join("000D0000/ArcadeGamePackage").is_file());
        assert!(title_dir.join("00000002/DlcPackage").is_file());

        let games = crate::game::scan_drive(&root);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].format, crate::game::GameFormat::Arcade);
        assert_eq!(games[0].id, "58410889");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn installs_bare_stfs_package() {
        let dir = std::env::temp_dir().join("txbm-convert-test-bare");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let package = dir.join("SomeArcadeGame");
        std::fs::write(&package, stfs_package(stfs::CONTENT_TYPE_ARCADE, 0x584108A1)).unwrap();

        let root = dir.join("root");
        convert_into(&package, &root, &|_| {}).unwrap();
        assert!(
            root.join(CONTENT_DIR)
                .join("584108A1/000D0000/SomeArcadeGame")
                .is_file()
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_archive_without_arcade_package() {
        let dir = std::env::temp_dir().join("txbm-convert-test-noarc");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let zip_path = dir.join("dlc-only.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("DlcPackage", options).unwrap();
        zip.write_all(&stfs_package(stfs::CONTENT_TYPE_DLC, 0x58410889))
            .unwrap();
        zip.finish().unwrap();

        let root = dir.join("root");
        let err = convert_into(&zip_path, &root, &|_| {}).unwrap_err();
        assert!(err.to_string().contains("no Arcade package"));

        std::fs::remove_dir_all(&dir).unwrap();
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
