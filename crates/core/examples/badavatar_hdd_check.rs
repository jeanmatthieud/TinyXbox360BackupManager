// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification of the BadAvatar hard-drive install, end to end, against
//! a disk image built here rather than against a real console drive:
//! `cargo run -p txbm-core --example badavatar_hdd_check [-- --with-aurora]`
//!
//! Downloads the real components (ABadAvatar, XeUnshackle), installs them on a
//! blank FATX image that already holds an Aurora and a game, checks what landed
//! there, then removes it and checks the drive is back to where it was. With
//! `--with-aurora` the image starts without Aurora, so the install also has to
//! download and write it (from its default URL), and the removal to take it away.

use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::AtomicBool;
use txbm_core::badavatar::BadAvatarConfig;
use txbm_core::badavatar_hdd::{self, HddStatus};
use txbm_core::fatx::FatxConfig;
use txbm_core::remote_fs::RemoteFs;
use txbm_core::target::Target;

static NO_CANCEL: AtomicBool = AtomicBool::new(false);

const SECTOR_SIZE: u64 = 512;
const SECTORS_PER_CLUSTER: u32 = 32;
const BYTES_PER_CLUSTER: u64 = SECTOR_SIZE * SECTORS_PER_CLUSTER as u64;
const PARTITION_SIZE: u64 = 512 * 1024 * 1024;
const SUPERBLOCK_SIZE: usize = 4096;
const FAT_OFFSET: u64 = 4096;
const SIGNATURE: u32 = 0x5854_4146;
const ROOT_CLUSTER: u32 = 1;

const GAME_DIR: &str = "/Hdd1/Content/0000000000000000/4D5307D5/00007000";

fn main() -> anyhow::Result<()> {
    let with_aurora = std::env::args().any(|a| a == "--with-aurora");
    let cfg = BadAvatarConfig::default();

    let image = std::env::temp_dir().join("txbm-badavatar-hdd-check.img");
    let offset = fatx::PartitionMapEntry::from_x360_name("data")
        .expect("the data partition is part of the 360 map")
        .offset_bytes;
    write_blank_filesystem(&image, offset)?;
    let target = Target::Fatx(FatxConfig::new(image.clone()));
    println!("image: {}", image.display());

    // A game, which nothing here may touch, and an Aurora of the user's own
    // unless the install is to bring one.
    {
        let mut s = target.open_remote(true)?;
        s.put_bytes(GAME_DIR, "0000000000000000000000000000", &[0x42; 4096])?;
        if !with_aurora {
            s.put_bytes("/Hdd1/Aurora", "Aurora.xex", b"not really aurora")?;
        }
        s.quit()?;
    }

    let before = badavatar_hdd::inspect(&target)?;
    println!("before: {:?}, aurora {:?}", before.status, before.aurora);
    assert!(matches!(before.status, HddStatus::Retail));
    assert_eq!(before.aurora.is_some(), !with_aurora);

    // Cancelled before the first write: nothing may be on the drive after.
    let cancel = AtomicBool::new(true);
    let res = badavatar_hdd::install(&target, &before, &cfg, &cancel, &|_| {}, &|| {
        panic!("a cancelled install must not start writing")
    });
    println!("cancelled install: {:?}", res.as_ref().err().map(|e| e.to_string()));
    assert!(res.is_err());
    assert!(matches!(badavatar_hdd::inspect(&target)?.status, HddStatus::Retail));

    // The install itself.
    let status = |line: &str| {
        if !line.is_empty() {
            println!("  · {line}");
        }
    };
    let wrote = std::cell::Cell::new(false);
    badavatar_hdd::install(&target, &before, &cfg, &NO_CANCEL, &status, &|| {
        wrote.set(true)
    })?;
    assert!(wrote.get());

    let after = badavatar_hdd::inspect(&target)?;
    println!("after install: {:?}", after.status);
    assert!(matches!(after.status, HddStatus::Installed { complete: true, .. }));

    let mut s = target.open_remote(false)?;
    let root: Vec<String> = s.list_dir("/Hdd1").into_iter().map(|e| e.name).collect();
    println!("root: {root:?}");
    let payload: Vec<String> = s
        .list_dir("/Hdd1/BadUpdatePayload")
        .into_iter()
        .map(|e| e.name)
        .collect();
    println!("payload: {payload:?}");
    for file in ["GamerProfile.xex", "default.xex", "BadUpdateExploit-Avatar-Data.bin"] {
        assert!(payload.iter().any(|p| p == file), "{file} missing from the payload");
    }
    assert_eq!(
        s.sha1_file("/Hdd1/BadUpdatePayload/GamerProfile.xex")?,
        "d2eaaa5c3b5b85ba98252d93b80c0521047d71e6"
    );
    let profile = s.download_file(
        "/Hdd1/Content/E0002FF78DFBDE7B/FFFE07D1/00010000/E0002FF78DFBDE7B",
    )?;
    assert_eq!(&profile[..4], b"CON ");
    let ini = String::from_utf8(s.download_file("/Hdd1/launch.ini")?)?;
    for line in ini.lines().filter(|l| !l.trim_start().starts_with(';')) {
        assert!(!line.to_ascii_lowercase().contains("usb:\\"), "left on USB: {line}");
        if line.to_ascii_lowercase().starts_with("default") || line.starts_with("plugin") {
            println!("  launch.ini: {line}");
        }
    }
    assert!(ini.contains("Default = Hdd:\\Aurora\\Aurora.xex"));
    // Every plugin left in launch.ini is on the drive: Dashlaunch crashes the
    // console on one that isn't.
    let root: Vec<String> = s.list_dir("/Hdd1").into_iter().map(|e| e.name.to_ascii_lowercase()).collect();
    for line in ini.lines().filter(|l| l.trim_start().starts_with("plugin")) {
        let value = line.split_once('=').unwrap().1.split(';').next().unwrap().trim();
        if let Some(file) = value.strip_prefix("Hdd:\\") {
            assert!(root.contains(&file.to_ascii_lowercase()), "plugin not on the drive: {line}");
        }
    }
    if !with_aurora {
        // The user's own Aurora was not replaced.
        assert_eq!(s.download_file("/Hdd1/Aurora/Aurora.xex")?, b"not really aurora");
    }
    s.quit()?;

    // Installing again is refused, even from the stale look taken before.
    assert!(
        badavatar_hdd::install(&target, &after, &cfg, &NO_CANCEL, &|_| {}, &|| {})
            .is_err()
    );
    let res = badavatar_hdd::install(&target, &before, &cfg, &NO_CANCEL, &|_| {}, &|| {});
    println!("install over our own: {:?}", res.as_ref().err().map(|e| e.to_string()));
    assert!(res.is_err());

    // What XeUnshackle writes by itself on a console with a hard drive.
    {
        let mut s = target.open_remote(true)?;
        s.put_bytes("/Hdd1", "lhelper.xex", b"dashlaunch helper")?;
        s.put_bytes("/Hdd1/BadUpdatePayload", "MAC_backup.bin", b"mac")?;
        s.quit()?;
    }

    let listed = badavatar_hdd::inspect(&target)?;
    println!("removal list:");
    for item in &listed.removal {
        println!("  - {item}");
    }

    badavatar_hdd::uninstall(&target, &status)?;
    let gone = badavatar_hdd::inspect(&target)?;
    println!("after removal: {:?}, aurora {:?}", gone.status, gone.aurora);
    assert!(matches!(gone.status, HddStatus::Retail));
    // Aurora stays exactly when it was the user's.
    assert_eq!(gone.aurora.is_some(), !with_aurora);

    let mut s = target.open_remote(false)?;
    let root: Vec<String> = s.list_dir("/Hdd1").into_iter().map(|e| e.name).collect();
    println!("root: {root:?}");
    assert!(!root.iter().any(|n| n.eq_ignore_ascii_case("lhelper.xex")));
    let content: Vec<String> = s
        .list_dir("/Hdd1/Content")
        .into_iter()
        .map(|e| e.name)
        .collect();
    println!("content: {content:?}");
    assert_eq!(content, vec!["0000000000000000".to_string()]);
    assert_eq!(s.download_file(&format!("{GAME_DIR}/0000000000000000000000000000"))?, vec![0x42; 4096]);
    s.quit()?;

    // An Aurora folder without Aurora.xex is not taken over.
    if with_aurora {
        {
            let mut s = target.open_remote(true)?;
            s.put_bytes("/Hdd1/Aurora/Data", "settings.db", b"the user's")?;
            s.quit()?;
        }
        let stray = badavatar_hdd::inspect(&target)?;
        println!("with a stray Aurora folder: {:?}", stray.stray_aurora);
        assert!(stray.stray_aurora.is_some());
        assert!(
            badavatar_hdd::install(&target, &stray, &cfg, &NO_CANCEL, &|_| {}, &|| {
                panic!("a stray Aurora folder must stop the install before it writes")
            })
            .is_err()
        );
    }

    // Someone else's launch.ini makes the drive foreign, and blocks an install.
    {
        let mut s = target.open_remote(true)?;
        s.put_bytes("/Hdd1", "launch.ini", b"[Paths]\r\n")?;
        s.quit()?;
    }
    let foreign = badavatar_hdd::inspect(&target)?;
    println!("with a stray launch.ini: {:?}", foreign.status);
    assert!(matches!(foreign.status, HddStatus::Foreign { .. }));
    assert!(badavatar_hdd::uninstall(&target, &|_| {}).is_err());

    let _ = std::fs::remove_file(&image);
    println!("OK");
    Ok(())
}

fn write_blank_filesystem(path: &std::path::Path, offset: u64) -> anyhow::Result<()> {
    let _ = std::fs::remove_file(path);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    file.set_len(offset + PARTITION_SIZE)?;

    let mut superblock = Vec::new();
    superblock.extend_from_slice(&SIGNATURE.to_be_bytes());
    for value in [0xcafe_babeu32, SECTORS_PER_CLUSTER, ROOT_CLUSTER] {
        superblock.extend_from_slice(&value.to_be_bytes());
    }
    superblock.extend_from_slice(&0u16.to_be_bytes());
    superblock.resize(SUPERBLOCK_SIZE, 0xff);
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&superblock)?;

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
