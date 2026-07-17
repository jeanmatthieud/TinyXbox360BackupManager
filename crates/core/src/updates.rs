// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use semver::Version;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const URL: &str =
    "https://api.github.com/repos/jeanmatthieud/TinyXbox360BackupManager/releases/latest";

#[derive(serde::Deserialize)]
struct Response {
    pub tag_name: String,
}

pub fn check() -> Result<Option<Version>> {
    if cfg!(debug_assertions) || std::env::var("TXBM_DISABLE_UPDATES").is_ok_and(|v| v == "1") {
        return Ok(None);
    }

    let resp = ureq::get(URL)
        .header("User-Agent", concat!("TinyXbox360BackupManager/", env!("CARGO_PKG_VERSION")))
        .call()?
        .body_mut()
        .read_json::<Response>()?;

    let version = match resp.tag_name.strip_prefix('v') {
        Some(v) => v,
        None => &resp.tag_name,
    };

    let current_version = Version::parse(CURRENT_VERSION)?;
    let version = Version::parse(version)?;

    if version > current_version {
        Ok(Some(version))
    } else {
        Ok(None)
    }
}
