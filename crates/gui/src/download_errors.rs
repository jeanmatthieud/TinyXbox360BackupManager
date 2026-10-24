// SPDX-License-Identifier: GPL-3.0-only

//! User-facing wording for a failed Toolbox download.
//!
//! The raw error ("requesting https://…: http status: 500") says nothing the
//! user can act on, so a download failure is looked for in the error chain and
//! turned into a short sentence with a way out. The raw error still goes to
//! stderr, for a bug report.

use txbm_core::{abadavatar::ComponentDownloadError, download::DownloadError};

/// Where the Toolbox download URLs are edited.
const DOWNLOAD_SOURCES: &str = "Paramètres › Sources de téléchargement";

/// Rewords an ABadAvatar component download failure, pointing at the settings
/// where its source can be changed. `None` if `err` is not one.
pub fn abadavatar(err: &anyhow::Error) -> Option<String> {
    let failure = ComponentDownloadError::find(err)?;
    log(&failure.cause);
    let advice = if failure.field.sources().is_empty() {
        format!("Réessayez plus tard ou choisissez une autre adresse dans {DOWNLOAD_SOURCES}.")
    } else {
        format!("Choisissez une autre source dans {DOWNLOAD_SOURCES}.")
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
        "{failure}. Le serveur est peut-être indisponible : réessayez plus tard ou choisissez une autre source dans {DOWNLOAD_SOURCES}."
    ))
}

fn log(failure: &DownloadError) {
    eprintln!(
        "Échec du téléchargement de {} ({}) : {}",
        failure.label, failure.url, failure.detail
    );
}
