// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{AppWindow, Dispatcher, Message, UiState};
use slint::{ComponentHandle, SharedString, Weak};
use std::sync::{Arc, atomic::AtomicBool};
use txbm_core::{config::Config, conversion_queue::QueuedConversion};

pub fn perform_conversion(
    conv: QueuedConversion,
    config: &Config,
    cancel: Arc<AtomicBool>,
    weak: &Weak<AppWindow>,
) {
    let res = match conv {
        QueuedConversion::Standard(in_path) => {
            let filename = in_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let weak2 = weak.clone();
            let update_progress = move |percentage, speed: Option<f64>| {
                let status = match speed {
                    Some(mbps) => {
                        slint::format!("↑  Adding  {filename}  {percentage}%  ({mbps:.1} MB/s)")
                    }
                    None => slint::format!("↑  Adding  {filename}  {percentage}%"),
                };

                let _ = weak2.upgrade_in_event_loop(move |app| {
                    app.global::<UiState<'_>>().set_status(status);
                });
            };

            // Phases with no measurable progress (the post-cancellation
            // cleanup) take over the status line: without this the UI looks
            // frozen while thousands of extracted files are deleted.
            let weak3 = weak.clone();
            let set_status = move |text: &str| {
                let text = SharedString::from(text);
                let _ = weak3.upgrade_in_event_loop(move |app| {
                    app.global::<UiState<'_>>().set_status(text);
                });
            };

            txbm_core::convert::perform(in_path, config, &cancel, &update_progress, &set_status)
        }
    };

    let _ = weak.upgrade_in_event_loop(move |app| {
        let dispatcher = app.global::<Dispatcher<'_>>();

        dispatcher.invoke_dispatch(Message::SetStatus, SharedString::new());

        let succeeded = res.is_ok();

        match res {
            Ok(()) => {}
            // The whole cause chain is searched: the cancellation marker is
            // often wrapped in a context (e.g. "extracting foo.xex: …").
            Err(e)
                if format!("{e:#}").contains(txbm_core::convert::CONVERSION_CANCELLED) =>
            {
                dispatcher
                    .invoke_dispatch(Message::NotifyInfo, "Conversion cancelled".into());
            }
            Err(e) => {
                let text = slint::format!("Conversion failed: {e:#}");
                dispatcher.invoke_dispatch(Message::NotifyError, text);
            }
        }

        // Drop the finished conversion and move on to the next one (even on
        // failure, so the queue doesn't stall). The payload tells the handler
        // whether this one is worth celebrating when the queue drains.
        let outcome = if succeeded { "ok" } else { "" };
        dispatcher.invoke_dispatch(Message::ConversionFinished, outcome.into());

        dispatcher.invoke_dispatch(Message::RefreshAll, SharedString::new());
    });
}
