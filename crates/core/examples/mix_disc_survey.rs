// SPDX-License-Identifier: GPL-3.0-only
//! Survey of the discs that carry *both* an executable and a bundled
//! `Content/0000000000000000` tree, to decide whether a "Mix" disc (one that
//! needs a GOD **and** its packages installed — Splinter Cell: Blacklist D2)
//! can be told apart from an ordinary bonus disc automatically.
//!
//! See `docs/iso2god-compatibility-notes.md`, gap 1. The measurement runs
//! through the very readers the app uses (`XdvdImage`, `stfs::inspect_reader`),
//! so any criterion it validates is one the app can actually compute at run
//! time — an ad-hoc parser could see things the app never will.
//!
//! ```text
//! cargo run --release -p txbm-core --example mix_disc_survey -- \
//!     <dir|file>… [--jobs N] [--json out.jsonl] [--tmp dir] [--exclude pattern]…
//! ```
//!
//! Archives are extracted to a temporary directory and deleted right after;
//! the input directory itself is only ever read.

use anyhow::{Result, anyhow, bail};
use iso2god::executable::xex::XexHeader;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use txbm_core::iso_info;
use txbm_core::stfs;
use txbm_core::xdvd::XdvdImage;

/// Same prefix `XdvdImage::title_info` reads: every XEX field we want lives
/// well inside it.
const XEX_PREFIX: u32 = 64 << 10;

/// Enough to hold a whole STFS header (`stfs::HEADER_SIZE`), read at the
/// start of every candidate file instead of the file itself.
const STFS_PREFIX: u32 = stfs::HEADER_SIZE as u32;

// ---------------------------------------------------------------- reporting

#[derive(Serialize)]
struct XexReport {
    title_id: String,
    media_id: String,
    disc_number: u8,
    disc_count: u8,
    /// Criterion B: `TITLE_MODULE` vs. `MODULE_PATCH`/`FULL_PATCH`/`DELTA_PATCH`.
    module_flags: String,
    module_flags_raw: u32,
    /// Criterion C: the module's original PE name, e.g. `ExpansionInstaller`.
    original_pe_name: Option<String>,
    xex_size: u64,
}

#[derive(Serialize)]
struct PackageReport {
    path: String,
    content_type: String,
    title_id: String,
    media_id: String,
    name: Option<String>,
    disc_number: u8,
    disc_in_set: u8,
    size: u64,
}

#[derive(Serialize)]
struct DiscReport {
    file: String,
    /// Set when the input was skipped on purpose (`--exclude`); it still shows
    /// up in the summary so the measured corpus is readable without guessing
    /// what is missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    excluded: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measure: Option<Measure>,
}

#[derive(Serialize)]
struct Measure {
    /// What `iso_info::inspect` decides today, verbatim.
    kind: String,
    iso_size: u64,
    root_offset: u64,
    max_used_prefix_size: u64,
    has_xex: bool,
    has_xbe: bool,
    has_bundled_content: bool,
    xex: Option<XexReport>,
    root_entries: Vec<String>,
    markers: Vec<String>,
    /// Criterion D: bytes under `/Content` against every file on the disc.
    content_bytes: u64,
    total_file_bytes: u64,
    packages: Vec<PackageReport>,
}

impl Measure {
    /// Criterion D, as a ratio; `None` on an empty image rather than a
    /// division by zero.
    fn content_ratio(&self) -> Option<f64> {
        (self.total_file_bytes > 0)
            .then(|| self.content_bytes as f64 / self.total_file_bytes as f64)
    }

    /// The distinct TitleIDs the bundled packages declare in their own STFS
    /// headers, sorted. `$SystemUpdate` payloads are left in: they are part of
    /// what the disc carries, and excluding them here would hide it.
    fn package_title_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> =
            self.packages.iter().map(|p| p.title_id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Criterion E: does the disc's own executable claim the same TitleID as
    /// the packages it carries?
    ///
    /// A bonus disc's executable is a stand-in (the Skyrim Legendary installer
    /// declares the `FFED2000` placeholder in the XEX itself, not merely in a
    /// folder name), while a "Mix" disc is a real half of the game and claims
    /// the game's own TitleID. `None` when there is no XEX or no package.
    fn xex_is_one_of_the_packages(&self) -> Option<bool> {
        let xex = self.xex.as_ref()?;
        let ids = self.package_title_ids();
        (!ids.is_empty()).then(|| ids.contains(&xex.title_id))
    }
}

// ------------------------------------------------------------------ XEX 0xFF

/// Reads the XEX optional header `OriginalPeName` (`0x000183FF`) out of the
/// prefix `XdvdImage::read_prefix` already returned.
///
/// `iso2god` lists the id but only parses `ExecutionId`, so this is done by
/// hand. The low byte of an id says how its value is stored: `0xFF` means the
/// value is an offset, from the start of the header, to a `u32` byte count
/// (counting itself) followed by the data — here a NUL-terminated name.
///
/// It stays local to this survey: nothing goes into production until the
/// criterion is actually retained.
fn original_pe_name(buf: &[u8]) -> Option<String> {
    let be32 = |at: usize| -> Option<u32> {
        let b = buf.get(at..at + 4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };

    if buf.get(..4)? != b"XEX2" {
        return None;
    }
    let field_count = be32(0x14)? as usize;
    for i in 0..field_count {
        let at = 0x18 + i * 8;
        if be32(at)? != 0x0001_83FF {
            continue;
        }
        let offset = be32(at + 4)? as usize;
        let size = be32(offset)? as usize;
        let data = buf.get(offset + 4..offset + size.max(4))?;
        let name: String = data
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        let name = name.trim().to_string();
        return (!name.is_empty()).then_some(name);
    }
    None
}

// ------------------------------------------------------------------ measuring

fn measure_iso(iso: &Path) -> Result<Measure> {
    let iso_size = std::fs::metadata(iso)?.len();
    // The app's own verdict, obtained exactly the way the app obtains it.
    let kind = iso_info::inspect(iso)?.kind.label().to_string();

    let mut image = XdvdImage::open(iso)?;
    let root_offset = image.root_offset()?;
    let max_used_prefix_size = image.max_used_prefix_size()?;

    let xex_node = image.find("/default.xex")?;
    let has_xex = xex_node.is_some();
    let has_xbe = image.find("/default.xbe")?.is_some();
    let has_bundled_content = image.find("/Content/0000000000000000")?.is_some();

    let xex = match &xex_node {
        Some(node) => {
            let buf = image.read_prefix(node, XEX_PREFIX)?;
            let header = XexHeader::read(std::io::Cursor::new(&buf[..]))
                .map_err(|e| anyhow!("reading default.xex: {e}"))?;
            let exe = header
                .fields
                .execution_info
                .ok_or_else(|| anyhow!("no execution info in the default.xex header"))?;
            Some(XexReport {
                title_id: format!("{:08X}", exe.title_id),
                media_id: format!("{:08X}", exe.media_id),
                disc_number: exe.disc_number,
                disc_count: exe.disc_count,
                module_flags: format!("{:?}", header.module_flags),
                module_flags_raw: header.module_flags.bits(),
                original_pe_name: original_pe_name(&buf),
                xex_size: node.node.dirent.data.size() as u64,
            })
        }
        None => None,
    };

    let tree = image.file_tree()?;
    let mut root_entries = Vec::new();
    let mut content_bytes = 0u64;
    let mut total_file_bytes = 0u64;
    let mut packages = Vec::new();
    let mut has_placeholder_dir = false;

    for (dir, node) in &tree {
        let Ok(name) = node.name_str::<std::io::Error>() else {
            continue;
        };
        if dir.is_empty() {
            root_entries.push(name.to_string());
        }
        // A placeholder TitleID folder is what makes gap 3 (`ContentDisc`
        // trusting folder names) reachable, so it is recorded here too.
        if name.eq_ignore_ascii_case("FFED2000") || name.eq_ignore_ascii_case("FFFFFFFF") {
            has_placeholder_dir = true;
        }
        if node.node.dirent.is_directory() {
            continue;
        }

        let size = node.node.dirent.data.size() as u64;
        total_file_bytes += size;
        let under_content = dir.len() >= 8 && dir[..8].eq_ignore_ascii_case("/Content");
        if under_content {
            content_bytes += size;
        }

        // Only a file big enough to hold a header can be a package; the magic
        // check costs 4 bytes, the full header 6 KiB, never the payload.
        if size < stfs::HEADER_SIZE as u64 {
            continue;
        }
        let Ok(magic) = image.read_prefix(node, 4) else {
            continue;
        };
        let Ok(magic) = <[u8; 4]>::try_from(&magic[..]) else {
            continue;
        };
        if !stfs::is_stfs_magic(&magic) {
            continue;
        }
        let entry_path = format!("{dir}/{name}");
        let Ok(buf) = image.read_prefix(node, STFS_PREFIX) else {
            continue;
        };
        let mut cursor = std::io::Cursor::new(buf);
        if let Ok(Some(info)) = stfs::inspect_reader(&mut cursor, entry_path.clone().into()) {
            packages.push(PackageReport {
                path: entry_path,
                content_type: format!("{:08X}", info.content_type),
                title_id: info.title_id.clone(),
                media_id: info.media_id.clone(),
                name: info.name().map(str::to_string),
                disc_number: info.disc_number,
                disc_in_set: info.disc_in_set,
                size,
            });
        }
    }

    root_entries.sort_by_key(|n| n.to_lowercase());
    let markers = markers_of(&root_entries, has_placeholder_dir);

    Ok(Measure {
        kind,
        iso_size,
        root_offset,
        max_used_prefix_size,
        has_xex,
        has_xbe,
        has_bundled_content,
        xex,
        root_entries,
        markers,
        content_bytes,
        total_file_bytes,
        packages,
    })
}

/// Root-level shapes worth flagging, including the ones that feed gaps 2 and 3
/// rather than gap 1.
fn markers_of(root_entries: &[String], has_placeholder_dir: bool) -> Vec<String> {
    let has = |needle: &str| {
        root_entries
            .iter()
            .any(|n| n.eq_ignore_ascii_case(needle))
    };
    let mut markers = Vec::new();
    if has("installation1") && has("installation2") {
        // Gap 2: the Ubisoft install-disc pair (Watch_Dogs).
        markers.push("installation1+installation2".into());
    }
    if root_entries
        .iter()
        .any(|n| n.len() == 6 && n[..5].eq_ignore_ascii_case("Disc.") && n.ends_with(|c: char| c.is_ascii_digit()))
    {
        // The zero-byte multi-disc marker that breaks iso2god's own reader.
        markers.push("Disc.N".into());
    }
    if has("title_update.bin") {
        markers.push("title_update.bin".into());
    }
    if has("nxeart") {
        markers.push("nxeart".into());
    }
    if has_placeholder_dir {
        // Gap 3: a placeholder TitleID folder name.
        markers.push("placeholder-dir".into());
    }
    markers
}

/// Extracts an archive into `tmp` and returns the single ISO it wraps.
fn iso_from_archive(archive: &Path, tmp: &Path) -> Result<PathBuf> {
    let cancel = AtomicBool::new(false);
    txbm_core::archive::extract_to(archive, tmp, &cancel, &mut |_, _| {})?;
    let mut found = Vec::new();
    collect_isos(tmp, &mut found)?;
    match found.len() {
        1 => Ok(found.pop().unwrap()),
        0 => bail!("no ISO inside the archive"),
        n => bail!("{n} ISOs inside the archive"),
    }
}

fn collect_isos(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_isos(&path, out)?;
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("iso"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn measure_input(input: &Path, tmp: &Path) -> Result<Measure> {
    if txbm_core::archive::is_supported_archive(input) {
        let _ = std::fs::remove_dir_all(tmp);
        let result = iso_from_archive(input, tmp).and_then(|iso| measure_iso(&iso));
        let _ = std::fs::remove_dir_all(tmp);
        result
    } else {
        measure_iso(input)
    }
}

// ------------------------------------------------------------------------ CLI

struct Args {
    inputs: Vec<PathBuf>,
    jobs: usize,
    json: Option<PathBuf>,
    tmp: PathBuf,
    exclude: Vec<String>,
}

fn parse_args() -> Result<Args> {
    let mut inputs = Vec::new();
    let mut jobs = 1usize;
    let mut json = None;
    let mut tmp = std::env::temp_dir().join("txbm-mix-survey");
    let mut exclude = Vec::new();

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--jobs" | "-j" => {
                jobs = it.next().ok_or_else(|| anyhow!("--jobs needs a value"))?.parse()?
            }
            "--json" => json = Some(it.next().ok_or_else(|| anyhow!("--json needs a path"))?.into()),
            "--tmp" => tmp = it.next().ok_or_else(|| anyhow!("--tmp needs a path"))?.into(),
            "--exclude" => {
                exclude.push(it.next().ok_or_else(|| anyhow!("--exclude needs a pattern"))?)
            }
            other if other.starts_with('-') => bail!("unknown option {other}"),
            other => inputs.push(PathBuf::from(other)),
        }
    }
    if inputs.is_empty() {
        bail!(
            "usage: mix_disc_survey <dir|file>… [--jobs N] [--json out.jsonl] \
             [--tmp dir] [--exclude pattern]…"
        );
    }
    Ok(Args { inputs, jobs: jobs.max(1), json, tmp, exclude })
}

/// Expands directories one level deep into the images they hold.
fn expand(inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for input in inputs {
        if input.is_dir() {
            for entry in std::fs::read_dir(input)?.flatten() {
                let path = entry.path();
                let is_image = txbm_core::archive::is_supported_archive(&path)
                    || path.extension().is_some_and(|e| e.eq_ignore_ascii_case("iso"));
                if path.is_file() && is_image {
                    out.push(path);
                }
            }
        } else {
            out.push(input.clone());
        }
    }
    out.sort();
    Ok(out)
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let images = expand(&args.inputs)?;
    std::fs::create_dir_all(&args.tmp)?;

    let excluded = |path: &Path| -> Option<String> {
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        args.exclude
            .iter()
            .find(|p| name.contains(&p.to_lowercase()))
            .cloned()
    };

    eprintln!("{} image(s), {} job(s)", images.len(), args.jobs);

    let queue = Mutex::new(images.iter().cloned().enumerate().collect::<VecDeque<_>>());
    let reports = Mutex::new(Vec::<DiscReport>::new());
    let done = Mutex::new(0usize);
    let total = images.len();

    std::thread::scope(|scope| {
        for worker in 0..args.jobs {
            let (queue, reports, done, args, excluded) =
                (&queue, &reports, &done, &args, &excluded);
            scope.spawn(move || {
                let tmp = args.tmp.join(format!("worker-{worker}"));
                loop {
                    let Some((_, path)) = queue.lock().unwrap().pop_front() else {
                        return;
                    };
                    let file = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let report = match excluded(&path) {
                        Some(pattern) => DiscReport {
                            file,
                            excluded: Some(pattern),
                            error: None,
                            measure: None,
                        },
                        None => match measure_input(&path, &tmp) {
                            Ok(measure) => DiscReport {
                                file,
                                excluded: None,
                                error: None,
                                measure: Some(measure),
                            },
                            Err(e) => DiscReport {
                                file,
                                excluded: None,
                                error: Some(format!("{e:#}")),
                                measure: None,
                            },
                        },
                    };
                    let mut n = done.lock().unwrap();
                    *n += 1;
                    eprintln!("[{}/{total}] {}", *n, report.file);
                    drop(n);
                    reports.lock().unwrap().push(report);
                }
            });
        }
    });

    let _ = std::fs::remove_dir_all(&args.tmp);

    let mut reports = reports.into_inner().unwrap();
    reports.sort_by_key(|r| r.file.to_lowercase());

    if let Some(path) = &args.json {
        let mut out = String::new();
        for report in &reports {
            out.push_str(&serde_json::to_string(report)?);
            out.push('\n');
        }
        std::fs::write(path, out)?;
        eprintln!("wrote {}", path.display());
    }

    print_summary(&reports);
    Ok(())
}

// --------------------------------------------------------------- presentation

fn print_summary(reports: &[DiscReport]) {
    println!();
    println!("== every image ==");
    println!(
        "{:<58} {:<26} {:>8} {:>7} {:>8} {:>6} {:>4}",
        "file", "kind", "titleid", "disc", "content%", "pkgs", "xex?"
    );
    for report in reports {
        let file = elide(&report.file, 58);
        let Some(measure) = &report.measure else {
            let why = report
                .excluded
                .as_ref()
                .map(|p| format!("excluded ({p})"))
                .or_else(|| report.error.clone())
                .unwrap_or_default();
            println!("{file:<58} {}", elide(&why, 60));
            continue;
        };
        let (title_id, disc) = match &measure.xex {
            Some(xex) => (
                xex.title_id.clone(),
                format!("{}/{}", xex.disc_number, xex.disc_count),
            ),
            None => ("-".into(), "-".into()),
        };
        let ratio = measure
            .content_ratio()
            .map(|r| format!("{:.1}%", r * 100.0))
            .unwrap_or_else(|| "-".into());
        println!(
            "{file:<58} {:<26} {title_id:>8} {disc:>7} {ratio:>8} {:>6} {:>4}",
            elide(&measure.kind, 26),
            measure.packages.len(),
            if measure.has_xex { "yes" } else { "no" },
        );
    }

    let bundled: Vec<&DiscReport> = reports
        .iter()
        .filter(|r| {
            r.measure
                .as_ref()
                .is_some_and(|m| m.has_bundled_content && (m.has_xex || m.has_xbe))
        })
        .collect();

    println!();
    println!(
        "== the population to split: {} disc(s) with an executable AND bundled content ==",
        bundled.len()
    );
    println!(
        "{:<48} {:>7} {:>8} {:>10} {:<22} {:>8} {:<20} {:>6} B module flags",
        "file",
        "A disc",
        "D cont%",
        "xex bytes",
        "C original PE name",
        "xex tid",
        "package tids",
        "E same",
    );
    for report in &bundled {
        let measure = report.measure.as_ref().unwrap();
        let (disc, xex_size, pe_name, xex_tid, flags) = match &measure.xex {
            Some(xex) => (
                format!("{}/{}", xex.disc_number, xex.disc_count),
                xex.xex_size.to_string(),
                xex.original_pe_name.clone().unwrap_or_else(|| "-".into()),
                xex.title_id.clone(),
                xex.module_flags.clone(),
            ),
            None => ("-".into(), "-".into(), "-".into(), "-".into(), "-".into()),
        };
        let ratio = measure
            .content_ratio()
            .map(|r| format!("{:.1}%", r * 100.0))
            .unwrap_or_else(|| "-".into());
        let same = match measure.xex_is_one_of_the_packages() {
            Some(true) => "yes",
            Some(false) => "no",
            None => "-",
        };
        println!(
            "{:<48} {disc:>7} {ratio:>8} {xex_size:>10} {:<22} {xex_tid:>8} {:<20} {same:>6} {flags}",
            elide(&report.file, 48),
            elide(&pe_name, 22),
            elide(&measure.package_title_ids().join(","), 20),
        );
    }

    println!();
    println!("== markers (gaps 2 and 3) ==");
    for report in reports {
        let Some(measure) = &report.measure else { continue };
        if measure.markers.is_empty() {
            continue;
        }
        println!(
            "{:<58} {}",
            elide(&report.file, 58),
            measure.markers.join(", ")
        );
    }
}

fn elide(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    let kept: String = s.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}
