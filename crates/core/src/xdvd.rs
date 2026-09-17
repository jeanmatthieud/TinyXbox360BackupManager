// SPDX-License-Identifier: GPL-3.0-only

//! Reader for the XDVDFS file system of an Xbox / Xbox 360 disc image.
//!
//! `iso2god` ships its own XDVDFS reader, but it walks a directory table
//! *sequentially* and stops at the first entry whose size is 0, while XDVDFS
//! stores each table as a **binary tree** (every entry carries the offsets of
//! its left and right children). Any image whose first physical root entry is
//! a zero-byte marker file therefore reads back as an empty root: Redump dumps
//! of multi-disc games carry exactly that (a 0-byte `Disc.1` file), so their
//! `default.xex` was never found and they were mistaken for install discs.
//! Worse, `get_max_used_prefix_size()` builds on the same walk, so a truncated
//! tree silently under-reports the used size of the volume and yields a
//! truncated GOD.
//!
//! `xdvdfs` — already used for extraction — walks the tree properly, so every
//! lookup goes through here rather than through `iso2god::iso::IsoReader`.

use anyhow::{Context, Result, anyhow};
use iso2god::executable::{TitleExecutionInfo, xbe::XbeHeader, xex::XexHeader};
use iso2god::god::ContentType;
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;
use xdvdfs::blockdev::{BlockDeviceRead, OffsetWrapper};
use xdvdfs::layout::{DirectoryEntryNode, DirectoryEntryTable, SECTOR_SIZE, VolumeDescriptor};

/// How much of an executable to read before parsing its header. Every field we
/// need sits in the first few KiB (the XEX `ExecutionId` sits before
/// `code_offset`, the XBE certificate before `certificate_addr - base_addr`),
/// so 64 KiB is already a generous margin. Inspection runs on the UI thread
/// for every picked file, so this is deliberately not sized in megabytes.
const EXECUTABLE_PREFIX_SIZE: u32 = 64 << 10;

type Device = OffsetWrapper<BufReader<File>, std::io::Error>;

/// Metadata of the image's executable, in the shape `iso2god` expects when
/// building a GOD container (its own `TitleInfo`, read over a correctly
/// walked directory tree).
pub struct ImageTitleInfo {
    pub content_type: ContentType,
    pub execution_info: TitleExecutionInfo,
}

/// An opened XDVDFS image.
pub struct XdvdImage {
    dev: Device,
    volume: VolumeDescriptor,
}

impl XdvdImage {
    /// Opens `path` and reads its volume descriptor. The XGD layout (and thus
    /// the offset the volume starts at inside the file) is detected by
    /// `xdvdfs` itself.
    pub fn open(path: &Path) -> Result<Self> {
        let file =
            File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut dev = OffsetWrapper::new(BufReader::new(file))
            .map_err(|e| anyhow!("invalid XDVDFS image: {e}"))?;
        let volume = xdvdfs::read::read_volume(&mut dev)
            .map_err(|e| anyhow!("reading XDVDFS volume: {e}"))?;
        Ok(Self { dev, volume })
    }

    /// Offset of the volume inside the file (0 for a plain XISO, the XGD1/2/3
    /// video-partition size for a full disc dump).
    ///
    /// `OffsetWrapper` keeps that offset private but rebases every seek on it,
    /// so seeking to the start of the volume reports it.
    pub fn root_offset(&mut self) -> Result<u64> {
        Ok(self.dev.seek(SeekFrom::Start(0))?)
    }

    /// Looks a path up in the image, `/`-separated and case-insensitive.
    /// Returns `None` when any component is missing, or when a component that
    /// has to be traversed is not a directory.
    pub fn find(&mut self, path: &str) -> Result<Option<DirectoryEntryNode>> {
        let mut table = self.volume.root_table;
        let mut components = path.split('/').filter(|s| !s.is_empty()).peekable();

        while let Some(name) = components.next() {
            let Some(node) = self.find_in(table, name)? else {
                return Ok(None);
            };
            if components.peek().is_none() {
                return Ok(Some(node));
            }
            match node.node.dirent.dirent_table() {
                Some(subdir) => table = subdir,
                None => return Ok(None),
            }
        }
        Ok(None)
    }

    /// Reads at most `max` bytes of an entry's data.
    pub fn read_prefix(&mut self, node: &DirectoryEntryNode, max: u32) -> Result<Vec<u8>> {
        let size = node.node.dirent.data.size().min(max);
        let mut buf = vec![0u8; size as usize];
        if size > 0 {
            let offset = node
                .node
                .dirent
                .data
                .offset::<std::io::Error>(0)
                .map_err(|e| anyhow!("invalid file region: {e}"))?;
            self.dev
                .read(offset, &mut buf)
                .context("reading file data")?;
        }
        Ok(buf)
    }

    /// Size of the used part of the volume: the end of the furthest region any
    /// file or directory table occupies. Everything past it is padding, which
    /// GOD conversion trims away.
    pub fn max_used_prefix_size(&mut self) -> Result<u64> {
        let tree = self
            .volume
            .root_table
            .file_tree(&mut self.dev)
            .map_err(|e| anyhow!("reading file tree: {e}"))?;
        let max = tree
            .iter()
            .map(|(_, node)| region_end(&node.node.dirent.data))
            .chain(std::iter::once(region_end(&self.volume.root_table.region)))
            .max()
            .unwrap_or(0);
        Ok(max)
    }

    /// Reads the image's executable metadata, the way `iso2god`'s
    /// `TitleInfo::from_image` does.
    pub fn title_info(&mut self) -> Result<ImageTitleInfo> {
        if let Some(node) = self.find("/default.xex")? {
            let buf = self.read_prefix(&node, EXECUTABLE_PREFIX_SIZE)?;
            let header = XexHeader::read(std::io::Cursor::new(buf))
                .context("reading default.xex")?;
            let execution_info = header
                .fields
                .execution_info
                .context("no execution info in the default.xex header")?;
            Ok(ImageTitleInfo {
                content_type: ContentType::GamesOnDemand,
                execution_info,
            })
        } else if let Some(node) = self.find("/default.xbe")? {
            let buf = self.read_prefix(&node, EXECUTABLE_PREFIX_SIZE)?;
            let header = XbeHeader::read(std::io::Cursor::new(buf))
                .context("reading default.xbe")?;
            let execution_info = header
                .fields
                .execution_info
                .context("no execution info in the default.xbe header")?;
            Ok(ImageTitleInfo {
                content_type: ContentType::XboxOriginal,
                execution_info,
            })
        } else {
            anyhow::bail!("no default.xex or default.xbe in this image")
        }
    }

    /// Finds one entry by name in a single directory table.
    ///
    /// The whole table is walked instead of using `xdvdfs`'s ordered lookup:
    /// the tree's ordering is only as trustworthy as the tool that built the
    /// image, and a root table is a couple of sectors at most.
    fn find_in(
        &mut self,
        table: DirectoryEntryTable,
        name: &str,
    ) -> Result<Option<DirectoryEntryNode>> {
        if table.is_empty() {
            return Ok(None);
        }
        let entries = table
            .walk_dirent_tree(&mut self.dev)
            .map_err(|e| anyhow!("reading directory table: {e}"))?;
        for entry in entries {
            // A name that does not decode as WINDOWS-1252 cannot be the one we
            // are looking for, and must not fail the whole lookup: a single odd
            // byte in some unrelated file name would otherwise make an image
            // whose `default.xex` is perfectly readable unusable.
            let Ok(entry_name) = entry.name_str::<std::io::Error>() else {
                continue;
            };
            if entry_name.eq_ignore_ascii_case(name) {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }
}

/// End offset of an on-disk region, relative to the start of the volume.
fn region_end(region: &xdvdfs::layout::DiskRegion) -> u64 {
    let sector = region.sector as u64;
    sector * SECTOR_SIZE as u64 + region.size() as u64
}
