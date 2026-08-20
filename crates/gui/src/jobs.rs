// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

//! Execution of one queued job on a worker thread. Every operation that writes
//! to the target goes through here, one at a time — see
//! [`txbm_core::job_queue::QueuedJob`].

use crate::{AppWindow, Dispatcher, DisplayedJob, JobKind, Message, UiState};
use slint::{ComponentHandle, SharedString, ToSharedString, Weak};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
use txbm_core::{
    config::Config,
    game::Game,
    game_details::ContentKind,
    job_queue::{self, QueuedJob},
    target::{DELETION_CANCELLED, Target},
};

/// Queue row for a job: the label it shows, plus the kind driving its icon.
impl From<&QueuedJob> for DisplayedJob {
    fn from(job: &QueuedJob) -> Self {
        Self {
            label: job.label().to_shared_string(),
            kind: job.kind().into(),
        }
    }
}

/// The Slint mirror of the core's job kinds (`ui/types.slint`).
impl From<job_queue::JobKind> for JobKind {
    fn from(kind: job_queue::JobKind) -> Self {
        match kind {
            job_queue::JobKind::Add => JobKind::Add,
            job_queue::JobKind::Delete => JobKind::Delete,
            job_queue::JobKind::DeleteContent => JobKind::DeleteContent,
        }
    }
}

/// Runs `job` to completion, reports it, then hands the queue back to the
/// update loop with a `JobFinished` message. Called on a worker thread.
pub fn perform_job(
    job: QueuedJob,
    config: &Config,
    cancel: Arc<AtomicBool>,
    weak: &Weak<AppWindow>,
) {
    let res = match &job {
        QueuedJob::Add(in_path) => perform_add(in_path, config, &cancel, weak),
        QueuedJob::Delete(game) => perform_delete(game, config, &cancel, weak),
        QueuedJob::DeleteContent {
            game,
            kind,
            file_name,
            ..
        } => perform_delete_content(game, *kind, file_name, config, &cancel, weak),
    };

    let _ = weak.upgrade_in_event_loop(move |app| {
        let dispatcher = app.global::<Dispatcher<'_>>();

        dispatcher.invoke_dispatch(Message::SetStatus, SharedString::new());

        let succeeded = res.is_ok();

        match res {
            Ok(()) => {
                if let Some(text) = success_text(&job) {
                    dispatcher.invoke_dispatch(Message::NotifyInfo, text);
                }
            }
            // The whole cause chain is searched: the cancellation marker is
            // often wrapped in a context (e.g. "extracting foo.xex: …").
            Err(e) if is_cancellation(&e) => {
                dispatcher.invoke_dispatch(Message::NotifyInfo, cancelled_text(&job));
            }
            Err(e) => {
                dispatcher.invoke_dispatch(Message::NotifyError, failure_text(&job, &e));
            }
        }

        // Drop the finished job and move on to the next one (even on failure,
        // so the queue doesn't stall). The payload tells the handler whether
        // this one is worth celebrating when the queue drains.
        let outcome = if succeeded { "ok" } else { "" };
        dispatcher.invoke_dispatch(Message::JobFinished, outcome.into());

        match &job {
            // Both change what the library holds: rescan it.
            QueuedJob::Add(_) | QueuedJob::Delete(_) => {
                dispatcher.invoke_dispatch(Message::RefreshAll, SharedString::new());
            }
            // A component removal only changes what is inside one game, so it
            // refreshes the info modal (when still open on that game) rather
            // than paying for a full rescan of the target.
            QueuedJob::DeleteContent { game, .. } => {
                let ui_state = app.global::<UiState<'_>>();
                let current = ui_state.get_current_game();
                if Path::new(current.path.as_str()) == game.path {
                    dispatcher.invoke_dispatch(Message::FetchGameDetails, current.path);
                }
            }
        }
    });
}

/// True when `e` (or any of its causes) is one of the core's cancellation
/// markers rather than a genuine failure.
fn is_cancellation(e: &anyhow::Error) -> bool {
    let text = format!("{e:#}");
    text.contains(txbm_core::convert::CONVERSION_CANCELLED) || text.contains(DELETION_CANCELLED)
}

/// Notification shown when the job succeeded, if it deserves one. An addition
/// stays silent: the library refresh speaks for it.
fn success_text(job: &QueuedJob) -> Option<SharedString> {
    match job {
        QueuedJob::Add(_) => None,
        QueuedJob::Delete(game) => Some(slint::format!("{} deleted", game.title)),
        QueuedJob::DeleteContent { .. } => Some("Content deleted".into()),
    }
}

fn cancelled_text(job: &QueuedJob) -> SharedString {
    match job {
        QueuedJob::Add(_) => "Conversion cancelled".into(),
        // Unlike a conversion, a cancelled deletion leaves whatever it already
        // removed removed: say so instead of implying nothing happened.
        QueuedJob::Delete(game) => slint::format!(
            "Deletion cancelled\n{} is only partially removed; delete it again to finish",
            game.title
        ),
        QueuedJob::DeleteContent { .. } => {
            "Deletion cancelled\nThe content is only partially removed".into()
        }
    }
}

fn failure_text(job: &QueuedJob, e: &anyhow::Error) -> SharedString {
    match job {
        QueuedJob::Add(_) => slint::format!("Conversion failed: {e:#}"),
        QueuedJob::Delete(_) => slint::format!("Failed to delete game: {e:#}"),
        QueuedJob::DeleteContent { .. } => slint::format!("Failed to delete content: {e:#}"),
    }
}

/// ISO (or archive) added to the target: GOD conversion or extraction, then
/// transfer.
fn perform_add(
    in_path: &Path,
    config: &Config,
    cancel: &AtomicBool,
    weak: &Weak<AppWindow>,
) -> anyhow::Result<()> {
    let filename = in_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let weak2 = weak.clone();
    let update_progress = move |percentage: u32, speed: Option<f64>| {
        let status = match speed {
            Some(mbps) => {
                slint::format!("↑  Adding  {filename}  {percentage}%  ({mbps:.1} MB/s)")
            }
            None => slint::format!("↑  Adding  {filename}  {percentage}%"),
        };

        // The same progress feeds the status bar (visible from every
        // page) and, as structured values, the card on the queue page.
        // A missing speed (first callback of each uploaded file, before
        // any time has elapsed) keeps the previous one rather than
        // blanking the card once per file.
        let speed = speed.map(|mbps| slint::format!("{mbps:.1} MB/s"));

        let _ = weak2.upgrade_in_event_loop(move |app| {
            let ui = app.global::<UiState<'_>>();
            ui.set_status(status);
            ui.set_job_progress(percentage as f32 / 100.0);
            if let Some(speed) = speed {
                ui.set_job_speed(speed);
            }
        });
    };

    // Phases with no measurable progress (the post-cancellation
    // cleanup) take over the status line: without this the UI looks
    // frozen while thousands of extracted files are deleted.
    let weak3 = weak.clone();
    let set_status = move |text: &str| {
        let text = SharedString::from(text);
        let _ = weak3.upgrade_in_event_loop(move |app| {
            let ui = app.global::<UiState<'_>>();
            ui.set_status(text.clone());
            // These phases have no progress of their own, so they take
            // over the card's phase line too until it is cleared. The
            // speed measured by the previous phase no longer applies.
            ui.set_job_phase(text);
            ui.set_job_speed(SharedString::new());
        });
    };

    // Step the percentage currently refers to, shown on the queue page.
    let weak4 = weak.clone();
    let set_phase = move |text: &str| {
        let text = SharedString::from(text);
        let _ = weak4.upgrade_in_event_loop(move |app| {
            let ui = app.global::<UiState<'_>>();
            ui.set_job_phase(text);
            // Each step measures (or doesn't measure) its own speed.
            ui.set_job_speed(SharedString::new());
        });
    };

    txbm_core::convert::perform(
        in_path.to_path_buf(),
        config,
        cancel,
        &update_progress,
        &set_status,
        &set_phase,
    )
}

/// Game removed from the target, along with its separate DLC/title-update
/// folder when it has one.
fn perform_delete(
    game: &Game,
    config: &Config,
    cancel: &AtomicBool,
    weak: &Weak<AppWindow>,
) -> anyhow::Result<()> {
    let target = target_or_err(config)?;
    let update_progress = progress_reporter(
        weak.clone(),
        slint::format!("✕  Deleting  {}", game.title),
    );

    target.delete_game(game, cancel, &update_progress)
}

/// One stored component (a disc package or a DLC file) removed from a game.
fn perform_delete_content(
    game: &Game,
    kind: ContentKind,
    file_name: &str,
    config: &Config,
    cancel: &AtomicBool,
    weak: &Weak<AppWindow>,
) -> anyhow::Result<()> {
    let target = target_or_err(config)?;
    let update_progress =
        progress_reporter(weak.clone(), SharedString::from("✕  Deleting content"));

    target.delete_content(game, kind, file_name, cancel, &update_progress)
}

fn target_or_err(config: &Config) -> anyhow::Result<Target> {
    Target::from_config(&config.contents).ok_or_else(|| anyhow::anyhow!("no target selected"))
}

/// Percentage callback feeding both the status bar and the progress card, with
/// a fixed `prefix` naming what is being removed. Deletions report no speed,
/// so the card's speed line stays empty.
fn progress_reporter(weak: Weak<AppWindow>, prefix: SharedString) -> impl Fn(u32) {
    move |percentage: u32| {
        let status = slint::format!("{prefix}  {percentage}%");
        let _ = weak.upgrade_in_event_loop(move |app| {
            let ui = app.global::<UiState<'_>>();
            ui.set_status(status);
            ui.set_job_progress(percentage as f32 / 100.0);
        });
    }
}
