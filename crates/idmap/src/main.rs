// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
// SPDX-License-Identifier: MIT OR Apache-2.0

#![warn(clippy::all, rust_2018_idioms)]

const USAGE: &str = "Usage: idmap <GAMEID>";

fn main() {
    let Some(game_id) = std::env::args().nth(1) else {
        eprintln!("{USAGE}");
        std::process::exit(1);
    };

    let Ok(game_id) = u32::from_str_radix(&game_id, 36) else {
        eprintln!("{USAGE}");
        std::process::exit(1);
    };

    let title = twbm_idmap::get_title(game_id);
    let ghid = twbm_idmap::get_ghid(game_id);

    println!("Title: {title:?}");
    println!("GameHacking ID: {ghid:?}");
}
