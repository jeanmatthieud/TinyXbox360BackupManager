// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AppWindow, Dispatcher, DisplayedConfig, DisplayedDriveInfo, DisplayedFatxDrive, DisplayedGame,
    DisplayedGameToAdd, DisplayedJob, DisplayedStoragePath, DisplayedTitleUpdate, JobKind, Message,
    Notification, Page,
    PendingQueueAction, UiState, covers, dialogs, game_details, jobs::perform_job, state::State,
    title_updates, util,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, ToSharedString, VecModel, Weak};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use txbm_core::{
    badavatar::UrlField, config::TargetKind, data_dir::DATA_DIR, drive_info::DriveInfo,
    ftp::FtpSession, game::Game, game_details::ContentKind, job_queue::QueuedJob,
    target::{StorageConfig, Target, TargetAnalysis},
};

/// Shown once per console drive, the first time it is opened directly. Writing
/// to FATX from a PC is young, and a mistake here costs the user their whole
/// game library — so they get told before, not after.
const NEW_FATX_DRIVE_TEXT: &str = "Xbox 360 hard drive connected\nThis writes to the console's own filesystem directly. Back the drive up before adding or deleting games.";

const NEW_DRIVE_TEXT: &str = "New drive detected\nOnce the games are on the console, remember to add the content paths in Aurora\n(Settings > Content Paths)";

/// Result of the asynchronous scan of the target, deposited by the scan thread
/// then retrieved in the ScanFinished handler.
static SCAN_RESULT: Mutex<Option<anyhow::Result<(Vec<Game>, DriveInfo)>>> = Mutex::new(None);

/// Result of the asynchronous target analysis started when connecting to a
/// target with no `.txbm.json` yet, retrieved in TargetAnalysisFinished.
static ANALYSIS_RESULT: Mutex<Option<anyhow::Result<TargetAnalysis>>> = Mutex::new(None);

/// Result of the asynchronous network scan started from the FTP modal:
/// `Some(ip)` when a console was found, `None` when the scan finished without
/// a match. Deposited by the scan thread, retrieved in FtpScanFinished.
static FTP_SCAN_RESULT: Mutex<Option<Option<String>>> = Mutex::new(None);

/// Result of the asynchronous disc/DLC fetch for the game shown in the
/// info modal, deposited by the fetch thread then retrieved in the
/// GameDetailsFetched handler.
static GAME_DETAILS_RESULT: Mutex<Option<(PathBuf, txbm_core::game_details::GameDetails)>> =
    Mutex::new(None);

/// Result of the asynchronous title-update fetch for the Title Updates
/// window, deposited by the fetch thread then retrieved in the
/// TitleUpdatesFetched handler.
#[allow(clippy::type_complexity)]
static TITLE_UPDATES_RESULT: Mutex<
    Option<(PathBuf, anyhow::Result<Vec<DisplayedTitleUpdate>>)>,
> = Mutex::new(None);

impl State {
    /// Fills the "add these conversions to the queue?" confirmation from the
    /// picked files, dropping those already queued and flagging each one that
    /// would overwrite an installed game.
    ///
    /// The check is done here, before the queue is confirmed, rather than left
    /// to the conversion: the user gets to see it while they can still back
    /// out. It costs no I/O — the TitleID comes from the inspection the pick
    /// already did, and the installed games are the ones the last scan found.
    ///
    /// It is therefore only as good as the TitleID the pick could read: ISOs
    /// and Arcade packages are covered, archives never are (see
    /// [`util::PickedGame::installs_title_id`]). An input with no TitleID is
    /// queued silently — the conversion overwrites just the same, it simply
    /// isn't announced.
    fn set_games_to_add(&mut self, mut picked: Vec<util::PickedGame>, weak: &Weak<AppWindow>) {
        // Files already waiting in the queue (index 0 — the running one —
        // included) never make it to the confirmation: queueing the same input
        // twice would just convert it twice over the same output. Duplicates
        // inside the picked batch itself are dropped the same way.
        let mut skipped = Vec::new();
        let mut kept: Vec<PathBuf> = Vec::new();
        picked.retain(|game| {
            let queued = self
                .job_queue
                .iter()
                .any(|job| job.path() == game.path)
                || kept.contains(&game.path);

            if queued {
                skipped.push(
                    game.path
                        .file_name()
                        .unwrap_or(game.path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                );
            } else {
                kept.push(game.path.clone());
            }
            !queued
        });

        if !skipped.is_empty() {
            let text = match skipped.as_slice() {
                [name] => format!("Already in the queue: {name}"),
                names => format!("{} files are already in the queue", names.len()),
            };
            self.notifications.push(Notification::info(text));
        }

        // Nothing new: leave the pending confirmation (if any) untouched
        // instead of clearing it with an empty selection.
        if picked.is_empty() {
            return;
        }

        let displayed = picked
            .iter()
            .map(|game| {
                let name = match game.path.file_name() {
                    Some(filename) => filename.to_string_lossy().to_shared_string(),
                    None => "?".to_shared_string(),
                };

                // An `incomplete` entry only holds DLC/title updates for that
                // TitleID: installing the game itself completes it instead of
                // overwriting anything, so it stays unflagged even though a
                // card for that game is visible in the library.
                let installed = game.installs_title_id.as_deref().and_then(|tid| {
                    self.games
                        .iter()
                        .find(|g| !g.incomplete && g.id.eq_ignore_ascii_case(tid))
                });

                DisplayedGameToAdd {
                    name,
                    already_installed: installed.is_some(),
                    installed_title: installed
                        .map(|g| g.title.to_shared_string())
                        .unwrap_or_default(),
                }
            })
            .collect::<Vec<_>>();

        let conflicts = displayed.iter().filter(|g| g.already_installed).count() as i32;
        self.games_to_add = picked.into_iter().collect();
        self.displayed_games_to_add.set_vec(displayed);

        let app = weak.upgrade().unwrap();
        app.global::<UiState<'_>>()
            .set_games_to_add_conflicts(conflicts);
    }

    /// The job currently running, if any. It sits at index 0 of the queue
    /// until `JobFinished` removes it.
    fn running_job(&self) -> Option<&QueuedJob> {
        self.is_job_running.then(|| self.job_queue.front()).flatten()
    }

    /// True when a job is already writing to a console-shaped target, and so
    /// when nothing else may write to it.
    ///
    /// Only writes are exclusive: reading beside them is fine, and several
    /// connections may read at once. What must never happen is a second write
    /// — two uploads at once over FTP leave games the console cannot read, and
    /// on a FATX drive two writers would each rearrange a FAT the other holds
    /// a stale copy of. Serializing that single write is the whole point of
    /// the job queue, so "a job is running" is exactly "a write is in flight",
    /// and anything writing outside the queue has to check this.
    ///
    /// A local drive has no such constraint, and neither has "no target".
    fn console_write_in_flight(&self) -> bool {
        self.is_job_running
            && matches!(
                Target::from_config(&self.config.contents),
                Some(Target::Ftp(_) | Target::Fatx(_))
            )
    }

    /// Cancels the whole job queue: the running item (index 0) is only
    /// signalled to stop — it bails at its next cancellation checkpoint, cleans
    /// up its partial output, and is removed by `JobFinished` — while the
    /// pending ones are dropped right away.
    fn cancel_all_jobs(&mut self, weak: &Weak<AppWindow>) {
        // An aborted batch doesn't get confetti, however many items it had
        // already converted — including the running one, which may well finish
        // cleanly before it notices the cancel flag.
        self.adds_done = 0;
        self.jobs_failed = 0;
        self.batch_cancelled = true;

        if self.is_job_running {
            self.job_cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        let pending_start = if self.is_job_running { 1 } else { 0 };
        while self.job_queue.len() > pending_start {
            let _ = self.job_queue.remove(pending_start);
            let _ = self.displayed_job_queue.remove(pending_start);
        }

        // Only meaningful while the running job is still bailing out; with
        // nothing left the queue is already gone.
        let app = weak.upgrade().unwrap();
        app.global::<UiState<'_>>()
            .set_cancelling_queue(!self.job_queue.is_empty());
    }

    /// Appends `job` to the queue and starts the runner when it is idle.
    /// A job already queued (the very same work, running one included) is
    /// reported instead of being queued twice.
    fn enqueue_job(
        &mut self,
        job: QueuedJob,
        message_queue: &mut VecDeque<(Message, SharedString)>,
        weak: &Weak<AppWindow>,
    ) {
        if self.job_queue.iter().any(|queued| queued.is_same_as(&job)) {
            self.notifications
                .push(Notification::info("Already in the queue"));
            return;
        }

        self.displayed_job_queue.push(DisplayedJob::from(&job));
        self.job_queue.push_back(job);

        // Queueing new work supersedes an in-flight "cancel all", so the fresh
        // batch is eligible for the celebration again.
        let app = weak.upgrade().unwrap();
        app.global::<UiState<'_>>().set_cancelling_queue(false);
        self.batch_cancelled = false;

        if !self.is_job_running {
            self.is_job_running = true;
            message_queue.push_back((Message::TriggerJob, SharedString::new()));
        }
    }

    /// Carries out the disconnect/quit the user asked for while the queue was
    /// still busy, now that it has been drained. No-op when nothing is pending.
    fn run_pending_queue_action(
        &mut self,
        message_queue: &mut VecDeque<(Message, SharedString)>,
        weak: &Weak<AppWindow>,
    ) {
        let app = weak.upgrade().unwrap();
        let ui = app.global::<UiState<'_>>();

        let action = ui.get_pending_queue_action();
        ui.set_pending_queue_action(PendingQueueAction::None);
        ui.set_draining_queue(false);

        match action {
            PendingQueueAction::None => {}
            PendingQueueAction::Disconnect => {
                message_queue.push_back((Message::Disconnect, SharedString::new()));
            }
            PendingQueueAction::Quit => {
                let _ = slint::quit_event_loop();
            }
        }
    }

    /// Switches the target to the Xbox 360 hard drive (or disk image) at
    /// `device`, records it in the recent locations, and queues a config sync
    /// + target analysis. Shared by the FATX drive picker and the hidden
    /// disk-image picker.
    fn select_fatx_device(
        &mut self,
        device: PathBuf,
        message_queue: &mut VecDeque<(Message, SharedString)>,
    ) {
        self.config.contents.target_kind = TargetKind::Fatx;
        self.config.contents.fatx = txbm_core::fatx::FatxConfig::new(device.clone());

        if self.config.check_known_drive(&device) {
            self.notifications
                .push(Notification::error(NEW_FATX_DRIVE_TEXT));
        }
        self.config.contents.record_recent_location();

        message_queue.push_back((Message::SyncConfig, SharedString::new()));
        message_queue.push_back((Message::StartTargetAnalysis, SharedString::new()));
    }

    /// Switches the target to the local drive mounted at `path`, records it in
    /// the recent locations, and queues a config sync + target analysis. Shared
    /// by the removable-drive picker and the debug folder picker.
    fn select_local_mount(
        &mut self,
        path: PathBuf,
        message_queue: &mut VecDeque<(Message, SharedString)>,
    ) {
        self.config.contents.target_kind = TargetKind::Local;
        self.config.contents.mount_point = path;

        if self.config.check_mount_point() {
            self.notifications.push(Notification::info(NEW_DRIVE_TEXT));
        }
        self.config.contents.record_recent_location();

        message_queue.push_back((Message::SyncConfig, SharedString::new()));
        message_queue.push_back((Message::StartTargetAnalysis, SharedString::new()));
    }

    pub fn update(
        &mut self,
        message: Message,
        payload: SharedString,
        message_queue: &mut VecDeque<(Message, SharedString)>,
        weak: &Weak<AppWindow>,
    ) {
        match message {
            Message::NotifyInfo => {
                self.notifications.push(Notification::info(payload));
            }
            Message::NotifySuccess => {
                self.notifications.push(Notification::success(payload));
            }
            Message::NotifySuccessSticky => {
                self.notifications.push(Notification::success_sticky(payload));
            }
            Message::NotifyError => {
                self.notifications.push(Notification::error(payload));
            }
            Message::SyncConfig => {
                let app = weak.upgrade().unwrap();

                let ui_state = app.global::<UiState<'_>>();
                ui_state.set_config(DisplayedConfig::from(&self.config));
                ui_state.set_badavatar(crate::config::displayed_badavatar(&self.config));
                ui_state.set_recent_locations(ModelRc::from(Rc::new(VecModel::from(
                    crate::config::recent_locations(&self.config),
                ))));

                if let Err(e) = self.config.write() {
                    let text = slint::format!("Failed to write config: {e}");
                    self.notifications.push(Notification::error(text));
                }
            }
            Message::PickMountPoint => {
                let app = weak.upgrade().unwrap();
                let window_handle = app.window().window_handle();

                if let Some(path) = dialogs::pick_mount_point(&window_handle) {
                    self.select_local_mount(path, message_queue);
                }
            }
            Message::RefreshRemovableDrives => {
                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_removable_drives(ModelRc::from(
                    Rc::new(VecModel::from(crate::config::removable_drives())),
                ));
            }
            Message::SelectRemovableDrive => {
                let path = PathBuf::from(payload.as_str());
                if !path.is_dir() {
                    self.notifications
                        .push(Notification::error("This drive is no longer available"));
                    // The confirm button already closed the modal; re-open it so
                    // the user can pick another drive instead of being stuck.
                    let app = weak.upgrade().unwrap();
                    app.global::<UiState<'_>>().set_selecting_target(true);
                    return;
                }
                self.select_local_mount(path, message_queue);
            }
            Message::RefreshFatxDrives => {
                let app = weak.upgrade().unwrap();
                let ui_state = app.global::<UiState<'_>>();
                if ui_state.get_scanning_fatx_drives() {
                    return;
                }

                // Enumerating means opening every physical disk and reading a
                // block several gigabytes into it: a sleeping external drive
                // takes seconds to answer, and each candidate is tried in turn.
                // Far too much for the event loop, so the picker shows a
                // progress line while a thread does it.
                ui_state.set_scanning_fatx_drives(true);
                ui_state.set_fatx_drives(ModelRc::from(Rc::new(
                    VecModel::<DisplayedFatxDrive>::default(),
                )));

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let drives = txbm_core::fatx_dev::list_fatx_drives();

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let ui_state = app.global::<UiState<'_>>();
                        ui_state.set_fatx_drives(ModelRc::from(Rc::new(VecModel::from(
                            crate::config::displayed_fatx_drives(drives),
                        ))));
                        ui_state.set_scanning_fatx_drives(false);
                    });
                });
            }
            Message::SelectFatxDrive => {
                let device = PathBuf::from(payload.as_str());
                // Probing again on confirmation catches a drive unplugged
                // between the listing and the click, and reports the reason
                // (permissions above all) rather than a bare failure later.
                let probe = txbm_core::fatx_dev::probe_path(&device);
                if !probe.is_usable() {
                    let text = slint::format!("{}: {}", device.display(), probe.label());
                    self.notifications.push(Notification::error(text));
                    // The confirm button already closed the modal; re-open it so
                    // the user can pick another drive instead of being stuck.
                    let app = weak.upgrade().unwrap();
                    app.global::<UiState<'_>>().set_selecting_target(true);
                    return;
                }
                self.select_fatx_device(device, message_queue);
            }
            Message::PickFatxImage => {
                let app = weak.upgrade().unwrap();
                let window_handle = app.window().window_handle();

                let Some(path) = dialogs::pick_fatx_image(&window_handle) else {
                    return;
                };
                let probe = txbm_core::fatx_dev::probe_path(&path);
                if !probe.is_usable() {
                    let text = slint::format!("{}: {}", path.display(), probe.label());
                    self.notifications.push(Notification::error(text));
                    return;
                }
                self.select_fatx_device(path, message_queue);
            }
            Message::RefreshDisplayedGames => {
                // The game the running job is writing into, if it names one:
                // its folder is being rewritten (a second disc, a DLC, a
                // deletion…), so the library veils it rather than let it be
                // opened on contents that are in flux.
                let running = self.running_job();
                let busy_title_id = running.and_then(QueuedJob::writes_title_id);
                let busy_path = running.map(QueuedJob::path);

                let displayed_games = self
                    .games
                    .iter()
                    .filter(|game| {
                        let shown = match game.format {
                            txbm_core::game::GameFormat::Arcade => {
                                self.config.contents.show_arcade
                            }
                            _ if game.is_x360 => self.config.contents.show_x360,
                            _ => self.config.contents.show_og,
                        };
                        shown
                            && (self.games_filter.is_empty()
                                || game.search_term.contains(&self.games_filter))
                    })
                    .map(|game| {
                        let mut displayed = DisplayedGame::from(game);
                        // By TitleID, so a second disc queued for a game
                        // already installed veils that game; by path too, for
                        // a deletion of an extracted game that has no
                        // readable TitleID at all.
                        displayed.busy = busy_title_id
                            .is_some_and(|id| id.eq_ignore_ascii_case(&game.id))
                            || busy_path == Some(game.path.as_path());
                        displayed
                    })
                    .collect::<Vec<_>>();

                self.displayed_games.set_vec(displayed_games);

                // Whether the (unfiltered) library holds any game at all, so the
                // grid can tell "empty library" apart from "everything filtered
                // out" (see game-grid-page.slint).
                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>()
                    .set_has_games(!self.games.is_empty());
            }
            Message::ToggleShowX360 => {
                self.config.contents.show_x360 = !self.config.contents.show_x360;

                message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));
                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::ToggleShowArcade => {
                self.config.contents.show_arcade = !self.config.contents.show_arcade;

                message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));
                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::ToggleShowOg => {
                self.config.contents.show_og = !self.config.contents.show_og;

                message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));
                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::SetRemoveSourcesGames => {
                let value = payload.parse().unwrap();
                self.config.contents.remove_sources_games = value;

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::SetXbox360Format => {
                let value = payload.parse().unwrap();
                self.config.contents.xbox360_format = value;

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::SetThemePreference => {
                let value = payload.parse().unwrap();
                self.config.contents.theme_preference = value;

                #[cfg(windows)]
                if value == txbm_core::config::ThemePreference::Light {
                    crate::window_color::set(false);
                } else if value == txbm_core::config::ThemePreference::Dark {
                    crate::window_color::set(true);
                }

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::SetCoverSource => {
                let value = payload.parse().unwrap();
                self.config.contents.cover_source = value;

                // Deliberately nothing else: the cover cache is agnostic of
                // the source it was filled from, so switching must neither
                // clear it nor trigger a re-download. The new source only
                // applies to covers still missing at the next pass.
                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::SetAutoReconnect => {
                let value = payload.parse().unwrap();
                self.config.contents.auto_reconnect = value;

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::SetViewAs => {
                let value = payload.parse().unwrap();
                self.config.contents.view_as = value;

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::SetSortBy => {
                let value = payload.parse().unwrap();
                self.config.contents.sort_by = value;

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
                message_queue.push_back((Message::RefreshSorting, SharedString::new()));
            }
            Message::SetConsoleIp => {
                self.config.contents.console_ip = payload.to_string();
                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::SetFtpPort => {
                self.config.contents.ftp_port = payload.to_string();
                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::SetFtpUser => {
                self.config.contents.ftp_user = payload.to_string();
                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::SetFtpPassword => {
                self.config.contents.ftp_password = payload.to_string();
                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::RefreshSorting => {
                let compare_games = txbm_core::game::get_compare_fn(self.config.contents.sort_by);
                self.games.sort_by(compare_games);

                message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));
            }
            Message::RefreshAll => {
                let Some(target) = Target::from_config(&self.config.contents) else {
                    self.games.clear();
                    self.drive_info = DriveInfo::default();
                    let app = weak.upgrade().unwrap();
                    app.global::<UiState<'_>>()
                        .set_drive_info(DisplayedDriveInfo::from(&self.drive_info));
                    message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));
                    return;
                };

                // A scan already under way started before whatever just changed
                // the library, so its result cannot show it: the refresh is
                // held back rather than dropped. Dropping it left a game that
                // a queued add had just written invisible until the user
                // rescanned by hand.
                if self.is_scanning {
                    self.rescan_deferred = true;
                    return;
                }

                // A scan walks the whole library, the folder the running job is
                // writing to included, and would list a game that is only half
                // copied. Reading beside a write is allowed (see
                // `console_write_in_flight`), but reading *that* is pointless,
                // so the scan waits for the queue to drain. A local drive has
                // no such constraint, and keeps refreshing after every job of
                // a batch.
                if self.console_write_in_flight() {
                    self.rescan_deferred = true;
                    return;
                }

                self.is_scanning = true;
                self.scan_cancel.store(false, std::sync::atomic::Ordering::Relaxed);

                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_scanning(true);

                message_queue.push_back((
                    Message::SetStatus,
                    slint::format!("⟳  Scanning  {}", target.display()),
                ));

                let weak = weak.clone();
                let cancel = self.scan_cancel.clone();
                std::thread::spawn(move || {
                    let res = target.scan(&cancel);
                    *SCAN_RESULT.lock().unwrap() = Some(res);

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        app.global::<Dispatcher<'_>>()
                            .invoke_dispatch(Message::ScanFinished, SharedString::new());
                    });
                });
            }
            Message::CancelScan => {
                if self.is_scanning {
                    self.scan_cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    message_queue
                        .push_back((Message::SetStatus, "⟳  Cancelling…".to_shared_string()));
                }
            }
            Message::ScanFinished => {
                self.is_scanning = false;

                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_scanning(false);

                message_queue.push_back((Message::SetStatus, SharedString::new()));

                // A refresh asked for while this scan was running: it is only
                // worth anything now the scan is out of the way. Taken here so
                // it is dropped along with a cancelled or failed scan — the
                // user cancelled the walk, replaying it right away would undo
                // exactly what they asked for.
                let deferred = std::mem::take(&mut self.rescan_deferred);

                match SCAN_RESULT.lock().unwrap().take() {
                    Some(Ok((games, drive_info))) => {
                        self.games = games;
                        self.drive_info = drive_info;

                        app.global::<UiState<'_>>()
                            .set_drive_info(DisplayedDriveInfo::from(&self.drive_info));

                        if deferred {
                            message_queue.push_back((Message::RefreshAll, SharedString::new()));
                            return;
                        }

                        message_queue.push_back((Message::RefreshSorting, SharedString::new()));
                        message_queue.push_back((Message::DownloadCovers, SharedString::new()));
                    }
                    Some(Err(e)) if e.to_string().contains(txbm_core::target::SCAN_CANCELLED) => {
                        self.notifications.push(Notification::info("Scan cancelled"));
                    }
                    Some(Err(e)) => {
                        // A real scan failure (timeout, unreachable console...)
                        // means the target can no longer be trusted: disconnect,
                        // same as the explicit "Disconnect" button, so the UI
                        // doesn't keep looking connected (stale games/drive info,
                        // "Disconnect from..." button) after a failed target.
                        message_queue.push_back((Message::Disconnect, SharedString::new()));

                        let text = slint::format!("Failed to scan the target: {e:#}");
                        self.notifications.push(Notification::error(text));
                    }
                    None => {}
                }
            }
            Message::SetTargetFtp => {
                if self.config.contents.console_ip.trim().is_empty() {
                    let text = "No console IP configured";
                    self.notifications.push(Notification::error(text));
                    return;
                }

                self.config.contents.target_kind = TargetKind::Ftp;
                self.config.contents.record_recent_location();

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
                message_queue.push_back((Message::StartTargetAnalysis, SharedString::new()));
            }
            Message::ConnectRecentLocation => {
                let i: usize = payload.parse().unwrap();
                let Some(loc) = self.config.contents.recent_locations.get(i).cloned() else {
                    return;
                };

                match loc.kind {
                    TargetKind::Local => {
                        // The drive may have been unplugged since it was
                        // recorded: bail out and re-open the target selection.
                        if !loc.mount_point.is_dir() {
                            self.notifications.push(Notification::error(
                                "This location is no longer available (drive unplugged?)",
                            ));
                            let app = weak.upgrade().unwrap();
                            app.global::<UiState<'_>>().set_selecting_target(true);
                            return;
                        }
                        self.config.contents.target_kind = TargetKind::Local;
                        self.config.contents.mount_point = loc.mount_point;
                    }
                    TargetKind::Fatx => {
                        // The drive may have been unplugged since it was
                        // recorded, or the machine rebooted with the disks in a
                        // different order: probe before trusting the path.
                        let probe = txbm_core::fatx_dev::probe_path(&loc.fatx.device);
                        if !probe.is_usable() {
                            let text = slint::format!(
                                "{}: {}",
                                loc.fatx.device.display(),
                                probe.label()
                            );
                            self.notifications.push(Notification::error(text));
                            let app = weak.upgrade().unwrap();
                            app.global::<UiState<'_>>().set_selecting_target(true);
                            return;
                        }
                        self.config.contents.target_kind = TargetKind::Fatx;
                        self.config.contents.fatx = loc.fatx;
                    }
                    TargetKind::Ftp => {
                        self.config.contents.target_kind = TargetKind::Ftp;
                        self.config.contents.console_ip = loc.console_ip;
                        self.config.contents.ftp_port = loc.ftp_port;
                        self.config.contents.ftp_user = loc.ftp_user;
                        self.config.contents.ftp_password = loc.ftp_password;
                    }
                }
                // Move it back to the top of the list.
                self.config.contents.record_recent_location();

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
                message_queue.push_back((Message::StartTargetAnalysis, SharedString::new()));
            }
            Message::RemoveRecentLocation => {
                let i: usize = payload.parse().unwrap();
                if i < self.config.contents.recent_locations.len() {
                    self.config.contents.recent_locations.remove(i);
                    message_queue.push_back((Message::SyncConfig, SharedString::new()));
                }
            }
            Message::RequestDisconnect | Message::RequestQuit => {
                let action = if matches!(message, Message::RequestQuit) {
                    PendingQueueAction::Quit
                } else {
                    PendingQueueAction::Disconnect
                };

                // Nothing queued: go ahead straight away.
                if self.job_queue.is_empty() {
                    match action {
                        PendingQueueAction::Quit => {
                            let _ = slint::quit_event_loop();
                        }
                        _ => message_queue.push_back((Message::Disconnect, SharedString::new())),
                    }
                    return;
                }

                // Otherwise ask whether the queue should be cancelled first.
                let app = weak.upgrade().unwrap();
                let ui = app.global::<UiState<'_>>();
                ui.set_pending_queue_action(action);

                // Asking again while the queue is already draining: the
                // confirmation modal is suppressed in that state, so send the
                // user to the queue page instead of doing nothing visible.
                // The banner there reports the pending action and offers the
                // "Terminate" shortcut for those who don't want to wait.
                if ui.get_draining_queue() {
                    ui.set_current_page(Page::Jobs);
                }
            }
            Message::ForceQueueAction => {
                // "Terminate": go through with the disconnect/quit without
                // waiting for the job being cancelled to bail out.
                self.run_pending_queue_action(message_queue, weak);
            }
            Message::AbortQueueAction => {
                // The user declined to cancel the queue: drop the action.
                let app = weak.upgrade().unwrap();
                let ui = app.global::<UiState<'_>>();
                ui.set_pending_queue_action(PendingQueueAction::None);
                ui.set_draining_queue(false);
            }
            Message::ConfirmCancelQueueAction => {
                // Show the queue draining, then replay the action once empty.
                let app = weak.upgrade().unwrap();
                let ui = app.global::<UiState<'_>>();
                ui.set_current_page(Page::Jobs);
                ui.set_draining_queue(true);

                self.cancel_all_jobs(weak);

                // The running job (if any) stays at index 0 until it bails:
                // the action is then replayed from TriggerJob.
                if self.job_queue.is_empty() {
                    self.run_pending_queue_action(message_queue, weak);
                }
            }
            Message::Disconnect => {
                // We forget the target but keep the FTP credentials
                // for the next connection.
                self.config.contents.target_kind = TargetKind::Local;
                self.config.contents.mount_point = PathBuf::new();
                self.config.contents.fatx = txbm_core::fatx::FatxConfig::default();

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
                message_queue.push_back((Message::RefreshAll, SharedString::new()));
            }
            Message::EditStorageConfig => {
                // Re-open the storage modal on an already-configured target: the
                // analysis runs again (pre-filling the current paths) but the
                // form is shown instead of being skipped.
                self.editing_storage = true;
                message_queue.push_back((Message::StartTargetAnalysis, SharedString::new()));
            }
            Message::StartTargetAnalysis => {
                let Some(target) = Target::from_config(&self.config.contents) else {
                    return;
                };

                let app = weak.upgrade().unwrap();
                let ui = app.global::<UiState<'_>>();
                ui.set_configuring_storage(true);
                ui.set_analyzing_target(true);
                // A console-shaped target stores absolute `/Hdd1/...` paths, which
                // no local folder picker can browse.
                ui.set_storage_is_console(!matches!(target, Target::Local(_)));

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let res = target.analyze();
                    *ANALYSIS_RESULT.lock().unwrap() = Some(res);

                    let _ = weak.upgrade_in_event_loop(|app| {
                        app.global::<Dispatcher<'_>>().invoke_dispatch(
                            Message::TargetAnalysisFinished,
                            SharedString::new(),
                        );
                    });
                });
            }
            Message::TargetAnalysisFinished => {
                let app = weak.upgrade().unwrap();
                let ui = app.global::<UiState<'_>>();
                ui.set_analyzing_target(false);

                match ANALYSIS_RESULT.lock().unwrap().take() {
                    Some(Ok(analysis)) => {
                        if analysis.already_configured && !self.editing_storage {
                            // Target already carries a manifest and we're just
                            // connecting: skip the modal and scan straight away.
                            ui.set_configuring_storage(false);
                            message_queue.push_back((Message::RefreshAll, SharedString::new()));
                            // Otherwise the Toolbox "Library storage" card (folders
                            // + GOD layout) keeps showing whatever a previous
                            // connection in this session left behind.
                            message_queue
                                .push_back((Message::FetchAuroraPaths, SharedString::new()));
                        } else {
                            let mut candidates: Vec<SharedString> = analysis
                                .candidates
                                .iter()
                                .map(|s| SharedString::from(s.as_str()))
                                .collect();
                            // Sentinel appended for the "custom path" case; the
                            // modal reveals a free-text / browse field for it.
                            // Must match the OTHER literal in the Slint modal.
                            candidates.push(SharedString::from("Other…"));
                            ui.set_storage_candidates(ModelRc::from(Rc::new(VecModel::from(
                                candidates,
                            ))));
                            ui.set_storage_god_dir(analysis.suggested.god_dir.as_str().into());
                            ui.set_storage_xbe_dir(
                                analysis.suggested.xbe_dir.as_str().into(),
                            );
                            ui.set_storage_xex_dir(
                                analysis.suggested.xex_dir.as_str().into(),
                            );
                            ui.set_storage_god_layout(analysis.suggested.god_layout.into());
                            // The modal stays open, now showing the form.
                        }
                    }
                    Some(Err(e)) => {
                        ui.set_configuring_storage(false);
                        let text = slint::format!("Failed to analyze the target: {e:#}");
                        self.notifications.push(Notification::error(text));
                        message_queue.push_back((Message::Disconnect, SharedString::new()));
                    }
                    None => {}
                }
            }
            Message::ConfirmStorageConfig => {
                let Some(target) = Target::from_config(&self.config.contents) else {
                    return;
                };
                self.editing_storage = false;

                let app = weak.upgrade().unwrap();
                let ui = app.global::<UiState<'_>>();
                // UI values are in manifest form (relative for a USB drive);
                // resolve them to absolute paths before applying.
                let storage = StorageConfig {
                    god_dir: target.resolve_path(ui.get_storage_god_dir().as_str()),
                    xbe_dir: target.resolve_path(ui.get_storage_xbe_dir().as_str()),
                    xex_dir: target.resolve_path(ui.get_storage_xex_dir().as_str()),
                    god_layout: ui.get_storage_god_layout().into(),
                };
                // Reuse the spinner while the folders are created and the
                // manifest is written.
                ui.set_analyzing_target(true);

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let res = target.apply_storage(&storage);

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let ui = app.global::<UiState<'_>>();
                        ui.set_analyzing_target(false);
                        ui.set_configuring_storage(false);

                        let dispatcher = app.global::<Dispatcher<'_>>();
                        match res {
                            Ok(()) => {
                                // Rescan the library and refresh the Toolbox
                                // storage cards with the new paths.
                                dispatcher
                                    .invoke_dispatch(Message::RefreshAll, SharedString::new());
                                dispatcher.invoke_dispatch(
                                    Message::FetchAuroraPaths,
                                    SharedString::new(),
                                );
                            }
                            Err(e) => {
                                dispatcher.invoke_dispatch(
                                    Message::NotifyError,
                                    slint::format!("Failed to save the storage configuration: {e:#}"),
                                );
                                dispatcher
                                    .invoke_dispatch(Message::Disconnect, SharedString::new());
                            }
                        }
                    });
                });
            }
            Message::CancelStorageConfig => {
                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_configuring_storage(false);
                if self.editing_storage {
                    // Editing an already-connected target: just close the modal,
                    // keep the current configuration and connection.
                    self.editing_storage = false;
                } else {
                    // Abandoning the initial configuration abandons the
                    // connection.
                    message_queue.push_back((Message::Disconnect, SharedString::new()));
                }
            }
            Message::PickStorageGodDir => {
                let app = weak.upgrade().unwrap();
                let window_handle = app.window().window_handle();
                let start = self.config.contents.mount_point.clone();
                if let Some(path) = dialogs::pick_storage_folder(&window_handle, &start) {
                    match self.storage_relative(&path) {
                        Some(form) => app
                            .global::<UiState<'_>>()
                            .set_storage_god_dir(SharedString::from(form.as_str())),
                        None => self.notifications.push(Notification::error(self.off_drive_text())),
                    }
                }
            }
            Message::PickStorageXbeDir => {
                let app = weak.upgrade().unwrap();
                let window_handle = app.window().window_handle();
                let start = self.config.contents.mount_point.clone();
                if let Some(path) = dialogs::pick_storage_folder(&window_handle, &start) {
                    match self.storage_relative(&path) {
                        Some(form) => app
                            .global::<UiState<'_>>()
                            .set_storage_xbe_dir(SharedString::from(form.as_str())),
                        None => self.notifications.push(Notification::error(self.off_drive_text())),
                    }
                }
            }
            Message::PickStorageXexDir => {
                let app = weak.upgrade().unwrap();
                let window_handle = app.window().window_handle();
                let start = self.config.contents.mount_point.clone();
                if let Some(path) = dialogs::pick_storage_folder(&window_handle, &start) {
                    match self.storage_relative(&path) {
                        Some(form) => app
                            .global::<UiState<'_>>()
                            .set_storage_xex_dir(SharedString::from(form.as_str())),
                        None => self.notifications.push(Notification::error(self.off_drive_text())),
                    }
                }
            }
            Message::DownloadCovers => {
                if !self.is_downloading_covers {
                    self.is_downloading_covers = true;

                    let app = weak.upgrade().unwrap();
                    app.global::<UiState<'_>>().set_downloading_covers(true);

                    let games = self.games.clone();
                    let target = Target::from_config(&self.config.contents);
                    let source = self.config.contents.cover_source;

                    let weak = weak.clone();

                    let _ = std::thread::spawn(move || {
                        let res = covers::download_covers(games, target, source, &weak);

                        let _ = weak.upgrade_in_event_loop(move |app| {
                            let dispatcher = app.global::<Dispatcher<'_>>();

                            if let Err(e) = res {
                                let text = slint::format!("Could not download covers: {e}");
                                dispatcher.invoke_dispatch(Message::NotifyError, text);
                            }

                            dispatcher.invoke_dispatch(
                                Message::FinishedDownloadingCovers,
                                SharedString::new(),
                            );
                        });
                    });
                }
            }
            Message::SetGameId => {
                // Payload: alternating "<path>\n<TitleID>" lines — the whole
                // batch the covers pass resolved over FTP, in one message.
                // The merge and the model rebuild below are O(N) over the
                // library each, so they run once for the batch, not once per
                // game.
                let mut lines = payload.lines();
                let mut any = false;
                while let (Some(path), Some(id)) = (lines.next(), lines.next()) {
                    let path = Path::new(path);
                    for game in self.games.iter_mut().filter(|g| g.path == path) {
                        game.id = id.to_string();
                        game.search_term = format!("{}\0{id}", game.title).to_lowercase();
                    }
                    any = true;
                }
                if any {
                    // The IDs were unknown at scan time, so any orphaned
                    // DLC/title update entry sharing one couldn't be folded in
                    // yet.
                    txbm_core::game::merge_extracted_content(&mut self.games);
                    message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));
                }
            }
            Message::FinishedDownloadingCovers => {
                self.is_downloading_covers = false;

                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_downloading_covers(false);

                // Turns the remaining spinners into "no cover" icons.
                message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));
            }
            Message::RedownloadAllCovers => {
                let covers_dir = DATA_DIR.join("covers");
                if let Err(e) = fs::remove_dir_all(&covers_dir)
                    && covers_dir.exists()
                {
                    let text = slint::format!("Failed to clear the covers cache: {e}");
                    self.notifications.push(Notification::error(text));
                    return;
                }
                // Also drop the Original Xbox covers database; it is
                // re-downloaded by the covers pass right after.
                txbm_core::mobcat::clear_db();

                // Drop the in-memory thumbnails too, otherwise the wiped
                // covers would keep showing until the app restarts.
                crate::games::clear_thumb_cache();

                self.notifications
                    .push(Notification::info("Covers cache cleared"));

                message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));
                message_queue.push_back((Message::DownloadCovers, SharedString::new()));
            }
            Message::OpenThat => {
                if let Err(e) = open::that(&payload) {
                    let text = slint::format!("Failed to open URL: {e}");
                    self.notifications.push(Notification::error(text));
                }
            }
            Message::CheckForUpdates => {
                let weak = weak.clone();

                std::thread::spawn(move || {
                    let res = txbm_core::updates::check();

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let dispatcher = app.global::<Dispatcher<'_>>();

                        match res {
                            Ok(Some(version)) => {
                                let value = slint::format!("v{version}");
                                dispatcher.invoke_dispatch(Message::SetLatestVersion, value);
                            }
                            Ok(None) => {
                                eprintln!("No updates available");
                            }
                            Err(e) => {
                                eprintln!("Failed to check for updates: {e}");
                            }
                        }
                    });
                });
            }
            Message::FilterGames => {
                self.games_filter = payload.to_lowercase();
                message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));
            }
            Message::CloseNotification => {
                // Keyed by id, not by row: a toast that times out at the same
                // moment as another one would otherwise remove its neighbour.
                let id: i32 = payload.parse().unwrap();
                if let Some(i) = self.notifications.iter().position(|n| n.id == id) {
                    self.notifications.remove(i);
                }
            }
            Message::PickGames => {
                let app = weak.upgrade().unwrap();
                let window_handle = app.window().window_handle();
                let recursively = payload.parse().unwrap();

                let paths = if recursively {
                    dialogs::pick_games_r(&window_handle)
                } else {
                    dialogs::pick_games(&window_handle)
                };

                let picked = paths
                    .into_iter()
                    .filter_map(util::should_add_game)
                    .collect();

                self.set_games_to_add(picked, weak);
            }
            Message::ConfirmGamesToAdd => {
                while let Some(picked) = self.games_to_add.pop_front() {
                    let _ = self.displayed_games_to_add.remove(0);
                    self.enqueue_job(
                        QueuedJob::Add {
                            path: picked.path,
                            title_id: picked.installs_title_id,
                        },
                        message_queue,
                        weak,
                    );
                }

                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_games_to_add_conflicts(0);
            }
            Message::TriggerJob => {
                // Keep the running job at index 0 of both queues so it stays
                // visible (and the navbar icon stays shown) until it actually
                // finishes. It is removed on `JobFinished`.
                let Some(job) = self.job_queue.front().cloned() else {
                    self.is_job_running = false;
                    let app = weak.upgrade().unwrap();
                    let ui = app.global::<UiState<'_>>();
                    ui.set_job_running(false);
                    ui.set_cancelling_queue(false);
                    ui.set_cancelling_current(false);
                    ui.set_confirming_cancel_current(false);
                    ui.set_job_label(SharedString::new());
                    ui.set_job_progress(0.0);
                    ui.set_job_speed(SharedString::new());
                    ui.set_job_phase(SharedString::new());

                    // The whole batch went through: celebrate. Skipped when a
                    // disconnect/quit is waiting on the queue — the window is
                    // about to go away.
                    if self.adds_done > 0
                        && self.jobs_failed == 0
                        && !self.batch_cancelled
                        && ui.get_pending_queue_action() == PendingQueueAction::None
                    {
                        ui.set_celebrating(true);
                    }
                    self.adds_done = 0;
                    self.jobs_failed = 0;
                    self.batch_cancelled = false;

                    // A rescan held back while the queue was writing to the
                    // console can run now — unless the user is only waiting for
                    // the queue to drain to disconnect or quit.
                    let deferred = std::mem::take(&mut self.rescan_deferred);
                    if deferred && ui.get_pending_queue_action() == PendingQueueAction::None {
                        message_queue.push_back((Message::RefreshAll, SharedString::new()));
                    }

                    // Nothing is being written to any more, so the cards that
                    // were greyed out come back (see `running_job`).
                    message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));

                    // The queue is now drained: carry out the disconnect/quit
                    // the user was waiting on, if any.
                    self.run_pending_queue_action(message_queue, weak);
                    return;
                };

                let app = weak.upgrade().unwrap();
                let ui = app.global::<UiState<'_>>();
                ui.set_job_running(true);

                // Fresh progress card for this item: the previous one's
                // percentage/speed must not linger while this job starts.
                ui.set_job_label(job.label().to_shared_string());
                ui.set_job_kind(JobKind::from(job.kind()));
                ui.set_job_progress(0.0);
                ui.set_job_speed(SharedString::new());
                ui.set_job_phase(SharedString::new());
                ui.set_cancelling_current(false);
                // The previous item may have finished on its own while its
                // "cancel this job?" modal was still up: closing it here makes
                // sure a late confirmation can't hit the next one.
                ui.set_confirming_cancel_current(false);

                self.job_cancel
                    .store(false, std::sync::atomic::Ordering::Relaxed);

                // This job's target game (when it names one) is greyed out in
                // the library for as long as it runs — see `running_job`.
                message_queue.push_back((Message::RefreshDisplayedGames, SharedString::new()));

                let weak = weak.clone();
                let config = self.config.clone();
                let cancel = self.job_cancel.clone();

                let _ = std::thread::spawn(move || {
                    perform_job(job, &config, cancel, &weak);
                });
            }
            Message::JobFinished => {
                // Only a successful *addition* is worth celebrating: a batch of
                // deletions drains without confetti.
                let was_add = self
                    .job_queue
                    .front()
                    .is_some_and(|job| job.kind() == txbm_core::job_queue::JobKind::Add);

                if payload == "ok" {
                    if was_add {
                        self.adds_done += 1;
                    }
                } else {
                    self.jobs_failed += 1;
                }

                // Drop the job that just finished (done or failed) and move on
                // to the next one.
                let _ = self.job_queue.pop_front();
                let _ = self.displayed_job_queue.remove(0);
                message_queue.push_back((Message::TriggerJob, SharedString::new()));
            }
            Message::ClearGamesToAdd => {
                self.games_to_add.clear();
                self.displayed_games_to_add.clear();
                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_games_to_add_conflicts(0);
            }
            Message::TestFtp => {
                let ftp_config = self.config.contents.ftp_config();

                if ftp_config.host.is_empty() {
                    let text = "No console IP configured";
                    self.notifications.push(Notification::error(text));
                    return;
                }

                let text = slint::format!("Testing FTP connection to {}...", ftp_config.host);
                self.notifications.push(Notification::info(text));

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let res = FtpSession::connect(&ftp_config).and_then(|mut session| {
                        let roots = session.list_root()?;
                        session.quit();
                        Ok(roots)
                    });

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let dispatcher = app.global::<Dispatcher<'_>>();

                        match res {
                            Ok(roots) => {
                                let text = slint::format!(
                                    "FTP connection successful\nConsole root: {}",
                                    roots.join(", ")
                                );
                                dispatcher.invoke_dispatch(Message::NotifySuccess, text);
                            }
                            Err(e) => {
                                let text = slint::format!("FTP connection failed: {e:#}");
                                dispatcher.invoke_dispatch(Message::NotifyError, text);
                            }
                        }
                    });
                });
            }
            Message::StartFtpScan => {
                // Fresh scan: hand this run its own cancel flag. A previous run's
                // flag (already true if it was cancelled) is thus left untouched,
                // so an in-flight older thread stays cancelled instead of being
                // revived by a shared `store(false)`.
                self.ftp_scan_cancel = Arc::new(AtomicBool::new(false));

                let app = weak.upgrade().unwrap();
                let ui = app.global::<UiState<'_>>();
                ui.set_ftp_scanning(true);
                ui.set_ftp_scan_found_ip(SharedString::new());

                let cfg = self.config.contents.ftp_config();
                let cancel = self.ftp_scan_cancel.clone();
                let weak = weak.clone();
                std::thread::spawn(move || {
                    let res = txbm_core::ftp::scan_network(
                        cfg.port,
                        &cfg.user,
                        &cfg.password,
                        &cancel,
                    );

                    // A cancelled run stays silent: it neither publishes its
                    // (aborted) result nor wakes FtpScanFinished, so it can't
                    // clobber a newer scan that replaced its cancel flag.
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    *FTP_SCAN_RESULT.lock().unwrap() = Some(res);

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        app.global::<Dispatcher<'_>>()
                            .invoke_dispatch(Message::FtpScanFinished, SharedString::new());
                    });
                });
            }
            Message::CancelFtpScan => {
                self.ftp_scan_cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_ftp_scanning(false);
            }
            Message::FtpScanFinished => {
                let app = weak.upgrade().unwrap();
                let ui = app.global::<UiState<'_>>();
                ui.set_ftp_scanning(false);

                // Cancelled (user switched to manual entry or closed the modal):
                // ignore whatever the thread found.
                if self
                    .ftp_scan_cancel
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    return;
                }

                match FTP_SCAN_RESULT.lock().unwrap().take() {
                    Some(Some(ip)) => {
                        // Prefill the config (and thus the manual form) with the
                        // discovered IP, and surface it in the scan view.
                        self.config.contents.console_ip = ip.clone();
                        ui.set_ftp_scan_found_ip(ip.into());
                        message_queue.push_back((Message::SyncConfig, SharedString::new()));
                    }
                    _ => {
                        ui.set_ftp_scan_found_ip(SharedString::new());
                    }
                }
            }
            Message::RestartAurora => {
                let ftp_config = self.config.contents.ftp_config();

                if ftp_config.host.is_empty() {
                    let text = "No console IP configured";
                    self.notifications.push(Notification::error(text));
                    return;
                }

                let text = "Restarting Aurora...";
                self.notifications.push(Notification::info(text));

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let res = FtpSession::connect(&ftp_config).and_then(|mut session| {
                        session.restart_aurora()?;
                        session.quit();
                        Ok(())
                    });

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let dispatcher = app.global::<Dispatcher<'_>>();

                        match res {
                            Ok(()) => {
                                let text = "Aurora is ready 🎉";
                                dispatcher.invoke_dispatch(
                                    Message::NotifySuccess,
                                    SharedString::from(text),
                                );
                            }
                            Err(e) => {
                                let text = slint::format!("Failed to restart Aurora: {e:#}");
                                dispatcher.invoke_dispatch(Message::NotifyError, text);
                            }
                        }
                    });
                });
            }
            Message::FetchAuroraPaths => {
                let app = weak.upgrade().unwrap();
                match Target::from_config(&self.config.contents) {
                    None => return,
                    // Local drive: read the layout (and any Aurora install on
                    // the drive itself) synchronously. This is fast local disk
                    // I/O (a manifest read plus two small SQLite opens and a
                    // few `is_dir` checks), so it is run inline rather than on a
                    // worker thread like the FTP branch below.
                    Some(Target::Local(mount)) => {
                        app.global::<UiState<'_>>().set_fetching_aurora_paths(true);
                        let status = txbm_core::target::local_storage_status(&mount);
                        set_storage_status(&app, status);
                    }
                    // Console-shaped target: reading it means either network
                    // round trips or raw disk I/O, so it runs on a thread.
                    Some(target) => {
                        let ui_state = app.global::<UiState<'_>>();
                        ui_state.set_fetching_aurora_paths(true);
                        ui_state.set_aurora_paths_error(SharedString::new());

                        let weak = weak.clone();
                        std::thread::spawn(move || {
                            // One session feeds both Toolbox cards.
                            let res = target.storage_status();

                            let _ = weak.upgrade_in_event_loop(move |app| match res {
                                Ok(status) => set_storage_status(&app, status),
                                Err(e) => {
                                    let ui_state = app.global::<UiState<'_>>();
                                    ui_state.set_fetching_aurora_paths(false);
                                    ui_state.set_aurora_paths_loaded(true);
                                    ui_state.set_aurora_scan_paths(ModelRc::from(Rc::new(
                                        VecModel::<SharedString>::default(),
                                    )));
                                    ui_state.set_aurora_install_dir(SharedString::new());
                                    ui_state.set_aurora_paths_error(slint::format!("{e:#}"));
                                    ui_state.set_app_storage_paths(ModelRc::from(Rc::new(
                                        VecModel::<DisplayedStoragePath>::default(),
                                    )));
                                    ui_state.set_storage_has_uncovered(false);
                                    ui_state.set_storage_aurora_compared(false);
                                }
                            });
                        });
                    }
                }
            }
            Message::DeleteGame => {
                // Deletions go through the queue like additions: they write to
                // the target, so only one may ever be in flight.
                let path = Path::new(&payload);
                let Some(game) = self.games.iter().find(|g| g.path == path).cloned() else {
                    return;
                };

                self.enqueue_job(QueuedJob::Delete(Box::new(game)), message_queue, weak);
            }
            Message::DeleteContent => {
                // Payload: "<game path>\n<kind>\n<file name>\n<description>".
                let mut parts = payload.splitn(4, '\n');
                let (Some(path), Some(kind_str), Some(file_name), Some(description)) =
                    (parts.next(), parts.next(), parts.next(), parts.next())
                else {
                    return;
                };
                let path = Path::new(path);
                let Some(game) = self.games.iter().find(|g| g.path == path).cloned() else {
                    return;
                };
                let kind = match kind_str {
                    "Disc" => ContentKind::Disc,
                    "DLC" => ContentKind::Dlc,
                    _ => return,
                };

                self.enqueue_job(
                    QueuedJob::DeleteContent {
                        game: Box::new(game),
                        kind,
                        file_name: file_name.to_string(),
                        description: description.to_string(),
                    },
                    message_queue,
                    weak,
                );
            }
            Message::CancelJob => {
                let i = payload.parse().unwrap();

                // Index 0 is the job currently running: it can't be dropped
                // from the queue on the spot, only signalled to stop. It bails
                // at its next checkpoint and `JobFinished` removes it, so the
                // queue carries on with the next item — unlike "cancel all",
                // the pending ones are left alone.
                if self.is_job_running && i == 0 {
                    self.job_cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);

                    // A batch the user cancelled part of isn't worth confetti,
                    // even if this item happens to finish cleanly before it
                    // notices the flag.
                    self.batch_cancelled = true;

                    let app = weak.upgrade().unwrap();
                    app.global::<UiState<'_>>().set_cancelling_current(true);
                    return;
                }

                let _ = self.job_queue.remove(i);
                let _ = self.displayed_job_queue.remove(i);
            }
            Message::MoveJobUp | Message::MoveJobDown => {
                let i: usize = payload.parse().unwrap();
                let up = message == Message::MoveJobUp;
                let j = if up { i.wrapping_sub(1) } else { i + 1 };

                // The running job (index 0) is pinned: nothing may be moved
                // into its slot, and it can't be moved itself.
                let first_pending = if self.is_job_running { 1 } else { 0 };
                if i < first_pending
                    || j < first_pending
                    || i >= self.job_queue.len()
                    || j >= self.job_queue.len()
                {
                    return;
                }

                self.job_queue.swap(i, j);

                // VecModel has no swap: take the lower one out and put it back
                // at the higher index, which shifts the other one up by one.
                let (lo, hi) = if up { (j, i) } else { (i, j) };
                let row = self.displayed_job_queue.remove(lo);
                self.displayed_job_queue.insert(hi, row);
            }
            Message::CancelAllJobs => {
                self.cancel_all_jobs(weak);
            }
            Message::SetLatestVersion => {
                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_latest_version(payload);
            }
            Message::CheckMountPoint => {
                if self.config.check_mount_point() {
                    self.notifications.push(Notification::info(NEW_DRIVE_TEXT));
                }
            }
            Message::SetStatus => {
                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_status(payload);
            }
            Message::FileDropped => {
                let app = weak.upgrade().unwrap();

                if app.global::<UiState<'_>>().get_current_page() == Page::Games {
                    let path = PathBuf::from(&payload);

                    if let Some(picked) = util::should_add_game(path) {
                        self.set_games_to_add(vec![picked], weak);
                    }
                }
            }
            Message::FetchGameDetails => {
                let app = weak.upgrade().unwrap();
                let ui_state = app.global::<UiState<'_>>();
                if ui_state.get_fetching_game_details() {
                    return;
                }

                let path = Path::new(payload.as_str());
                let Some(game) = self.games.iter().find(|g| g.path == path).cloned() else {
                    return;
                };
                let Some(target) = Target::from_config(&self.config.contents) else {
                    return;
                };

                ui_state.set_fetching_game_details(true);
                ui_state.set_current_game_components(ModelRc::default());

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let details = target.game_details(&game).unwrap_or_default();
                    *GAME_DETAILS_RESULT.lock().unwrap() = Some((game.path.clone(), details));

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        app.global::<Dispatcher<'_>>()
                            .invoke_dispatch(Message::GameDetailsFetched, SharedString::new());
                    });
                });
            }
            Message::GameDetailsFetched => {
                let app = weak.upgrade().unwrap();
                let ui_state = app.global::<UiState<'_>>();
                ui_state.set_fetching_game_details(false);

                let Some((path, details)) = GAME_DETAILS_RESULT.lock().unwrap().take() else {
                    return;
                };
                // Discard if the user closed the modal or switched games
                // while the fetch was running.
                if Path::new(ui_state.get_current_game().path.as_str()) != path {
                    return;
                }

                let game = ui_state.get_current_game();
                ui_state.set_current_game_components(ModelRc::from(Rc::new(VecModel::from(
                    game_details::components(&details, &game),
                ))));
            }
            Message::FetchTitleUpdates => {
                let app = weak.upgrade().unwrap();
                let ui_state = app.global::<UiState<'_>>();
                ui_state.set_showing_title_updates(true);
                if ui_state.get_fetching_title_updates() {
                    return;
                }

                let path = Path::new(payload.as_str());
                let Some(game) = self.games.iter().find(|g| g.path == path).cloned() else {
                    return;
                };
                let Some(target) = Target::from_config(&self.config.contents) else {
                    return;
                };

                ui_state.set_fetching_title_updates(true);
                self.displayed_title_updates.set_vec(Vec::new());

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let result = title_updates::fetch(&target, &game);
                    *TITLE_UPDATES_RESULT.lock().unwrap() = Some((game.path.clone(), result));

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        app.global::<Dispatcher<'_>>()
                            .invoke_dispatch(Message::TitleUpdatesFetched, SharedString::new());
                    });
                });
            }
            Message::TitleUpdatesFetched => {
                let app = weak.upgrade().unwrap();
                let ui_state = app.global::<UiState<'_>>();
                ui_state.set_fetching_title_updates(false);

                let Some((path, result)) = TITLE_UPDATES_RESULT.lock().unwrap().take() else {
                    return;
                };
                // Discard if the user closed the modal or switched games
                // while the fetch was running.
                if Path::new(ui_state.get_current_game().path.as_str()) != path {
                    return;
                }

                match result {
                    Ok(updates) => self.displayed_title_updates.set_vec(updates),
                    Err(e) => {
                        let text = slint::format!("Failed to fetch title updates: {e:#}");
                        self.notifications.push(Notification::error(text));
                    }
                }
            }
            Message::ActivateTitleUpdate => {
                // This one *writes* to the console, so it must not run beside
                // the queue's own write either (see `console_write_in_flight`).
                if self.console_write_in_flight() {
                    self.notifications.push(Notification::info(
                        "Wait for the transfer queue to finish before changing a title update",
                    ));
                    return;
                }
                let Some((path, hash)) = payload.split_once('\n') else {
                    return;
                };
                let path = Path::new(path);
                let Some(game) = self.games.iter().find(|g| g.path == path).cloned() else {
                    return;
                };
                let Some(target) = Target::from_config(&self.config.contents) else {
                    return;
                };
                let hash = hash.to_string();

                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_updating_title_update(true);

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let res = target.activate_title_update(&game, &hash);

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let dispatcher = app.global::<Dispatcher<'_>>();
                        match res {
                            Ok(()) => {
                                dispatcher.invoke_dispatch(
                                    Message::NotifyInfo,
                                    "Title update installed".to_shared_string(),
                                );
                            }
                            Err(e) => {
                                let text =
                                    slint::format!("Failed to install title update: {e:#}");
                                dispatcher.invoke_dispatch(Message::NotifyError, text);
                            }
                        }
                        dispatcher
                            .invoke_dispatch(Message::TitleUpdateChanged, SharedString::new());
                    });
                });
            }
            Message::DeactivateTitleUpdate => {
                // A write too: same rule as `ActivateTitleUpdate` above.
                if self.console_write_in_flight() {
                    self.notifications.push(Notification::info(
                        "Wait for the transfer queue to finish before changing a title update",
                    ));
                    return;
                }
                let Some((path, file_name)) = payload.split_once('\n') else {
                    return;
                };
                let path = Path::new(path);
                let Some(game) = self.games.iter().find(|g| g.path == path).cloned() else {
                    return;
                };
                let Some(target) = Target::from_config(&self.config.contents) else {
                    return;
                };
                let file_name = file_name.to_string();

                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_updating_title_update(true);

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let res = target.deactivate_title_update(&game, &file_name);

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let dispatcher = app.global::<Dispatcher<'_>>();
                        match res {
                            Ok(()) => {
                                dispatcher.invoke_dispatch(
                                    Message::NotifyInfo,
                                    "Title update removed".to_shared_string(),
                                );
                            }
                            Err(e) => {
                                let text = slint::format!("Failed to remove title update: {e:#}");
                                dispatcher.invoke_dispatch(Message::NotifyError, text);
                            }
                        }
                        dispatcher
                            .invoke_dispatch(Message::TitleUpdateChanged, SharedString::new());
                    });
                });
            }
            Message::TitleUpdateChanged => {
                let app = weak.upgrade().unwrap();
                let ui_state = app.global::<UiState<'_>>();
                ui_state.set_updating_title_update(false);

                let path = ui_state.get_current_game().path;
                if !path.is_empty() {
                    message_queue.push_back((Message::FetchTitleUpdates, path));
                }
            }
            Message::SetBadAvatarUrl => {
                // Payload: "<key>\n<url>", key being the BadAvatar component.
                if let Some((key, value)) = payload.split_once('\n')
                    && let Some(field) = UrlField::from_key(key)
                {
                    self.config.contents.badavatar.set_url(field, value.to_string());
                    message_queue.push_back((Message::SyncConfig, SharedString::new()));
                }
            }
            Message::SetBadAvatarVersion => {
                // Payload: the row picked in the ABadAvatar version drop-down.
                if let Ok(index) = payload.parse::<usize>()
                    && let Some((_, url)) = txbm_core::badavatar::ABADAVATAR_VERSIONS.get(index)
                {
                    // The built-in default is stored as "no override", so the
                    // per-field reset button stays hidden for it.
                    if *url == UrlField::Abadavatar.default_url() {
                        self.config.contents.badavatar.reset_url(UrlField::Abadavatar);
                    } else {
                        self.config
                            .contents
                            .badavatar
                            .set_url(UrlField::Abadavatar, url.to_string());
                    }
                    message_queue.push_back((Message::SyncConfig, SharedString::new()));
                }
            }
            Message::ResetBadAvatarUrl => {
                if let Some(field) = UrlField::from_key(payload.as_str()) {
                    self.config.contents.badavatar.reset_url(field);
                    message_queue.push_back((Message::SyncConfig, SharedString::new()));
                }
            }
            Message::ToggleBadAvatarSystemUpdate => {
                let flag = &mut self.config.contents.badavatar.include_system_update;
                *flag = !*flag;
                message_queue.push_back((Message::SyncConfig, SharedString::new()));
            }
            Message::CreateBadAvatar => {
                if self.is_creating_badavatar {
                    return;
                }

                // Open the in-app removable-drive picker (same one used for
                // target selection), pre-populated with the detected drives.
                let app = weak.upgrade().unwrap();
                let ui_state = app.global::<UiState<'_>>();
                ui_state.set_removable_drives(ModelRc::from(Rc::new(VecModel::from(
                    crate::config::removable_drives(),
                ))));
                ui_state.set_selecting_badavatar_target(true);
            }
            Message::SelectBadAvatarDrive => {
                // Payload: the mount point picked in the drive-picker modal.
                let dest = PathBuf::from(payload.as_str());
                if !dest.is_dir() {
                    self.notifications
                        .push(Notification::error("This drive is no longer available"));
                    return;
                }
                let path_text = dest.to_string_lossy().to_shared_string();
                self.badavatar_pending_dest = Some(dest);
                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>()
                    .set_badavatar_pending_path(path_text);
            }
            Message::PickBadAvatarMountPoint => {
                // Hidden escape hatch (long press): pick any folder with the
                // native OS picker instead of a detected removable drive.
                if self.is_creating_badavatar {
                    return;
                }

                let app = weak.upgrade().unwrap();
                let window_handle = app.window().window_handle();
                let Some(dest) = dialogs::pick_mount_point(&window_handle) else {
                    return;
                };

                let path_text = dest.to_string_lossy().to_shared_string();
                self.badavatar_pending_dest = Some(dest);
                app.global::<UiState<'_>>()
                    .set_badavatar_pending_path(path_text);
            }
            Message::CancelCreateBadAvatar => {
                self.badavatar_pending_dest = None;
                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>()
                    .set_badavatar_pending_path(SharedString::new());
            }
            Message::ConfirmCreateBadAvatar => {
                if self.is_creating_badavatar {
                    return;
                }
                let Some(dest) = self.badavatar_pending_dest.take() else {
                    return;
                };

                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>()
                    .set_badavatar_pending_path(SharedString::new());

                let cfg = self.config.contents.badavatar.clone();
                self.is_creating_badavatar = true;
                self.badavatar_cancel
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                let cancel = self.badavatar_cancel.clone();
                app.global::<UiState<'_>>().set_creating_badavatar(true);

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let weak_status = weak.clone();
                    let status = move |line: &str| {
                        let text = SharedString::from(line);
                        let _ = weak_status.upgrade_in_event_loop(move |app| {
                            app.global::<UiState<'_>>().set_status(text);
                        });
                    };

                    status("Creating the BadAvatar USB key…");

                    let res =
                        txbm_core::badavatar::create_badavatar(&dest, &cfg, &cancel, &status);

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let dispatcher = app.global::<Dispatcher<'_>>();

                        dispatcher.invoke_dispatch(Message::SetStatus, SharedString::new());
                        dispatcher.invoke_dispatch(Message::BadAvatarCreated, SharedString::new());

                        match res {
                            Ok(()) => {
                                // Sticky toast plus a confetti burst: the build
                                // takes minutes, so the user is likely looking
                                // elsewhere when it lands.
                                dispatcher.invoke_dispatch(
                                    Message::NotifySuccessSticky,
                                    "BadAvatar USB key ready 🎉".to_shared_string(),
                                );
                                app.global::<UiState<'_>>().set_celebrating(true);
                            }
                            Err(e)
                                if e.to_string()
                                    .contains(txbm_core::badavatar::BADAVATAR_CANCELLED) =>
                            {
                                dispatcher.invoke_dispatch(
                                    Message::NotifyInfo,
                                    "BadAvatar creation cancelled".to_shared_string(),
                                );
                            }
                            Err(e) => {
                                let text =
                                    slint::format!("Failed to create BadAvatar key: {e:#}");
                                dispatcher.invoke_dispatch(Message::NotifyError, text);
                            }
                        }
                    });
                });
            }
            Message::CancelBadAvatar => {
                if self.is_creating_badavatar {
                    self.badavatar_cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    message_queue
                        .push_back((Message::SetStatus, "⟳  Cancelling…".to_shared_string()));
                }
            }
            Message::BadAvatarCreated => {
                self.is_creating_badavatar = false;
                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_creating_badavatar(false);
            }
            #[cfg(windows)]
            Message::SetWindowColorLight => {
                crate::window_color::set(false);
            }
            #[cfg(windows)]
            Message::SetWindowColorDark => {
                crate::window_color::set(true);
            }
            #[cfg(not(windows))]
            Message::SetWindowColorLight | Message::SetWindowColorDark => {}
        }
    }
}

/// Populates the two Toolbox storage cards (Aurora scan paths + Library
/// storage) from a resolved [`StorageStatus`]. Runs on the UI thread.
fn set_storage_status(app: &AppWindow, status: txbm_core::target::StorageStatus) {
    let ui = app.global::<UiState<'_>>();
    ui.set_fetching_aurora_paths(false);
    ui.set_aurora_paths_loaded(true);
    ui.set_aurora_paths_error(status.aurora_error.clone().unwrap_or_default().into());
    ui.set_aurora_scan_paths(ModelRc::from(Rc::new(VecModel::from(
        status
            .aurora_lines
            .iter()
            .map(|l| SharedString::from(l.as_str()))
            .collect::<Vec<_>>(),
    ))));
    ui.set_aurora_install_dir(status.aurora_install_dir.clone().unwrap_or_default().into());

    let paths: Vec<DisplayedStoragePath> = status
        .paths
        .iter()
        .map(|p| DisplayedStoragePath {
            label: p.label.as_str().into(),
            path: p.path.as_str().into(),
            aurora_path: p.aurora_path.as_str().into(),
            covered: p.covered_by_aurora,
        })
        .collect();
    ui.set_app_storage_paths(ModelRc::from(Rc::new(VecModel::from(paths))));
    ui.set_storage_has_uncovered(status.has_uncovered);
    ui.set_storage_aurora_compared(status.aurora_compared);
    ui.set_storage_god_layout(status.god_layout.into());
}

impl State {
    /// Manifest-form (mount-relative for a USB drive) of a picked folder, or
    /// `None` when it lies outside the selected drive.
    fn storage_relative(&self, path: &Path) -> Option<String> {
        Target::from_config(&self.config.contents).and_then(|t| t.relative_within(path))
    }

    /// Error shown when a picked storage folder is not on the selected drive.
    fn off_drive_text(&self) -> SharedString {
        slint::format!(
            "That folder is not on the selected drive ({}).\nPick a folder inside it.",
            self.config.contents.mount_point.display()
        )
    }
}
