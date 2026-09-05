// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{DisplayedGame, DisplayedGameToAdd, DisplayedJob, DisplayedTitleUpdate, Notification};
use slint::VecModel;
use std::{
    collections::VecDeque,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};
use txbm_core::{config::Config, drive_info::DriveInfo, game::Game, job_queue::QueuedJob};

pub struct State {
    pub config: Config,
    pub games: Vec<Game>,
    pub drive_info: DriveInfo,
    pub displayed_games: Rc<VecModel<DisplayedGame>>,
    pub displayed_title_updates: Rc<VecModel<DisplayedTitleUpdate>>,
    /// Everything that writes to the target — additions and deletions alike —
    /// goes through this single queue, so only one such operation ever runs at
    /// a time. The job at index 0 is the one currently running.
    pub job_queue: VecDeque<QueuedJob>,
    pub displayed_job_queue: Rc<VecModel<DisplayedJob>>,
    /// Files picked for addition, awaiting the confirmation that queues them.
    /// The whole pick is kept, not just its path: the TitleID it read is what
    /// lets the queued job name the installed game it will write over.
    pub games_to_add: VecDeque<crate::util::PickedGame>,
    pub displayed_games_to_add: Rc<VecModel<DisplayedGameToAdd>>,
    pub notifications: Rc<VecModel<Notification>>,
    pub is_job_running: bool,
    /// Additions that succeeded since the queue was last empty. Drives the
    /// confetti burst when it drains, and is reset by a cancellation so an
    /// aborted batch doesn't get a celebration. Deletions never count: there
    /// is nothing to celebrate about removing a game.
    pub adds_done: usize,
    /// Jobs that failed since the queue was last empty. A non-zero count
    /// withholds the confetti burst: it's only for a batch that went through
    /// cleanly, not a partially-failed one.
    pub jobs_failed: usize,
    /// Set when the user cancels the queue, cleared only once it has drained.
    /// Zeroing the counters isn't enough on its own: the running job can still
    /// finish successfully before it reaches its next cancellation checkpoint,
    /// which would put `adds_done` back to 1 and celebrate a batch the user
    /// just aborted.
    pub batch_cancelled: bool,
    pub is_downloading_covers: bool,
    pub is_scanning: bool,
    /// Set when a rescan was asked for while a job was writing to a console
    /// over FTP: it is replayed once the queue drains. The console's FTP
    /// server tolerates no other connection alongside a write (see
    /// `crates/core/src/ftp.rs`), so the scan cannot just run anyway.
    pub rescan_deferred: bool,
    pub is_creating_badavatar: bool,
    /// Destination picked for the BadAvatar key, awaiting confirmation in the
    /// modal before the creation thread actually starts.
    pub badavatar_pending_dest: Option<PathBuf>,
    /// Flag shared with the scan thread to cancel it.
    pub scan_cancel: Arc<AtomicBool>,
    /// Flag shared with the network-discovery thread (FTP modal) to cancel it.
    pub ftp_scan_cancel: Arc<AtomicBool>,
    /// Flag shared with the running job thread to cancel it.
    pub job_cancel: Arc<AtomicBool>,
    /// Flag shared with the BadAvatar creation thread to cancel it.
    pub badavatar_cancel: Arc<AtomicBool>,
    pub games_filter: String,
    /// True when the storage-configuration modal was opened to *edit* an
    /// already-configured target (from the Toolbox), so it is shown even though
    /// a `.txbm.json` already exists.
    pub editing_storage: bool,
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
            job_queue: VecDeque::new(),
            displayed_job_queue: Rc::new(VecModel::from(Vec::new())),
            games_to_add: VecDeque::new(),
            displayed_games_to_add: Rc::new(VecModel::from(Vec::new())),
            notifications: Rc::new(VecModel::from(Vec::new())),
            is_job_running: false,
            adds_done: 0,
            jobs_failed: 0,
            batch_cancelled: false,
            is_downloading_covers: false,
            is_scanning: false,
            rescan_deferred: false,
            is_creating_badavatar: false,
            badavatar_pending_dest: None,
            scan_cancel: Arc::new(AtomicBool::new(false)),
            ftp_scan_cancel: Arc::new(AtomicBool::new(false)),
            job_cancel: Arc::new(AtomicBool::new(false)),
            badavatar_cancel: Arc::new(AtomicBool::new(false)),
            games_filter: String::new(),
            editing_storage: false,
        }
    }
}
