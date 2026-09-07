// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification of the FATX target, end to end, against a disk image
//! built here rather than against a real console drive:
//! `cargo run -p txbm-core --example fatx_target_check [-- <iso to add>]`
//!
//! The image is a sparse file laid out like a console drive — a blank Xbox 360
//! filesystem at the offset the `data` partition sits at — so everything the
//! app does to a real drive (probe, storage configuration, scan, install,
//! delete) runs against it unchanged. Sparse means the 5 GiB of nothing in
//! front of the partition costs no disk space.
//!
//! Pass an ISO to also exercise the conversion and the install path.

use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::AtomicBool;
use txbm_core::config::{Config, TargetKind};
use txbm_core::fatx::FatxConfig;
use txbm_core::remote_fs::RemoteFs;
use txbm_core::target::{StorageConfig, Target};

static NO_CANCEL: AtomicBool = AtomicBool::new(false);

/// Geometry of the filesystem written into the image. Any power-of-two cluster
/// size the format allows would do; 16 KiB matches a real console drive.
const SECTOR_SIZE: u64 = 512;
const SECTORS_PER_CLUSTER: u32 = 32;
const BYTES_PER_CLUSTER: u64 = SECTOR_SIZE * SECTORS_PER_CLUSTER as u64;
const PARTITION_SIZE: u64 = 256 * 1024 * 1024;
const SUPERBLOCK_SIZE: usize = 4096;
const FAT_OFFSET: u64 = 4096;
const SIGNATURE: u32 = 0x5854_4146;
const ROOT_CLUSTER: u32 = 1;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);

    let image = std::env::temp_dir().join("txbm-fatx-target-check.img");
    let partition_offset = fatx::PartitionMapEntry::from_x360_name("data")
        .expect("the data partition is part of the 360 map")
        .offset_bytes;
    write_blank_filesystem(&image, partition_offset)?;
    println!("image: {} (partition at {partition_offset:#x})", image.display());

    // What the picker does before offering a disk.
    let probe = txbm_core::fatx_dev::probe_path(&image);
    println!("probe: {probe:?} — {}", probe.label());
    assert!(probe.is_usable(), "the image was not recognized");

    let mut config = Config::load();
    config.contents.target_kind = TargetKind::Fatx;
    config.contents.fatx = FatxConfig::new(image.clone());
    // These are the developer's own settings, and `convert::perform` obeys
    // this one by deleting the file it was given: an ISO passed on the command
    // line must survive a check run.
    config.contents.remove_sources_games = false;

    let target = Target::from_config(&config.contents).unwrap();
    println!("target: {}", target.display());

    // A blank drive has no manifest and no Aurora install, so the analysis
    // must fall back to the built-in defaults under /Hdd1.
    let analysis = target.analyze()?;
    println!(
        "analysis: configured={} god={} xbe={} xex={}",
        analysis.already_configured,
        analysis.suggested.god_dir,
        analysis.suggested.xbe_dir,
        analysis.suggested.xex_dir
    );
    assert!(!analysis.already_configured);

    // Confirming the storage configuration creates the folders and writes the
    // manifest, exactly as the modal's "Confirm" does.
    let storage = StorageConfig {
        god_dir: analysis.suggested.god_dir.clone(),
        xbe_dir: analysis.suggested.xbe_dir.clone(),
        xex_dir: analysis.suggested.xex_dir.clone(),
        god_layout: analysis.suggested.god_layout,
    };
    target.apply_storage(&storage)?;

    let analysis = target.analyze()?;
    assert!(
        analysis.already_configured,
        "the manifest was not read back from the drive"
    );
    println!("manifest written and read back");

    // Plant a GOD game by hand, the way a scan expects to find one.
    {
        let mut session = target.open_remote(true)?;
        let title_dir = format!("{}/4D5307D5", storage.god_dir);
        session.ensure_dir(&format!("{title_dir}/00007000"))?;
        session.put_bytes(
            &format!("{title_dir}/00007000"),
            "0000000000000000000000000000",
            &vec![0u8; 3 * 1024 * 1024],
        )?;
        session.quit()?;
    }

    let (games, info) = target.scan(&NO_CANCEL)?;
    println!(
        "drive: {} [{}] — {} used of {}, {} in games",
        info.label,
        info.fs_label,
        txbm_core::util::human_size(info.used_bytes),
        txbm_core::util::human_size(info.total_bytes),
        txbm_core::util::human_size(info.games_bytes)
    );
    for game in &games {
        println!(
            "- [{}] {} ({}, {} bytes) @ {}",
            game.id,
            game.title,
            game.format.label(),
            game.size,
            game.path.display()
        );
    }
    assert_eq!(games.len(), 1, "the planted game was not found");
    assert!(info.total_bytes > 0, "free space was not reported");

    // The install path: the same tree copy `convert::perform` runs once a
    // conversion has been staged locally. Run twice, since re-adding a game
    // has to overwrite what is already there rather than fail on it.
    {
        let staging = std::env::temp_dir().join("txbm-fatx-staging/Halo 3");
        std::fs::create_dir_all(staging.join("00007000"))?;
        std::fs::write(staging.join("00007000/data"), vec![7u8; 5 * 1024 * 1024])?;
        std::fs::write(staging.join("00007000/000000001.data"), vec![8u8; 1024])?;

        let mut session = target.open_remote(true)?;
        let dest = format!("{}/4D530910", storage.god_dir);
        for pass in 1..=2 {
            let mut last = 0;
            session.upload_dir(&staging, &dest, &NO_CANCEL, &mut |sent, total, speed| {
                let pct = sent * 100 / total.max(1);
                if pct != last {
                    last = pct;
                    println!("  copy pass {pass}: {pct}% ({} MB/s)", speed.unwrap_or(0.0).round());
                }
            })?;
        }
        let copied = session.dir_size(&dest, 3);
        session.quit()?;
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("txbm-fatx-staging"));
        println!("copied {} to the drive", txbm_core::util::human_size(copied));
        assert_eq!(copied, 5 * 1024 * 1024 + 1024, "the copy is not the right size");
    }

    if let Some(iso) = args.next() {
        println!("adding {iso}…");
        txbm_core::convert::perform(
            iso.into(),
            &config,
            &NO_CANCEL,
            &|p, _| println!("  {p}%"),
            &|s| println!("  {s}"),
            &|s| println!("  [{s}]"),
        )?;
        let (games, _) = target.scan(&NO_CANCEL)?;
        println!("after the install: {} game(s)", games.len());
        for game in &games {
            println!("- [{}] {} ({})", game.id, game.title, game.format.label());
        }
    }

    // Deletion, and the space it gives back.
    let (games, before) = target.scan(&NO_CANCEL)?;
    for game in &games {
        println!("deleting {}…", game.title);
        target.delete_game(game, &NO_CANCEL, &|p| println!("  delete {p}%"))?;
    }
    let (games, after) = target.scan(&NO_CANCEL)?;
    println!(
        "after deletion: {} game(s), {} freed",
        games.len(),
        txbm_core::util::human_size(after.total_bytes.saturating_sub(after.used_bytes)
            - before.total_bytes.saturating_sub(before.used_bytes))
    );
    assert!(games.is_empty(), "a game survived its deletion");
    assert!(
        after.used_bytes < before.used_bytes,
        "deleting freed no space"
    );

    println!("OK");
    Ok(())
}

/// Writes a blank Xbox 360 filesystem at `offset` in a sparse image: a
/// superblock, a FAT holding nothing but the media descriptor and the root
/// directory, and a root cluster holding a single end-of-directory marker.
/// This is the on-disk layout the format specifies, written by hand rather
/// than through the library — the point is to check the app against a disk it
/// did not itself produce.
fn write_blank_filesystem(path: &std::path::Path, offset: u64) -> anyhow::Result<()> {
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

    // FAT16 as long as the partition holds fewer than 0xfff0 clusters, which
    // this one does; the library sizes its own copy the same way.
    let entries = PARTITION_SIZE / BYTES_PER_CLUSTER;
    let fat_size = ((entries * 2).next_multiple_of(4096)) as usize;
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
