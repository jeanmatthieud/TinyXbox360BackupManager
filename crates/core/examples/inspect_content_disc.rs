// SPDX-License-Identifier: GPL-3.0-only
//! Lists the file tree of an ISO (content disc / DLC or otherwise) without
//! extracting it, and inspects any STFS package header found in it:
//! `cargo run -p txbm-core --example inspect_content_disc -- <iso path>`

use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use txbm_core::stfs;
use txbm_core::xdvd::XdvdImage;
use xdvdfs::blockdev::OffsetWrapper;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: inspect_content_disc <iso path>");
    let path = PathBuf::from(path);

    let mut image = XdvdImage::open(&path)?;
    println!("volume offset: {:#x}", image.root_offset()?);
    println!("used size:     {}", image.max_used_prefix_size()?);

    // The listing needs the raw file tree, which `XdvdImage` does not expose.
    let file = std::fs::File::open(&path).context("opening ISO")?;
    let mut dev = OffsetWrapper::new(std::io::BufReader::new(file))
        .map_err(|e| anyhow!("invalid XDVDFS image: {e}"))?;
    let volume = xdvdfs::read::read_volume(&mut dev)
        .map_err(|e| anyhow!("reading XDVDFS volume: {e}"))?;
    let tree = volume
        .root_table
        .file_tree(&mut dev)
        .map_err(|e| anyhow!("reading file tree: {e}"))?;

    for (dir, node) in &tree {
        if node.node.dirent.is_directory() {
            continue;
        }
        let name = node
            .name_str::<std::io::Error>()
            .map_err(|e| anyhow!("invalid file name: {e}"))?;
        let entry_path = format!("{dir}/{name}");
        let size = node.node.dirent.data.size();
        println!("{size:12} {entry_path}");

        // A STFS header always sits in the first few KiB: no need to read
        // the whole (possibly huge) package.
        let Ok(buf) = image.read_prefix(node, 0x2000) else {
            continue;
        };
        let mut cursor = std::io::Cursor::new(buf);
        if let Ok(Some(info)) = stfs::inspect_reader(&mut cursor, entry_path.clone().into()) {
            println!(
                "    -> STFS: content_type={:08X} title_id={} media_id={} disc={}/{} \
                 display_name={:?} title_name={:?}",
                info.content_type,
                info.title_id,
                info.media_id,
                info.disc_number,
                info.disc_in_set,
                info.display_name,
                info.title_name
            );
        }
    }

    Ok(())
}
