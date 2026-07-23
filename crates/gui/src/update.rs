// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AppWindow, Dispatcher, DisplayedConfig, DisplayedDriveInfo, DisplayedGame,
    DisplayedStoragePath, DisplayedTitleUpdate, Message, Notification, Page, UiState,
    convert::perform_conversion, covers, dialogs, game_details, state::State, title_updates, util,
};
use slint::{ComponentHandle, ModelRc, SharedString, ToSharedString, VecModel, Weak};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use txbm_core::{
    badavatar::UrlField, config::TargetKind, conversion_queue::QueuedConversion,
    data_dir::DATA_DIR, drive_info::DriveInfo, ftp::FtpSession, game::Game,
    target::{StorageConfig, Target, TargetAnalysis},
};

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
                    return;
                }
                self.select_local_mount(path, message_queue);
            }
            Message::RefreshDisplayedGames => {
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
                    .map(DisplayedGame::from)
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

                if self.is_scanning {
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

                match SCAN_RESULT.lock().unwrap().take() {
                    Some(Ok((games, drive_info))) => {
                        self.games = games;
                        self.drive_info = drive_info;

                        app.global::<UiState<'_>>()
                            .set_drive_info(DisplayedDriveInfo::from(&self.drive_info));

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
            Message::Disconnect => {
                // We forget the target but keep the FTP credentials
                // for the next connection.
                self.config.contents.target_kind = TargetKind::Local;
                self.config.contents.mount_point = PathBuf::new();

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
                ui.set_storage_is_ftp(matches!(target, Target::Ftp(_)));

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

                    let weak = weak.clone();

                    let _ = std::thread::spawn(move || {
                        let res = covers::download_covers(games, target, &weak);

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
                // Payload: "<path>\n<TitleID>", sent by the covers pass
                // once a TitleID has been resolved over FTP.
                if let Some((path, id)) = payload.split_once('\n') {
                    let path = Path::new(path);
                    for game in self.games.iter_mut().filter(|g| g.path == path) {
                        game.id = id.to_string();
                        game.search_term = format!("{}\0{id}", game.title).to_lowercase();
                    }
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
                let i = payload.parse().unwrap();
                self.notifications.remove(i);
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

                self.games_to_add = paths
                    .into_iter()
                    .filter_map(util::should_add_game)
                    .collect();

                let displayed_games_to_add = self
                    .games_to_add
                    .iter()
                    .map(|p| match p.file_name() {
                        Some(filename) => filename.to_string_lossy().to_shared_string(),
                        None => "?".to_shared_string(),
                    })
                    .collect::<Vec<_>>();

                self.displayed_games_to_add.set_vec(displayed_games_to_add);
            }
            Message::ConfirmGamesToAdd => {
                while let Some(path) = self.games_to_add.pop_front() {
                    let _ = self.displayed_games_to_add.remove(0);

                    let conv = QueuedConversion::Standard(path);
                    let displayed_conv = conv.to_shared_string();
                    self.conversion_queue.push_back(conv);
                    self.displayed_conversion_queue.push(displayed_conv);
                }

                if !self.is_converting {
                    self.is_converting = true;
                    message_queue.push_back((Message::TriggerConversion, SharedString::new()));
                }
            }
            Message::TriggerConversion => {
                // Keep the item being converted at index 0 of both queues so
                // it stays visible (and the navbar icon stays shown) until the
                // conversion actually finishes. It is removed on
                // `ConversionFinished`.
                let Some(conv) = self.conversion_queue.front().cloned() else {
                    self.is_converting = false;
                    let app = weak.upgrade().unwrap();
                    app.global::<UiState<'_>>().set_converting(false);
                    let text = "Conversion queue empty";
                    self.notifications.push(Notification::info(text));
                    return;
                };

                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_converting(true);

                self.conversion_cancel
                    .store(false, std::sync::atomic::Ordering::Relaxed);

                let weak = weak.clone();
                let config = self.config.clone();
                let cancel = self.conversion_cancel.clone();

                let _ = std::thread::spawn(move || {
                    perform_conversion(conv, &config, cancel, &weak);
                });
            }
            Message::ConversionFinished => {
                // Drop the conversion that just finished (in progress or failed)
                // and move on to the next one.
                let _ = self.conversion_queue.pop_front();
                let _ = self.displayed_conversion_queue.remove(0);
                message_queue.push_back((Message::TriggerConversion, SharedString::new()));
            }
            Message::ClearGamesToAdd => {
                self.games_to_add.clear();
                self.displayed_games_to_add.clear();
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
                                dispatcher.invoke_dispatch(Message::NotifyInfo, text);
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
                                    Message::NotifyInfo,
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
                    // Console over FTP: read over the network on a thread.
                    Some(Target::Ftp(ftp)) => {
                        let ui_state = app.global::<UiState<'_>>();
                        ui_state.set_fetching_aurora_paths(true);
                        ui_state.set_aurora_paths_error(SharedString::new());

                        let weak = weak.clone();
                        std::thread::spawn(move || {
                            // One connection feeds both Toolbox cards.
                            let res = FtpSession::connect(&ftp).map(|mut session| {
                                let hdd = txbm_core::target::ftp_hdd_root(&mut session);
                                let status =
                                    txbm_core::target::ftp_storage_status(&mut session, &hdd);
                                session.quit();
                                status
                            });

                            let _ = weak.upgrade_in_event_loop(move |app| match res {
                                Ok(status) => set_storage_status(&app, status),
                                Err(e) => {
                                    let ui_state = app.global::<UiState<'_>>();
                                    ui_state.set_fetching_aurora_paths(false);
                                    ui_state.set_aurora_paths_loaded(true);
                                    ui_state.set_aurora_scan_paths(ModelRc::from(Rc::new(
                                        VecModel::<SharedString>::default(),
                                    )));
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
                let path = Path::new(&payload);
                let Some(game) = self.games.iter().find(|g| g.path == path).cloned() else {
                    return;
                };
                let Some(target) = Target::from_config(&self.config.contents) else {
                    return;
                };

                // Deleting over FTP can take a while: show progress in the
                // status bar until the background thread finishes.
                message_queue.push_back((
                    Message::SetStatus,
                    slint::format!("✕  Deleting  {}…", game.title),
                ));

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let weak2 = weak.clone();
                    let game_title = game.title.clone();
                    let update_progress = move |percentage| {
                        let status = slint::format!("✕  Deleting  {game_title}  {percentage}%");
                        let _ = weak2.upgrade_in_event_loop(move |app| {
                            app.global::<UiState<'_>>().set_status(status);
                        });
                    };

                    let res = target.delete_game(&game, &update_progress);

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let dispatcher = app.global::<Dispatcher<'_>>();

                        dispatcher.invoke_dispatch(Message::SetStatus, SharedString::new());

                        match res {
                            Ok(()) => {
                                let text = slint::format!("{} deleted", game.title);
                                dispatcher.invoke_dispatch(Message::NotifyInfo, text);
                            }
                            Err(e) => {
                                let text = slint::format!("Failed to delete game: {e:#}");
                                dispatcher.invoke_dispatch(Message::NotifyError, text);
                            }
                        }

                        dispatcher.invoke_dispatch(Message::RefreshAll, SharedString::new());
                    });
                });
            }
            Message::CancelConversion => {
                let i = payload.parse().unwrap();
                // Index 0 is the conversion currently running, which can't be
                // interrupted mid-flight; ignore a cancel request on it.
                if self.is_converting && i == 0 {
                    return;
                }
                let _ = self.conversion_queue.remove(i);
                let _ = self.displayed_conversion_queue.remove(i);
            }
            Message::CancelAllConversions => {
                // Signal the running conversion (index 0) to stop; it bails at
                // its next cancellation checkpoint, cleans up its partial local
                // output, and is removed from the queue by `ConversionFinished`.
                if self.is_converting {
                    self.conversion_cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                // Drop the pending items right away, keeping the running one
                // (index 0) until it finishes bailing.
                let pending_start = if self.is_converting { 1 } else { 0 };
                while self.conversion_queue.len() > pending_start {
                    let _ = self.conversion_queue.remove(pending_start);
                    let _ = self.displayed_conversion_queue.remove(pending_start);
                }
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

                    if let Some(path) = util::should_add_game(path)
                        && let Some(filename) = path.file_name()
                    {
                        let displayed_games_to_add =
                            vec![filename.to_string_lossy().to_shared_string()];

                        self.games_to_add = VecDeque::from([path]);
                        self.displayed_games_to_add.set_vec(displayed_games_to_add);
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
                ui_state.set_current_game_discs(ModelRc::default());
                ui_state.set_current_game_dlc(ModelRc::default());

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

                ui_state.set_current_game_discs(ModelRc::from(Rc::new(VecModel::from(
                    game_details::disc_lines(&details),
                ))));
                ui_state.set_current_game_dlc(ModelRc::from(Rc::new(VecModel::from(
                    game_details::dlc_lines(&details),
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
                                dispatcher.invoke_dispatch(
                                    Message::NotifyInfo,
                                    "BadAvatar USB key ready 🎉".to_shared_string(),
                                );
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
