//! Platform boundary. Everything that needs an OS-specific syscall lives in
//! `linux.rs` or `windows.rs`; the rest of the app is portable and testable.

use crate::model::Disk;
use std::fs::File;
use std::io::{Read, Seek, Write};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{0}")]
    Message(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct RawTarget {
    file: File,
}

impl RawTarget {
    pub fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), PlatformError> {
        self.file.seek(std::io::SeekFrom::Start(offset))?;
        self.file.write_all(data)?;
        Ok(())
    }
    pub fn sync(&mut self) -> Result<(), PlatformError> {
        self.file.sync_all()?;
        Ok(())
    }
    pub fn seek(&mut self, offset: u64) -> Result<(), PlatformError> {
        self.file.seek(std::io::SeekFrom::Start(offset))?;
        Ok(())
    }
}

impl Write for RawTarget {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl Seek for RawTarget {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl Read for RawTarget {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

#[cfg(target_os = "linux")]
pub use linux::{dismount_check, elevate_self, open_target, reboot, scan_disks, stage_uefi, system_info};
#[cfg(target_os = "windows")]
pub use windows::{dismount_check, elevate_self, open_target, reboot, scan_disks, stage_uefi, system_info};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_NAME: &str = "LoOS Switch";

pub fn is_elevated() -> bool {
    #[cfg(target_os = "linux")]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(target_os = "windows")]
    {
        windows::is_admin()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else if v >= 10.0 {
        format!("{v:.1} {}", UNITS[u])
    } else {
        format!("{v:.2} {}", UNITS[u])
    }
}

pub fn utc_now_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Looks for the mountpoint of the EFI System Partition from a disk list.
pub fn find_esp_mount(disks: &[Disk]) -> Option<std::path::PathBuf> {
    for d in disks {
        for p in &d.partitions {
            if p.is_esp {
                if let Some(m) = p.mountpoints.first() {
                    if !m.is_empty() && !m.starts_with('[') {
                        return Some(std::path::PathBuf::from(m));
                    }
                }
            }
        }
    }
    None
}

