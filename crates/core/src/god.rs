// SPDX-License-Identifier: GPL-3.0-only
// Pipeline de conversion adapté du binaire iso2god (https://github.com/iliazeus/iso2god-rs).

use anyhow::{Context, Result};
use iso2god::executable::TitleInfo;
use iso2god::{game_list, god, iso};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Convertit une ISO de jeu Xbox 360 en GOD dans `content_dir`
/// (le dossier `Content/0000000000000000` de la cible).
/// Retourne le dossier du titre créé (`content_dir/<TitleID>`).
///
/// `progress(fait, total)` est appelé à chaque part écrite.
pub fn convert_to_god(
    source_iso: &Path,
    content_dir: &Path,
    game_title: Option<&str>,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf> {
    let source_iso_file =
        File::open(source_iso).context("ouverture de l'ISO source")?;
    let source_iso_file_meta =
        fs::metadata(source_iso).context("lecture des métadonnées de l'ISO")?;

    let mut iso_reader =
        iso::IsoReader::read(source_iso_file).context("lecture de l'ISO source")?;

    let title_info =
        TitleInfo::from_image(&mut iso_reader).context("lecture de l'exécutable du jeu")?;
    let exe_info = title_info.execution_info;
    let content_type = title_info.content_type;

    // On retire l'espace inutilisé en fin d'image (équivalent de --trim=from-end).
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
    fs::create_dir_all(&data_dir).context("création du dossier de données GOD")?;

    progress(0, part_count);

    for part_index in 0..part_count {
        let mut iso_data_volume = File::open(source_iso)?;
        iso_data_volume.seek(SeekFrom::Start(iso_reader.volume_descriptor.root_offset))?;

        let part_file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(file_layout.part_file_path(part_index))
            .context("création d'un fichier de part GOD")?;

        god::write_part(iso_data_volume, part_index, part_file)
            .context("écriture d'un fichier de part GOD")?;

        progress(part_index + 1, part_count);
    }

    // Chaîne de hachage MHT, de la dernière part vers la première.
    let mut mht =
        read_part_mht(&file_layout, part_count - 1).context("lecture du MHT d'une part")?;

    for prev_part_index in (0..part_count - 1).rev() {
        let mut prev_mht = read_part_mht(&file_layout, prev_part_index)
            .context("lecture du MHT d'une part")?;
        prev_mht.add_hash(&mht.digest());
        write_part_mht(&file_layout, prev_part_index, &prev_mht)
            .context("écriture du MHT d'une part")?;
        mht = prev_mht;
    }

    let last_part_size = fs::metadata(file_layout.part_file_path(part_count - 1))
        .map(|m| m.len())
        .context("lecture de la dernière part")?;

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
        .context("création du fichier d'en-tête CON")?;
    con_header_file
        .write_all(&con_header)
        .context("écriture de l'en-tête CON")?;

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
