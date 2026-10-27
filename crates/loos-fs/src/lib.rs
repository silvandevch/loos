//! Non-btrfs filesystems for LoOS.
//!
//! btrfs is the only filesystem a user can *choose* (see `docs/BTRFS.md`); this crate holds the
//! filesystems the system still has to speak:
//!
//! * [`fat`] – FAT12/16/32 with long names, read/write (the EFI System Partition, boot media).
//! * [`exfat`] – exFAT, read/write (removable media interchange).
//! * [`ntfs`] – NTFS, **read-only** (legacy Windows volumes; never a system filesystem).
//! * [`iso9660`] – ISO 9660 + Joliet + Rock Ridge, read-only (install media).
//! * [`udf`] – UDF 1.02–2.60, read-only (DVD/BD media).
//!
//! Everything works over the [`BlockDev`] trait (byte-addressed), allocates through `alloc` only,
//! contains no `unsafe`, and treats every on-disk field as hostile input.
#![no_std]
#![forbid(unsafe_code)]
// The crate keeps an older-toolchain-friendly style (no `div_ceil`/`is_multiple_of`).
#![allow(clippy::manual_div_ceil, clippy::manual_is_multiple_of, clippy::too_many_arguments)]

extern crate alloc;
#[cfg(any(feature = "std", test))]
extern crate std;

pub mod exfat;
pub mod fat;
pub mod iso9660;
pub mod ntfs;
pub mod udf;
mod util;

use alloc::vec::Vec;

/// Filesystem errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    /// The block device failed or the access was out of range.
    Io,
    /// Not this filesystem (bad signature / geometry).
    NotRecognized,
    /// The structure is inconsistent.
    Corrupt,
    /// Path component does not exist.
    NotFound,
    /// A non-final path component, or the target of a directory operation, is a file.
    NotDir,
    /// A file operation was applied to a directory.
    IsDir,
    /// Name already present.
    Exists,
    /// Directory is not empty.
    NotEmpty,
    /// No free clusters / directory slots.
    NoSpace,
    /// Name is empty, too long or has forbidden characters.
    InvalidName,
    /// Valid on disk but a feature we do not implement (e.g. encrypted NTFS data).
    Unsupported,
    /// The filesystem or the mount is read-only.
    ReadOnly,
}

impl core::fmt::Display for FsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            FsError::Io => "I/O error",
            FsError::NotRecognized => "not recognized",
            FsError::Corrupt => "corrupt filesystem",
            FsError::NotFound => "not found",
            FsError::NotDir => "not a directory",
            FsError::IsDir => "is a directory",
            FsError::Exists => "already exists",
            FsError::NotEmpty => "directory not empty",
            FsError::NoSpace => "no space left",
            FsError::InvalidName => "invalid name",
            FsError::Unsupported => "unsupported feature",
            FsError::ReadOnly => "read-only",
        };
        f.write_str(s)
    }
}

/// `Result` alias used throughout the crate.
pub type Result<T> = core::result::Result<T, FsError>;

/// A byte-addressed block device. Offsets are relative to the start of the device (or partition).
pub trait BlockDev {
    /// Fills `buf` from `offset`; short reads are errors.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()>;
    /// Writes `buf` at `offset`. Read-only devices return [`FsError::ReadOnly`].
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<()>;
    /// Makes completed writes durable.
    fn flush(&mut self) -> Result<()>;
    /// Device size in bytes.
    fn len(&self) -> u64;
    /// True for a zero-length device.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// In-memory device, also usable as a RAM disk.
#[derive(Clone, Debug)]
pub struct MemDev {
    /// Backing bytes.
    pub data: Vec<u8>,
    /// Reject writes when true.
    pub read_only: bool,
}

impl MemDev {
    /// Zero-filled device of `len` bytes.
    pub fn new(len: usize) -> Self {
        MemDev { data: alloc::vec![0; len], read_only: false }
    }
    /// Wraps existing bytes read-only.
    pub fn from_image(data: Vec<u8>) -> Self {
        MemDev { data, read_only: true }
    }
}

impl BlockDev for MemDev {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let start = usize::try_from(offset).map_err(|_| FsError::Io)?;
        let end = start.checked_add(buf.len()).ok_or(FsError::Io)?;
        buf.copy_from_slice(self.data.get(start..end).ok_or(FsError::Io)?);
        Ok(())
    }
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<()> {
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        let start = usize::try_from(offset).map_err(|_| FsError::Io)?;
        let end = start.checked_add(buf.len()).ok_or(FsError::Io)?;
        self.data.get_mut(start..end).ok_or(FsError::Io)?.copy_from_slice(buf);
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
    fn len(&self) -> u64 {
        self.data.len() as u64
    }
}

/// File-backed device for host tests and tools.
#[cfg(any(feature = "std", test))]
pub struct FileDev {
    file: std::fs::File,
    len: u64,
    read_only: bool,
}

#[cfg(any(feature = "std", test))]
impl FileDev {
    /// Opens an image read-only.
    pub fn open_ro(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(FileDev { file, len, read_only: true })
    }
    /// Opens an image read-write.
    pub fn open_rw(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
        let len = file.metadata()?.len();
        Ok(FileDev { file, len, read_only: false })
    }
}

#[cfg(any(feature = "std", test))]
impl BlockDev for FileDev {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        if offset.checked_add(buf.len() as u64).map_or(true, |e| e > self.len) {
            return Err(FsError::Io);
        }
        self.file.seek(SeekFrom::Start(offset)).map_err(|_| FsError::Io)?;
        self.file.read_exact(buf).map_err(|_| FsError::Io)
    }
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        if self.read_only {
            return Err(FsError::ReadOnly);
        }
        if offset.checked_add(buf.len() as u64).map_or(true, |e| e > self.len) {
            return Err(FsError::Io);
        }
        self.file.seek(SeekFrom::Start(offset)).map_err(|_| FsError::Io)?;
        self.file.write_all(buf).map_err(|_| FsError::Io)
    }
    fn flush(&mut self) -> Result<()> {
        use std::io::Write;
        self.file.flush().map_err(|_| FsError::Io)?;
        self.file.sync_all().map_err(|_| FsError::Io)
    }
    fn len(&self) -> u64 {
        self.len
    }
}

/// Directory entry returned by the read-mostly filesystems (ISO 9660, UDF, NTFS, exFAT-style APIs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Name (UTF-8).
    pub name: alloc::string::String,
    /// File, directory or symlink.
    pub kind: FileKind,
    /// Size in bytes (0 for directories).
    pub size: u64,
    /// Last modification, Unix seconds (0 if unknown).
    pub mtime: u64,
    /// POSIX permission bits if the filesystem carries them, else 0.
    pub mode: u32,
    /// Owner id if known.
    pub uid: u32,
    /// Group id if known.
    pub gid: u32,
    /// Symlink target when `kind == Symlink`.
    pub target: Option<alloc::string::String>,
    /// Filesystem-specific identity (extent LBA, MFT reference, ICB location).
    pub id: u64,
}

/// Kind of a directory entry, common to all filesystems.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// Regular file.
    File,
    /// Directory.
    Dir,
    /// Symbolic link / reparse symlink (read-only filesystems only).
    Symlink,
}
