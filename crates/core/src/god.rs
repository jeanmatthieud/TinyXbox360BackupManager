// SPDX-License-Identifier: GPL-3.0-only
// Conversion pipeline adapted from the iso2god binary (https://github.com/iliazeus/iso2god-rs).

use anyhow::{Context, Result};
use iso2god::executable::TitleInfo;
use iso2god::{game_list, god, iso};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Converts an Xbox 360 game ISO to GOD in `content_dir`
/// (the `Content/0000000000000000` folder of the target).
/// Returns the created title folder (`content_dir/<TitleID>`).
///
/// `progress(done, total)` is called at each written part.
pub fn convert_to_god(
    source_iso: &Path,
    content_dir: &Path,
    game_title: Option<&str>,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf> {
    let source_iso_file =
        File::open(source_iso).context("opening source ISO")?;
    let source_iso_file_meta =
        fs::metadata(source_iso).context("reading ISO metadata")?;

    let mut iso_reader =
        iso::IsoReader::read(source_iso_file).context("reading source ISO")?;

    let title_info =
        TitleInfo::from_image(&mut iso_reader).context("reading game executable")?;
    let exe_info = title_info.execution_info;
    let content_type = title_info.content_type;

    // Remove unused space at the end of the image (equivalent to --trim=from-end).
    let data_size = iso_reader
        .get_max_used_prefix_size()
        .min(source_iso_file_meta.len() - iso_reader.volume_descriptor.root_offset);

    let block_count = data_size.div_ceil(god::BLOCK_SIZE);
    let part_count = block_count.div_ceil(god::BLOCKS_PER_PART);

    let file_layout = god::FileLayout::new(content_dir, &exe_info, content_type);

    let data_dir = file_layout.data_dir_path();
    if fs::exists(&data_dir)? {
        fs::remove_dir_all(&data_dir)?;
    }
    fs::create_dir_all(&data_dir).context("creating GOD data folder")?;

    progress(0, part_count);

    for part_index in 0..part_count {
        let mut iso_data_volume = File::open(source_iso)?;
        iso_data_volume.seek(SeekFrom::Start(iso_reader.volume_descriptor.root_offset))?;

        let part_file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(file_layout.part_file_path(part_index))
            .context("creating GOD part file")?;

        god::write_part(iso_data_volume, part_index, part_file)
            .context("writing GOD part file")?;

        progress(part_index + 1, part_count);
    }

    // MHT hash chain, from the last part to the first.
    let mut mht =
        read_part_mht(&file_layout, part_count - 1).context("reading a part's MHT")?;

    for prev_part_index in (0..part_count - 1).rev() {
        let mut prev_mht = read_part_mht(&file_layout, prev_part_index)
            .context("reading a part's MHT")?;
        prev_mht.add_hash(&mht.digest());
        write_part_mht(&file_layout, prev_part_index, &prev_mht)
            .context("writing a part's MHT")?;
        mht = prev_mht;
    }

    let last_part_size = fs::metadata(file_layout.part_file_path(part_count - 1))
        .map(|m| m.len())
        .context("reading the last part")?;

    let mut con_header = god::ConHeaderBuilder::new()
        .with_execution_info(&exe_info)
        .with_block_counts(block_count as u32, 0)
        .with_data_parts_info(
            part_count as u32,
            last_part_size + (part_count - 1) * god::BLOCK_SIZE * 0xa290,
        )
        .with_content_type(content_type)
        .with_mht_hash(&mht.digest());

    let title = game_title
        .map(str::to_owned)
        .or_else(|| game_list::find_title_by_id(exe_info.title_id));
    if let Some(title) = title {
        con_header = con_header.with_game_title(&title);
    }

    let con_header = con_header.finalize();

    let mut con_header_file = File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(file_layout.con_header_file_path())
        .context("creating CON header file")?;
    con_header_file
        .write_all(&con_header)
        .context("writing CON header")?;

    Ok(content_dir.join(format!("{:08X}", exe_info.title_id)))
}

fn read_part_mht(file_layout: &god::FileLayout<'_>, part_index: u64) -> Result<god::HashList> {
    let mut part_file = File::options()
        .read(true)
        .open(file_layout.part_file_path(part_index))?;
    Ok(god::HashList::read(&mut part_file)?)
}

fn write_part_mht(
    file_layout: &god::FileLayout<'_>,
    part_index: u64,
    mht: &god::HashList,
) -> Result<()> {
    let mut part_file = File::options()
        .write(true)
        .open(file_layout.part_file_path(part_index))?;
    mht.write(&mut part_file)?;
    Ok(())
}
