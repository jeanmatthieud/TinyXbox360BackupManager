// SPDX-License-Identifier: GPL-3.0-only

//! User-facing wording for a failed Toolbox download.
//!
//! The raw error ("requesting https://…: http status: 500") says nothing the
//! user can act on, so a download failure is looked for in the error chain and
//! turned into a short sentence with a way out. The raw error still goes to
//! stderr, for a bug report.

use txbm_core::{badavatar::ComponentDownloadError, download::DownloadError};

/// Where the Toolbox download URLs are edited.
const DOWNLOAD_SOURCES: &str = "Settings › Download sources";

/// Rewords a BadAvatar component download failure, pointing at the settings
/// where its source can be changed. `None` if `err` is not one.
pub fn badavatar(err: &anyhow::Error) -> Option<String> {
    let failure = ComponentDownloadError::find(err)?;
    log(&failure.cause);
    let advice = if failure.field.sources().is_empty() {
        format!("Try again later, or point it at another URL in {DOWNLOAD_SOURCES}.")
    } else {
        format!("Pick another source for it in {DOWNLOAD_SOURCES}.")
    };
    Some(format!("{}. {advice}", failure.cause))
}

/// Rewords a compatibility pack or configs download failure. A built-in pack's
/// URL cannot be changed, but another pack can be added and the configs URL
/// edited, both in the settings. `None` if `err` is not a download failure.
pub fn compat(err: &anyhow::Error) -> Option<String> {
    let failure = DownloadError::find(err)?;
    log(failure);
    Some(format!(
        "{failure}. The host may be down for now — try again later, or use another \
         source in {DOWNLOAD_SOURCES}."
    ))
}

fn log(failure: &DownloadError) {
    eprintln!(
        "Download of {} failed ({}): {}",
        failure.label, failure.url, failure.detail
    );
}
