// SPDX-License-Identifier: GPL-3.0-only

//! Manual check: installs an XBLA input (archive .7z/.zip or bare STFS
//! package) into a local target folder, then rescans it.
//!
//! Usage: cargo run --example xbla_check -- <input> <target-dir>

use anyhow::{Context, Result};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = args.next().context("usage: xbla_check <input> <target-dir>")?;
    let target = args.next().context("usage: xbla_check <input> <target-dir>")?;

    let mut config = txbm_core::config::Config {
        path: std::env::temp_dir().join("txbm-xbla-check.json"),
        contents: Default::default(),
    };
    config.contents.target_kind = txbm_core::config::TargetKind::Local;
    config.contents.mount_point = target.clone().into();

    println!("installing {input} into {target}…");
    txbm_core::convert::perform(input.into(), &config, &|p, _| println!("  {p}%"))?;

    println!("rescan:");
    for game in txbm_core::game::scan_drive(std::path::Path::new(&target)) {
        println!(
            "- [{}] {} ({}, {} bytes) @ {}",
            game.id,
            game.title,
            game.format.label(),
            game.size,
            game.path.display()
        );
    }

    Ok(())
}
