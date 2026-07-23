// SPDX-License-Identifier: GPL-3.0-only

//! High-level pipeline: from an ISO, does what is needed on the target
//! (local drive or FTP console).

use crate::config::Config;
use crate::data_dir::DATA_DIR;
use crate::ftp::FtpSession;
use crate::iso_info::{self, IsoInfo, IsoKind};
use crate::stfs::{self, StfsInfo};
use crate::target::{Target, ftp_hdd_root, ftp_layout, local_layout};
use crate::util::sanitize_name;
use crate::{DEFAULT_GOD_DIR, DEFAULT_XBE_DIR, DEFAULT_XEX_DIR, archive, extract, god, unity};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Install destinations for a conversion, decoupled from any fixed folder
/// layout: `god_dir` receives `<TitleID>` GOD/Arcade folders, the two
/// `extracted_*_dir` receive `<Name>` extracted-game folders (Original Xbox /
/// Xbox 360), and `work_dir` is scratch space for intermediate extraction
/// (kept on the same filesystem for cheap moves).
struct ConvertDest {
    god_dir: PathBuf,
    xbe_dir: PathBuf,
    // Reserved for the future "extract Xbox 360 game" path; the destination is
    // already wired end-to-end (config, manifest, upload) so only the
    // extraction branch remains to be added.
    #[allow(dead_code)]
    xex_dir: PathBuf,
    work_dir: PathBuf,
}

impl ConvertDest {
    /// Standard staging layout under `root` (`Content/0000000000000000`,
    /// `Games Xbox`, `Games Xbox360`), used for the FTP staging folder and archive
    /// recursion.
    fn under(root: &Path) -> Self {
        Self {
            god_dir: root.join(DEFAULT_GOD_DIR),
            xbe_dir: root.join(DEFAULT_XBE_DIR),
            xex_dir: root.join(DEFAULT_XEX_DIR),
            work_dir: root.to_path_buf(),
        }
    }
}

/// Error message used to signal that a conversion was cancelled by the user
/// (mirrors [`crate::target::SCAN_CANCELLED`]). Recognized by the GUI to show
/// an informational "cancelled" notice rather than a failure.
pub const CONVERSION_CANCELLED: &str = "conversion cancelled";

/// Returns whether the shared cancel flag has been raised.
pub(crate) fn is_cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Relaxed)
}

/// Kind of input file accepted by the conversion pipeline.
pub enum InputKind {
    /// Optical disc image (360 game, OG game or content disc).
    Iso(IsoInfo),
    /// .7z / .zip archive: either XBLA (STFS) packages or a wrapped ISO.
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
    cancel: &AtomicBool,
    update_progress: &dyn Fn(u32, Option<f64>),
) -> Result<()> {
    let target =
        Target::from_config(&config.contents).context("no target selected")?;

    match &target {
        Target::Local(root) => {
            let storage = local_layout(root).storage;
            let dest = ConvertDest {
                god_dir: PathBuf::from(&storage.god_dir),
                xbe_dir: PathBuf::from(&storage.xbe_dir),
                xex_dir: PathBuf::from(&storage.xex_dir),
                work_dir: root.clone(),
            };
            convert_into(&in_path, &dest, cancel, &|p| update_progress(p, None))?;
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
                convert_into(&in_path, &ConvertDest::under(&staging), cancel, &|p| {
                    update_progress(p * 50 / 100, None)
                })?;

                // Direct upload to the console, to its resolved storage
                // locations: 50-100%.
                let mut session = FtpSession::connect(ftp)?;
                let hdd = ftp_hdd_root(&mut session);
                let storage = ftp_layout(&mut session, &hdd).storage;

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

                    // Each staging sub-tree is uploaded to its own storage
                    // directory: GOD containers, extracted Original Xbox (XBE)
                    // and extracted Xbox 360 (XEX) games.
                    let uploads: [(PathBuf, &String); 3] = [
                        (staging.join(DEFAULT_GOD_DIR), &storage.god_dir),
                        (staging.join(DEFAULT_XBE_DIR), &storage.xbe_dir),
                        (staging.join(DEFAULT_XEX_DIR), &storage.xex_dir),
                    ];
                    for (staging_sub, remote) in &uploads {
                        if !staging_sub.is_dir() {
                            continue;
                        }
                        for entry in std::fs::read_dir(staging_sub)?.flatten() {
                            if is_cancelled(cancel) {
                                bail!(CONVERSION_CANCELLED);
                            }
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

/// Converts/extracts `in_path` into `dest`'s storage folders.
fn convert_into(
    in_path: &Path,
    dest: &ConvertDest,
    cancel: &AtomicBool,
    update_progress: &dyn Fn(u32),
) -> Result<()> {
    let info = match inspect_input(in_path)? {
        InputKind::StfsPackage(package) => {
            install_stfs_package(&package, &dest.god_dir, cancel, &mut |done, total| {
                update_progress((done * 100 / total.max(1)) as u32);
            })?;
            return Ok(());
        }
        InputKind::Archive => return install_archive(in_path, dest, cancel, update_progress),
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
            let content_dir = &dest.god_dir;
            std::fs::create_dir_all(content_dir)?;

            god::convert_to_god(in_path, content_dir, title.as_deref(), cancel, &mut |done, total| {
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
            let game_dir = dest.xbe_dir.join(&name);
            if game_dir.exists() {
                bail!("the folder {} already exists", game_dir.display());
            }

            let res = extract::extract_iso(in_path, &game_dir, cancel, &mut |done, total| {
                update_progress((done * 100 / total.max(1)) as u32);
            });
            // On cancellation, drop the partially-extracted folder (it is a
            // fresh folder — we bailed above if it already existed).
            if res.is_err() && is_cancelled(cancel) {
                let _ = std::fs::remove_dir_all(&game_dir);
            }
            res?;
        }
        IsoKind::ContentDisc => {
            let stem = sanitize_name(
                in_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("disc"),
            );
            let tmp = dest.work_dir.join(".txbm-tmp").join(&stem);
            if tmp.exists() {
                std::fs::remove_dir_all(&tmp)?;
            }

            let result = (|| -> Result<()> {
                extract::extract_iso(in_path, &tmp, cancel, &mut |done, total| {
                    update_progress((done * 100 / total.max(1)) as u32);
                })?;

                // Expected structure: Content/0000000000000000/<TitleID>/...
                let extracted_content = find_dir_ci(&tmp, "Content")
                    .and_then(|c| find_dir_ci(&c, "0000000000000000"))
                    .context(
                        "unexpected structure: no Content/0000000000000000 folder \
                         in this image (is it really an install disc / DLC?)",
                    )?;

                let content_target = &dest.god_dir;
                std::fs::create_dir_all(content_target)?;
                for entry in std::fs::read_dir(&extracted_content)?.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    merge_move(&entry.path(), &content_target.join(&name))?;
                }
                Ok(())
            })();

            let _ = std::fs::remove_dir_all(dest.work_dir.join(".txbm-tmp"));
            result?;
        }
        IsoKind::BundledContent => {
            let stem = sanitize_name(
                in_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("disc"),
            );
            let tmp = dest.work_dir.join(".txbm-tmp").join(&stem);
            if tmp.exists() {
                std::fs::remove_dir_all(&tmp)?;
            }

            let result = (|| -> Result<()> {
                // Extraction: 0-80%.
                extract::extract_iso(in_path, &tmp, cancel, &mut |done, total| {
                    update_progress((done * 80 / total.max(1)) as u32);
                })?;

                // Scan the whole disc, not just its Content folder: some
                // bonus discs also carry a loose title update at the root
                // (e.g. `title_update.bin`). `$SystemUpdate` is excluded:
                // it holds a generic dashboard update, not game content.
                // Each found package is installed under its own internal
                // TitleID, not the installer's own (often a placeholder).
                let packages = find_installable_packages_excluding(&tmp, &["$SystemUpdate"])?;
                if packages.is_empty() {
                    bail!(
                        "no installable DLC/title-update/Arcade package found \
                         in this image's bundled content"
                    );
                }

                // Installation: 80-100%.
                install_packages(&packages, &dest.god_dir, cancel, &|p| {
                    update_progress(80 + p * 20 / 100)
                })?;
                update_progress(100);
                Ok(())
            })();

            let _ = std::fs::remove_dir_all(dest.work_dir.join(".txbm-tmp"));
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
    god_dir: &Path,
    cancel: &AtomicBool,
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

    // Title updates aren't identified by name on the console (nor by
    // Aurora, whose own cache keys them by content hash too): the source
    // name is often a generic one shared by unrelated discs/updates (e.g.
    // "title_update.bin"), which would otherwise silently overwrite an
    // unrelated update installed under the same TitleID.
    let file_name = if info.content_type == stfs::CONTENT_TYPE_TITLE_UPDATE {
        crate::util::sha1_hex_file(&info.path)
            .with_context(|| format!("hashing {}", info.path.display()))?
    } else {
        let file_name = info
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("package");
        file_name.chars().take(crate::game::FATX_MAX_NAME).collect()
    };

    let dest_dir = god_dir
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
        if is_cancelled(cancel) {
            // Drop the partially-written destination file before bailing.
            drop(dst);
            let _ = std::fs::remove_file(&dest);
            bail!(CONVERSION_CANCELLED);
        }
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

/// Extracts a .7z/.zip archive and installs what it contains. Two shapes are
/// supported: an archive wrapping a single ISO (plus optional .txt/.md notes),
/// which is converted like a normal ISO import; or a set of XBLA/DLC/title-
/// update STFS packages, in which case an Arcade package is required.
fn install_archive(
    in_path: &Path,
    dest: &ConvertDest,
    cancel: &AtomicBool,
    update_progress: &dyn Fn(u32),
) -> Result<()> {
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
        // Extraction: 0-50%.
        archive::extract_to(in_path, &tmp, &mut |done, total| {
            update_progress((done * 50 / total.max(1)) as u32);
        })?;
        if is_cancelled(cancel) {
            bail!(CONVERSION_CANCELLED);
        }

        // Archive wrapping a single ISO: convert it as a normal ISO import
        // (0-50% extraction already done, conversion runs on 50-100%). This
        // saves the user from manually unpacking the ISO first.
        if let Some(iso) = single_iso_in(&tmp)? {
            return convert_into(&iso, dest, cancel, &|p| update_progress(50 + p * 50 / 100));
        }

        // Otherwise: a set of XBLA/DLC/title-update packages.
        let packages = find_installable_packages(&tmp)?;
        if !packages
            .iter()
            .any(|p| p.content_type == stfs::CONTENT_TYPE_ARCADE)
        {
            bail!(
                "no ISO and no Arcade package (content type 000D0000) found in this \
                 archive: is it really an XBLA game or an ISO archive?"
            );
        }

        // Installation: 50-100%, weighted by package size.
        install_packages(&packages, &dest.god_dir, cancel, &|p| {
            update_progress(50 + p * 50 / 100)
        })?;
        update_progress(100);
        Ok(())
    })();

    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Detects the "archive wrapping an ISO" case among the files under `dir`.
///
/// - No ISO at all → `Ok(None)` (the archive is treated as XBLA packages).
/// - Exactly one ISO, every other file being a `.txt`/`.md` note (directories
///   are ignored) → `Ok(Some(iso))`.
/// - More than one ISO, or an ISO alongside any other kind of file → error,
///   so the ambiguous archive is rejected rather than silently mishandled.
fn single_iso_in(dir: &Path) -> Result<Option<PathBuf>> {
    let has_ext = |p: &Path, ext: &str| {
        p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
    };

    let mut files = Vec::new();
    collect_files(dir, &[], &mut files)?;

    let isos: Vec<&PathBuf> = files.iter().filter(|f| has_ext(f, "iso")).collect();
    if isos.is_empty() {
        return Ok(None);
    }
    if isos.len() > 1 {
        bail!(
            "archive contains {} ISO images; only a single ISO (plus optional \
             .txt/.md notes) is supported",
            isos.len()
        );
    }

    // Every remaining file must be a text note; anything else is unexpected.
    let extra: Vec<&PathBuf> = files
        .iter()
        .filter(|f| !has_ext(f, "iso") && !has_ext(f, "txt") && !has_ext(f, "md"))
        .collect();
    if let Some(unexpected) = extra.first() {
        bail!(
            "archive contains an ISO alongside an unexpected file ({}); only \
             .txt/.md notes may accompany the ISO",
            unexpected
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
    }

    Ok(Some(isos[0].clone()))
}

/// Recursively finds every DLC / title-update / Arcade STFS package under
/// `dir`, meant to be installed each under its own internal TitleID.
fn find_installable_packages(dir: &Path) -> Result<Vec<StfsInfo>> {
    find_installable_packages_excluding(dir, &[])
}

/// Like `find_installable_packages`, but does not descend into
/// subdirectories (anywhere in the tree) whose name matches `exclude`
/// (case-insensitive) — e.g. a disc's `$SystemUpdate` folder, which holds a
/// generic dashboard update rather than game-specific content.
fn find_installable_packages_excluding(dir: &Path, exclude: &[&str]) -> Result<Vec<StfsInfo>> {
    let mut files = Vec::new();
    collect_files(dir, exclude, &mut files)?;
    Ok(files
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
        .collect())
}

/// Installs every package (each under its own internal TitleID/content-type
/// folder), weighting `update_progress` (0-100) by package size.
fn install_packages(
    packages: &[StfsInfo],
    god_dir: &Path,
    cancel: &AtomicBool,
    update_progress: &dyn Fn(u32),
) -> Result<()> {
    let total: u64 = packages
        .iter()
        .map(|p| std::fs::metadata(&p.path).map(|m| m.len()).unwrap_or(0))
        .sum();
    let mut done_before: u64 = 0;
    for package in packages {
        install_stfs_package(package, god_dir, cancel, &mut |done, _| {
            update_progress(((done_before + done) * 100 / total.max(1)) as u32);
        })?;
        done_before += std::fs::metadata(&package.path).map(|m| m.len()).unwrap_or(0);
    }
    Ok(())
}

/// Recursively collects the files under `dir`, skipping subdirectories
/// whose name (case-insensitive) is in `exclude`.
fn collect_files(dir: &Path, exclude: &[&str], files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if exclude
                .iter()
                .any(|e| name.to_string_lossy().eq_ignore_ascii_case(e))
            {
                continue;
            }
            collect_files(&path, exclude, files)?;
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
        convert_into(&zip_path, &ConvertDest::under(&root), &AtomicBool::new(false), &|_| {}).unwrap();

        let title_dir = root.join(DEFAULT_GOD_DIR).join("58410889");
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
        convert_into(&package, &ConvertDest::under(&root), &AtomicBool::new(false), &|_| {}).unwrap();
        assert!(
            root.join(DEFAULT_GOD_DIR)
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
        let err = convert_into(&zip_path, &ConvertDest::under(&root), &AtomicBool::new(false), &|_| {}).unwrap_err();
        assert!(err.to_string().contains("no Arcade package"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn single_iso_detection() {
        let dir = std::env::temp_dir().join("txbm-single-iso");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();

        // No ISO → None (falls back to the XBLA flow).
        std::fs::write(dir.join("readme.txt"), b"hi").unwrap();
        assert!(single_iso_in(&dir).unwrap().is_none());

        // One ISO + notes (nested) → Some(iso).
        let iso = dir.join("sub/Game.iso");
        std::fs::write(&iso, b"fake").unwrap();
        std::fs::write(dir.join("notes.md"), b"# notes").unwrap();
        assert_eq!(single_iso_in(&dir).unwrap().as_deref(), Some(iso.as_path()));

        // ISO alongside a non-note file → error.
        std::fs::write(dir.join("extra.bin"), b"x").unwrap();
        assert!(single_iso_in(&dir).is_err());
        std::fs::remove_file(dir.join("extra.bin")).unwrap();

        // A second ISO → error.
        std::fs::write(dir.join("Other.ISO"), b"fake").unwrap();
        assert!(single_iso_in(&dir).is_err());

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
