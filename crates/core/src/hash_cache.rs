// SPDX-License-Identifier: GPL-3.0-only

//! A persistent path → SHA1 cache for files that live on a console.
//!
//! Identifying an installed title update means hashing it, and the only way
//! to hash a file sitting on the console is to pull all of it across — a
//! routinely 100 MB transfer, paid again every time the Title Updates window
//! opens and after every activate/deactivate. The content of a given path
//! does not change unless this app (or Aurora) rewrites it, so the digest is
//! remembered between runs, keyed by the file's size as well as its path so a
//! rewrite of a different length is never served from the cache.

use crate::data_dir::DATA_DIR;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct HashCache {
    path: PathBuf,
    map: HashMap<String, String>,
}

impl HashCache {
    pub fn load() -> Self {
        let path = DATA_DIR.join("file-hashes.json");
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, map }
    }

    pub fn get(&self, host: &str, remote_path: &str, size: u64) -> Option<&String> {
        self.map.get(&Self::key(host, remote_path, size))
    }

    pub fn insert(&mut self, host: &str, remote_path: &str, size: u64, hash: String) {
        self.map.insert(Self::key(host, remote_path, size), hash);
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.map) {
            let _ = crate::data_dir::ensure_data_dir();
            let _ = std::fs::write(&self.path, json);
        }
    }

    fn key(host: &str, remote_path: &str, size: u64) -> String {
        format!("{host}|{size}|{remote_path}")
    }
}
