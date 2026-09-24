// SPDX-License-Identifier: GPL-3.0-only

//! Creation of a "BadAvatar" homebrew boot USB key for the Xbox 360
//! (ABadAvatar boot chain + XeUnshackle + Aurora dashboard).
//!
//! The user must plug in a **FAT32-formatted** USB key and select its mount
//! point; this module never formats or partitions a disk, so no elevated
//! privileges are required. It downloads each component from its (configurable)
//! URL, extracts the archives, assembles the file structure on the key and
//! configures `launch.ini`.

use crate::archive;
use crate::data_dir::TMP_DIR;
use crate::download::{self, archive_extension, find_entry};
use crate::drive_info::DriveInfo;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use which_fs::FsKind;

/// Default download URLs. Each can be overridden by the user in the Toolbox
/// advanced settings (see [`BadAvatarConfig`]); a `None` override falls back to
/// the constant here. They are pinned on purpose (the feature stores URLs in
/// code rather than scraping "latest release" pages) — bump them here when a
/// component publishes a new release.
pub const DEFAULT_ABADAVATAR_URL: &str =
    "https://github.com/bibarub/Xbox360BadUpdate/releases/download/avatar-v1.3-beta/ABadAvatar_v1.3-beta.zip";
pub const DEFAULT_XEUNSHACKLE_URL: &str =
    "https://github.com/Byrom90/XeUnshackle/releases/download/v1.03/XeUnshackle-BETA-v1_03.zip";
/// XeUnshackle Max, a fork of XeUnshackle that can skip its boot video and exit
/// straight to the Dashlaunch default item. Used instead of
/// [`DEFAULT_XEUNSHACKLE_URL`] when [`BadAvatarConfig::xeunshackle_autostart`]
/// is set. Its package deliberately leaves out `Xbdm.xex` (a Microsoft binary),
/// which Dashlaunch then simply fails to load as a plugin.
pub const DEFAULT_XEUNSHACKLE_MAX_URL: &str =
    "https://github.com/klofi/XeUnshackle-Max/releases/download/v1.0.0/XeUnshackle-Max-v1.0.0.zip";
/// Aurora is officially distributed as a `.rar` (phoenix.xboxunity.net), which
/// this app cannot extract: adding an UnRAR dependency would conflict with the
/// project's GPL-3.0-only license. We use a `.zip` / `.7z` mirror
/// instead.
pub const DEFAULT_AURORA_URL: &str = "https://archive.org/download/aurora-0.7b.-2-release-package_202607/Aurora%200.7b.2%20-%20Release%20Package.zip";
pub const DEFAULT_SYSTEM_UPDATE_URL: &str =
    "https://archive.org/download/xbox-360-system-update-17559-usb/SystemUpdate_17559_USB.zip";

/// Known ABadAvatar releases, offered as a drop-down in the UI: `(label, url)`.
/// The first entry is the built-in default ([`DEFAULT_ABADAVATAR_URL`]). A URL
/// that isn't in this list is a custom one the user typed themselves.
pub const ABADAVATAR_VERSIONS: &[(&str, &str)] = &[
    ("bibarub/v1.3-beta", DEFAULT_ABADAVATAR_URL),
    (
        "shutterbug2000/v1.0-beta",
        "https://github.com/shutterbug2000/ABadAvatar/releases/download/vPB1.0/ABadAvatar-publicbeta1.0.zip",
    ),
];

/// Row of `url` in [`ABADAVATAR_VERSIONS`], or `None` when it matches no known
/// release — which is what hides the version drop-down in the UI.
pub fn abadavatar_version_index(url: &str) -> Option<usize> {
    ABADAVATAR_VERSIONS.iter().position(|(_, u)| *u == url)
}

/// The downloadable components, used as the stable key for per-field
/// URL overrides and reset in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlField {
    Abadavatar,
    Xeunshackle,
    XeunshackleMax,
    Aurora,
    SystemUpdate,
}

impl UrlField {
    /// Parses the string key used as the message payload from the UI.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "abadavatar" => Some(Self::Abadavatar),
            "xeunshackle" => Some(Self::Xeunshackle),
            "xeunshackle_max" => Some(Self::XeunshackleMax),
            "aurora" => Some(Self::Aurora),
            "system_update" => Some(Self::SystemUpdate),
            _ => None,
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            Self::Abadavatar => DEFAULT_ABADAVATAR_URL,
            Self::Xeunshackle => DEFAULT_XEUNSHACKLE_URL,
            Self::XeunshackleMax => DEFAULT_XEUNSHACKLE_MAX_URL,
            Self::Aurora => DEFAULT_AURORA_URL,
            Self::SystemUpdate => DEFAULT_SYSTEM_UPDATE_URL,
        }
    }

    /// Human-readable label used in progress messages.
    fn label(self) -> &'static str {
        match self {
            Self::Abadavatar => "ABadAvatar",
            Self::Xeunshackle => "XeUnshackle",
            Self::XeunshackleMax => "XeUnshackle Max",
            Self::Aurora => "Aurora",
            Self::SystemUpdate => "system update",
        }
    }
}

/// Persisted BadAvatar settings: per-component URL overrides (`None` = use the
/// built-in default) plus whether to also fetch the official system update and
/// whether to boot straight into Aurora with XeUnshackle Max.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BadAvatarConfig {
    pub abadavatar_url: Option<String>,
    pub xeunshackle_url: Option<String>,
    pub xeunshackle_max_url: Option<String>,
    pub aurora_url: Option<String>,
    pub system_update_url: Option<String>,
    pub include_system_update: bool,
    /// Use XeUnshackle Max, configured to skip its boot video and exit to
    /// Aurora without delay, instead of the stock XeUnshackle.
    pub xeunshackle_autostart: bool,
}

impl BadAvatarConfig {
    fn slot(&self, field: UrlField) -> &Option<String> {
        match field {
            UrlField::Abadavatar => &self.abadavatar_url,
            UrlField::Xeunshackle => &self.xeunshackle_url,
            UrlField::XeunshackleMax => &self.xeunshackle_max_url,
            UrlField::Aurora => &self.aurora_url,
            UrlField::SystemUpdate => &self.system_update_url,
        }
    }

    fn slot_mut(&mut self, field: UrlField) -> &mut Option<String> {
        match field {
            UrlField::Abadavatar => &mut self.abadavatar_url,
            UrlField::Xeunshackle => &mut self.xeunshackle_url,
            UrlField::XeunshackleMax => &mut self.xeunshackle_max_url,
            UrlField::Aurora => &mut self.aurora_url,
            UrlField::SystemUpdate => &mut self.system_update_url,
        }
    }

    /// Effective URL for a component: the user override, or the built-in default.
    pub fn url(&self, field: UrlField) -> &str {
        self.slot(field)
            .as_deref()
            .unwrap_or_else(|| field.default_url())
    }

    /// The XeUnshackle flavour this configuration installs.
    pub fn xeunshackle_field(&self) -> UrlField {
        if self.xeunshackle_autostart {
            UrlField::XeunshackleMax
        } else {
            UrlField::Xeunshackle
        }
    }

    /// Records a user override for a component's URL.
    pub fn set_url(&mut self, field: UrlField, value: String) {
        *self.slot_mut(field) = Some(value);
    }

    /// Drops the override, reverting the component to its built-in default URL.
    pub fn reset_url(&mut self, field: UrlField) {
        *self.slot_mut(field) = None;
    }
}

/// Error message used to signal that the user cancelled the operation, matched
/// by the GUI to show a friendly notice instead of an error (mirrors
/// `convert::CONVERSION_CANCELLED`).
pub const BADAVATAR_CANCELLED: &str = "badavatar creation cancelled";

/// Creates a BadAvatar USB key at `dest` (the mount point of an already
/// FAT32-formatted key). `status` receives short human-readable progress lines
/// for the status bar. `cancel` is polled between phases.
pub fn create_badavatar(
    dest: &Path,
    cfg: &BadAvatarConfig,
    cancel: &AtomicBool,
    status: &dyn Fn(&str),
) -> Result<()> {
    // 1. The key must be FAT32 (the console cannot read anything else off USB).
    status("Checking the USB key…");
    let info = DriveInfo::from_path(dest)
        .with_context(|| format!("inspecting {}", dest.display()))?;
    if info.fs_kind != FsKind::Fat32 {
        bail!(
            "The selected drive is {} — the Xbox 360 only boots from a FAT32 USB key. \
             Reformat it as FAT32 first, then try again.",
            info.fs_kind
        );
    }
    check_cancel(cancel)?;

    // Fresh working directory for downloads and extraction.
    let work = TMP_DIR.join("badavatar");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;

    // 2. Download every component into the working directory.
    let aba_archive = download_component(UrlField::Abadavatar, cfg, &work, cancel, status)?;
    let xe_field = cfg.xeunshackle_field();
    let xe_archive = download_component(xe_field, cfg, &work, cancel, status)?;
    let aurora_archive = download_component(UrlField::Aurora, cfg, &work, cancel, status)?;
    let su_archive = if cfg.include_system_update {
        Some(download_component(UrlField::SystemUpdate, cfg, &work, cancel, status)?)
    } else {
        None
    };

    // 3. Extract each archive into its own subfolder.
    let aba_dir =
        extract_component(&aba_archive, "abadavatar", UrlField::Abadavatar, cancel, status)?;
    let xe_dir =
        extract_component(&xe_archive, "xeunshackle", xe_field, cancel, status)?;
    let aurora_dir =
        extract_component(&aurora_archive, "aurora", UrlField::Aurora, cancel, status)?;
    let su_dir = match &su_archive {
        Some(a) => Some(extract_component(
            a,
            "systemupdate",
            UrlField::SystemUpdate,
            cancel,
            status,
        )?),
        None => None,
    };
    check_cancel(cancel)?;

    // 4. Assemble the file structure directly on the key.
    status("Assembling files on the USB key…");
    assemble(dest, &aba_dir, &xe_dir, &aurora_dir)?;
    check_cancel(cancel)?;

    // 5. Point launch.ini's default at Aurora, and have XeUnshackle Max exit
    //    to it straight away.
    set_launch_default(dest)?;
    if cfg.xeunshackle_autostart {
        write_xeunshackle_max_config(dest)?;
    }

    // 6. Optional official system update.
    if let Some(su_dir) = &su_dir {
        status("Adding the system update…");
        copy_tree(su_dir, dest)?;
    }

    // 7. Installation notes on the key.
    write_install_notes(dest, cfg)?;

    // 8. Best-effort cleanup of the working directory.
    let _ = fs::remove_dir_all(&work);

    status("");
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    download::check_cancel(cancel, BADAVATAR_CANCELLED)
}

/// Downloads one component to a file in `work`, returning its path.
fn download_component(
    field: UrlField,
    cfg: &BadAvatarConfig,
    work: &Path,
    cancel: &AtomicBool,
    status: &dyn Fn(&str),
) -> Result<PathBuf> {
    check_cancel(cancel)?;
    let url = cfg.url(field).trim();
    let label = field.label();

    if url.is_empty() {
        bail!(
            "No download URL configured for {label}. Open the BadAvatar advanced \
             settings and paste a .zip/.7z URL for it."
        );
    }

    let ext = archive_extension(url).with_context(|| {
        format!(
            "{label}: only .zip and .7z archives are supported (got {url}). \
             Use a .zip/.7z mirror."
        )
    })?;
    let dest = work.join(format!("{}.{ext}", key_of(field)));

    status(&format!("Downloading {label}…"));
    let res = download::download_to_file(url, &dest, label, cancel, status, BADAVATAR_CANCELLED);
    // Like the extraction below: a cancelled download is reported with this
    // module's own marker as the top-level message, since the GUI matches on
    // `to_string()` (the outermost context) and not on the chain.
    check_cancel(cancel)?;
    res.with_context(|| format!("downloading {label}"))?;
    Ok(dest)
}

/// Extracts `archive` into `work_subdir` under the same parent and returns the
/// extraction root.
fn extract_component(
    archive_path: &Path,
    subdir: &str,
    field: UrlField,
    cancel: &AtomicBool,
    status: &dyn Fn(&str),
) -> Result<PathBuf> {
    status(&format!("Extracting {}…", field.label()));
    let out = archive_path
        .parent()
        .unwrap_or(archive_path)
        .join(subdir);
    let res = archive::extract_to(archive_path, &out, cancel, &mut |_done, _total| {});
    // A cancelled extraction is reported with this module's own marker, as the
    // top-level message (the GUI matches on `to_string()`, not on the chain).
    check_cancel(cancel)?;
    res.with_context(|| format!("extracting {}", field.label()))?;
    Ok(out)
}

/// Copies the boot-chain pieces from the extracted components onto the key,
/// following the canonical BadAvatar layout.
fn assemble(dest: &Path, aba_dir: &Path, xe_dir: &Path, aurora_dir: &Path) -> Result<()> {
    let content_dir = dest.join("Content");
    let payload_dir = dest.join("BadUpdatePayload");
    let aurora_out = dest.join("Aurora");
    fs::create_dir_all(&content_dir)?;
    fs::create_dir_all(&payload_dir)?;
    fs::create_dir_all(&aurora_out)?;

    // ABadAvatar: the trigger profile under Content/ and the *.bin stages.
    let aba_content =
        find_entry(aba_dir, "Content", true).context("ABadAvatar: Content folder not found")?;
    copy_tree(&aba_content, &content_dir)?;
    if let Some(aba_payload) = find_entry(aba_dir, "BadUpdatePayload", true) {
        for entry in fs::read_dir(&aba_payload)?.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("bin")) {
                fs::copy(&path, payload_dir.join(entry.file_name()))?;
            }
        }
    }

    // XeUnshackle: the folder that directly contains launch.ini holds the full
    // BadUpdatePayload plus the JRPC2/Xbdm modules and launch.ini itself.
    let launch_ini =
        find_entry(xe_dir, "launch.ini", false).context("XeUnshackle: launch.ini not found")?;
    let xu_dir = launch_ini
        .parent()
        .context("XeUnshackle: unexpected layout")?;
    if let Some(xu_payload) = find_entry(xu_dir, "BadUpdatePayload", true) {
        copy_tree(&xu_payload, &payload_dir)?;
    }
    for module in ["JRPC2.xex", "Xbdm.xex"] {
        if let Some(src) = find_entry(xu_dir, module, false) {
            fs::copy(&src, dest.join(module))?;
        }
    }
    fs::copy(&launch_ini, dest.join("launch.ini"))?;

    // Aurora: copy the folder that holds Aurora.xex into Aurora/ on the key,
    // tolerating an extra wrapping folder inside the archive.
    let aurora_xex = find_entry(aurora_dir, "Aurora.xex", false)
        .context("Aurora: Aurora.xex not found in the archive")?;
    let aurora_src = aurora_xex.parent().context("Aurora: unexpected layout")?;
    copy_tree(aurora_src, &aurora_out)?;

    Ok(())
}

/// Writes XeUnshackle Max's `XeUnshackleConfig.txt` (read from the folder of its
/// `default.xex`): no boot video, and an immediate exit to the Dashlaunch
/// default item, i.e. Aurora. The other keys keep the app's own defaults but are
/// spelled out, as the app itself does when it creates the file.
fn write_xeunshackle_max_config(dest: &Path) -> Result<()> {
    const CONFIG: &str = "AutoStartDelay=0\r\nPlayVideo=0\r\nShowKeys=1\r\nVideoVolume=100\r\n";
    fs::write(dest.join("BadUpdatePayload").join("XeUnshackleConfig.txt"), CONFIG)
        .context("writing XeUnshackleConfig.txt")
}

/// Rewrites `launch.ini` so Aurora is the default entry booted from the key.
fn set_launch_default(dest: &Path) -> Result<()> {
    const DEFAULT_LINE: &str = "Default = Usb:\\Aurora\\Aurora.xex";
    let path = dest.join("launch.ini");
    let text = fs::read_to_string(&path).context("reading launch.ini")?;

    // Preserve the file's existing line ending: these homebrew configs ship as
    // CRLF and the on-console INI parser can be line-ending sensitive, so we
    // must not silently downgrade CRLF to LF.
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };

    let mut replaced = false;
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        // Match the exact `Default` key (the text before `=`), not merely any
        // line starting with "default" — the loose prefix would also catch a
        // key like `DefaultTimeout` and clobber it.
        let is_default_key = line
            .split_once('=')
            .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("default"));
        if !replaced && is_default_key {
            out.push(DEFAULT_LINE.to_string());
            replaced = true;
        } else {
            out.push(line.to_string());
        }
    }
    if !replaced {
        out.push(DEFAULT_LINE.to_string());
    }

    fs::write(&path, out.join(newline) + newline).context("writing launch.ini")?;
    Ok(())
}

fn write_install_notes(dest: &Path, cfg: &BadAvatarConfig) -> Result<()> {
    let mut notes = String::new();
    notes.push_str(&format!(
        "BadAvatar USB key — created by TinyXbox360BackupManager v{}\n",
        env!("CARGO_PKG_VERSION")
    ));
    notes.push_str("https://github.com/jeanmatthieud/TinyXbox360BackupManager\n\n");
    notes.push_str("Components (source URLs used):\n");
    notes.push_str(&format!("- ABadAvatar:    {}\n", cfg.url(UrlField::Abadavatar)));
    let xe_field = cfg.xeunshackle_field();
    notes.push_str(&format!("- {:<14} {}\n", format!("{}:", xe_field.label()), cfg.url(xe_field)));
    notes.push_str(&format!("- Aurora:        {}\n", cfg.url(UrlField::Aurora)));
    if cfg.include_system_update {
        notes.push_str(&format!(
            "- System update: {}\n",
            cfg.url(UrlField::SystemUpdate)
        ));
    }
    notes.push_str("\nlaunch.ini Default set to: Usb:\\Aurora\\Aurora.xex\n");
    if cfg.xeunshackle_autostart {
        notes.push_str(
            "XeUnshackle Max: boot video off, exits to Aurora without delay \
             (BadUpdatePayload\\XeUnshackleConfig.txt).\n",
        );
    }
    notes.push('\n');
    notes.push_str("Usage reminders:\n");
    notes.push_str("- Disconnect Wi-Fi/Ethernet before booting the console (avoids an Xbox Live ban).\n");
    notes.push_str("- The boot chain triggers on the profile/avatar selection screen.\n");
    notes.push_str("- Not persistent: repeat on every console reboot.\n");
    notes.push_str("- FAT32 limits files to 4 GB: manage your game library with TinyXbox360BackupManager.\n");

    fs::write(dest.join("INSTALL_NOTES.txt"), notes).context("writing INSTALL_NOTES.txt")?;
    Ok(())
}

// --- small filesystem / URL helpers ---------------------------------------

fn key_of(field: UrlField) -> &'static str {
    match field {
        UrlField::Abadavatar => "abadavatar",
        UrlField::Xeunshackle => "xeunshackle",
        UrlField::XeunshackleMax => "xeunshacklemax",
        UrlField::Aurora => "aurora",
        UrlField::SystemUpdate => "systemupdate",
    }
}

/// Recursively copies `src` into `dst`, merging into any existing directories.
fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if from.is_dir() {
                copy_tree(&from, &to)?;
            } else {
                fs::copy(&from, &to)
                    .with_context(|| format!("copying {}", from.display()))?;
            }
        }
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst).with_context(|| format!("copying {}", src.display()))?;
    }
    Ok(())
}
