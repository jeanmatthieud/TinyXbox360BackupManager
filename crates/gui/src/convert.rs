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
                    ui.set_conversion_progress(percentage as f32 / 100.0);
                    if let Some(speed) = speed {
                        ui.set_conversion_speed(speed);
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
                    ui.set_conversion_phase(text);
                    ui.set_conversion_speed(SharedString::new());
                });
            };

            // Step the percentage currently refers to, shown on the queue page.
            let weak4 = weak.clone();
            let set_phase = move |text: &str| {
                let text = SharedString::from(text);
                let _ = weak4.upgrade_in_event_loop(move |app| {
                    let ui = app.global::<UiState<'_>>();
                    ui.set_conversion_phase(text);
                    // Each step measures (or doesn't measure) its own speed.
                    ui.set_conversion_speed(SharedString::new());
                });
            };

            txbm_core::convert::perform(
                in_path,
                config,
                &cancel,
                &update_progress,
                &set_status,
                &set_phase,
            )
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
