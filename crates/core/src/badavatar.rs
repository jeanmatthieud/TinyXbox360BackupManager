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
use crate::download::{self, DownloadError, archive_extension, find_entry};
use crate::drive_info::DriveInfo;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use which_fs::FsKind;

/// Default download URLs. Each can be overridden by the user in the Settings
/// page (see [`BadAvatarConfig`]); a `None` override falls back to the
/// constant here. They are pinned on purpose (the feature stores URLs in code
/// rather than scraping "latest release" pages) — bump them here when a
/// component publishes a new release.
///
/// ABadAvatar 1.3-beta: bibarub's port of the ABadAvatar entry point onto
/// BadUpdate 1.3. The only release that also boots from the hard drive (see
/// [`crate::badavatar_hdd`]).
pub const DEFAULT_ABADAVATAR_V13_URL: &str =
    "https://github.com/bibarub/Xbox360BadUpdate/releases/download/avatar-v1.3-beta/ABadAvatar_v1.3-beta.zip";
/// ABadAvatar 1.0-beta: shutterbug2000's original public beta.
pub const DEFAULT_ABADAVATAR_V10_URL: &str =
    "https://github.com/shutterbug2000/ABadAvatar/releases/download/vPB1.0/ABadAvatar-publicbeta1.0.zip";
pub const DEFAULT_XEUNSHACKLE_URL: &str =
    "https://github.com/Byrom90/XeUnshackle/releases/download/v1.03/XeUnshackle-BETA-v1_03.zip";
/// XeUnshackle Max, a fork of XeUnshackle that can skip its boot video and exit
/// straight to the Dashlaunch default item. Used instead of
/// [`DEFAULT_XEUNSHACKLE_URL`] when Aurora is to start without delay. Its
/// package deliberately leaves out `Xbdm.xex` (a Microsoft binary) but keeps
/// it as a plugin in its `launch.ini` (klofi/XeUnshackle-Max#2). USB key only:
/// with Aurora on the console's hard drive, Aurora crashes under it whatever
/// the setup, so the hard-drive install always uses the stock XeUnshackle.
pub const DEFAULT_XEUNSHACKLE_MAX_URL: &str =
    "https://github.com/klofi/XeUnshackle-Max/releases/download/v1.0.0/XeUnshackle-Max-v1.0.0.zip";
/// Aurora's official release package, a `.rar` served by the Phoenix team's
/// own site. See [`AURORA_SOURCES`] for the mirrors offered beside it.
pub const DEFAULT_AURORA_URL: &str =
    "https://phoenix.xboxunity.net/downloads/Aurora%200.7b.2%20-%20Release%20Package.rar";

/// A known place to download a component from, offered in the settings as a
/// one-click alternative to typing a URL. Picking one only fills the field:
/// nothing checks what comes back, so a newer release (or another dashboard)
/// can still be pointed at by hand.
#[derive(Debug, Clone, Copy)]
pub struct UrlSource {
    pub label: &'static str,
    pub url: &'static str,
}

/// Where Aurora can be fetched from, the default first. All three carry the
/// same files.
///
/// consolemods.org sits behind a Cloudflare challenge that answers every
/// non-browser client with a 403, so its copy is reached through a pinned
/// web.archive.org snapshot (same approach as [`crate::ogxbox_compat`]).
pub const AURORA_SOURCES: &[UrlSource] = &[
    UrlSource {
        label: "Aurora Website (default)",
        url: DEFAULT_AURORA_URL,
    },
    UrlSource {
        label: "ConsoleMods mirror",
        url: "https://web.archive.org/web/20241222195356id_/https://consolemods.org/wiki/images/d/dd/Aurora_0.7b.2_-_Release_Package.rar",
    },
    UrlSource {
        label: "Internet Archive",
        url: "https://archive.org/download/aurora-0.7b.-2-release-package_202607/Aurora%200.7b.2%20-%20Release%20Package.zip",
    },
];
pub const DEFAULT_SYSTEM_UPDATE_URL: &str =
    "https://archive.org/download/xbox-360-system-update-17559-usb/SystemUpdate_17559_USB.zip";

/// The ABadAvatar releases the app knows how to install.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AbadavatarVersion {
    #[serde(rename = "1.0-beta")]
    V10,
    #[default]
    #[serde(rename = "1.3-beta")]
    V13,
}

impl AbadavatarVersion {
    /// In the order the UI offers them.
    pub const ALL: [Self; 2] = [Self::V10, Self::V13];

    pub fn label(self) -> &'static str {
        match self {
            Self::V10 => "1.0-beta",
            Self::V13 => "1.3-beta",
        }
    }

    pub fn url_field(self) -> UrlField {
        match self {
            Self::V10 => UrlField::AbadavatarV10,
            Self::V13 => UrlField::AbadavatarV13,
        }
    }
}

/// The downloadable components, used as the stable key for per-field
/// URL overrides and reset in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlField {
    AbadavatarV10,
    AbadavatarV13,
    Xeunshackle,
    XeunshackleMax,
    Aurora,
    SystemUpdate,
}

impl UrlField {
    /// Parses the string key used as the message payload from the UI.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "abadavatar_v10" => Some(Self::AbadavatarV10),
            "abadavatar_v13" => Some(Self::AbadavatarV13),
            "xeunshackle" => Some(Self::Xeunshackle),
            "xeunshackle_max" => Some(Self::XeunshackleMax),
            "aurora" => Some(Self::Aurora),
            "system_update" => Some(Self::SystemUpdate),
            _ => None,
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            Self::AbadavatarV10 => DEFAULT_ABADAVATAR_V10_URL,
            Self::AbadavatarV13 => DEFAULT_ABADAVATAR_V13_URL,
            Self::Xeunshackle => DEFAULT_XEUNSHACKLE_URL,
            Self::XeunshackleMax => DEFAULT_XEUNSHACKLE_MAX_URL,
            Self::Aurora => DEFAULT_AURORA_URL,
            Self::SystemUpdate => DEFAULT_SYSTEM_UPDATE_URL,
        }
    }

    /// The known sources offered for this component, empty when it has only
    /// its default.
    pub fn sources(self) -> &'static [UrlSource] {
        match self {
            Self::Aurora => AURORA_SOURCES,
            _ => &[],
        }
    }

    /// Human-readable label used in progress messages.
    fn label(self) -> &'static str {
        match self {
            Self::AbadavatarV10 => "ABadAvatar 1.0-beta",
            Self::AbadavatarV13 => "ABadAvatar 1.3-beta",
            Self::Xeunshackle => "XeUnshackle",
            Self::XeunshackleMax => "XeUnshackle Max",
            Self::Aurora => "Aurora",
            Self::SystemUpdate => "system update",
        }
    }
}

/// Persisted BadAvatar settings: which ABadAvatar release the USB key gets,
/// per-component URL overrides (`None` = use the built-in default), whether
/// to also fetch the official system update, and — separately for the USB key
/// and the hard drive — whether to boot straight into Aurora with XeUnshackle
/// Max.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BadAvatarConfig {
    pub abadavatar_version: AbadavatarVersion,
    pub abadavatar_v10_url: Option<String>,
    pub abadavatar_v13_url: Option<String>,
    pub xeunshackle_url: Option<String>,
    pub xeunshackle_max_url: Option<String>,
    pub aurora_url: Option<String>,
    pub system_update_url: Option<String>,
    pub include_system_update: bool,
    /// USB key: use XeUnshackle Max, configured to skip its boot video and
    /// exit to Aurora without delay, instead of the stock XeUnshackle. The
    /// hard-drive install never does (see [`DEFAULT_XEUNSHACKLE_MAX_URL`]).
    pub xeunshackle_autostart: bool,
    /// Single ABadAvatar URL of older settings files, which also stood for the
    /// version. Read once by [`Self::migrate`], never written back.
    #[serde(rename = "abadavatar_url", skip_serializing)]
    legacy_abadavatar_url: Option<String>,
}

impl BadAvatarConfig {
    /// Folds a settings file from before the per-version URLs into the current
    /// shape: a known release URL becomes the version choice, and a URL of the
    /// user's own becomes the 1.3-beta override.
    pub fn migrate(&mut self) {
        let Some(url) = self.legacy_abadavatar_url.take() else {
            return;
        };
        if url == DEFAULT_ABADAVATAR_V10_URL {
            self.abadavatar_version = AbadavatarVersion::V10;
        } else {
            self.abadavatar_version = AbadavatarVersion::V13;
            if url != DEFAULT_ABADAVATAR_V13_URL {
                self.abadavatar_v13_url = Some(url);
            }
        }
    }

    fn slot(&self, field: UrlField) -> &Option<String> {
        match field {
            UrlField::AbadavatarV10 => &self.abadavatar_v10_url,
            UrlField::AbadavatarV13 => &self.abadavatar_v13_url,
            UrlField::Xeunshackle => &self.xeunshackle_url,
            UrlField::XeunshackleMax => &self.xeunshackle_max_url,
            UrlField::Aurora => &self.aurora_url,
            UrlField::SystemUpdate => &self.system_update_url,
        }
    }

    fn slot_mut(&mut self, field: UrlField) -> &mut Option<String> {
        match field {
            UrlField::AbadavatarV10 => &mut self.abadavatar_v10_url,
            UrlField::AbadavatarV13 => &mut self.abadavatar_v13_url,
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

    /// The ABadAvatar release this configuration installs.
    pub fn abadavatar_field(&self) -> UrlField {
        self.abadavatar_version.url_field()
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

/// A component download that failed, tagged with the component so the GUI can
/// point at the right field in the settings — and offer its mirrors when it
/// has any. The [`DownloadError`] stays reachable as its `source()`.
#[derive(Debug)]
pub struct ComponentDownloadError {
    pub field: UrlField,
    pub cause: DownloadError,
}

impl ComponentDownloadError {
    /// Finds a component download failure anywhere in an error's chain.
    pub fn find(err: &anyhow::Error) -> Option<&Self> {
        err.chain().find_map(|e| e.downcast_ref::<Self>())
    }
}

impl std::fmt::Display for ComponentDownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.cause.fmt(f)
    }
}

impl std::error::Error for ComponentDownloadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

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

    // 2-3. Download and extract every component.
    let work = TMP_DIR.join("badavatar");
    let staged = stage_components(&work, cfg, true, cfg.include_system_update, cancel, status)?;
    check_cancel(cancel)?;

    // 4. Assemble the file structure directly on the key.
    status("Assembling files on the USB key…");
    assemble(dest, &staged.abadavatar, &staged.xeunshackle)?;
    if let Some(aurora_dir) = &staged.aurora {
        copy_aurora(aurora_dir, &dest.join("Aurora"))?;
    }
    check_cancel(cancel)?;

    // 5. Point launch.ini's default at Aurora, and have XeUnshackle Max exit
    //    to it straight away.
    set_launch_default(dest, "Usb:\\Aurora\\Aurora.xex")?;
    if cfg.xeunshackle_autostart {
        write_xeunshackle_max_config(dest)?;
    }

    // 6. Optional official system update.
    if let Some(su_dir) = &staged.system_update {
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

/// The components, downloaded and extracted: the root each archive was
/// unpacked into.
pub(crate) struct StagedComponents {
    pub abadavatar: PathBuf,
    pub xeunshackle: PathBuf,
    pub aurora: Option<PathBuf>,
    pub system_update: Option<PathBuf>,
}

/// Downloads and extracts the components `cfg` names into a fresh `work`
/// directory: ABadAvatar and the configured XeUnshackle flavour always, Aurora
/// and the system update when asked for. Touches nothing but `work`.
pub(crate) fn stage_components(
    work: &Path,
    cfg: &BadAvatarConfig,
    with_aurora: bool,
    with_system_update: bool,
    cancel: &AtomicBool,
    status: &dyn Fn(&str),
) -> Result<StagedComponents> {
    let _ = fs::remove_dir_all(work);
    fs::create_dir_all(work).with_context(|| format!("creating {}", work.display()))?;

    // Every download first, then every extraction: a dead link is reported
    // before minutes are spent unpacking the others.
    let aba_field = cfg.abadavatar_field();
    let aba_archive = download_component(aba_field, cfg, work, cancel, status)?;
    let xe_field = cfg.xeunshackle_field();
    let xe_archive = download_component(xe_field, cfg, work, cancel, status)?;
    let aurora_archive = if with_aurora {
        Some(download_component(UrlField::Aurora, cfg, work, cancel, status)?)
    } else {
        None
    };
    let su_archive = if with_system_update {
        Some(download_component(UrlField::SystemUpdate, cfg, work, cancel, status)?)
    } else {
        None
    };

    let abadavatar =
        extract_component(&aba_archive, "abadavatar", aba_field, cancel, status)?;
    let xeunshackle = extract_component(&xe_archive, "xeunshackle", xe_field, cancel, status)?;
    let aurora = match &aurora_archive {
        Some(a) => Some(extract_component(a, "aurora", UrlField::Aurora, cancel, status)?),
        None => None,
    };
    let system_update = match &su_archive {
        Some(a) => Some(extract_component(
            a,
            "systemupdate",
            UrlField::SystemUpdate,
            cancel,
            status,
        )?),
        None => None,
    };

    Ok(StagedComponents {
        abadavatar,
        xeunshackle,
        aurora,
        system_update,
    })
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
             settings and paste a .zip/.7z/.rar URL for it."
        );
    }

    let ext = archive_extension(url).with_context(|| {
        format!("{label}: only .zip, .7z and .rar archives are supported (got {url}).")
    })?;
    let dest = work.join(format!("{}.{ext}", key_of(field)));

    status(&format!("Downloading {label}…"));
    let res = download::download_to_file(url, &dest, label, cancel, status, BADAVATAR_CANCELLED);
    // Like the extraction below: a cancelled download is reported with this
    // module's own marker as the top-level message, since the GUI matches on
    // `to_string()` (the outermost context) and not on the chain.
    check_cancel(cancel)?;
    res.map_err(|e| match e.downcast::<DownloadError>() {
        Ok(cause) => ComponentDownloadError { field, cause }.into(),
        Err(e) => e.context(format!("downloading {label}")),
    })?;
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

/// Copies the boot-chain pieces from the extracted components into `dest`
/// (the key's root, or a staging folder mirroring the hard drive's root),
/// following the canonical BadAvatar layout. Aurora is copied separately, by
/// [`copy_aurora`].
pub(crate) fn assemble(dest: &Path, aba_dir: &Path, xe_dir: &Path) -> Result<()> {
    let content_dir = dest.join("Content");
    let payload_dir = dest.join("BadUpdatePayload");
    fs::create_dir_all(&content_dir)?;
    fs::create_dir_all(&payload_dir)?;

    // ABadAvatar: the trigger profile under Content/ and its whole payload
    // folder, as released. The payload is not only the *.bin stages: v1.3
    // also ships GamerProfile.xex, the clean copy its recovery mode (Y held
    // while the exploit triggers) writes back to flash right after deleting
    // the console's own — leaving it out would wipe that file from the NAND.
    let aba_content =
        find_entry(aba_dir, "Content", true).context("ABadAvatar: Content folder not found")?;
    copy_tree(&aba_content, &content_dir)?;
    if let Some(aba_payload) = find_entry(aba_dir, "BadUpdatePayload", true) {
        copy_tree(&aba_payload, &payload_dir)?;
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

    Ok(())
}

/// Copies the folder of the extracted Aurora package that holds Aurora.xex
/// into `aurora_out`, tolerating an extra wrapping folder inside the archive.
pub(crate) fn copy_aurora(aurora_dir: &Path, aurora_out: &Path) -> Result<()> {
    let aurora_xex = find_entry(aurora_dir, "Aurora.xex", false)
        .context("Aurora: Aurora.xex not found in the archive")?;
    let aurora_src = aurora_xex.parent().context("Aurora: unexpected layout")?;
    copy_tree(aurora_src, aurora_out)
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

/// Rewrites the `launch.ini` in `dest` so `aurora_xex` (a console path such as
/// `Usb:\Aurora\Aurora.xex`) is the default entry Dashlaunch boots.
pub(crate) fn set_launch_default(dest: &Path, aurora_xex: &str) -> Result<()> {
    let default_line = format!("Default = {aurora_xex}");
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
            out.push(default_line.clone());
            replaced = true;
        } else {
            out.push(line.to_string());
        }
    }
    if !replaced {
        out.push(default_line);
    }

    fs::write(&path, out.join(newline) + newline).context("writing launch.ini")?;
    Ok(())
}

/// Points every `Usb:\` path of the `launch.ini` in `dest` at the hard drive
/// instead (`Hdd:\`). XeUnshackle's stock file loads its plugins from the USB
/// key; installed on the hard drive, the same entries must name it, or
/// Dashlaunch looks for them on a key that is no longer there. Comment lines
/// are left as they are.
pub(crate) fn retarget_launch_ini_to_hdd(dest: &Path) -> Result<()> {
    let path = dest.join("launch.ini");
    let text = fs::read_to_string(&path).context("reading launch.ini")?;
    let out: String = text
        .split_inclusive('\n')
        .map(|line| {
            if line.trim_start().starts_with(';') {
                return line.to_string();
            }
            let mut rewritten = String::with_capacity(line.len());
            let mut rest = line;
            while let Some(at) = rest.to_ascii_lowercase().find("usb:\\") {
                rewritten.push_str(&rest[..at]);
                rewritten.push_str("Hdd:\\");
                rest = &rest[at + "usb:\\".len()..];
            }
            rewritten.push_str(rest);
            rewritten
        })
        .collect();
    fs::write(&path, out).context("writing launch.ini")
}

fn write_install_notes(dest: &Path, cfg: &BadAvatarConfig) -> Result<()> {
    let mut notes = String::new();
    notes.push_str(&format!(
        "BadAvatar USB key — created by TinyXbox360BackupManager v{}\n",
        env!("CARGO_PKG_VERSION")
    ));
    notes.push_str("https://github.com/jeanmatthieud/TinyXbox360BackupManager\n\n");
    notes.push_str("Components (source URLs used):\n");
    let aba_field = cfg.abadavatar_field();
    notes.push_str(&format!("- {:<14} {}\n", format!("{}:", aba_field.label()), cfg.url(aba_field)));
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
        UrlField::AbadavatarV10 | UrlField::AbadavatarV13 => "abadavatar",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn migrated(json: &str) -> BadAvatarConfig {
        let mut cfg: BadAvatarConfig = serde_json::from_str(json).unwrap();
        cfg.migrate();
        cfg
    }

    #[test]
    fn migrates_the_single_legacy_url() {
        let v10 = migrated(&format!(r#"{{"abadavatar_url":"{DEFAULT_ABADAVATAR_V10_URL}"}}"#));
        assert_eq!(v10.abadavatar_version, AbadavatarVersion::V10);
        assert_eq!(v10.url(UrlField::AbadavatarV13), DEFAULT_ABADAVATAR_V13_URL);

        let mirror = migrated(r#"{"abadavatar_url":"https://example.org/aba.zip"}"#);
        assert_eq!(mirror.abadavatar_version, AbadavatarVersion::V13);
        assert_eq!(mirror.url(UrlField::AbadavatarV13), "https://example.org/aba.zip");

        let none = migrated(r#"{"abadavatar_url":null,"xeunshackle_autostart":true}"#);
        assert_eq!(none.abadavatar_version, AbadavatarVersion::V13);
        assert!(none.xeunshackle_autostart);
    }

    #[test]
    fn never_writes_the_legacy_url_back() {
        let json = serde_json::to_string(&migrated(r#"{"abadavatar_url":"https://x/y.zip"}"#)).unwrap();
        assert!(!json.contains("\"abadavatar_url\""));
        assert!(json.contains("\"abadavatar_version\":\"1.3-beta\""));
    }
}
