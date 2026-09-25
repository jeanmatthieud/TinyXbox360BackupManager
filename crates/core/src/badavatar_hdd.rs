// SPDX-License-Identifier: GPL-3.0-only

//! Installing the BadAvatar boot chain on the console's own hard drive, and
//! removing it again.
//!
//! ABadAvatar v1.3 (bibarub's fork of BadUpdate 1.3) looks for its payload
//! folder on three devices in turn: `\Device\Mass0` (USB), `\Device\Mu0`, then
//! `\Device\Harddisk0\Partition1` — the `Hdd1` a [`crate::fatx::FatxSession`]
//! exposes. So the very files the USB key carries work from the hard drive,
//! with no USB key left plugged in: the trigger profile under `Content/`, the
//! `BadUpdatePayload` folder, and the Dashlaunch `launch.ini` with its plugins.
//! Only that release has the hard-drive lookup, which is why the install always
//! uses the 1.3-beta, whichever version the user picked for the USB key.
//!
//! Nothing here ever reaches the console's NAND: the drive is plugged into this
//! computer. The one way the exploit itself writes to flash is its recovery
//! mode (Y held while it triggers), which deletes `flash:\GamerProfile.xex` and
//! then writes back the copy in the payload folder. That copy is therefore
//! checked against the release's own before anything is written, and read
//! back once it is on the drive.
//!
//! Writes go profile-last on install and profile-first on removal: without the
//! profile nothing triggers, so a run interrupted halfway leaves a drive the
//! console boots normally from. A manifest written first, inside the payload
//! folder, lists what was put there; it is what tells our install apart from
//! anyone else's, and what the removal goes by.

use crate::badavatar::{self, AbadavatarVersion, BADAVATAR_CANCELLED, BadAvatarConfig, UrlField};
use crate::data_dir::TMP_DIR;
use crate::ftp::RemoteEntry;
use crate::remote_fs::{RemoteFs, RemoteSession};
use crate::target::{Target, find_aurora_dir_on_root, remote_hdd_root};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// The ABadAvatar release installed on a hard drive, as the UI names it: the
/// only one that looks for its payload there. Downloaded from the 1.3-beta URL
/// of the settings, whichever version the USB key uses.
pub const ABADAVATAR_RELEASE: &str = "1.3-beta";

/// SHA-1 of the `GamerProfile.xex` shipped by [`ABADAVATAR_RELEASE`]. This is
/// the file the recovery mode writes to flash right after deleting the
/// console's own, so a copy that differs — a re-uploaded release, a corrupt
/// download — is refused rather than installed. SHA-1 is enough here: the check
/// guards against an accidental change, and whoever could forge a collision in
/// that release could as well change the exploit stages it ships beside it.
const GAMER_PROFILE_SHA1: &str = "d2eaaa5c3b5b85ba98252d93b80c0521047d71e6";
const GAMER_PROFILE: &str = "GamerProfile.xex";

const PAYLOAD_DIR: &str = "BadUpdatePayload";
/// XUID of the ABadAvatar trigger profile, which also names its package file.
const PROFILE_XUID: &str = "E0002FF78DFBDE7B";
/// Folders from the drive root down to the profile package.
const PROFILE_DIRS: [&str; 4] = ["Content", PROFILE_XUID, "FFFE07D1", "00010000"];
const MANIFEST_NAME: &str = "txbm-badavatar.json";

/// Files at the drive root that belong to a BadAvatar setup. Any of them on a
/// drive we did not install makes it "not retail".
const ROOT_FILES: [&str; 3] = ["launch.ini", "JRPC2.xex", "Xbdm.xex"];
/// Dashlaunch's helper, which XeUnshackle copies to the hard drive by itself
/// whenever the console has one — so nearly every console that ever ran
/// BadAvatar from a USB key has it. Not a sign of anyone's install, but still
/// removed with ours.
const LHELPER: &str = "lhelper.xex";

/// What our install put on the drive, stored in the payload folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HddManifest {
    pub app_version: String,
    pub abadavatar: String,
    pub xeunshackle_url: String,
    pub aurora_url: String,
    /// Files written at the drive root.
    pub root_files: Vec<String>,
    /// The Aurora folder we installed, relative to the drive root
    /// (`Aurora`). `None` when Aurora was already there: it is then the user's,
    /// and stays when BadAvatar is removed.
    pub aurora_installed: Option<String>,
}

impl Default for HddManifest {
    fn default() -> Self {
        Self {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            abadavatar: ABADAVATAR_RELEASE.to_string(),
            xeunshackle_url: String::new(),
            aurora_url: String::new(),
            root_files: Vec::new(),
            aurora_installed: None,
        }
    }
}

/// State of the drive, as far as BadAvatar goes.
#[derive(Debug, Clone)]
pub enum HddStatus {
    /// No trace of BadAvatar: ready for an install.
    Retail,
    /// Our install. `complete` is false when a run was interrupted, or the
    /// profile or payload was removed by hand; removing it is still offered.
    Installed {
        manifest: HddManifest,
        complete: bool,
    },
    /// Someone else's install (another tool, a hand copy, an older release):
    /// left alone. Lists what was found, in console notation.
    Foreign { found: Vec<String> },
}

#[derive(Debug, Clone)]
pub struct HddInspection {
    pub status: HddStatus,
    /// Where Aurora is on the drive (`Hdd:\Aurora`), if it is.
    pub aurora: Option<String>,
    /// What a removal would delete, in console notation. Empty unless the
    /// install is ours.
    pub removal: Vec<String>,
}

/// Reads the drive's state. Read-only.
pub fn inspect(target: &Target) -> Result<HddInspection> {
    let mut session = open(target, false)?;
    let found = inspect_remote(&mut session)?;
    session.quit()?;
    Ok(found)
}

/// Downloads, checks and installs BadAvatar on a retail drive. `status`
/// receives short progress lines. `cancel` is honoured until the first write;
/// `writing` is called right before it, after which the run can no longer be
/// stopped (it is a few megabytes, plus Aurora when it has to be installed).
pub fn install(
    target: &Target,
    cfg: &BadAvatarConfig,
    cancel: &AtomicBool,
    status: &dyn Fn(&str),
    writing: &dyn Fn(),
) -> Result<()> {
    status("Checking the hard drive…");
    let before = inspect(target)?;
    ensure_retail(&before.status)?;
    check_cancel(cancel)?;

    // Always the 1.3-beta release, whichever the USB key uses: only this one
    // boots from the hard drive. Its URL may still be a mirror of the user's —
    // `check_gamer_profile` refuses any other release. And always the stock
    // XeUnshackle: Aurora installed on the hard drive crashes under XeUnshackle
    // Max, with or without its auto start (see `DEFAULT_XEUNSHACKLE_MAX_URL`).
    let mut cfg = cfg.clone();
    cfg.abadavatar_version = AbadavatarVersion::V13;
    cfg.xeunshackle_autostart = false;
    let need_aurora = before.aurora.is_none();

    let work = TMP_DIR.join("badavatar-hdd");
    let res = (|| {
        let staged =
            badavatar::stage_components(&work, &cfg, need_aurora, false, cancel, status)?;
        check_cancel(cancel)?;

        status("Preparing the files…");
        let root = work.join("hdd");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        badavatar::assemble(&root, &staged.abadavatar, &staged.xeunshackle)?;

        let aurora_rel = match &staged.aurora {
            Some(aurora) => {
                badavatar::copy_aurora(aurora, &root.join("Aurora"))?;
                "Aurora".to_string()
            }
            None => {
                let dir = before.aurora.as_deref().unwrap_or("Hdd:\\Aurora");
                dir.trim_start_matches("Hdd:\\").to_string()
            }
        };
        badavatar::retarget_launch_ini_to_hdd(&root)?;
        badavatar::set_launch_default(&root, &format!("Hdd:\\{aurora_rel}\\Aurora.xex"))?;

        check_gamer_profile(&root.join(PAYLOAD_DIR).join(GAMER_PROFILE))?;
        if !root
            .join(PROFILE_DIRS.iter().collect::<std::path::PathBuf>())
            .join(PROFILE_XUID)
            .is_file()
        {
            bail!("the ABadAvatar release holds no trigger profile");
        }

        let manifest = HddManifest {
            xeunshackle_url: cfg.url(cfg.xeunshackle_field()).to_string(),
            aurora_url: cfg.url(UrlField::Aurora).to_string(),
            root_files: ROOT_FILES
                .iter()
                .filter(|name| root.join(name).is_file())
                .map(|name| name.to_string())
                .collect(),
            aurora_installed: need_aurora.then(|| aurora_rel.replace('\\', "/")),
            ..Default::default()
        };
        check_cancel(cancel)?;

        writing();
        let mut session = open(target, true)?;
        check_space(&mut session, &root)?;
        install_remote(&mut session, &root, &manifest, status)?;
        session.quit()
    })();

    let _ = fs::remove_dir_all(&work);
    status("");
    res
}

/// Removes our install: the profile first, so the exploit is disarmed before
/// anything else goes, then the files at the root, the Aurora folder when we
/// installed it, and the payload folder last.
pub fn uninstall(target: &Target, status: &dyn Fn(&str)) -> Result<()> {
    status("Removing BadAvatar from the hard drive…");
    let mut session = open(target, true)?;
    let res = uninstall_remote(&mut session, status);
    // Flushed even after a failure: whatever was removed must reach the disk
    // in a consistent state.
    let quit = session.quit();
    status("");
    res.and(quit)
}

fn open(target: &Target, writable: bool) -> Result<RemoteSession> {
    match target {
        Target::Fatx(_) => target.open_remote(writable),
        _ => bail!("BadAvatar can only be installed on a console hard drive connected here"),
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    crate::download::check_cancel(cancel, BADAVATAR_CANCELLED)
}

fn ensure_retail(status: &HddStatus) -> Result<()> {
    match status {
        HddStatus::Retail => Ok(()),
        HddStatus::Installed { .. } => bail!("BadAvatar is already installed on this hard drive"),
        HddStatus::Foreign { .. } => bail!(
            "this hard drive already holds another BadAvatar setup — restore it to its \
             retail state first"
        ),
    }
}

/// Refuses a `GamerProfile.xex` that isn't the release's own.
fn check_gamer_profile(path: &Path) -> Result<()> {
    let sha1 = crate::util::sha1_hex_file(path)
        .with_context(|| format!("reading {GAMER_PROFILE} from the ABadAvatar release"))?;
    if sha1 != GAMER_PROFILE_SHA1 {
        bail!(
            "the {GAMER_PROFILE} in the downloaded ABadAvatar release is not the expected \
             one — nothing was installed"
        );
    }
    Ok(())
}

/// Refuses an install that cannot fit, before anything is written.
fn check_space(session: &mut RemoteSession, root: &Path) -> Result<()> {
    if let RemoteSession::Fatx(fatx) = session
        && let Ok(space) = fatx.space()
    {
        let needed = crate::util::dir_size_on(root, space.bytes_per_cluster);
        if needed > space.free_bytes {
            bail!(
                "not enough space on the console drive: {} needed, {} free",
                crate::util::human_size(needed),
                crate::util::human_size(space.free_bytes)
            );
        }
    }
    Ok(())
}

// --- console-shaped side ---------------------------------------------------

/// Console notation for a path under the drive root (`Hdd:\BadUpdatePayload`).
fn console_path(rel: &str) -> String {
    format!("Hdd:\\{}", rel.replace('/', "\\"))
}

fn find_ci<'a>(entries: &'a [RemoteEntry], name: &str, dir: bool) -> Option<&'a RemoteEntry> {
    entries
        .iter()
        .find(|e| e.is_dir == dir && e.name.eq_ignore_ascii_case(name))
}

/// Follows `dirs` down from `root`, matching each level case-insensitively as
/// the console does. Returns the real path, or `None` when a level is missing.
fn walk_ci(fs: &mut dyn RemoteFs, root: &str, dirs: &[&str]) -> Result<Option<String>> {
    let mut path = root.to_string();
    for name in dirs {
        let entries = fs.try_list_dir(&path)?;
        let Some(entry) = find_ci(&entries, name, true) else {
            return Ok(None);
        };
        path = format!("{path}/{}", entry.name);
    }
    Ok(Some(path))
}

/// Everything [`inspect_remote`] and the removal need to know about the drive.
struct Layout {
    root: String,
    /// Real name of the payload folder, when there is one.
    payload: Option<String>,
    manifest: Option<Result<HddManifest>>,
    has_gamer_profile: bool,
    /// Folder holding the profile package, and the package's real name.
    profile: Option<(String, String)>,
    /// Real names of the known root files present.
    root_files: Vec<String>,
    lhelper: Option<String>,
    aurora: Option<String>,
}

/// Listed the strict way throughout: a folder that cannot be read must never
/// pass for one that does not exist, or a drive holding someone else's install
/// could be taken for a retail one.
fn read_layout(fs: &mut dyn RemoteFs) -> Result<Layout> {
    let root = format!("/{}", remote_hdd_root(fs));
    let entries = fs
        .try_list_dir(&root)
        .context("listing the hard drive's root")?;

    let payload = find_ci(&entries, PAYLOAD_DIR, true).map(|e| e.name.clone());
    let (mut manifest, mut has_gamer_profile) = (None, false);
    if let Some(name) = &payload {
        let dir = format!("{root}/{name}");
        let files = fs.try_list_dir(&dir).context("listing the payload folder")?;
        has_gamer_profile = find_ci(&files, GAMER_PROFILE, false).is_some();
        if let Some(entry) = find_ci(&files, MANIFEST_NAME, false) {
            let parsed = fs
                .download_file(&format!("{dir}/{}", entry.name))
                .and_then(|bytes| Ok(serde_json::from_slice(&bytes)?));
            manifest = Some(parsed);
        }
    }

    let profile = match walk_ci(fs, &root, &PROFILE_DIRS)? {
        Some(dir) => {
            let files = fs.try_list_dir(&dir)?;
            find_ci(&files, PROFILE_XUID, false).map(|e| (dir, e.name.clone()))
        }
        None => None,
    };

    let root_files = ROOT_FILES
        .iter()
        .filter_map(|name| find_ci(&entries, name, false).map(|e| e.name.clone()))
        .collect();
    let lhelper = find_ci(&entries, LHELPER, false).map(|e| e.name.clone());

    let aurora = find_aurora_dir_on_root(fs, root.trim_start_matches('/'))
        .map(|dir| dir.trim_start_matches(&format!("{root}/")).to_string());

    Ok(Layout {
        root,
        payload,
        manifest,
        has_gamer_profile,
        profile,
        root_files,
        lhelper,
        aurora,
    })
}

pub fn inspect_remote(fs: &mut dyn RemoteFs) -> Result<HddInspection> {
    let layout = read_layout(fs)?;
    let aurora = layout.aurora.as_deref().map(console_path);

    let status = match &layout.manifest {
        Some(Ok(manifest)) => HddStatus::Installed {
            manifest: manifest.clone(),
            complete: layout.profile.is_some() && layout.has_gamer_profile,
        },
        _ => {
            let mut found = Vec::new();
            if let Some((dir, name)) = &layout.profile {
                let rel = format!("{dir}/{name}");
                found.push(console_path(rel.trim_start_matches(&format!("{}/", layout.root))));
            }
            if let Some(name) = &layout.payload {
                found.push(console_path(name));
            }
            found.extend(layout.root_files.iter().map(|name| console_path(name)));
            if found.is_empty() {
                HddStatus::Retail
            } else {
                HddStatus::Foreign { found }
            }
        }
    };

    let removal = match &status {
        HddStatus::Installed { manifest, .. } => removal_list(&layout, manifest),
        _ => Vec::new(),
    };

    Ok(HddInspection {
        status,
        aurora,
        removal,
    })
}

/// What [`uninstall_remote`] deletes, in the order it does, for the
/// confirmation modal.
fn removal_list(layout: &Layout, manifest: &HddManifest) -> Vec<String> {
    let mut list = Vec::new();
    if layout.profile.is_some() {
        list.push(format!(
            "The ABadAvatar profile ({})",
            console_path(&format!("{}/{PROFILE_XUID}", PROFILE_DIRS.join("/")))
        ));
    }
    for name in &layout.root_files {
        if manifest.root_files.iter().any(|f| f.eq_ignore_ascii_case(name)) {
            list.push(console_path(name));
        }
    }
    if let Some(name) = &layout.lhelper {
        list.push(console_path(name));
    }
    if let Some(dir) = &manifest.aurora_installed {
        list.push(format!(
            "{} — Aurora, with its settings, databases and cached covers",
            console_path(dir)
        ));
    }
    if let Some(name) = &layout.payload {
        list.push(format!(
            "{} — the whole folder, including what XeUnshackle saved there (such as its \
             backup of the console's MAC address)",
            console_path(name)
        ));
    }
    list
}

/// Writes the staged tree `root` onto the drive. `fs` must be open for
/// writing, and must be the only session doing so.
fn install_remote(
    fs: &mut dyn RemoteFs,
    root: &Path,
    manifest: &HddManifest,
    status: &dyn Fn(&str),
) -> Result<()> {
    // Checked again on the session that writes: the drive may have been
    // swapped since the first look.
    let layout = read_layout(fs)?;
    if layout.payload.is_some() || layout.profile.is_some() || !layout.root_files.is_empty() {
        bail!(
            "this hard drive already holds another BadAvatar setup — restore it to its \
             retail state first"
        );
    }
    let hdd = layout.root;
    // The copies below have no reason to stop halfway once started.
    let never = AtomicBool::new(false);

    // 1. The manifest first: from here on, whatever happens, the drive reads
    //    as our install and can be removed from the card.
    status("Writing BadAvatar to the hard drive…");
    let payload = format!("{hdd}/{PAYLOAD_DIR}");
    fs.ensure_dir(&payload)?;
    let json = serde_json::to_vec_pretty(manifest)?;
    fs.put_bytes(&payload, MANIFEST_NAME, &json)
        .context("writing the install manifest")?;

    // 2. The payload, then the copy of GamerProfile.xex read back: it is the
    //    one file that can end up in flash.
    fs.upload_dir(&root.join(PAYLOAD_DIR), &payload, &never, &mut |_, _, _| {})
        .context("writing the payload folder")?;
    let written = fs
        .sha1_file(&format!("{payload}/{GAMER_PROFILE}"))
        .context("reading GamerProfile.xex back")?;
    if written != GAMER_PROFILE_SHA1 {
        let _ = fs.remove_file(&payload, GAMER_PROFILE);
        bail!(
            "{GAMER_PROFILE} did not read back intact from the hard drive, so it was removed \
             and the install stopped before the exploit could trigger. Remove BadAvatar from \
             the card, then try again."
        );
    }

    // 3. Dashlaunch's files at the root.
    for name in &manifest.root_files {
        let bytes = fs::read(root.join(name)).with_context(|| format!("reading {name}"))?;
        fs.put_bytes(&hdd, name, &bytes)
            .with_context(|| format!("writing {name}"))?;
    }

    // 4. Aurora, when the drive had none.
    if let Some(dir) = &manifest.aurora_installed {
        let dest = format!("{hdd}/{dir}");
        fs.upload_dir(&root.join(dir), &dest, &never, &mut |sent, total, _| {
            if let Some(percent) = (sent * 100).checked_div(total) {
                status(&format!("Writing Aurora… {percent}%"));
            }
        })
        .context("writing Aurora")?;
        status("Writing BadAvatar to the hard drive…");
    }

    // 5. The profile last: it is what arms the exploit at the next boot.
    let rel: std::path::PathBuf = PROFILE_DIRS.iter().collect();
    let bytes = fs::read(root.join(&rel).join(PROFILE_XUID)).context("reading the profile")?;
    let dir = format!("{hdd}/{}", PROFILE_DIRS.join("/"));
    fs.put_bytes(&dir, PROFILE_XUID, &bytes)
        .context("writing the ABadAvatar profile")?;
    Ok(())
}

fn uninstall_remote(fs: &mut dyn RemoteFs, status: &dyn Fn(&str)) -> Result<()> {
    let layout = read_layout(fs)?;
    let manifest = match &layout.manifest {
        Some(Ok(manifest)) => manifest.clone(),
        _ => bail!("BadAvatar wasn't installed on this hard drive by TinyXbox360BackupManager"),
    };
    let hdd = layout.root.clone();
    let never = AtomicBool::new(false);

    // 1. The profile, then the folders above it that it leaves empty. One that
    //    still holds anything (a save, if someone signed in to that profile)
    //    stays.
    if let Some((dir, name)) = &layout.profile {
        fs.remove_file(dir, name)
            .context("removing the ABadAvatar profile")?;
        let mut dir = dir.clone();
        for _ in 1..PROFILE_DIRS.len() {
            if !fs.remove_empty_dir(&dir).unwrap_or(false) {
                break;
            }
            match dir.rsplit_once('/') {
                Some((parent, _)) => dir = parent.to_string(),
                None => break,
            }
        }
    }

    // 2. The files at the root: ours, and Dashlaunch's helper.
    for name in &layout.root_files {
        if manifest.root_files.iter().any(|f| f.eq_ignore_ascii_case(name)) {
            fs.remove_file(&hdd, name)
                .with_context(|| format!("removing {name}"))?;
        }
    }
    if let Some(name) = &layout.lhelper {
        fs.remove_file(&hdd, name)
            .with_context(|| format!("removing {name}"))?;
    }

    // 3. Aurora, when we installed it.
    if let Some(rel) = &manifest.aurora_installed {
        let dir = format!("{hdd}/{rel}");
        if walk_ci(fs, &hdd, &rel.split('/').collect::<Vec<_>>())?.is_some() {
            status("Removing Aurora…");
            fs.remove_dir_recursive(&dir, &never, &mut |_, _| {})
                .context("removing Aurora")?;
        }
    }

    // 4. The payload folder, manifest included, last: until it goes, the drive
    //    still reads as our install and the removal can be run again.
    if let Some(name) = &layout.payload {
        status("Removing BadAvatar from the hard drive…");
        fs.remove_dir_recursive(&format!("{hdd}/{name}"), &never, &mut |_, _| {})
            .context("removing the payload folder")?;
    }
    Ok(())
}
