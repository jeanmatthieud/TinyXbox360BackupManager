// SPDX-License-Identifier: GPL-3.0-only
//! Vérification manuelle de la détection + extraction :
//! `cargo run -p txbm-core --example extract_check -- <image.iso> <dest>`

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let iso = PathBuf::from(args.next().expect("usage: extract_check <iso> <dest>"));
    let dest = PathBuf::from(args.next().expect("usage: extract_check <iso> <dest>"));

    let info = txbm_core::iso_info::inspect(&iso)?;
    println!("détection : {:?} ({})", info.kind, info.kind.label());
    println!("title_id={:?} media_id={:?} name={:?}", info.title_id, info.media_id, info.name);

    txbm_core::extract::extract_iso(&iso, &dest, &mut |done, total| {
        println!("extraction {done}/{total}");
    })?;
    Ok(())
}
