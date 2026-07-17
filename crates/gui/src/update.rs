// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AppWindow, Dispatcher, DisplayedConfig, DisplayedDriveInfo, DisplayedGame, Message,
    Notification, Page, UiState, convert::perform_conversion, covers, dialogs, state::State, util,
};
use slint::{ComponentHandle, SharedString, ToSharedString, Weak};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use txbm_core::{
    config::TargetKind, conversion_queue::QueuedConversion, data_dir::DATA_DIR,
    drive_info::DriveInfo, ftp::FtpSession, game::Game, target::Target,
};

const NEW_DRIVE_TEXT: &str = "New drive detected\nOnce the games are on the console, remember to add the content paths in Aurora\n(Settings > Content Paths: Hdd1:\\Content\\0000000000000000 and Hdd1:\\Games, Scan Depth 3+)";

/// Résultat du scan asynchrone de la cible, déposé par le thread de scan
/// puis récupéré dans le handler de ScanFinished.
static SCAN_RESULT: Mutex<Option<anyhow::Result<(Vec<Game>, DriveInfo)>>> = Mutex::new(None);

impl State {
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

                app.global::<UiState<'_>>()
                    .set_config(DisplayedConfig::from(&self.config));

                if let Err(e) = self.config.write() {
                    let text = slint::format!("Failed to write config: {e}");
                    self.notifications.push(Notification::error(text));
                }
            }
            Message::PickMountPoint => {
                let app = weak.upgrade().unwrap();
                let window_handle = app.window().window_handle();

                if let Some(path) = dialogs::pick_mount_point(&window_handle) {
                    self.config.contents.target_kind = TargetKind::Local;
                    self.config.contents.mount_point = path;

                    if self.config.check_mount_point() {
                        self.notifications.push(Notification::info(NEW_DRIVE_TEXT));
                    }
                }
                message_queue.push_back((Message::SyncConfig, SharedString::new()));
                message_queue.push_back((Message::RefreshAll, SharedString::new()));
            }
            Message::RefreshDisplayedGames => {
                let displayed_games = self
                    .games
                    .iter()
                    .filter(|game| {
                        ((self.config.contents.show_x360 && game.is_x360)
                            || (self.config.contents.show_og && !game.is_x360))
                            && (self.games_filter.is_empty()
                                || game.search_term.contains(&self.games_filter))
                    })
                    .map(DisplayedGame::from)
                    .collect::<Vec<_>>();

                self.displayed_games.set_vec(displayed_games);
            }
            Message::ToggleShowX360 => {
                self.config.contents.show_x360 = !self.config.contents.show_x360;

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

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
                message_queue.push_back((Message::RefreshAll, SharedString::new()));
            }
            Message::Disconnect => {
                // On oublie la cible mais on garde les identifiants FTP
                // pour la prochaine connexion.
                self.config.contents.target_kind = TargetKind::Local;
                self.config.contents.mount_point = PathBuf::new();

                message_queue.push_back((Message::SyncConfig, SharedString::new()));
                message_queue.push_back((Message::RefreshAll, SharedString::new()));
            }
            Message::DownloadCovers => {
                if !self.is_downloading_covers {
                    self.is_downloading_covers = true;

                    let app = weak.upgrade().unwrap();
                    app.global::<UiState<'_>>().set_downloading_covers(true);

                    let ids = self.games.iter().map(|g| g.id.clone()).collect::<Vec<_>>();

                    let weak = weak.clone();

                    let _ = std::thread::spawn(move || {
                        let res = covers::download_covers(ids, &weak);

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
            Message::FinishedDownloadingCovers => {
                self.is_downloading_covers = false;

                let app = weak.upgrade().unwrap();
                app.global::<UiState<'_>>().set_downloading_covers(false);

                // Transforme les spinners restants en icône « pas de jaquette ».
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
                let Some(conv) = self.conversion_queue.pop_front() else {
                    self.is_converting = false;
                    let text = "Conversion queue empty";
                    self.notifications.push(Notification::info(text));
                    return;
                };

                let _ = self.displayed_conversion_queue.remove(0);

                let weak = weak.clone();
                let config = self.config.clone();

                let _ = std::thread::spawn(move || {
                    perform_conversion(conv, &config, &weak);
                });
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
            Message::DeleteGame => {
                let path = Path::new(&payload);
                let Some(game) = self.games.iter().find(|g| g.path == path).cloned() else {
                    return;
                };
                let Some(target) = Target::from_config(&self.config.contents) else {
                    return;
                };

                let weak = weak.clone();
                std::thread::spawn(move || {
                    let res = target.delete_game(&game);

                    let _ = weak.upgrade_in_event_loop(move |app| {
                        let dispatcher = app.global::<Dispatcher<'_>>();

                        if let Err(e) = res {
                            let text = slint::format!("Failed to delete game: {e:#}");
                            dispatcher.invoke_dispatch(Message::NotifyError, text);
                        }

                        dispatcher.invoke_dispatch(Message::RefreshAll, SharedString::new());
                    });
                });
            }
            Message::CancelConversion => {
                let i = payload.parse().unwrap();
                let _ = self.conversion_queue.remove(i);
                let _ = self.displayed_conversion_queue.remove(i);
            }
            Message::CancelAllConversions => {
                self.conversion_queue.clear();
                self.displayed_conversion_queue.clear();
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
            #[cfg(target_os = "macos")]
            Message::RunDotClean => {
                let res = std::process::Command::new("dot_clean")
                    .arg("-m")
                    .arg(&self.config.contents.mount_point)
                    .status();

                match res {
                    Ok(status) if status.success() => {
                        let text = "Successfully ran dot_clean";
                        self.notifications.push(Notification::info(text));
                    }
                    Ok(status) => {
                        let text = slint::format!("dot_clean exited with {status}");
                        self.notifications.push(Notification::error(text));
                    }
                    Err(e) => {
                        let text = slint::format!("Failed to run dot_clean: {e}");
                        self.notifications.push(Notification::error(text));
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            Message::RunDotClean => {}
        }
    }
}
