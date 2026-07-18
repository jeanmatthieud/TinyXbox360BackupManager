// SPDX-License-Identifier: GPL-3.0-only
// Based on the architecture of TinyWiiBackupManager by Manuel Quarneti.

#![warn(clippy::all, rust_2018_idioms)]

pub mod config;
pub mod conversion_queue;
pub mod convert;
pub mod covers;
pub mod data_dir;
pub mod drive_info;
pub mod extract;
pub mod ftp;
pub mod game;
pub mod god;
pub mod iso_info;
pub mod mobcat;
pub mod target;
pub mod unity;
pub mod updates;
pub mod util;
pub mod xbe;

/// Standard folder of GOD / official content on the console.
pub const CONTENT_DIR: &str = "Content/0000000000000000";
/// Folder for extracted games (default.xex / default.xbe).
pub const GAMES_DIR: &str = "Games";
