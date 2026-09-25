// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    DisplayedBadAvatarConfig, DisplayedCompatConfig, DisplayedCompatPack, DisplayedConfig,
    DisplayedFatxDrive, DisplayedRecentLocation, DisplayedRemovableDrive, DisplayedUrlSource,
    GodLayout, TargetKind,
};
use crate::util::GIB;
use slint::{Model, ModelRc, SharedString, ToSharedString, VecModel};
use txbm_core::{
    badavatar::{AbadavatarVersion, UrlField},
    config::{Config, GodLayout as CoreGodLayout},
    ogxbox_compat::{COMPAT_PACKS, XEFU_CONFIGS_URL},
    target::Target,
};

/// Builds the model backing `UiState.badavatar` from the config, resolving each
/// URL (override or built-in default) and exposing the default alongside so the
/// UI can offer a per-field reset when the two differ.
pub fn displayed_badavatar(config: &Config) -> DisplayedBadAvatarConfig {
    let ba = &config.contents.badavatar;
    DisplayedBadAvatarConfig {
        abadavatar_v10_url: ba.url(UrlField::AbadavatarV10).to_shared_string(),
        abadavatar_v13_url: ba.url(UrlField::AbadavatarV13).to_shared_string(),
        xeunshackle_url: ba.url(UrlField::Xeunshackle).to_shared_string(),
        xeunshackle_max_url: ba.url(UrlField::XeunshackleMax).to_shared_string(),
        aurora_url: ba.url(UrlField::Aurora).to_shared_string(),
        system_update_url: ba.url(UrlField::SystemUpdate).to_shared_string(),
        abadavatar_v10_default_url: UrlField::AbadavatarV10.default_url().to_shared_string(),
        abadavatar_v13_default_url: UrlField::AbadavatarV13.default_url().to_shared_string(),
        xeunshackle_default_url: UrlField::Xeunshackle.default_url().to_shared_string(),
        xeunshackle_max_default_url: UrlField::XeunshackleMax.default_url().to_shared_string(),
        aurora_default_url: UrlField::Aurora.default_url().to_shared_string(),
        system_update_default_url: UrlField::SystemUpdate.default_url().to_shared_string(),
        aurora_sources: displayed_sources(UrlField::Aurora),
        include_system_update: ba.include_system_update,
        xeunshackle_autostart: ba.xeunshackle_autostart,
        abadavatar_versions: ModelRc::new(VecModel::from(
            AbadavatarVersion::ALL
                .iter()
                .map(|v| v.label().to_shared_string())
                .collect::<Vec<_>>(),
        )),
        abadavatar_version_index: AbadavatarVersion::ALL
            .iter()
            .position(|v| *v == ba.abadavatar_version)
            .unwrap_or(0) as i32,
    }
}

/// The known download sources of a component, for its URL field's picker.
fn displayed_sources(field: UrlField) -> ModelRc<DisplayedUrlSource> {
    ModelRc::new(VecModel::from(
        field
            .sources()
            .iter()
            .map(|s| DisplayedUrlSource {
                label: s.label.to_shared_string(),
                url: s.url.to_shared_string(),
                host: url_host(s.url).to_shared_string(),
            })
            .collect::<Vec<_>>(),
    ))
}

/// The host part of a URL (`archive.org` for `https://archive.org/…`), or the
/// URL itself when it has no scheme.
fn url_host(url: &str) -> &str {
    let Some((_, rest)) = url.split_once("://") else {
        return url;
    };
    rest.split('/').next().unwrap_or(rest)
}

/// Builds the model backing `UiState.compat` from the config: the pack list in
/// the drop-down's order, the chosen row with what is known of it, and the
/// configs URL alongside the built-in one so its field can offer a reset.
pub fn displayed_compat(config: &Config) -> DisplayedCompatConfig {
    let cfg = &config.contents.ogxbox_compat;
    let pack = cfg.pack();
    DisplayedCompatConfig {
        packs: ModelRc::new(VecModel::from(
            cfg.packs()
                .map(|p| p.label().to_shared_string())
                .collect::<Vec<_>>(),
        )),
        pack_index: cfg.packs().position(|p| p.key() == pack.key()).unwrap_or(0) as i32,
        pack_description: match pack.description() {
            Some(description) => description.to_shared_string(),
            None if pack.url().is_empty() => {
                "Custom pack, with no URL yet: set one in Settings › Download sources."
                    .to_shared_string()
            }
            None => slint::format!("Custom pack, downloaded from {}.", url_host(pack.url())),
        },
        needs_exploit: pack.needs_exploit(),
        configs_url: cfg.configs_url().to_shared_string(),
        configs_default_url: XEFU_CONFIGS_URL.to_shared_string(),
        // The built-in URL is a branch archive: naming the repository says
        // more than naming GitHub. Anything else is known by its host alone.
        configs_source: if cfg.configs_url() == XEFU_CONFIGS_URL {
            "github.com/Goatman13/xefu".to_shared_string()
        } else {
            url_host(cfg.configs_url().trim()).to_shared_string()
        },
        backup_first: cfg.backup_first,
        update_configs: cfg.update_configs,
    }
}

/// The built-in emulator packs, for the settings list. They never change, so
/// this is set once at startup.
pub fn builtin_compat_packs() -> Vec<DisplayedCompatPack> {
    COMPAT_PACKS
        .iter()
        .map(|p| DisplayedCompatPack {
            key: p.key.to_shared_string(),
            label: p.label.to_shared_string(),
            url: p.url.to_shared_string(),
        })
        .collect()
}

/// Brings `model` (backing `UiState.compat-custom-packs`) in line with the
/// config's custom packs.
///
/// The model is updated rather than replaced: each row is a pair of text
/// fields, and handing Slint a new model recreates every row, which would
/// take the focus away from the one being typed in. With the same packs in
/// the same order — the case of every keystroke — only rows that differ are
/// rewritten, which Slint applies to the existing row. An addition or a
/// removal (a button click, no field focused) rebuilds the list.
pub fn sync_custom_packs(model: &VecModel<DisplayedCompatPack>, config: &Config) {
    let wanted: Vec<DisplayedCompatPack> = config
        .contents
        .ogxbox_compat
        .custom_packs
        .iter()
        .map(|p| DisplayedCompatPack {
            key: p.key.to_shared_string(),
            label: p.label.to_shared_string(),
            url: p.url.to_shared_string(),
        })
        .collect();

    let same_rows = model.row_count() == wanted.len()
        && model.iter().zip(&wanted).all(|(have, want)| have.key == want.key);
    if !same_rows {
        model.set_vec(wanted);
        return;
    }
    for (i, want) in wanted.into_iter().enumerate() {
        if model.row_data(i).as_ref() != Some(&want) {
            model.set_row_data(i, want);
        }
    }
}

/// Builds the model backing `UiState.recent-locations` from the config.
pub fn recent_locations(config: &Config) -> Vec<DisplayedRecentLocation> {
    config
        .contents
        .recent_locations
        .iter()
        .map(|l| DisplayedRecentLocation {
            name: l.display_name().to_shared_string(),
            kind: l.kind.into(),
        })
        .collect()
}

/// Builds the model backing `UiState.removable-drives` from the currently
/// attached drives (queried live, not from the config).
pub fn removable_drives() -> Vec<DisplayedRemovableDrive> {
    txbm_core::drives::list_removable_drives()
        .into_iter()
        .map(|d| {
            let size_text = if d.total_bytes > 0 {
                SharedString::from(format!(
                    "{:.1} GiB free of {:.1} GiB",
                    d.available_bytes as f32 / GIB,
                    d.total_bytes as f32 / GIB
                ))
            } else {
                SharedString::new()
            };
            DisplayedRemovableDrive {
                name: d.name.to_shared_string(),
                mount_point: d.mount_point.to_string_lossy().to_shared_string(),
                size_text,
                is_removable: d.is_removable,
                fs_label: d.fs_label.to_shared_string(),
                is_fat32: d.is_fat32,
            }
        })
        .collect()
}

impl From<txbm_core::config::TargetKind> for TargetKind {
    fn from(kind: txbm_core::config::TargetKind) -> Self {
        match kind {
            txbm_core::config::TargetKind::Local => TargetKind::Local,
            txbm_core::config::TargetKind::Ftp => TargetKind::Ftp,
            txbm_core::config::TargetKind::Fatx => TargetKind::Fatx,
        }
    }
}

/// Builds the model backing `UiState.fatx-drives` from a live enumeration of
/// the raw disks attached to this computer (see
/// [`txbm_core::fatx_dev::list_fatx_drives`], which opens and probes every one
/// of them and therefore runs on a worker thread, never on the event loop).
pub fn displayed_fatx_drives(
    drives: Vec<txbm_core::fatx_dev::FatxDrive>,
) -> Vec<DisplayedFatxDrive> {
    drives
        .into_iter()
        .map(|d| DisplayedFatxDrive {
            name: d.name.to_shared_string(),
            path: d.path.to_string_lossy().to_shared_string(),
            size_text: if d.size_bytes > 0 {
                SharedString::from(format!("{:.1} GiB", d.size_bytes as f32 / GIB))
            } else {
                SharedString::new()
            },
            detail: d.probe.label().to_shared_string(),
            usable: d.probe.is_usable(),
        })
        .collect()
}

/// Link to the README section explaining how to grant raw-disk access, aimed
/// straight at the paragraph for the platform this build runs on (the README
/// has one heading per OS). Shown by the FATX picker when a disk came back
/// "access denied". Falls back to the section itself on other platforms.
pub fn fatx_privileges_help_url() -> SharedString {
    const BASE: &str =
        "https://github.com/jeanmatthieud/TinyXbox360BackupManager#electric_plug-using-the-consoles-hard-drive";
    let url = if cfg!(target_os = "linux") {
        "https://github.com/jeanmatthieud/TinyXbox360BackupManager#penguin-linux"
    } else if cfg!(target_os = "windows") {
        "https://github.com/jeanmatthieud/TinyXbox360BackupManager#window-windows"
    } else if cfg!(target_os = "macos") {
        "https://github.com/jeanmatthieud/TinyXbox360BackupManager#apple-macos"
    } else {
        BASE
    };
    SharedString::from(url)
}

impl From<CoreGodLayout> for GodLayout {
    fn from(layout: CoreGodLayout) -> Self {
        match layout {
            CoreGodLayout::TitleId => GodLayout::TitleId,
            CoreGodLayout::NameSlashTitleId => GodLayout::NameSlashTitleId,
            CoreGodLayout::NameDashTitleId => GodLayout::NameDashTitleId,
            CoreGodLayout::TitleIdDashName => GodLayout::TitleIdDashName,
        }
    }
}

impl From<GodLayout> for CoreGodLayout {
    fn from(layout: GodLayout) -> Self {
        match layout {
            GodLayout::TitleId => CoreGodLayout::TitleId,
            GodLayout::NameSlashTitleId => CoreGodLayout::NameSlashTitleId,
            GodLayout::NameDashTitleId => CoreGodLayout::NameDashTitleId,
            GodLayout::TitleIdDashName => CoreGodLayout::TitleIdDashName,
        }
    }
}

impl From<&Config> for DisplayedConfig {
    fn from(config: &Config) -> Self {
        let target = Target::from_config(&config.contents)
            .map(|t| t.display())
            .unwrap_or_default();

        Self {
            path: config.path.to_string_lossy().to_shared_string(),
            target: target.to_shared_string(),
            target_kind: config.contents.target_kind.into(),
            mount_point: config
                .contents
                .mount_point
                .to_string_lossy()
                .to_shared_string(),
            remove_sources_games: config.contents.remove_sources_games.to_shared_string(),
            xbox360_format: config.contents.xbox360_format.to_shared_string(),
            sort_by: config.contents.sort_by.to_shared_string(),
            view_as: config.contents.view_as.to_shared_string(),
            theme_preference: config.contents.theme_preference.to_shared_string(),
            auto_reconnect: config.contents.auto_reconnect.to_shared_string(),
            cover_source: config.contents.cover_source.to_shared_string(),
            show_x360: config.contents.show_x360,
            show_arcade: config.contents.show_arcade,
            show_og: config.contents.show_og,
            console_ip: config.contents.console_ip.to_shared_string(),
            ftp_port: config.contents.ftp_port.to_shared_string(),
            ftp_user: config.contents.ftp_user.to_shared_string(),
            ftp_password: config.contents.ftp_password.to_shared_string(),
        }
    }
}
