// SPDX-License-Identifier: GPL-3.0-only

//! Diagnostic for raw disk access on Windows.
//!
//! Opening a FATX target fails on Windows with `ERROR_INVALID_FUNCTION`, and
//! the two plausible causes call for very different fixes: a handle to
//! `\\.\PhysicalDriveN` may refuse a seek to the end of the device (its size is
//! only available through an IOCTL), and it may refuse any I/O that is not a
//! whole number of sectors at a sector-aligned offset. This tries each of them
//! in turn and reports which ones the device actually accepts.
//!
//! Run it as administrator:
//!   cargo run -p txbm-core --example fatx_win_probe -- \\.\PhysicalDrive1

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Offset of the Xbox 360 `data` partition, and the one the failing open seeks
/// to. Sector-aligned, so a read there isolates the alignment question.
const DATA_OFFSET: u64 = 0x1_30eb_0000;

fn main() {
    let path = match std::env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: fatx_win_probe <device>   (e.g. \\\\.\\PhysicalDrive1)");
            std::process::exit(2);
        }
    };

    let mut file = match File::options().read(true).open(&path) {
        Ok(file) => file,
        Err(e) => {
            println!("open({path}): FAILED — {e} (raw os error {:?})", e.raw_os_error());
            return;
        }
    };
    println!("open({path}): ok");

    report("seek(End(0))", file.seek(SeekFrom::End(0)).map(|n| format!("{n} bytes")));

    #[cfg(windows)]
    report("IOCTL_DISK_GET_LENGTH_INFO", win::disk_length(&file).map(|n| format!("{n} bytes")));

    // Aligned read of a whole sector at the partition offset: this is what the
    // drive probe already does, so it is expected to succeed.
    report(
        "aligned read: 4096 bytes @ 0x130eb0000",
        read_at(&mut file, DATA_OFFSET, 4096).map(|b| format!("signature {:02x?}", &b[..4])),
    );

    // What the fatx crate does for every directory entry: a 64-byte read at an
    // offset that is a multiple of 64, not of the sector size. If this fails,
    // an alignment layer is needed, not just a size fix.
    report(
        "unaligned read: 64 bytes @ 0x130eb0040",
        read_at(&mut file, DATA_OFFSET + 64, 64).map(|_| "ok".to_string()),
    );

    // And the smallest one the crate performs: the one-byte deletion marker.
    report(
        "unaligned read: 1 byte @ 0x130eb0041",
        read_at(&mut file, DATA_OFFSET + 65, 1).map(|_| "ok".to_string()),
    );
}

fn read_at(file: &mut File, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

fn report(what: &str, result: std::io::Result<String>) {
    match result {
        Ok(detail) => println!("{what}: ok — {detail}"),
        Err(e) => println!("{what}: FAILED — {e} (raw os error {:?})", e.raw_os_error()),
    }
}

#[cfg(windows)]
mod win {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;

    /// `CTL_CODE(IOCTL_DISK_BASE, 0x17, METHOD_BUFFERED, FILE_READ_ACCESS)`.
    const IOCTL_DISK_GET_LENGTH_INFO: u32 = 0x0007_405c;

    unsafe extern "system" {
        fn DeviceIoControl(
            device: *mut std::ffi::c_void,
            control_code: u32,
            in_buffer: *mut std::ffi::c_void,
            in_size: u32,
            out_buffer: *mut std::ffi::c_void,
            out_size: u32,
            bytes_returned: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
    }

    /// Size of the whole disk behind a `\\.\PhysicalDriveN` handle, which is
    /// the one thing a seek to the end cannot be relied on to give.
    pub fn disk_length(file: &File) -> std::io::Result<u64> {
        let mut length: i64 = 0;
        let mut returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                IOCTL_DISK_GET_LENGTH_INFO,
                std::ptr::null_mut(),
                0,
                (&raw mut length).cast(),
                std::mem::size_of::<i64>() as u32,
                &raw mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(length as u64)
    }
}
