// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![recursion_limit = "256"]

mod config;
mod covers;
mod dialogs;
mod drive_info;
mod file_drop;
mod game_details;
mod games;
mod jobs;
mod notification;
mod state;
mod title_updates;
mod update;
mod util;

#[cfg(windows)]
mod window_color;

use crate::{file_drop::FileDropHandler, state::State};
use anyhow::{Result, bail};
use slint::{BackendSelector, ComponentHandle, ModelRc, SharedString, ToSharedString};
use std::{collections::VecDeque, process::Command};
use txbm_core::data_dir::{DATA_DIR, sweep_retired};

slint::include_modules!();

fn restart_with_sw_rendering() -> Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.env("SLINT_BACKEND", "winit-software");
    let _ = cmd.spawn()?;

    std::process::exit(0);
}

fn main() -> Result<()> {
    if DATA_DIR.as_os_str().is_empty() {
        bail!("Failed to get data dir");
    }

    // Only one copy of the app may run: a second one moves the scratch folders
    // the first is converting into aside as debris and deletes them, and both
    // would write to the same console at once. Taken before anything else
    // touches those folders, and held for the whole process.
    let _instance = match txbm_core::instance::lock() {
        txbm_core::instance::InstanceGuard::Acquired(lock) => Some(lock),
        txbm_core::instance::InstanceGuard::AlreadyRunning => {
            let message = "TinyXbox360BackupManager is already running.";
            eprintln!("{message}");
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Info)
                .set_title("TinyXbox360BackupManager")
                .set_description(message)
                .show();
            return Ok(());
        }
        // No usable lock file: carry on rather than refuse to start.
        txbm_core::instance::InstanceGuard::Unavailable => None,
    };

    let (file_drop_handler, file_drop_dispatcher) = FileDropHandler::new();

    BackendSelector::new()
        .with_winit_custom_application_handler(file_drop_handler)
        .select()?;

    #[cfg(target_os = "linux")]
    let _ = slint::set_xdg_app_id("fr.dechriste.TinyXbox360BackupManager");

    let app = AppWindow::new()?;
    let dispatcher = app.global::<Dispatcher<'_>>();

    // Enable file drop handling
    file_drop_dispatcher
        .borrow_mut()
        .write(dispatcher.as_weak());

    let mut state = State::new();

    // Initialize UI state
    let ui_state = app.global::<UiState<'_>>();
    ui_state.set_app_version(env!("CARGO_PKG_VERSION").to_shared_string());
    ui_state.set_data_dir(DATA_DIR.to_string_lossy().to_shared_string());
    ui_state.set_config(DisplayedConfig::from(&state.config));
    ui_state.set_badavatar(config::displayed_badavatar(&state.config));
    ui_state.set_recent_locations(ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
        config::recent_locations(&state.config),
    ))));
    ui_state.set_games(ModelRc::from(state.displayed_games.clone()));
    ui_state.set_title_updates(ModelRc::from(state.displayed_title_updates.clone()));
    ui_state.set_notifications(ModelRc::from(state.notifications.clone()));
    ui_state.set_job_queue(ModelRc::from(state.displayed_job_queue.clone()));
    ui_state.set_games_to_add(ModelRc::from(state.displayed_games_to_add.clone()));

    // Process messages
    dispatcher.on_dispatch({
        let weak = app.as_weak();
        let mut message_queue = VecDeque::new();

        move |message, payload| {
            message_queue.push_back((message, payload));

            while let Some((message, payload)) = message_queue.pop_front() {
                state.update(message, payload, &mut message_queue, &weak);
            }
        }
    });

    // Closing the window with a busy job queue asks whether it should be
    // cancelled first; the quit is then replayed once the queue is drained.
    //
    // Whether the queue is busy is decided by `RequestQuit` alone, on the Rust
    // side: testing the Slint model here too would give two sources of truth
    // that can disagree (the model only follows in the message handlers) and
    // leave the window shown while the handler takes the "nothing queued" path.
    // The dispatch is synchronous, so an empty queue quits the event loop
    // before we return and `KeepWindowShown` never applies.
    app.window().on_close_requested({
        let weak = app.as_weak();

        move || {
            let app = weak.upgrade().unwrap();

            app.global::<Dispatcher<'_>>()
                .invoke_dispatch(Message::RequestQuit, SharedString::new());

            slint::CloseRequestResponse::KeepWindowShown
        }
    });

    // Drop whatever a previous run left in the scratch folders, and say so:
    // a killed import strands the whole game it was extracting, and a user
    // whose disk quietly lost 8 GB has no way of connecting the two.
    //
    // The debris is moved aside right here, synchronously — this run must not
    // start writing in those folders (the first scan already stages Aurora's
    // databases in them) while a deletion is still walking them. That rename
    // is instant; the deletion itself then runs on a thread, on folders
    // nothing else will ever touch again.
    {
        let retired = txbm_core::data_dir::retire_work_dirs();
        let weak = app.as_weak();
        std::thread::spawn(move || {
            let reclaimed = sweep_retired(retired);
            if reclaimed == 0 {
                return;
            }
            let _ = weak.upgrade_in_event_loop(move |app| {
                let text = slint::format!(
                    "Reclaimed {} of temporary files left by an interrupted run",
                    txbm_core::util::human_size(reclaimed)
                );
                app.global::<Dispatcher<'_>>()
                    .invoke_dispatch(Message::NotifyInfo, text);
            });
        });
    }

    // Initialize
    dispatcher.invoke_dispatch(Message::RefreshAll, SharedString::new());

    if let Err(e) = app.run() {
        if std::env::var("SLINT_BACKEND").unwrap_or_default() == "winit-software" {
            bail!(e);
        }

        // Release the single-instance lock first: the replacement process
        // starts before this one exits and would otherwise find itself locked
        // out by its own parent.
        drop(_instance);
        return restart_with_sw_rendering();
    }

    Ok(())
}
