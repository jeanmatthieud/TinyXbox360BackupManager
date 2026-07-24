// SPDX-License-Identifier: GPL-3.0-only
//! Probe whether the Aurora FTP server supports partial reads, which the
//! DLC-name feature would rely on (read only the first ~0x1800 bytes of a
//! monolithic STFS package instead of downloading the whole thing).
//!
//! Each scenario runs on its OWN fresh connection so a desync in one does
//! not poison the next diagnostic.
//!
//! Usage:
//!   cargo run -p txbm-core --example ftp_partial_read_check -- 192.168.1.67:21

use std::io::Read;
use std::str::FromStr;
use std::time::Instant;
use suppaftp::list::File as ListFile;
use suppaftp::types::FileType;
use suppaftp::FtpStream;

const HEADER_BYTES: usize = 0x1800; // covers STFS name fields

fn main() -> anyhow::Result<()> {
    let hostport = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "192.168.1.67:21".to_string());

    // Locate a big file once.
    let (parts, file_name, size) = {
        let mut ftp = connect(&hostport)?;
        let found = find_large_file(&mut ftp)?;
        let _ = ftp.quit();
        found.ok_or_else(|| anyhow::anyhow!("no large file found under Content"))?
    };
    println!(
        "probe target: {}/{file_name}  ({size} bytes)\n",
        parts.join("/")
    );

    scenario_abor(&hostport, &parts, &file_name)?;
    scenario_finalize(&hostport, &parts, &file_name)?;
    scenario_rest(&hostport, &parts, &file_name, size)?;
    scenario_reconnect_per_read(&hostport, &parts, &file_name)?;

    println!("\ndone.");
    Ok(())
}

/// A: read first N bytes, then ABOR. Check the session survives.
fn scenario_abor(host: &str, parts: &[String], file: &str) -> anyhow::Result<()> {
    println!("=== A: partial RETR + ABOR ===");
    let mut ftp = connect(host)?;
    cwd_parts(&mut ftp, parts)?;
    let mut head = vec![0u8; HEADER_BYTES];
    let t = Instant::now();
    let mut stream = ftp.retr_as_stream(file)?;
    read_capped(&mut stream, &mut head)?;
    println!("  read {} bytes in {:?}, first4={:02X?}", head.len(), t.elapsed(), &head[..4]);
    match ftp.abort(stream) {
        Ok(()) => println!("  ABOR ok"),
        Err(e) => println!("  ABOR err: {e}"),
    }
    report_alive(&mut ftp);
    let _ = ftp.quit();
    Ok(())
}

/// B: read first N bytes, then finalize_retr_stream (just close data conn,
/// no ABOR). Check the session survives.
fn scenario_finalize(host: &str, parts: &[String], file: &str) -> anyhow::Result<()> {
    println!("\n=== B: partial RETR + finalize (close data, no ABOR) ===");
    let mut ftp = connect(host)?;
    cwd_parts(&mut ftp, parts)?;
    let mut head = vec![0u8; HEADER_BYTES];
    let mut stream = ftp.retr_as_stream(file)?;
    read_capped(&mut stream, &mut head)?;
    match ftp.finalize_retr_stream(stream) {
        Ok(()) => println!("  finalize ok"),
        Err(e) => println!("  finalize err: {e}"),
    }
    report_alive(&mut ftp);
    let _ = ftp.quit();
    Ok(())
}

/// C: REST <offset> on a clean connection.
fn scenario_rest(host: &str, parts: &[String], file: &str, size: u64) -> anyhow::Result<()> {
    println!("\n=== C: REST <offset> ===");
    let mut ftp = connect(host)?;
    cwd_parts(&mut ftp, parts)?;
    let offset = (size / 2).min(1_000_000) as usize;
    match ftp.resume_transfer(offset) {
        Ok(()) => {
            println!("  REST {offset} accepted (350)");
            let mut buf = vec![0u8; 64];
            let mut stream = ftp.retr_as_stream(file)?;
            read_capped(&mut stream, &mut buf)?;
            println!("  bytes@{offset}: {:02X?}", &buf[..16]);
            let _ = ftp.abort(stream);
        }
        Err(e) => println!("  REST rejected: {e}"),
    }
    let _ = ftp.quit();
    Ok(())
}

/// D: the pragmatic fallback — one fresh connection per header read, no
/// abort at all: read N bytes then immediately QUIT.
fn scenario_reconnect_per_read(host: &str, parts: &[String], file: &str) -> anyhow::Result<()> {
    println!("\n=== D: fresh connection per read (read N then QUIT) ===");
    let t = Instant::now();
    let mut ftp = connect(host)?;
    cwd_parts(&mut ftp, parts)?;
    let mut head = vec![0u8; HEADER_BYTES];
    let mut stream = ftp.retr_as_stream(file)?;
    read_capped(&mut stream, &mut head)?;
    println!("  read {} bytes, first4={:02X?}", head.len(), &head[..4]);
    // Just quit without finalize/abort; the OS closes the data socket.
    let _ = ftp.quit();
    println!("  full connect+read+quit cycle: {:?}", t.elapsed());
    Ok(())
}

fn report_alive(ftp: &mut FtpStream) {
    match ftp.noop() {
        Ok(()) => match ftp.pwd() {
            Ok(p) => println!("  session ALIVE (pwd={p})"),
            Err(e) => println!("  NOOP ok but pwd err: {e}"),
        },
        Err(e) => println!("  session BROKEN: {e}"),
    }
}

fn connect(hostport: &str) -> anyhow::Result<FtpStream> {
    let user = std::env::var("TXBM_FTP_USER").unwrap_or_else(|_| "xboxftp".into());
    let pass = std::env::var("TXBM_FTP_PASSWORD").unwrap_or_else(|_| "xboxftp".into());
    let mut ftp = FtpStream::connect(hostport)?;
    ftp.login(&user, &pass)?;
    ftp.transfer_type(FileType::Binary)?;
    Ok(ftp)
}

fn read_capped(r: &mut impl Read, buf: &mut [u8]) -> anyhow::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = r.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(())
}

fn cwd_parts(ftp: &mut FtpStream, parts: &[String]) -> anyhow::Result<()> {
    ftp.cwd("/")?;
    for p in parts {
        ftp.cwd(p)?;
    }
    Ok(())
}

fn find_large_file(
    ftp: &mut FtpStream,
) -> anyhow::Result<Option<(Vec<String>, String, u64)>> {
    let base = vec![
        "Hdd1".to_string(),
        "Content".to_string(),
        "0000000000000000".to_string(),
    ];
    for title in list_dirs_at(ftp, &base)? {
        let title_path = with(&base, &title);
        for ctype in list_dirs_at(ftp, &title_path)? {
            let ctype_path = with(&title_path, &ctype);
            for data in list_dirs_at(ftp, &ctype_path)?
                .into_iter()
                .filter(|n| n.ends_with(".data"))
            {
                let data_path = with(&ctype_path, &data);
                let biggest = list_at(ftp, &data_path)?
                    .into_iter()
                    .filter(|e| !e.is_directory())
                    .max_by_key(|e| e.size());
                if let Some(frag) = biggest {
                    return Ok(Some((data_path, frag.name().to_string(), frag.size() as u64)));
                }
            }
        }
    }
    Ok(None)
}

fn with(parts: &[String], child: &str) -> Vec<String> {
    let mut v = parts.to_vec();
    v.push(child.to_string());
    v
}

fn list_at(ftp: &mut FtpStream, parts: &[String]) -> anyhow::Result<Vec<ListFile>> {
    cwd_parts(ftp, parts)?;
    let mut out = Vec::new();
    for line in ftp.list(None)? {
        if let Ok(f) = ListFile::from_str(&line) {
            out.push(f);
        }
    }
    Ok(out)
}

fn list_dirs_at(ftp: &mut FtpStream, parts: &[String]) -> anyhow::Result<Vec<String>> {
    Ok(list_at(ftp, parts)?
        .into_iter()
        .filter(|e| e.is_directory())
        .map(|e| e.name().to_string())
        .collect())
}
