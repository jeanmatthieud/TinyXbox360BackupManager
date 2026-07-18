// SPDX-License-Identifier: GPL-3.0-only
//! Manual verification of Original Xbox covers (MobCat database):
//! `cargo run -p txbm-core --example og_cover_check -- <TitleID hex> [dest dir]`
//! Also checks the folder-name TitleID suffix helpers.

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let title_id = args.next().expect("usage: og_cover_check <TitleID> [dest]");
    let dest = PathBuf::from(args.next().unwrap_or_else(|| ".".into()));

    let name = txbm_core::game::og_folder_name(
        "Some Very Long Game Title That Overflows FATX",
        &title_id,
    );
    println!("folder name: {name:?} ({} chars)", name.chars().count());
    println!("parsed back: {:?}", txbm_core::game::split_title_id_suffix(&name));

    println!("updating MobCat database…");
    txbm_core::mobcat::ensure_db();
    println!("database: {}", txbm_core::mobcat::db_path().display());

    let downloaded = txbm_core::covers::download_cover(&dest, &title_id, false)?;
    println!(
        "cover: {:?} (downloaded: {downloaded})",
        txbm_core::covers::cached_cover(&dest, &title_id)
    );
    Ok(())
}
