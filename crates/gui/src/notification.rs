// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me> (TinyWiiBackupManager)
// SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
// SPDX-License-Identifier: GPL-3.0-only

use crate::{Notification, NotificationKind};
use slint::SharedString;
use std::sync::atomic::{AtomicI32, Ordering};

/// Handed out to every notification so toasts can dismiss themselves by
/// identity. Several can time out in the same frame, and removing one by row
/// would shift the ones after it.
static NEXT_ID: AtomicI32 = AtomicI32::new(1);

impl Notification {
    fn new(text: impl Into<SharedString>, kind: NotificationKind) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            text: text.into(),
            kind,
            sticky: false,
        }
    }

    pub fn info(text: impl Into<SharedString>) -> Self {
        Self::new(text, NotificationKind::Info)
    }

    pub fn success(text: impl Into<SharedString>) -> Self {
        Self::new(text, NotificationKind::Success)
    }

    /// Success toast that stays up until the user closes it, for the end of a
    /// long operation they may not have been watching.
    pub fn success_sticky(text: impl Into<SharedString>) -> Self {
        Self {
            sticky: true,
            ..Self::new(text, NotificationKind::Success)
        }
    }

    pub fn error(text: impl Into<SharedString>) -> Self {
        Self::new(text, NotificationKind::Error)
    }
}
