// SPDX-License-Identifier: GPL-3.0-only

//! Reading and editing the `[Settings]` of Dashlaunch's `launch.ini`.
//!
//! Dashlaunch (`launch.xex`) is the system module RGH/JTAG consoles run at
//! boot, and the one XeUnshackle loads from memory after the BadUpdate exploit.
//! Either way it reads the same `launch.ini`, taking the first one it finds at
//! the root of a USB drive, then of the internal hard drive. It is only read
//! when Dashlaunch starts: an edit applies at the next boot (the next exploit
//! run, on a BadUpdate console).
//!
//! Only a handful of settings are exposed — those a non-technical user has a
//! reason to care about. The file is edited line by line, never re-serialized:
//! its comments (which document every setting) and its line endings survive.

use crate::remote_fs::RemoteFs;
use crate::target::{Target, find_child_ci};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::PathBuf;

const INI_NAME: &str = "launch.ini";

/// Patch header license bits of add-on content (`00000002`) signed for
/// another console.
pub const CONTPATCH: &str = "contpatch";
/// Same for Xbox Live Arcade titles (`000D0000`).
pub const XBLAPATCH: &str = "xblapatch";
/// Spoofs `XamContentGetLicenseMask`, which the two above usually need (and
/// which an extracted XBLA needs on its own).
pub const LICPATCH: &str = "licpatch";
/// Blocks the DNS lookups of Xbox Live: a modded retail console that reaches
/// Live gets banned for good.
pub const LIVEBLOCK: &str = "liveblock";
/// Hides system updates, one of which could close the exploit for good.
pub const NOUPDATER: &str = "noupdater";

/// The three settings that together let content bought on another console run
/// on this one.
pub const LICENSE_PATCHES: [&str; 3] = [CONTPATCH, XBLAPATCH, LICPATCH];

/// Value Dashlaunch uses when a key is missing, as documented in the stock
/// `launch.ini`. A missing key is common (hand-written or older files), so it
/// must never be read as `false`.
fn default_of(key: &str) -> bool {
    matches!(key, LIVEBLOCK | NOUPDATER)
}

/// The settings shown in the Device status page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DashlaunchSettings {
    pub contpatch: bool,
    pub xblapatch: bool,
    pub licpatch: bool,
    pub liveblock: bool,
    pub noupdater: bool,
}

impl DashlaunchSettings {
    pub fn parse(text: &str) -> Self {
        let get = |key| get_bool(text, key).unwrap_or_else(|| default_of(key));
        Self {
            contpatch: get(CONTPATCH),
            xblapatch: get(XBLAPATCH),
            licpatch: get(LICPATCH),
            liveblock: get(LIVEBLOCK),
            noupdater: get(NOUPDATER),
        }
    }
}

/// Where a `launch.ini` was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IniLocation {
    /// A file on a drive mounted on this computer.
    Local(PathBuf),
    /// A file on a console-shaped target: its directory (`/Usb0`) and its name
    /// as the filesystem spells it.
    Remote { dir: String, name: String },
}

impl IniLocation {
    /// The path as shown to the user: the console's own notation
    /// (`Usb0:\launch.ini`) for a console, the OS path for a local drive.
    pub fn display(&self) -> String {
        match self {
            IniLocation::Local(path) => path.to_string_lossy().to_string(),
            IniLocation::Remote { dir, name } => {
                format!("{}:\\{name}", dir.trim_matches('/').replace('/', "\\"))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct LaunchIni {
    pub location: IniLocation,
    pub settings: DashlaunchSettings,
}

impl Target {
    /// Finds the `launch.ini` Dashlaunch would read on this target, if any.
    /// Involves I/O (network or raw disk for a console): run it off the UI
    /// thread.
    pub fn find_launch_ini(&self) -> Result<Option<LaunchIni>> {
        match self {
            Target::Local(mount) => {
                let Some(path) = find_child_ci(mount, INI_NAME).filter(|p| p.is_file()) else {
                    return Ok(None);
                };
                let text = read_text(fs::read(&path).context("reading launch.ini")?)?;
                Ok(Some(LaunchIni {
                    location: IniLocation::Local(path),
                    settings: DashlaunchSettings::parse(&text),
                }))
            }
            _ => {
                let mut session = self.open_remote(false)?;
                let found = find_remote(&mut session)?;
                session.quit()?;
                Ok(found)
            }
        }
    }

    /// Sets `changes` in the `launch.ini` at `location` and returns the
    /// settings as written. The file is read again right before the write, so
    /// an edit made elsewhere in the meantime is kept.
    pub fn update_launch_ini(
        &self,
        location: &IniLocation,
        changes: &[(&str, bool)],
    ) -> Result<DashlaunchSettings> {
        match (self, location) {
            (Target::Local(_), IniLocation::Local(path)) => {
                let text = read_text(fs::read(path).context("reading launch.ini")?)?;
                let text = apply(&text, changes)?;
                // Written beside it then renamed over it, so a pulled USB key
                // never leaves a half-written launch.ini behind.
                let tmp = path.with_extension("ini.txbm-tmp");
                fs::write(&tmp, &text).context("writing launch.ini")?;
                fs::rename(&tmp, path).context("replacing launch.ini")?;
                Ok(DashlaunchSettings::parse(&text))
            }
            (Target::Ftp(_) | Target::Fatx(_), IniLocation::Remote { dir, name }) => {
                let mut session = self.open_remote(true)?;
                let text = read_text(session.download_file(&format!("{dir}/{name}"))?)?;
                let text = apply(&text, changes)?;
                session.put_bytes(dir, name, text.as_bytes())?;
                // For a FATX drive this is the flush that makes it durable.
                session.quit()?;
                Ok(DashlaunchSettings::parse(&text))
            }
            _ => bail!("the target changed since launch.ini was found"),
        }
    }
}

/// Looks in the order Dashlaunch does: USB drives first, then the hard drive.
fn find_remote(session: &mut dyn RemoteFs) -> Result<Option<LaunchIni>> {
    let mut roots: Vec<String> = session
        .list_root()?
        .into_iter()
        .filter(|r| {
            let r = r.to_ascii_lowercase();
            r.starts_with("usb") || r == "hdd1"
        })
        .collect();
    // `false` sorts first: the USB drives, in name order, then Hdd1.
    roots.sort_by_key(|r| (r.eq_ignore_ascii_case("hdd1"), r.to_ascii_lowercase()));

    for root in roots {
        let dir = format!("/{root}");
        let Some(entry) = session
            .list_dir(&dir)
            .into_iter()
            .find(|e| !e.is_dir && e.name.eq_ignore_ascii_case(INI_NAME))
        else {
            continue;
        };
        let text = read_text(session.download_file(&format!("{dir}/{}", entry.name))?)?;
        return Ok(Some(LaunchIni {
            location: IniLocation::Remote {
                dir,
                name: entry.name,
            },
            settings: DashlaunchSettings::parse(&text),
        }));
    }
    Ok(None)
}

fn read_text(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).context("launch.ini isn't a plain text file")
}

/// Splits a line into `(key, value)` when it is a `key = value` entry. A
/// trailing `; comment` is not part of the value.
fn entry(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.starts_with(';') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let value = value.split(';').next().unwrap_or_default();
    Some((key.trim(), value.trim()))
}

/// Section name when the line is a `[Section]` header.
fn section(line: &str) -> Option<&str> {
    line.trim().strip_prefix('[')?.strip_suffix(']').map(str::trim)
}

fn get_bool(text: &str, key: &str) -> Option<bool> {
    let mut in_settings = false;
    for line in text.lines() {
        if let Some(name) = section(line) {
            in_settings = name.eq_ignore_ascii_case("Settings");
            continue;
        }
        if !in_settings {
            continue;
        }
        if let Some((k, v)) = entry(line)
            && k.eq_ignore_ascii_case(key)
        {
            return match v.to_ascii_lowercase().as_str() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                // An unreadable value: Dashlaunch falls back to its default.
                _ => None,
            };
        }
    }
    None
}

/// Applies every change to the file's text, touching nothing else.
fn apply(text: &str, changes: &[(&str, bool)]) -> Result<String> {
    let mut text = text.to_string();
    for &(key, value) in changes {
        text = set_bool(&text, key, value)?;
    }
    Ok(text)
}

/// Rewrites only the value of `key` in `[Settings]`, keeping the spacing
/// around it, any trailing comment and the line ending. A missing key is added
/// after the section's last entry.
fn set_bool(text: &str, key: &str, value: bool) -> Result<String> {
    let value = if value { "true" } else { "false" };
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };

    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    let mut in_settings = false;
    let mut found_section = false;
    let mut found_key = false;
    // Index after which a missing key goes: the section's last entry, or its
    // header while it has none.
    let mut insert_after = None;

    for (i, line) in lines.iter_mut().enumerate() {
        if let Some(name) = section(line) {
            in_settings = name.eq_ignore_ascii_case("Settings");
            if in_settings {
                found_section = true;
                insert_after = Some(i);
            }
            continue;
        }
        if !in_settings {
            continue;
        }
        let Some((k, _)) = entry(line) else {
            continue;
        };
        insert_after = Some(i);
        if !k.eq_ignore_ascii_case(key) {
            continue;
        }
        // Dashlaunch reads the first occurrence, but a duplicate is rewritten
        // too so the file never says two things.
        found_key = true;
        let eq = line.find('=').expect("an entry has an '='");
        let rest = &line[eq + 1..];
        let start = eq + 1 + (rest.len() - rest.trim_start_matches([' ', '\t']).len());
        let end = start
            + line[start..]
                .find([' ', '\t', ';', '\r', '\n'])
                .unwrap_or(line.len() - start);
        line.replace_range(start..end, value);
    }

    if !found_section {
        bail!("launch.ini has no [Settings] section");
    }
    if !found_key {
        let at = insert_after.expect("set with the section");
        // The line we insert after may be the file's last one, without a
        // line ending of its own.
        if !lines[at].ends_with('\n') {
            lines[at].push_str(newline);
        }
        lines.insert(at + 1, format!("{key} = {value}{newline}"));
    }
    Ok(lines.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INI: &str = "[Paths]\r\ncontpatch = true\r\n\r\n[Settings]\r\n; comment xblapatch = true\r\ncontpatch = false ; inline\r\nliveblock=false\r\n\r\n[Plugins]\r\nplugin1 = \r\n";

    #[test]
    fn reads_only_settings_with_defaults() {
        let s = DashlaunchSettings::parse(INI);
        assert!(!s.contpatch);
        assert!(!s.xblapatch);
        assert!(!s.liveblock);
        // Missing: Dashlaunch's default.
        assert!(s.noupdater);
    }

    #[test]
    fn rewrites_the_value_only() {
        let out = set_bool(INI, CONTPATCH, true).unwrap();
        assert!(out.contains("contpatch = true ; inline\r\n"));
        // The [Paths] one and the comment are left alone.
        assert!(out.starts_with("[Paths]\r\ncontpatch = true\r\n"));
        assert!(out.contains("; comment xblapatch = true\r\n"));
        let out = set_bool(&out, LIVEBLOCK, true).unwrap();
        assert!(out.contains("liveblock=true\r\n"));
    }

    #[test]
    fn inserts_a_missing_key_at_the_end_of_the_section() {
        let out = set_bool(INI, LICPATCH, true).unwrap();
        assert!(out.contains("liveblock=false\r\nlicpatch = true\r\n\r\n[Plugins]"));
        let out = set_bool("[Settings]", XBLAPATCH, true).unwrap();
        assert_eq!(out, "[Settings]\nxblapatch = true\n");
    }

    #[test]
    fn refuses_a_file_without_settings() {
        assert!(set_bool("[Paths]\n", CONTPATCH, true).is_err());
    }

    #[test]
    fn displays_console_paths() {
        let loc = IniLocation::Remote {
            dir: "/Usb0".into(),
            name: "launch.ini".into(),
        };
        assert_eq!(loc.display(), "Usb0:\\launch.ini");
    }
}
