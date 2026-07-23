// SPDX-License-Identifier: GPL-3.0-only
// Based on the architecture of TinyWiiBackupManager by Manuel Quarneti.

#![warn(clippy::all, rust_2018_idioms)]

pub mod archive;
pub mod badavatar;
pub mod config;
pub mod conversion_queue;
pub mod convert;
pub mod covers;
pub mod data_dir;
pub mod drive_info;
pub mod extract;
pub mod ftp;
pub mod game;
pub mod game_details;
pub mod god;
pub mod iso_info;
pub mod mobcat;
pub mod scan;
pub mod stfs;
pub mod target;
pub mod title_updates;
pub mod unity;
pub mod updates;
pub mod util;
pub mod xbe;

/// Standard folder of GOD / official content on the console.
pub const DEFAULT_GOD_DIR: &str = "Content/0000000000000000";
/// Folder for Xbox OG games (default.xbe).
pub const DEFAULT_XBE_DIR: &str = "Games Xbox";
/// Folder for Xbox360 extracted games (default.xex).
pub const DEFAULT_XEX_DIR: &str = "Games Xbox360";
