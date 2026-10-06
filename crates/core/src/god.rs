// SPDX-License-Identifier: GPL-3.0-only
// Conversion pipeline adapted from the iso2god binary (https://github.com/iliazeus/iso2god-rs).

use crate::xdvd::XdvdImage;
use anyhow::{Context, Result};
use iso2god::{game_list, god};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// Removes the output of a cancelled conversion: the `<mediaID>.data` folder
/// holding its parts, its CON header, and then — only while they are empty —
/// the package and `<TitleID>` folders above them.
///
/// The two `remove_dir` calls are deliberately not recursive: they succeed
/// exactly when this conversion is the only thing that ever put anything
/// there, and fail harmlessly the moment another disc, a DLC folder or a
/// title update shares the tree.
fn cleanup_partial(file_layout: &god::FileLayout<'_>, title_dir: &Path) {
    let data_dir = file_layout.data_dir_path();
    let _ = fs::remove_dir_all(&data_dir);
    let _ = fs::remove_file(file_layout.con_header_file_path());
    if let Some(package_dir) = data_dir.parent() {
        let _ = fs::remove_dir(package_dir);
    }
    let _ = fs::remove_dir(title_dir);
}

/// Image data held by one GOD part.
const PART_DATA_SIZE: u64 = god::BLOCKS_PER_PART * god::BLOCK_SIZE;

/// Writes part `part_index` of the image: `iso2god`'s `god::write_part`
/// (v1.8.0), with a cancellation check and a progress report at each subpart.
///
/// A part is 170 MB of data written in a single call, so the upstream function
/// leaves both the progress bar and the Cancel button dead for seconds at a
/// time on a slow drive. Its structure is kept as is, `io::copy` between the
/// two files included: on Linux that copy can use `copy_file_range`, which a
/// wrapper around either file would rule out.
///
/// `on_progress` receives the bytes of image data written into this part so
/// far.
///
/// This is a copy, not a call: it must be kept in step with upstream. When
/// the `iso2god` tag is bumped, diff its `god::write_part` against this one and
/// carry any change over — the parts must stay byte-identical to what
/// `iso2god` writes, or the console rejects the container.
fn write_part(
    data_volume: &mut File,
    part_index: u64,
    part_file: &mut File,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64),
) -> Result<()> {
    data_volume.seek_relative((part_index * PART_DATA_SIZE) as i64)?;

    let mut master_hash_list = god::HashList::new();
    let master_hash_list_position = part_file.stream_position()?;
    master_hash_list.write(&mut *part_file)?;

    let mut subpart_buf = Vec::with_capacity(god::SUBPART_SIZE as usize);
    let mut written: u64 = 0;

    for _ in 0..god::SUBPARTS_PER_PART {
        if crate::convert::is_cancelled(cancel) {
            anyhow::bail!(crate::convert::CONVERSION_CANCELLED);
        }

        // Each subpart is read twice: once to hash its blocks, then again to
        // copy it after its hash list.
        (&mut *data_volume)
            .take(god::SUBPART_SIZE)
            .read_to_end(&mut subpart_buf)?;
        if subpart_buf.is_empty() {
            break;
        }

        let mut sub_hash_list = god::HashList::new();
        for block in subpart_buf.chunks(god::BLOCK_SIZE as usize) {
            sub_hash_list.add_block_hash(block);
        }
        sub_hash_list.write(&mut *part_file)?;
        master_hash_list.add_block_hash(sub_hash_list.bytes());

        data_volume.seek_relative(-(subpart_buf.len() as i64))?;
        std::io::copy(&mut (&mut *data_volume).take(god::SUBPART_SIZE), part_file)?;

        written += subpart_buf.len() as u64;
        on_progress(written);

        if subpart_buf.len() < god::SUBPART_SIZE as usize {
            break;
        }
        subpart_buf.clear();
    }

    part_file.seek(SeekFrom::Start(master_hash_list_position))?;
    master_hash_list.write(&mut *part_file)?;
    Ok(())
}

/// Converts an Xbox 360 game ISO to GOD in `content_dir`
/// (the `Content/0000000000000000` folder of the target).
/// Returns the created title folder (`content_dir/<TitleID>`).
///
/// `progress(done, total)` counts bytes of the image's data, and is called
/// while each part is being written (at most every
/// [`crate::util::PROGRESS_DEBOUNCE`]) as well as when it is done.
pub fn convert_to_god(
    source_iso: &Path,
    content_dir: &Path,
    game_title: Option<&str>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf> {
    let source_iso_file_meta =
        fs::metadata(source_iso).context("reading ISO metadata")?;

    let mut image = XdvdImage::open(source_iso).context("reading source ISO")?;
    let root_offset = image.root_offset()?;

    let title_info = image.title_info().context("reading game executable")?;
    let exe_info = title_info.execution_info;
    let content_type = title_info.content_type;

    // Remove unused space at the end of the image (equivalent to --trim=from-end).
    // A truncated or malformed image can put the root past the end of the file,
    // and an image whose filesystem reports nothing in use yields no part at
    // all — both would underflow the arithmetic below (`part_count - 1` reaches
    // `u64::MAX` in a release build, where overflow checks are off).
    let available = source_iso_file_meta
        .len()
        .checked_sub(root_offset)
        .with_context(|| {
            format!(
                "{} is truncated: its filesystem starts past the end of the file",
                source_iso.display()
            )
        })?;
    let data_size = image.max_used_prefix_size()?.min(available);

    let block_count = data_size.div_ceil(god::BLOCK_SIZE);
    let part_count = block_count.div_ceil(god::BLOCKS_PER_PART);
    if part_count == 0 {
        anyhow::bail!(
            "{} holds no data to convert: it is truncated or not a readable image",
            source_iso.display()
        );
    }

    let file_layout = god::FileLayout::new(content_dir, &exe_info, content_type);

    let data_dir = file_layout.data_dir_path();
    if fs::exists(&data_dir)? {
        fs::remove_dir_all(&data_dir)?;
    }
    fs::create_dir_all(&data_dir).context("creating GOD data folder")?;

    let title_dir = content_dir.join(format!("{:08X}", exe_info.title_id));

    progress(0, data_size);
    let mut last_notify = Instant::now();

    for part_index in 0..part_count {
        if crate::convert::is_cancelled(cancel) {
            // Drop what this conversion wrote — and nothing else. The
            // `<TitleID>` folder is shared: the game's DLC (`00000002`), its
            // title updates (`000B0000`) and its other discs (same package
            // folder, another media ID) all sit in it, and re-adding a game
            // that is already installed converts straight into the live
            // folder. Removing the whole thing on a cancel destroyed all of
            // it.
            cleanup_partial(&file_layout, &title_dir);
            anyhow::bail!(crate::convert::CONVERSION_CANCELLED);
        }
        let mut iso_data_volume = File::open(source_iso)?;
        iso_data_volume.seek(SeekFrom::Start(root_offset))?;

        let mut part_file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(file_layout.part_file_path(part_index))
            .context("creating GOD part file")?;

        let base = part_index * PART_DATA_SIZE;
        let res = write_part(
            &mut iso_data_volume,
            part_index,
            &mut part_file,
            cancel,
            &mut |written| {
                let now = Instant::now();
                if now.duration_since(last_notify) >= crate::util::PROGRESS_DEBOUNCE {
                    last_notify = now;
                    progress((base + written).min(data_size), data_size);
                }
            },
        );
        // Closed before any cleanup: Windows will not delete an open file.
        drop(part_file);
        if let Err(e) = res {
            if crate::convert::is_cancelled(cancel) {
                cleanup_partial(&file_layout, &title_dir);
                anyhow::bail!(crate::convert::CONVERSION_CANCELLED);
            }
            return Err(e.context("writing GOD part file"));
        }

        // The last part reads up to the end of the file, which may lie past
        // the trimmed `data_size`.
        progress(((part_index + 1) * PART_DATA_SIZE).min(data_size), data_size);
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

    Ok(title_dir)
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
