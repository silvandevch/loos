//! Byte-addressed backing devices.
use alloc::vec::Vec;

/// Device errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevError {
    /// Out of range or the device failed.
    Io,
    /// Write to a read-only device.
    ReadOnly,
}

/// A byte-addressed device (one btrfs member device).
pub trait Dev {
    /// Fills `buf` from `offset`.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), DevError>;
    /// Writes `buf` at `offset`.
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<(), DevError>;
    /// Makes writes durable.
    fn flush(&mut self) -> Result<(), DevError>;
    /// Size in bytes.
    fn len(&self) -> u64;
    /// True for a zero-length device.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// RAM-backed device.
#[derive(Clone, Debug)]
pub struct MemDev {
    /// Contents.
    pub data: Vec<u8>,
    /// Reject writes.
    pub read_only: bool,
}

impl MemDev {
    /// Zero-filled device.
    pub fn new(len: usize) -> Self {
        MemDev { data: alloc::vec![0; len], read_only: false }
    }
    /// Read-only device over an image.
    pub fn from_image(data: Vec<u8>) -> Self {
        MemDev { data, read_only: true }
    }
}

impl Dev for MemDev {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), DevError> {
        let s = usize::try_from(offset).map_err(|_| DevError::Io)?;
        let e = s.checked_add(buf.len()).ok_or(DevError::Io)?;
        buf.copy_from_slice(self.data.get(s..e).ok_or(DevError::Io)?);
        Ok(())
    }
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<(), DevError> {
        if self.read_only {
            return Err(DevError::ReadOnly);
        }
        let s = usize::try_from(offset).map_err(|_| DevError::Io)?;
        let e = s.checked_add(buf.len()).ok_or(DevError::Io)?;
        self.data.get_mut(s..e).ok_or(DevError::Io)?.copy_from_slice(buf);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), DevError> {
        Ok(())
    }
    fn len(&self) -> u64 {
        self.data.len() as u64
    }
}

/// File-backed device (host tools and tests).
#[cfg(any(feature = "std", test))]
pub struct FileDev {
    file: std::fs::File,
    len: u64,
    read_only: bool,
}

#[cfg(any(feature = "std", test))]
impl FileDev {
    /// Opens read-only.
    pub fn open_ro(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(FileDev { file, len, read_only: true })
    }
    /// Opens read-write.
    pub fn open_rw(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
        let len = file.metadata()?.len();
        Ok(FileDev { file, len, read_only: false })
    }
}

#[cfg(any(feature = "std", test))]
impl Dev for FileDev {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), DevError> {
        use std::io::{Read, Seek, SeekFrom};
        if offset.checked_add(buf.len() as u64).map_or(true, |e| e > self.len) {
            return Err(DevError::Io);
        }
        self.file.seek(SeekFrom::Start(offset)).map_err(|_| DevError::Io)?;
        self.file.read_exact(buf).map_err(|_| DevError::Io)
    }
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<(), DevError> {
        use std::io::{Seek, SeekFrom, Write};
        if self.read_only {
            return Err(DevError::ReadOnly);
        }
        if offset.checked_add(buf.len() as u64).map_or(true, |e| e > self.len) {
            return Err(DevError::Io);
        }
        self.file.seek(SeekFrom::Start(offset)).map_err(|_| DevError::Io)?;
        self.file.write_all(buf).map_err(|_| DevError::Io)
    }
    fn flush(&mut self) -> Result<(), DevError> {
        use std::io::Write;
        self.file.flush().map_err(|_| DevError::Io)?;
        self.file.sync_all().map_err(|_| DevError::Io)
    }
    fn len(&self) -> u64 {
        self.len
    }
}
