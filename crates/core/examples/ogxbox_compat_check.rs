// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification of the original-Xbox compatibility tool, end to end,
//! against a disk image built here rather than a real console drive:
//! `cargo run -p txbm-core --example ogxbox_compat_check`
//!
//! The image is a sparse file laid out like a console drive, with a blank
//! Xbox 360 filesystem at the offset the *compatibility* partition sits at —
//! the one the console calls `HddX`. Everything the tool does to a real drive
//! (probe, inspect, back up, replace) therefore runs against it unchanged.
//! Sparse means the 4.5 GiB of nothing in front of the partition costs no disk
//! space.
//!
//! What it does not cover: downloading a pack. `stage_pack` is exercised by
//! hand (it needs the network), so the check starts from a local folder shaped
//! like the one an unpacked pack yields.

use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use txbm_core::fatx::{COMPAT_PARTITION, FatxConfig, FatxSession};
use txbm_core::ogxbox_compat::{self, COMPAT_PATH};
use txbm_core::remote_fs::RemoteFs;

static NO_CANCEL: AtomicBool = AtomicBool::new(false);
/// Where `install_remote` reports that it has started replacing the partition.
/// Only the GUI acts on it — here it is a sink.
static WROTE: AtomicBool = AtomicBool::new(false);

/// Geometry of the filesystem written into the image, matching
/// `fatx_target_check`: any power-of-two cluster size the format allows would
/// do, and 16 KiB is what a real console drive uses.
const SECTOR_SIZE: u64 = 512;
const SECTORS_PER_CLUSTER: u32 = 32;
const BYTES_PER_CLUSTER: u64 = SECTOR_SIZE * SECTORS_PER_CLUSTER as u64;
/// The compatibility partition is exactly this big on a console drive.
const PARTITION_SIZE: u64 = 0x1000_0000;
const SUPERBLOCK_SIZE: usize = 4096;
const FAT_OFFSET: u64 = 4096;
const SIGNATURE: u32 = 0x5854_4146;
const ROOT_CLUSTER: u32 = 1;
/// The FAT reserves one entry before the first usable cluster.
const FAT_RESERVED_ENTRIES: u64 = 1;

fn main() -> anyhow::Result<()> {
    let image = std::env::temp_dir().join("txbm-ogxbox-compat-check.img");
    let offset = fatx::PartitionMapEntry::from_x360_name(COMPAT_PARTITION)
        .expect("the compat partition is part of the 360 map")
        .offset_bytes;
    write_blank_filesystem(&image, offset)?;
    println!("image: {} (compat partition at {offset:#x})", image.display());

    assert_reference_geometry(&image, offset)?;

    // What the tool does before offering a disk: the *data* partition is blank
    // here, so only the compat probe may say yes.
    let data_probe = txbm_core::fatx_dev::probe_path(&image);
    let compat_probe = txbm_core::fatx_dev::probe_partition(&image, COMPAT_PARTITION);
    println!("probe: data={data_probe:?}  compat={compat_probe:?}");
    assert!(
        compat_probe.is_usable(),
        "the compatibility partition was not recognized"
    );

    // ── A blank partition ────────────────────────────────────────────────
    let mut session = open(&image, false)?;
    let devices = session.list_root()?;
    println!("volumes: {devices:?}");
    assert_eq!(devices, vec!["HddX".to_string()], "wrong device name");

    assert!(
        !ogxbox_compat::inspect_remote(&mut session)?,
        "a blank partition reported files"
    );
    println!("before: nothing installed");
    session.quit()?;

    // ── First install ────────────────────────────────────────────────────
    let work = std::env::temp_dir().join("txbm-ogxbox-compat-check");
    let _ = std::fs::remove_dir_all(&work);
    let first = fake_pack(&work.join("first"), &["xbox.xex", "xefu.xex", "xefu7.xex"])?;

    let mut session = open(&image, true)?;
    ogxbox_compat::install_remote(
        &mut session,
        &first,
        &NO_CANCEL,
        &WROTE,
        &mut |_done, _total| {},
        &mut |_sent, _total, _speed| {},
    )?;
    session.quit()?;

    let mut session = open(&image, false)?;
    assert!(
        ogxbox_compat::inspect_remote(&mut session)?,
        "the files were not installed"
    );
    // Three files at the root plus the two dash stubs `fake_pack` plants.
    let installed = list_remote(&mut session, COMPAT_PATH);
    println!("after first install: {installed:?}");
    assert_eq!(installed.len(), 5, "unexpected file count");
    session.quit()?;

    // ── Back up, then replace with a different set ───────────────────────
    // The backup is a zip, and deliberately shaped like a published pack: its
    // entries sit under `Compatibility/`, so restoring one later is the very
    // same `stage_pack` path, pointed at a local file.
    let backup = work.join("backup.zip");
    let mut session = open(&image, false)?;
    let saved_files =
        ogxbox_compat::backup_remote(&mut session, &backup, &NO_CANCEL, &mut |_d, _t| {})?;
    session.quit()?;
    assert_eq!(saved_files, 5, "unexpected number of files in the backup");
    let saved = list_zip(&backup)?;
    println!("backup: {saved:?}");
    assert!(
        saved.contains(&"Compatibility/xefu7.xex".to_string())
            && saved.contains(&"Compatibility/dash/xboxdash.xbe".to_string()),
        "the backup is missing files"
    );

    // The restore path accepts it, and refuses an archive that is not one.
    assert!(
        ogxbox_compat::zip_holds_compatibility(&backup)?,
        "a backup must be recognised as restorable"
    );
    let not_a_backup = work.join("not-a-backup.zip");
    write_zip(&not_a_backup, &["Aurora/Aurora.xex", "launch.ini"])?;
    assert!(
        !ogxbox_compat::zip_holds_compatibility(&not_a_backup)?,
        "an unrelated archive must be refused"
    );
    let empty_folder = work.join("empty-folder.zip");
    write_zip(&empty_folder, &["Compatibility/"])?;
    assert!(
        !ogxbox_compat::zip_holds_compatibility(&empty_folder)?,
        "an empty Compatibility folder is not a backup"
    );
    println!("validation: backup accepted, unrelated and empty archives refused");

    // And staging it yields the same folder a downloaded pack would.
    let from_backup = ogxbox_compat::stage_backup(&backup, &NO_CANCEL, &|_| {})?;
    println!("restorable from: {}", from_backup.display());
    assert_eq!(list_local(&from_backup).len(), 5, "the backup lost files");

    // The second set drops `xefu7.xex`: the point of deleting before writing
    // is that a leftover emulator from the previous pack must not survive.
    let second = fake_pack(&work.join("second"), &["xbox.xex", "xefu.xex"])?;
    let mut session = open(&image, true)?;
    ogxbox_compat::install_remote(
        &mut session,
        &second,
        &NO_CANCEL,
        &WROTE,
        &mut |_done, _total| {},
        &mut |_sent, _total, _speed| {},
    )?;
    session.quit()?;

    let mut session = open(&image, false)?;
    let installed = list_remote(&mut session, COMPAT_PATH);
    println!("after second install: {installed:?}");
    assert!(
        !installed.iter().any(|n| n.ends_with("xefu7.xex")),
        "a file from the previous pack survived the replacement"
    );
    assert_eq!(installed.len(), 4, "unexpected file count");
    session.quit()?;

    let _ = std::fs::remove_dir_all(&work);
    let _ = std::fs::remove_file(&image);
    println!("\nok");
    Ok(())
}

/// Size of the FAT as `libfatx` — and therefore the console — computes it.
fn reference_fat_size() -> u64 {
    ((PARTITION_SIZE / BYTES_PER_CLUSTER + FAT_RESERVED_ENTRIES) * 2).next_multiple_of(4096)
}

/// Checks that the reader looks for the root directory where the console
/// actually wrote it.
///
/// This is not paranoia: the reserved FAT entry is normally swallowed by the
/// rounding to a 4 KiB page, so dropping it changes nothing on most
/// partitions — but the compatibility partition is 256 MiB of 16 KiB clusters,
/// i.e. 16384 entries, i.e. exactly eight pages, so there the whole cluster
/// area moves by a page. Everything then reads as empty, and a write would
/// land a page away from the real data. Letting this example build its image
/// with the reader's own arithmetic would make it blind to exactly that, so a
/// directory entry is planted at the reference offset instead and the reader
/// is asked to find it.
fn assert_reference_geometry(image: &Path, offset: u64) -> anyhow::Result<()> {
    const NAME: &[u8] = b"GEOMETRY";

    let mut entry = vec![0xffu8; 64];
    entry[0] = NAME.len() as u8;
    entry[1] = 0x10; // directory
    entry[2..2 + NAME.len()].copy_from_slice(NAME);
    entry[44..48].copy_from_slice(&2u32.to_be_bytes()); // first cluster
    entry[48..52].copy_from_slice(&0u32.to_be_bytes()); // size
    // End-of-directory marker right after it.
    entry.extend_from_slice(&[0xffu8; 64]);

    let root_at = offset + FAT_OFFSET + reference_fat_size();
    let mut file = std::fs::OpenOptions::new().write(true).open(image)?;
    file.seek(SeekFrom::Start(root_at))?;
    file.write_all(&entry)?;
    file.sync_all()?;
    drop(file);

    let mut session = open(image, false)?;
    let names: Vec<String> = session
        .list_dir("/HddX")
        .into_iter()
        .map(|e| e.name)
        .collect();
    session.quit()?;
    println!("geometry: root at {root_at:#x} -> {names:?}");
    assert!(
        names.iter().any(|n| n == "GEOMETRY"),
        "the reader did not find the root directory where the console writes it \
         ({root_at:#x}): its cluster area is off. Check that the `fatx` crate adds the \
         reserved FAT entry before sizing the FAT, as libfatx does."
    );

    // Back to a blank partition for the rest of the checks.
    write_blank_filesystem(image, offset)
}

fn open(image: &Path, writable: bool) -> anyhow::Result<FatxSession> {
    let config = FatxConfig::for_partition(image.to_path_buf(), COMPAT_PARTITION);
    FatxSession::open(&config, writable)
}

/// Builds a local folder shaped like the `Compatibility` folder inside an
/// unpacked XeFu pack: the named emulator files at the root, plus the `dash`
/// sub-tree every pack carries (so the copy is not flat).
fn fake_pack(dir: &Path, files: &[&str]) -> anyhow::Result<std::path::PathBuf> {
    let root = dir.join("Compatibility");
    std::fs::create_dir_all(root.join("dash/xodash"))?;
    for (i, name) in files.iter().enumerate() {
        std::fs::write(root.join(name), vec![b'x'; 4096 * (i + 1)])?;
    }
    std::fs::write(root.join("dash/xboxdash.xbe"), b"stub")?;
    std::fs::write(root.join("dash/xodash/xonlinedash.xbe"), b"stub")?;
    Ok(root)
}

/// Every file under a directory on the target, as slash-separated paths
/// relative to it.
fn list_remote(session: &mut FatxSession, dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    for entry in session.list_dir(dir) {
        let child = format!("{dir}/{}", entry.name);
        if entry.is_dir {
            out.extend(
                list_remote(session, &child)
                    .into_iter()
                    .map(|p| format!("{}/{p}", entry.name)),
            );
        } else {
            out.push(entry.name);
        }
    }
    out.sort();
    out
}

/// Writes a zip holding the named entries, empty files unless the name ends in
/// a slash. Used to check what the restore path refuses.
fn write_zip(path: &Path, names: &[&str]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path)?);
    let options = zip::write::SimpleFileOptions::default();
    for name in names {
        if let Some(dir) = name.strip_suffix('/') {
            zip.add_directory(dir, options)?;
        } else {
            zip.start_file(*name, options)?;
            zip.write_all(b"x")?;
        }
    }
    zip.finish()?;
    Ok(())
}

/// Every file inside a zip archive, as the archive spells its paths.
fn list_zip(path: &Path) -> anyhow::Result<Vec<String>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i)?;
        if !entry.is_dir() {
            out.push(entry.name().to_string());
        }
    }
    out.sort();
    Ok(out)
}

/// Every file under `dir`, as slash-separated paths relative to it.
fn list_local(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if entry.path().is_dir() {
                walk(&entry.path(), &path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out);
    out.sort();
    out
}

fn write_blank_filesystem(path: &Path, offset: u64) -> anyhow::Result<()> {
    let _ = std::fs::remove_file(path);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    // Sparse: nothing before the partition is ever written, so it costs no
    // space on disk however far in the partition starts.
    file.set_len(offset + PARTITION_SIZE)?;

    let mut superblock = Vec::new();
    // Written big-endian, which is what makes this a 360 filesystem rather
    // than an original Xbox one.
    superblock.extend_from_slice(&SIGNATURE.to_be_bytes());
    for value in [0xcafe_babeu32, SECTORS_PER_CLUSTER, ROOT_CLUSTER] {
        superblock.extend_from_slice(&value.to_be_bytes());
    }
    superblock.extend_from_slice(&0u16.to_be_bytes());
    superblock.resize(SUPERBLOCK_SIZE, 0xff);
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&superblock)?;

    // Sized exactly as `libfatx` does it, which is what the console itself
    // wrote: the cluster count carries one reserved entry *before* being
    // widened to the FAT's entry size and rounded up to a 4 KiB page. That
    // reserved entry is easy to overlook because the rounding usually swallows
    // it — but on a partition whose entries land on a page boundary (the
    // 256 MiB compatibility partition, at 16 KiB clusters, is exactly that) it
    // moves the whole cluster area by a page. Mirroring the reader's own
    // arithmetic here instead would make this check blind to that class of bug.
    let fat_size = reference_fat_size() as usize;
    let mut fat = vec![0u8; fat_size];
    fat[0..2].copy_from_slice(&0xfff8u16.to_be_bytes());
    fat[2..4].copy_from_slice(&0xffffu16.to_be_bytes());
    file.seek(SeekFrom::Start(offset + FAT_OFFSET))?;
    file.write_all(&fat)?;

    let cluster_offset = offset + FAT_OFFSET + fat_size as u64;
    file.seek(SeekFrom::Start(cluster_offset))?;
    file.write_all(&[0xffu8])?;
    file.sync_all()?;
    Ok(())
}
