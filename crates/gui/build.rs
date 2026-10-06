// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
// SPDX-License-Identifier: GPL-3.0-only

use std::{collections::HashMap, env, path::PathBuf};

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // The .slint sources need no line here: `slint_build` declares every file
    // it compiles. The Windows resources below are read by `winresource`,
    // which declares nothing.
    println!("cargo::rerun-if-changed=../../package/windows/icon.ico");
    println!(
        "cargo::rerun-if-changed=../../package/windows/TinyXbox360BackupManager.exe.manifest"
    );

    let library = HashMap::from([("lucide".to_string(), PathBuf::from(lucide_slint::lib()))]);
    let config = slint_build::CompilerConfiguration::new().with_library_paths(library);

    slint_build::compile_with_config("../../ui/app-window.slint", config).unwrap();

    let target = env::var("TARGET").unwrap();

    if target.contains("-windows-") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../package/windows/icon.ico");
        res.set_manifest_file("../../package/windows/TinyXbox360BackupManager.exe.manifest");
        res.compile().unwrap();
    }
}
