// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification of a console drive formatted for Bad Storage, against a
//! disk image built here:
//! `cargo run -p txbm-core --example fatx_bad_storage_check [-- <image path>]`
//!
//! FATXplorer formats such a drive with a root cluster of 0 in the `data`
//! partition's superblock, plus a `BSTORAGE` marker at 0x858; the stock kernel
//! takes the root directory for corrupt until Bad Storage patches it to 1. The
//! app must use the drive as a FATX target all the same (add and delete games)
//! while refusing to install BadAvatar on it, and must never write that 0 back.
//!
//! The image is left behind, blank again, so it can be opened in the app with
//! "Pick FATX image". It is sparse: its `data` partition spans 16 GiB — enough
//! for a FAT32, like any real Bad Storage drive — at almost no cost on disk.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use txbm_core::badavatar_hdd::{self, HddStatus};
use txbm_core::fatx::FatxConfig;
use txbm_core::remote_fs::RemoteFs;
use txbm_core::target::{StorageConfig, Target};

static NO_CANCEL: AtomicBool = AtomicBool::new(false);

const SECTOR_SIZE: u64 = 512;
const SECTORS_PER_CLUSTER: u32 = 32;
const BYTES_PER_CLUSTER: u64 = SECTOR_SIZE * SECTORS_PER_CLUSTER as u64;
const PARTITION_SIZE: u64 = 16 * 1024 * 1024 * 1024;
const SUPERBLOCK_SIZE: usize = 4096;
const FAT_OFFSET: u64 = 4096;
const SIGNATURE: u32 = 0x5854_4146;
/// What FATXplorer records for a Bad Storage partition.
const ROOT_CLUSTER: u32 = 0;
const MARKER: &[u8; 8] = b"BSTORAGE";
const MARKER_OFFSET: usize = 0x858;

fn main() -> anyhow::Result<()> {
    let image = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("txbm-bad-storage.img"));
    let partition_offset = fatx::PartitionMapEntry::from_x360_name("data")
        .expect("the data partition is part of the 360 map")
        .offset_bytes;
    write_bad_storage_filesystem(&image, partition_offset)?;
    println!("image: {} (partition at {partition_offset:#x})", image.display());

    let probe = txbm_core::fatx_dev::probe_path(&image);
    println!("probe: {probe:?} — {}", probe.label());
    assert!(probe.is_usable(), "the image was not recognized");
    assert!(txbm_core::fatx_dev::is_bad_storage(&image));

    // BadAvatar: reported as incompatible, and refused.
    let target = Target::Fatx(FatxConfig::new(image.clone()));
    let inspection = badavatar_hdd::inspect(&target)?;
    println!("BadAvatar: {:?}", inspection.status);
    assert!(matches!(inspection.status, HddStatus::BadStorage));
    let refusal = badavatar_hdd::ensure_installable(&inspection).unwrap_err();
    println!("install refused: {refusal}");
    assert!(badavatar_hdd::uninstall(&target, &|_| {}).is_err());

    // A FATX target like any other.
    let analysis = target.analyze()?;
    assert!(!analysis.already_configured);
    let storage = StorageConfig {
        god_dir: analysis.suggested.god_dir.clone(),
        xbe_dir: analysis.suggested.xbe_dir.clone(),
        xex_dir: analysis.suggested.xex_dir.clone(),
        god_layout: analysis.suggested.god_layout,
    };
    target.apply_storage(&storage)?;
    assert!(target.analyze()?.already_configured);
    println!("storage configured under {}", storage.god_dir);

    {
        let staging = std::env::temp_dir().join("txbm-bad-storage-staging/Halo 3");
        std::fs::create_dir_all(staging.join("00007000"))?;
        std::fs::write(staging.join("00007000/data"), vec![7u8; 5 * 1024 * 1024])?;
        let mut session = target.open_remote(true)?;
        let dest = format!("{}/4D530910", storage.god_dir);
        session.upload_dir(&staging, &dest, &NO_CANCEL, &mut |_, _, _| {})?;
        let copied = session.dir_size(&dest, 3);
        session.quit()?;
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("txbm-bad-storage-staging"));
        assert_eq!(copied, 5 * 1024 * 1024, "the copy is not the right size");
        println!("copied {} to the drive", txbm_core::util::human_size(copied));
    }

    let (_, info) = target.scan(&NO_CANCEL)?;
    println!(
        "drive: {} used of {}",
        txbm_core::util::human_size(info.used_bytes),
        txbm_core::util::human_size(info.total_bytes)
    );
    assert!(info.used_bytes >= 5 * 1024 * 1024);

    // What the console relies on must still be on disk after all those writes.
    let superblock = read_superblock(&image, partition_offset)?;
    assert_eq!(superblock[12..16], ROOT_CLUSTER.to_be_bytes(), "the root cluster was rewritten");
    assert_eq!(&superblock[MARKER_OFFSET..][..MARKER.len()], MARKER, "the marker was lost");
    println!("superblock untouched: root cluster 0, marker present");

    // Leave a blank drive behind for the app.
    write_bad_storage_filesystem(&image, partition_offset)?;
    println!("OK — blank Bad Storage image left at {}", image.display());
    Ok(())
}

fn read_superblock(path: &Path, offset: u64) -> anyhow::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut superblock = vec![0u8; SUPERBLOCK_SIZE];
    file.read_exact(&mut superblock)?;
    Ok(superblock)
}

/// Writes a blank Bad Storage filesystem at `offset` in a sparse image, byte by
/// byte from the format rather than through the library: a superblock with a
/// root cluster of 0 and the marker, a FAT32 holding the media descriptor and
/// the root directory (cluster 1, where Bad Storage reads it), and a root
/// cluster with a single end-of-directory marker.
fn write_bad_storage_filesystem(path: &Path, offset: u64) -> anyhow::Result<()> {
    let _ = std::fs::remove_file(path);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    // The partition runs to the end of the disk, as on a real drive.
    file.set_len(offset + PARTITION_SIZE)?;

    let mut superblock = Vec::new();
    superblock.extend_from_slice(&SIGNATURE.to_be_bytes());
    for value in [0xcafe_babeu32, SECTORS_PER_CLUSTER, ROOT_CLUSTER] {
        superblock.extend_from_slice(&value.to_be_bytes());
    }
    superblock.extend_from_slice(&0u16.to_be_bytes());
    superblock.resize(SUPERBLOCK_SIZE, 0);
    superblock[MARKER_OFFSET..][..MARKER.len()].copy_from_slice(MARKER);
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&superblock)?;

    // FAT32 from 0xfff0 entries on, the reserved one included; this partition
    // has far more.
    let entries = PARTITION_SIZE / BYTES_PER_CLUSTER + 1;
    assert!(entries >= 0xfff0);
    let fat_size = (entries * 4).next_multiple_of(4096);
    let mut head = Vec::new();
    head.extend_from_slice(&0xffff_fff8u32.to_be_bytes());
    head.extend_from_slice(&0xffff_ffffu32.to_be_bytes());
    file.seek(SeekFrom::Start(offset + FAT_OFFSET))?;
    file.write_all(&head)?;

    // Cluster 1 is the first of the cluster area.
    file.seek(SeekFrom::Start(offset + FAT_OFFSET + fat_size))?;
    file.write_all(&[0xffu8])?;
    file.sync_all()?;
    Ok(())
}
