//! Safe, allocation-bounded Debian `ar`/tar data archive handling.
//!
//! This module deliberately does not execute maintainer scripts.  It extracts
//! only regular files, directories, and symlinks from an uncompressed
//! `data.tar` member; gzip/xz/zstd data members are reported as unsupported
//! until the corresponding offline decoder is wired into the package service.

use alloc::string::String;
use alloc::vec::Vec;

/// Archive parsing failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Header, size, alignment, or path is malformed.
    Malformed,
    /// The package uses a compressed data archive not handled by this slice.
    CompressionUnsupported,
    /// The package has no `data.tar` payload.
    MissingData,
}

/// A filesystem operation emitted while unpacking a package.
pub trait Sink {
    /// Create a directory with the package mode.
    fn mkdir(&mut self, path: &str, mode: u32) -> Result<(), ()>;
    /// Write or replace a regular file.
    fn file(&mut self, path: &str, mode: u32, data: &[u8]) -> Result<(), ()>;
    /// Create or replace a symbolic link.
    fn symlink(&mut self, path: &str, target: &str) -> Result<(), ()>;
}

fn field<'a>(h: &'a [u8], start: usize, len: usize) -> Result<&'a [u8], Error> {
    h.get(start..start + len).ok_or(Error::Malformed)
}

fn decimal(b: &[u8]) -> Result<usize, Error> {
    let s = core::str::from_utf8(b).map_err(|_| Error::Malformed)?.trim();
    s.parse().map_err(|_| Error::Malformed)
}

fn octal(b: &[u8]) -> Result<u32, Error> {
    let s = core::str::from_utf8(b).map_err(|_| Error::Malformed)?.trim_matches(|c| c == '\0' || c == ' ');
    u32::from_str_radix(s, 8).map_err(|_| Error::Malformed)
}

fn text(b: &[u8]) -> Result<String, Error> {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    let s = core::str::from_utf8(&b[..end]).map_err(|_| Error::Malformed)?.trim_end_matches('/');
    if s.is_empty() || s.starts_with('/') || s.split('/').any(|c| c.is_empty() || c == "." || c == ".." || c.bytes().any(|b| b == 0)) {
        return Err(Error::Malformed);
    }
    Ok(s.into())
}

fn data_member(deb: &[u8]) -> Result<&[u8], Error> {
    if deb.get(..8) != Some(b"!<arch>\n") {
        return Err(Error::Malformed);
    }
    let mut off = 8;
    while off < deb.len() {
        let h = deb.get(off..off + 60).ok_or(Error::Malformed)?;
        if &h[58..60] != b"`\n" {
            return Err(Error::Malformed);
        }
        let size = decimal(field(h, 48, 10)?)?;
        let body = off.checked_add(60).ok_or(Error::Malformed)?;
        let end = body.checked_add(size).ok_or(Error::Malformed)?;
        let payload = deb.get(body..end).ok_or(Error::Malformed)?;
        let name = core::str::from_utf8(field(h, 0, 16)?).map_err(|_| Error::Malformed)?.trim();
        if name == "data.tar" {
            return Ok(payload);
        }
        if name == "data.tar.gz" || name == "data.tar.xz" || name == "data.tar.zst" || name == "data.tar.zstd" {
            return Err(Error::CompressionUnsupported);
        }
        off = end.checked_add(size & 1).ok_or(Error::Malformed)?;
    }
    Err(Error::MissingData)
}

/// Extracts the uncompressed `data.tar` payload into a filesystem sink.
pub fn unpack(deb: &[u8], sink: &mut dyn Sink) -> Result<usize, Error> {
    let tar = data_member(deb)?;
    let mut off = 0;
    let mut count = 0;
    while off + 512 <= tar.len() {
        let h = &tar[off..off + 512];
        if h.iter().all(|&b| b == 0) {
            return Ok(count);
        }
        if field(h, 257, 5)? != b"ustar" {
            return Err(Error::Malformed);
        }
        let path = text(field(h, 0, 100)?)?;
        let mode = octal(field(h, 100, 8)?)? & 0o7777;
        let size = octal(field(h, 124, 12)?)? as usize;
        let body = off.checked_add(512).ok_or(Error::Malformed)?;
        let end = body.checked_add(size).ok_or(Error::Malformed)?;
        let data = tar.get(body..end).ok_or(Error::Malformed)?;
        match h[156] {
            b'0' | 0 => sink.file(&path, mode, data).map_err(|_| Error::Malformed)?,
            b'5' => sink.mkdir(&path, mode).map_err(|_| Error::Malformed)?,
            b'2' => {
                let target = core::str::from_utf8(field(h, 157, 100)?).map_err(|_| Error::Malformed)?.trim_end_matches('\0');
                if target.is_empty() || target.starts_with('/') || target.split('/').any(|c| c == "..") {
                    return Err(Error::Malformed);
                }
                sink.symlink(&path, target).map_err(|_| Error::Malformed)?;
            }
            _ => {}
        }
        count += 1;
        off = body.checked_add(size.div_ceil(512) * 512).ok_or(Error::Malformed)?;
    }
    Err(Error::Malformed)
}

/// Returns the uncompressed `data.tar` member for package-service decoders.
pub fn data_tar(deb: &[u8]) -> Result<&[u8], Error> {
    data_member(deb)
}
