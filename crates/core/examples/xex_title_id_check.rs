// SPDX-License-Identifier: GPL-3.0-only

//! Manual check: prints the TitleID read from a `default.xex`.
//!
//! Usage: `cargo run --example xex_title_id_check -- /path/to/default.xex`

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: xex_title_id_check <default.xex>");
        std::process::exit(2);
    };
    match txbm_core::xex::title_id_from_file(std::path::Path::new(&path)) {
        Ok(id) => println!("TitleID: {id}"),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}
