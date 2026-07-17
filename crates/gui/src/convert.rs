// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{AppWindow, Dispatcher, Message, UiState};
use slint::{ComponentHandle, SharedString, Weak};
use txbm_core::{config::Config, conversion_queue::QueuedConversion};

pub fn perform_conversion(conv: QueuedConversion, config: &Config, weak: &Weak<AppWindow>) {
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

            txbm_core::convert::perform(in_path, config, &update_progress)
        }
    };

    let _ = weak.upgrade_in_event_loop(move |app| {
        let dispatcher = app.global::<Dispatcher<'_>>();

        dispatcher.invoke_dispatch(Message::SetStatus, SharedString::new());

        if let Err(e) = res {
            let text = slint::format!("Conversion failed: {e:#}");
            dispatcher.invoke_dispatch(Message::NotifyError, text);
        }

        // Drop the finished conversion and move on to the next one (even on
        // failure, so the queue doesn't stall).
        dispatcher.invoke_dispatch(Message::ConversionFinished, SharedString::new());

        dispatcher.invoke_dispatch(Message::RefreshAll, SharedString::new());
    });
}
