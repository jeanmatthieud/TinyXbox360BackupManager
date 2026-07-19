// SPDX-License-Identifier: GPL-3.0-only
//! Lists the file tree of an ISO (content disc / DLC or otherwise) without
//! extracting it, and inspects any STFS package header found in it:
//! `cargo run -p txbm-core --example inspect_content_disc -- <iso path>`

use anyhow::{Context, Result};
use iso2god::iso::{self, DirectoryTable};
use std::fs::File;
use std::io::Read;
use txbm_core::stfs;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: inspect_content_disc <iso path>");

    let file = File::open(&path).context("opening ISO")?;
    let mut reader = iso::IsoReader::read(file).context("reading ISO (invalid XDVDFS?)")?;

    println!("{:?}", reader.volume_descriptor);

    let mut files = Vec::new();
    collect_files(String::new(), &reader.directory_table, &mut files);

    for (entry_path, size) in &files {
        println!("{size:12} {entry_path}");

        let windows_path: iso::WindowsPath = entry_path.as_str().into();
        let Ok(Some(reader_ref)) = reader.get_entry(&windows_path) else {
            continue;
        };
        // A STFS header always sits in the first few KiB: no need to read
        // the whole (possibly huge) package.
        let header_len = (*size as usize).min(0x2000);
        let mut buf = vec![0u8; header_len];
        if reader_ref.read_exact(&mut buf).is_err() {
            continue;
        }
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

fn collect_files(path: String, dir: &DirectoryTable, out: &mut Vec<(String, u32)>) {
    for entry in &dir.entries {
        let entry_path = format!("{path}\\{}", entry.name);
        if let Some(subdir) = &entry.subdirectory {
            collect_files(entry_path, subdir, out);
        } else {
            out.push((entry_path, entry.size));
        }
    }
}
