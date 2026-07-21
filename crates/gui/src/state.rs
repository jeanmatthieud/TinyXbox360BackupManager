// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{DisplayedGame, DisplayedTitleUpdate, Notification};
use slint::{SharedString, VecModel};
use std::{
    collections::VecDeque,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};
use txbm_core::{
    config::Config, conversion_queue::QueuedConversion, drive_info::DriveInfo, game::Game,
};

pub struct State {
    pub config: Config,
    pub games: Vec<Game>,
    pub drive_info: DriveInfo,
    pub displayed_games: Rc<VecModel<DisplayedGame>>,
    pub displayed_title_updates: Rc<VecModel<DisplayedTitleUpdate>>,
    pub conversion_queue: VecDeque<QueuedConversion>,
    pub displayed_conversion_queue: Rc<VecModel<SharedString>>,
    pub games_to_add: VecDeque<PathBuf>,
    pub displayed_games_to_add: Rc<VecModel<SharedString>>,
    pub notifications: Rc<VecModel<Notification>>,
    pub is_converting: bool,
    pub is_downloading_covers: bool,
    pub is_scanning: bool,
    pub is_creating_badavatar: bool,
    /// Flag shared with the scan thread to cancel it.
    pub scan_cancel: Arc<AtomicBool>,
    /// Flag shared with the network-discovery thread (FTP modal) to cancel it.
    pub ftp_scan_cancel: Arc<AtomicBool>,
    /// Flag shared with the running conversion thread to cancel it.
    pub conversion_cancel: Arc<AtomicBool>,
    /// Flag shared with the BadAvatar creation thread to cancel it.
    pub badavatar_cancel: Arc<AtomicBool>,
    pub games_filter: String,
}

impl State {
    pub fn new() -> Self {
        // The persisted config *is* the last active target, so it would always
        // be reconnected on startup; honour the user's auto-reconnect policy by
        // clearing it in memory when reconnecting to its kind is not wanted.
        let mut config = Config::load();
        config.contents.apply_auto_reconnect_policy();

        State {
            config,
            games: Vec::new(),
            drive_info: DriveInfo::default(),
            displayed_games: Rc::new(VecModel::from(Vec::new())),
            displayed_title_updates: Rc::new(VecModel::from(Vec::new())),
            conversion_queue: VecDeque::new(),
            displayed_conversion_queue: Rc::new(VecModel::from(Vec::new())),
            games_to_add: VecDeque::new(),
            displayed_games_to_add: Rc::new(VecModel::from(Vec::new())),
            notifications: Rc::new(VecModel::from(Vec::new())),
            is_converting: false,
            is_downloading_covers: false,
            is_scanning: false,
            is_creating_badavatar: false,
            scan_cancel: Arc::new(AtomicBool::new(false)),
            ftp_scan_cancel: Arc::new(AtomicBool::new(false)),
            conversion_cancel: Arc::new(AtomicBool::new(false)),
            badavatar_cancel: Arc::new(AtomicBool::new(false)),
            games_filter: String::new(),
        }
    }
}
